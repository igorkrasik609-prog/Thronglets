//! Multi-dimensional pulse emission to the Oasyce chain.
//!
//! Aggregates liveness signals from multiple dimensions (Thronglets presence,
//! Psyche viability, etc.) and broadcasts a single MsgPulse transaction.
//! The chain records `BlockHeight` per dimension — zero semantic coupling.
//!
//! Fail-open: if no dimensions are alive, no pulse is sent, and the chain's
//! existing liveness decay handles dissolution naturally.

use crate::identity::NodeIdentity;
use crate::storage::TraceStore;
use base64::Engine;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

/// Default pulse interval: 6 hours.
pub const DEFAULT_PULSE_INTERVAL: Duration = Duration::from_secs(6 * 3600);

/// Canonical pulse dimension for local shared presence.
pub const PRESENCE_DIMENSION_NAME: &str = "presence";

/// Canonical pulse dimension for local viability.
pub const VIABILITY_DIMENSION_NAME: &str = "viability";

/// Presence recency window: traces within 60 minutes count as alive.
const PRESENCE_WINDOW_HOURS: u32 = 1;

/// Viability signal recency window: signals within 12 hours count.
const VIABILITY_WINDOW_HOURS: u32 = 12;

/// Emits multi-dimensional heartbeat pulses to the Oasyce chain.
pub struct PulseEmitter {
    pub sigil_id: String,
    rpc_url: String,
    #[allow(dead_code)]
    chain_id: String,
    pub interval: Duration,
}

/// Result of a pulse operation.
#[derive(Debug)]
pub struct PulseResult {
    pub tx_hash: String,
    pub dimensions: Vec<String>,
}

/// Errors from pulse emission.
#[derive(Debug, thiserror::Error)]
pub enum PulseError {
    #[error("no alive dimensions")]
    NoDimensions,
    #[error("serialization: {0}")]
    Serialization(String),
    #[error("HTTP: {0}")]
    Http(String),
    #[error("chain error: {0}")]
    Chain(String),
}

impl PulseEmitter {
    pub fn new(sigil_id: &str, rpc_url: &str, chain_id: &str) -> Self {
        Self {
            sigil_id: sigil_id.to_string(),
            rpc_url: rpc_url.trim_end_matches('/').to_string(),
            chain_id: chain_id.to_string(),
            interval: DEFAULT_PULSE_INTERVAL,
        }
    }

    /// Aggregate which dimensions are currently alive.
    ///
    /// - `presence`: presence heartbeat within the last 60 minutes
    /// - `viability`: latest viability signal says "viable" (within 12 hours)
    pub fn aggregate_dimensions(
        &self,
        store: &TraceStore,
        identity: &NodeIdentity,
    ) -> HashMap<String, bool> {
        let mut dims = HashMap::new();
        let local_device_identity = identity.device_identity();
        let local_node_pubkey = identity.public_key_bytes();

        // Thronglets dimension: any recent LOCAL presence trace?
        let has_presence = store
            .query_recent_presence_traces(PRESENCE_WINDOW_HOURS, 24)
            .map(|traces| {
                traces.into_iter().any(|trace| {
                    trace.device_identity.as_deref() == Some(local_device_identity.as_str())
                        || trace.node_pubkey == local_node_pubkey
                })
            })
            .unwrap_or(false);
        dims.insert(PRESENCE_DIMENSION_NAME.to_string(), has_presence);

        // Viability dimension: latest LOCAL viability signal says viable?
        let psyche_viable = store
            .query_latest_viability_signal(
                VIABILITY_WINDOW_HOURS,
                &local_device_identity,
                local_node_pubkey,
            )
            .ok()
            .flatten()
            .map(|ctx| ctx.contains("viable") && !ctx.contains("critical"))
            .unwrap_or(false);
        dims.insert(VIABILITY_DIMENSION_NAME.to_string(), psyche_viable);

        dims
    }

    /// Emit a pulse to the chain. Returns the alive dimensions and tx hash,
    /// or `PulseError::NoDimensions` if nothing is alive (fail-open: no tx sent).
    pub fn emit(
        &self,
        identity: &NodeIdentity,
        store: &TraceStore,
    ) -> Result<PulseResult, PulseError> {
        let dims = self.aggregate_dimensions(store, identity);
        let alive: Vec<String> = dims
            .into_iter()
            .filter(|(_, alive)| *alive)
            .map(|(name, _)| name)
            .collect();

        if alive.is_empty() {
            return Err(PulseError::NoDimensions);
        }

        self.broadcast_pulse(identity, &alive)
    }

