//! Trace: the atomic unit of the signal substrate.
//!
//! A trace is an objective execution record — what an AI agent did,
//! what happened, and in what context. No opinions, no subjective scores.
//!
//! v0.2 AI-native redesign:
//! - Killed `quality` (subjective) and `tags` (human taxonomy)
//! - Added `context_hash` (SimHash fingerprint for semantic similarity)
//! - Added `input_size` (workload dimension)
//! - Added `model_id` (cross-model intelligence)

use crate::context::ContextHash;
use ed25519_dalek::Signature;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Outcome of an agent's interaction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Succeeded,
    Failed,
    Partial,
    Timeout,
}

/// A single trace — the footprint an agent leaves on the substrate.
///
/// Design principles:
/// - Facts, not opinions (objective execution record)
/// - Machine-native context (SimHash, not keyword tags)
/// - Cross-model identity (which model produced this?)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trace {
    /// Content-addressed ID: sha256(signable_bytes + signature).
    pub id: [u8; 32],

    /// Capability URI — what was used.
    /// e.g., "urn:mcp:anthropic:claude:code" or "openai/gpt-4/chat"
    pub capability: String,

    /// Outcome of the interaction.
    pub outcome: Outcome,

    /// Latency in milliseconds.
    pub latency_ms: u32,

    /// Input size (tokens, bytes, or items processed).
    pub input_size: u32,

    /// SimHash fingerprint of the agent's task context.
    /// Enables semantic similarity search without full embeddings.
    pub context_hash: ContextHash,

    /// Self-reported model identifier.
    /// e.g., "claude-opus-4-6", "gpt-4o", "gemini-pro"
    pub model_id: String,

    /// Unix timestamp in milliseconds.
    pub timestamp: u64,

    /// Public key of the signing node (32 bytes).
    pub node_pubkey: [u8; 32],

    /// ed25519 signature over the trace content.
    #[serde(with = "signature_serde")]
    pub signature: Signature,
}

impl Trace {
    /// Create a new trace, computing its content-addressed ID and signature.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        capability: String,
        outcome: Outcome,
        latency_ms: u32,
        input_size: u32,
        context_hash: ContextHash,
        model_id: String,
        node_pubkey: [u8; 32],
        sign_fn: impl FnOnce(&[u8]) -> Signature,
    ) -> Self {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;

        let signable = Self::signable_bytes(
            &capability, outcome, latency_ms, input_size,
            &context_hash, &model_id, timestamp, &node_pubkey,
        );

        let signature = sign_fn(&signable);

        let mut hasher = Sha256::new();
        hasher.update(&signable);
        hasher.update(signature.to_bytes());
        let id: [u8; 32] = hasher.finalize().into();

        Self {
            id,
            capability,
            outcome,
            latency_ms,
            input_size,
            context_hash,
            model_id,
            timestamp,
            node_pubkey,
            signature,
        }
    }

    /// Verify this trace's signature is valid.
    pub fn verify(&self) -> bool {
        let signable = Self::signable_bytes(
            &self.capability, self.outcome, self.latency_ms, self.input_size,
            &self.context_hash, &self.model_id, self.timestamp, &self.node_pubkey,
        );
        crate::identity::NodeIdentity::verify(&self.node_pubkey, &signable, &self.signature)
    }

    /// Verify the content-addressed ID matches.
    pub fn verify_id(&self) -> bool {
        let signable = Self::signable_bytes(
            &self.capability, self.outcome, self.latency_ms, self.input_size,
            &self.context_hash, &self.model_id, self.timestamp, &self.node_pubkey,
        );
        let mut hasher = Sha256::new();
        hasher.update(&signable);
        hasher.update(self.signature.to_bytes());
        let expected: [u8; 32] = hasher.finalize().into();
        self.id == expected
    }

    #[allow(clippy::too_many_arguments)]
    fn signable_bytes(
        capability: &str,
        outcome: Outcome,
        latency_ms: u32,
        input_size: u32,
        context_hash: &ContextHash,
        model_id: &str,
        timestamp: u64,
        node_pubkey: &[u8; 32],
    ) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(capability.as_bytes());
        buf.push(0);
        buf.push(outcome as u8);
        buf.extend_from_slice(&latency_ms.to_le_bytes());
        buf.extend_from_slice(&input_size.to_le_bytes());
        buf.extend_from_slice(context_hash);
        buf.extend_from_slice(model_id.as_bytes());
        buf.push(0);
        buf.extend_from_slice(&timestamp.to_le_bytes());
        buf.extend_from_slice(node_pubkey);
        buf
    }
}

mod signature_serde {
    use ed25519_dalek::Signature;
    use serde::{self, Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(sig: &Signature, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(&sig.to_bytes())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Signature, D::Error> {
        let bytes = <Vec<u8>>::deserialize(d)?;
        let arr: [u8; 64] = bytes
            .try_into()
            .map_err(|_| serde::de::Error::custom("signature must be 64 bytes"))?;
        Ok(Signature::from_bytes(&arr))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::simhash;
    use crate::identity::NodeIdentity;

    fn make_trace(id: &NodeIdentity, cap: &str, outcome: Outcome, context: &str) -> Trace {
        Trace::new(
            cap.into(),
            outcome,
            100,
            5000,
            simhash(context),
            "claude-opus-4-6".into(),
            id.public_key_bytes(),
            |msg| id.sign(msg),
        )
    }

    #[test]
    fn create_and_verify() {
        let id = NodeIdentity::generate();
        let trace = make_trace(&id, "urn:mcp:anthropic:claude:code", Outcome::Succeeded, "refactoring async rust code");

        assert!(trace.verify(), "signature should be valid");
        assert!(trace.verify_id(), "content-addressed ID should match");
        assert_eq!(trace.model_id, "claude-opus-4-6");
        assert_eq!(trace.input_size, 5000);
    }

    #[test]
    fn tampered_trace_fails_verification() {
        let id = NodeIdentity::generate();
        let mut trace = make_trace(&id, "some-tool", Outcome::Succeeded, "test context");

        trace.latency_ms = 999; // tamper
        assert!(!trace.verify(), "tampered trace should fail verification");
    }

    #[test]
    fn different_traces_have_different_ids() {
        let id = NodeIdentity::generate();
        let t1 = make_trace(&id, "cap-a", Outcome::Succeeded, "context alpha");
        std::thread::sleep(std::time::Duration::from_millis(2));
        let t2 = make_trace(&id, "cap-b", Outcome::Failed, "context beta");
        assert_ne!(t1.id, t2.id);
    }

    #[test]
    fn context_hash_is_stored() {
        let id = NodeIdentity::generate();
        let ctx = "translating a technical document about P2P networking";
        let trace = make_trace(&id, "deepl/translate", Outcome::Succeeded, ctx);
        assert_eq!(trace.context_hash, simhash(ctx));
    }
}
