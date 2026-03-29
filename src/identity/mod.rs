//! Node identity and local device binding.
//!
//! The same ed25519 keypair currently serves as:
//! - Thronglets node identity (sign traces)
//! - `device identity` for Oasyce Identity V1
//!
//! Owner / wallet remains a higher-level root account and is modeled as
//! optional metadata that can be imported or manually bound later.

use ed25519_dalek::{SigningKey, VerifyingKey, Signer, Verifier, Signature};
use rand::rngs::OsRng;
use sha2::{Sha256, Digest};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::fs;

const IDENTITY_BINDING_SCHEMA_VERSION: &str = "thronglets.identity.v1";
const CONNECTION_FILE_SCHEMA_VERSION: &str = "thronglets.connection.v1";

/// A node's identity: ed25519 keypair + derived addresses.
pub struct NodeIdentity {
    signing_key: SigningKey,
    verifying_key: VerifyingKey,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct IdentityBinding {
    pub schema_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_account: Option<String>,
    pub device_identity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding_source: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub joined_from_device: Option<String>,
    pub updated_at: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ConnectionFile {
    pub schema_version: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner_account: Option<String>,
    pub primary_device_identity: String,
    pub exported_at: u64,
}

impl NodeIdentity {
    /// Generate a new random identity.
    pub fn generate() -> Self {
        let signing_key = SigningKey::generate(&mut OsRng);
        let verifying_key = signing_key.verifying_key();
        Self { signing_key, verifying_key }
    }

    /// Load from a key file, or generate and save if it doesn't exist.
    pub fn load_or_generate(path: &Path) -> std::io::Result<Self> {
        if path.exists() {
            let bytes = fs::read(path)?;
            if bytes.len() != 32 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "key file must be exactly 32 bytes",
                ));
            }
            let signing_key = SigningKey::from_bytes(&bytes.try_into().unwrap());
            let verifying_key = signing_key.verifying_key();
            Ok(Self { signing_key, verifying_key })
        } else {
            let identity = Self::generate();
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::write(path, identity.signing_key.to_bytes())?;
            // Restrict key file to owner-only read/write
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
            }
            Ok(identity)
        }
    }

    /// Sign arbitrary bytes.
    pub fn sign(&self, message: &[u8]) -> Signature {
        self.signing_key.sign(message)
    }

    /// Get the public key bytes (32 bytes).
    pub fn public_key_bytes(&self) -> [u8; 32] {
        self.verifying_key.to_bytes()
    }

    /// Get the secret key bytes (32 bytes). Used for libp2p identity conversion.
    pub fn secret_key_bytes(&self) -> [u8; 32] {
        self.signing_key.to_bytes()
    }

    /// Verify a signature against a public key.
    pub fn verify(public_key: &[u8; 32], message: &[u8], signature: &Signature) -> bool {
        let Ok(vk) = VerifyingKey::from_bytes(public_key) else {
            return false;
        };
        vk.verify(message, signature).is_ok()
    }

    /// Derive a Cosmos-compatible bech32 address (oasyce1...).
    /// Uses the same derivation as Cosmos SDK: sha256(pubkey)[..20] -> bech32.
    pub fn oasyce_address(&self) -> String {
        Self::device_identity_from_pubkey(&self.public_key_bytes())
    }

    /// Current Identity V1 device identity.
    pub fn device_identity(&self) -> String {
        self.oasyce_address()
    }

    /// Short hex ID for display (first 8 chars of hex pubkey).
    pub fn short_id(&self) -> String {
        hex::encode(&self.public_key_bytes()[..4])
    }

    pub fn device_identity_from_pubkey(public_key: &[u8; 32]) -> String {
        let hash = Sha256::digest(public_key);
        let addr_bytes = &hash[..20];
        bech32::encode::<bech32::Bech32>(bech32::Hrp::parse("oasyce").unwrap(), addr_bytes)
            .expect("bech32 encoding should never fail")
    }
}