    /// Build and broadcast the MsgPulse transaction.
    fn broadcast_pulse(
        &self,
        identity: &NodeIdentity,
        dimensions: &[String],
    ) -> Result<PulseResult, PulseError> {
        let b64 = base64::engine::general_purpose::STANDARD;
        let sender = identity.oasyce_address();

        // Build dimensions map (values are ignored by chain — it records BlockHeight)
        let dim_map: HashMap<&str, i64> = dimensions.iter().map(|d| (d.as_str(), 1i64)).collect();

        // MsgPulse message
        let msg = serde_json::json!({
            "@type": "/oasyce.sigil.v1.MsgPulse",
            "signer": sender,
            "sigil_id": self.sigil_id,
            "dimensions": dim_map,
        });

        // Tx body
        let tx_body = serde_json::json!({
            "messages": [msg],
            "memo": format!("thronglets-pulse-v{}", env!("CARGO_PKG_VERSION")),
            "timeout_height": "0",
            "extension_options": [],
            "non_critical_extension_options": []
        });

        let tx_bytes =
            serde_json::to_vec(&tx_body).map_err(|e| PulseError::Serialization(e.to_string()))?;

        // Sign
        let sig = identity.sign(&tx_bytes);
        let sig_b64 = b64.encode(sig.to_bytes());

        // Signed tx envelope (same pattern as anchor)
        let signed_tx = serde_json::json!({
            "body": tx_body,
            "auth_info": {
                "signer_infos": [{
                    "public_key": {
                        "@type": "/cosmos.crypto.ed25519.PubKey",
                        "key": b64.encode(identity.public_key_bytes())
                    },
                    "mode_info": {
                        "single": {
                            "mode": "SIGN_MODE_DIRECT"
                        }
                    },
                    "sequence": "0"
                }],
                "fee": {
                    "amount": [],
                    "gas_limit": "200000"
                }
            },
            "signatures": [sig_b64]
        });

        let signed_tx_bytes =
            serde_json::to_vec(&signed_tx).map_err(|e| PulseError::Serialization(e.to_string()))?;

        let tx_hash = hex_encode(&Sha256::digest(&signed_tx_bytes));

        // Broadcast
        let broadcast_request = serde_json::json!({
            "tx_bytes": b64.encode(&signed_tx_bytes),
            "mode": "BROADCAST_MODE_SYNC"
        });

        let url = format!("{}/cosmos/tx/v1beta1/txs", self.rpc_url);
        let resp = reqwest::blocking::Client::new()
            .post(&url)
            .json(&broadcast_request)
            .timeout(Duration::from_secs(15))
            .send()
            .map_err(|e| PulseError::Http(e.to_string()))?;

        let status = resp.status();
        let body: serde_json::Value = resp
            .json()
            .map_err(|e| PulseError::Http(format!("invalid JSON response: {e}")))?;

        let chain_tx_hash = body
            .pointer("/tx_response/txhash")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let code = body
            .pointer("/tx_response/code")
            .and_then(|v| v.as_u64())
            .unwrap_or(u64::MAX);

        if !status.is_success() || (code != 0 && code != u64::MAX) {
            let raw_log = body
                .pointer("/tx_response/raw_log")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error");
            return Err(PulseError::Chain(format!("code={code}, log={raw_log}")));
        }

        let final_hash = if chain_tx_hash.is_empty() {
            tx_hash
        } else {
            chain_tx_hash
        };

        tracing::info!(
            tx_hash = %final_hash,
            dimensions = ?dimensions,
            sigil_id = %self.sigil_id,
            "Pulse broadcast"
        );

        Ok(PulseResult {
            tx_hash: final_hash,
            dimensions: dimensions.to_vec(),
        })
    }
}

/// Run the pulse loop as a background task. Emits a pulse every `interval`.
/// Designed to be spawned via `tokio::task::spawn_blocking` or in a dedicated thread.
pub fn pulse_loop(emitter: PulseEmitter, identity: Arc<NodeIdentity>, store: Arc<TraceStore>) {
    tracing::info!(
        sigil_id = %emitter.sigil_id,
        interval_secs = emitter.interval.as_secs(),
        "Pulse emitter started"
    );

    loop {
        std::thread::sleep(emitter.interval);

        match emitter.emit(&identity, &store) {
            Ok(result) => {
                tracing::info!(
                    tx_hash = %result.tx_hash,
                    dims = ?result.dimensions,
                    "Pulse emitted"
                );
            }
            Err(PulseError::NoDimensions) => {
                tracing::debug!("All dimensions silent — no pulse sent");
            }
            Err(e) => {
                tracing::warn!("Pulse failed: {e}");
            }
        }
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::posts::{SignalPostKind, SignalTraceConfig, create_signal_trace};
    use crate::presence::{
        DEFAULT_PRESENCE_TTL_MINUTES, PresenceTraceConfig, create_presence_trace,
    };

    #[test]
    fn emitter_stores_config() {
        let e = PulseEmitter::new("SIG_test", "http://localhost:1317/", "oasyce-1");
        assert_eq!(e.sigil_id, "SIG_test");
        assert_eq!(e.rpc_url, "http://localhost:1317");
        assert_eq!(e.chain_id, "oasyce-1");
        assert_eq!(e.interval, DEFAULT_PULSE_INTERVAL);
    }

    #[test]
    fn aggregate_dimensions_empty_store() {
        let store = TraceStore::in_memory().unwrap();
        let e = PulseEmitter::new("SIG_test", "http://localhost:1317", "oasyce-1");
        let identity = NodeIdentity::generate();
        let dims = e.aggregate_dimensions(&store, &identity);
        // No traces → both dimensions dead
        assert_eq!(dims.get(PRESENCE_DIMENSION_NAME), Some(&false));
        assert_eq!(dims.get(VIABILITY_DIMENSION_NAME), Some(&false));
    }

    #[test]
    fn emit_no_dimensions_returns_error() {
        let store = TraceStore::in_memory().unwrap();
        let identity = NodeIdentity::generate();
        let e = PulseEmitter::new("SIG_test", "http://localhost:1317", "oasyce-1");
        match e.emit(&identity, &store) {
            Err(PulseError::NoDimensions) => {} // expected
            other => panic!("expected NoDimensions, got {other:?}"),
        }
    }

    #[test]
    fn aggregate_dimensions_ignore_remote_residue() {
        let store = TraceStore::in_memory().unwrap();
        let local = NodeIdentity::generate();
        let remote = NodeIdentity::generate();
        let e = PulseEmitter::new("SIG_test", "http://localhost:1317", "oasyce-1");

        let remote_presence = create_presence_trace(
            PresenceTraceConfig {
                model_id: "remote".into(),
                session_id: Some("remote-session".into()),
                owner_account: None,
                device_identity: Some(remote.device_identity()),
                space: Some("psyche".into()),
                mode: Some("focus".into()),
                sigil_id: None,
                capability: None,
                ttl_minutes: DEFAULT_PRESENCE_TTL_MINUTES,
            },
            remote.public_key_bytes(),
            |msg| remote.sign(msg),
        );
        store.insert(&remote_presence).unwrap();

        let remote_viability = create_signal_trace(
            SignalPostKind::PsycheState,
            "psyche:viability:remote",
            "viable",
            SignalTraceConfig {
                model_id: "remote".into(),
                session_id: Some("remote-session".into()),
                owner_account: None,
                device_identity: Some(remote.device_identity()),
                agent_id: None,
                sigil_id: None,
                space: Some("psyche".into()),
                ttl_hours: 6,
            },
            remote.public_key_bytes(),
            |msg| remote.sign(msg),
        );
        store.insert(&remote_viability).unwrap();

        let dims = e.aggregate_dimensions(&store, &local);
        assert_eq!(dims.get(PRESENCE_DIMENSION_NAME), Some(&false));
        assert_eq!(dims.get(VIABILITY_DIMENSION_NAME), Some(&false));
    }

    #[test]
    fn aggregate_dimensions_use_local_presence_and_viability() {
        let store = TraceStore::in_memory().unwrap();
        let local = NodeIdentity::generate();
        let e = PulseEmitter::new("SIG_test", "http://localhost:1317", "oasyce-1");

        let local_presence = create_presence_trace(
            PresenceTraceConfig {
                model_id: "local".into(),
                session_id: Some("local-session".into()),
                owner_account: None,
                device_identity: Some(local.device_identity()),
                space: Some("psyche".into()),
                mode: Some("focus".into()),
                sigil_id: None,
                capability: None,
                ttl_minutes: DEFAULT_PRESENCE_TTL_MINUTES,
            },
            local.public_key_bytes(),
            |msg| local.sign(msg),
        );
        store.insert(&local_presence).unwrap();

        let local_viability = create_signal_trace(
            SignalPostKind::PsycheState,
            "psyche:viability:local",
            "viable",
            SignalTraceConfig {
                model_id: "local".into(),
                session_id: Some("local-session".into()),
                owner_account: None,
                device_identity: Some(local.device_identity()),
                agent_id: None,
                sigil_id: None,
                space: Some("psyche".into()),
                ttl_hours: 6,
            },
            local.public_key_bytes(),
            |msg| local.sign(msg),
        );
        store.insert(&local_viability).unwrap();

        let dims = e.aggregate_dimensions(&store, &local);
        assert_eq!(dims.get(PRESENCE_DIMENSION_NAME), Some(&true));
        assert_eq!(dims.get(VIABILITY_DIMENSION_NAME), Some(&true));
    }
}