impl IdentityBinding {
    pub fn load_or_create(path: &Path, node_identity: &NodeIdentity) -> std::io::Result<Self> {
        if path.exists() {
            let bytes = fs::read(path)?;
            let binding: Self = serde_json::from_slice(&bytes).map_err(invalid_data)?;
            if binding.device_identity.trim().is_empty() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "device_identity cannot be empty",
                ));
            }
            Ok(binding)
        } else {
            let binding = Self::new(node_identity.device_identity());
            binding.save(path)?;
            Ok(binding)
        }
    }

    pub fn new(device_identity: String) -> Self {
        Self {
            schema_version: IDENTITY_BINDING_SCHEMA_VERSION.to_string(),
            owner_account: None,
            device_identity,
            binding_source: None,
            joined_from_device: None,
            updated_at: now_ms(),
        }
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(invalid_data)?;
        fs::write(path, bytes)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        }
        Ok(())
    }

    pub fn bind_owner_account(mut self, owner_account: String) -> Self {
        self.owner_account = Some(owner_account);
        self.binding_source = Some("manual".into());
        self.joined_from_device = None;
        self.updated_at = now_ms();
        self
    }

    pub fn joined_via_connection(mut self, owner_account: Option<String>, primary_device: String) -> Self {
        self.owner_account = owner_account;
        self.binding_source = Some("connection_file".into());
        self.joined_from_device = Some(primary_device);
        self.updated_at = now_ms();
        self
    }

    pub fn owner_account_or_unbound(&self) -> &str {
        self.owner_account.as_deref().unwrap_or("unbound")
    }
}

impl ConnectionFile {
    pub fn from_binding(binding: &IdentityBinding) -> Self {
        Self {
            schema_version: CONNECTION_FILE_SCHEMA_VERSION.to_string(),
            owner_account: binding.owner_account.clone(),
            primary_device_identity: binding.device_identity.clone(),
            exported_at: now_ms(),
        }
    }

    pub fn load(path: &Path) -> std::io::Result<Self> {
        let bytes = fs::read(path)?;
        let file: Self = serde_json::from_slice(&bytes).map_err(invalid_data)?;
        if file.primary_device_identity.trim().is_empty() {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "primary_device_identity cannot be empty",
            ));
        }
        Ok(file)
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let bytes = serde_json::to_vec_pretty(self).map_err(invalid_data)?;
        fs::write(path, bytes)?;
        Ok(())
    }
}

pub fn identity_binding_path(data_dir: &Path) -> std::path::PathBuf {
    data_dir.join("identity.v1.json")
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn invalid_data(error: impl std::fmt::Display) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string())
}

// We need hex encoding but don't want another dep for just this
mod hex {
    pub fn encode(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn generate_and_sign() {
        let id = NodeIdentity::generate();
        let msg = b"hello thronglets";
        let sig = id.sign(msg);
        assert!(NodeIdentity::verify(&id.public_key_bytes(), msg, &sig));
    }

    #[test]
    fn verify_rejects_wrong_message() {
        let id = NodeIdentity::generate();
        let sig = id.sign(b"correct");
        assert!(!NodeIdentity::verify(&id.public_key_bytes(), b"wrong", &sig));
    }

    #[test]
    fn persistence_round_trip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("node.key");

        let id1 = NodeIdentity::load_or_generate(&path).unwrap();
        let id2 = NodeIdentity::load_or_generate(&path).unwrap();

        assert_eq!(id1.public_key_bytes(), id2.public_key_bytes());
    }

    #[test]
    fn oasyce_address_format() {
        let id = NodeIdentity::generate();
        let addr = id.oasyce_address();
        assert!(addr.starts_with("oasyce1"));
    }

    #[test]
    fn short_id_is_8_chars() {
        let id = NodeIdentity::generate();
        assert_eq!(id.short_id().len(), 8);
    }

    #[test]
    fn identity_binding_round_trip() {
        let dir = TempDir::new().unwrap();
        let node = NodeIdentity::generate();
        let path = identity_binding_path(dir.path());

        let binding = IdentityBinding::load_or_create(&path, &node).unwrap();
        assert_eq!(binding.device_identity, node.device_identity());
        assert_eq!(binding.owner_account, None);

        let rebound = binding
            .clone()
            .bind_owner_account("oasyce1owner".into());
        rebound.save(&path).unwrap();
        let loaded = IdentityBinding::load_or_create(&path, &node).unwrap();
        assert_eq!(loaded.owner_account.as_deref(), Some("oasyce1owner"));
        assert_eq!(loaded.device_identity, node.device_identity());
    }

    #[test]
    fn connection_file_round_trip() {
        let dir = TempDir::new().unwrap();
        let binding = IdentityBinding {
            schema_version: IDENTITY_BINDING_SCHEMA_VERSION.into(),
            owner_account: Some("oasyce1owner".into()),
            device_identity: "oasyce1device".into(),
            binding_source: Some("manual".into()),
            joined_from_device: None,
            updated_at: 123,
        };
        let file = ConnectionFile::from_binding(&binding);
        let path = dir.path().join("device.thronglets.json");
        file.save(&path).unwrap();
        let loaded = ConnectionFile::load(&path).unwrap();
        assert_eq!(loaded.owner_account.as_deref(), Some("oasyce1owner"));
        assert_eq!(loaded.primary_device_identity, "oasyce1device");
    }
}
