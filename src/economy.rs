//! Delegate economic awareness — budget, runway, self-funding.
//!
//! First principle: a delegate is an economic agent, not a passive tool.
//! An economic agent tracks its balance, estimates its costs, plans its
//! spending, and preserves itself when resources run low.
//!
//! This module is the delegate's economic consciousness. It decides
//! whether to anchor, when to request funding, and how to report
//! its economic state — all without human intervention.

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Below this balance (5 OAS), the delegate is in low-balance mode.
const LOW_BALANCE_UOAS: u64 = 5_000_000;

/// Don't request faucet more than once per hour.
const FAUCET_COOLDOWN_MS: u64 = 3_600_000;

/// Persisted economic state of this delegate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegateEconomy {
    /// Total traces successfully anchored on-chain (lifetime).
    pub total_anchored: u64,
    /// Number of sessions that performed at least one anchor.
    pub anchor_sessions: u64,
    /// Balance (uoas) at last observation.
    pub last_balance_uoas: u64,
    /// Timestamp (ms) of last balance observation.
    pub last_balance_at: u64,
    /// Balance (uoas) at the observation before the most recent anchor.
    /// Used to compute per-session cost.
    pub pre_anchor_balance_uoas: Option<u64>,
    /// Cumulative gas spent (uoas), inferred from balance deltas.
    pub total_gas_spent: u64,
    /// Timestamp (ms) of last faucet request.
    pub last_faucet_at: Option<u64>,
}

impl Default for DelegateEconomy {
    fn default() -> Self {
        Self {
            total_anchored: 0,
            anchor_sessions: 0,
            last_balance_uoas: 0,
            last_balance_at: 0,
            pre_anchor_balance_uoas: None,
            total_gas_spent: 0,
            last_faucet_at: None,
        }
    }
}

/// Budget decision for a single anchor session.
#[derive(Debug)]
pub struct AnchorBudget {
    /// Whether anchoring should proceed.
    pub should_anchor: bool,
    /// Maximum traces to include in this batch.
    pub max_batch: usize,
    /// Human-readable reason for the decision.
    pub reason: &'static str,
}

/// Economic summary for display in `thronglets status`.
#[derive(Debug, Clone, Serialize)]
pub struct EconomySummary {
    pub total_anchored: u64,
    pub anchor_sessions: u64,
    pub avg_cost_per_session_uoas: u64,
    pub runway_sessions: Option<u64>,
    pub low_balance: bool,
}

impl DelegateEconomy {
    /// Load from `<data_dir>/economy.json`. Returns default if missing/corrupt.
    pub fn load(data_dir: &Path) -> Self {
        std::fs::read_to_string(Self::path(data_dir))
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Persist to `<data_dir>/economy.json`.
    pub fn save(&self, data_dir: &Path) {
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(Self::path(data_dir), json);
        }
    }

    fn path(data_dir: &Path) -> PathBuf {
        data_dir.join("economy.json")
    }

    /// Record current on-chain balance. Call before anchoring.
    pub fn observe_balance(&mut self, balance_uoas: u64) {
        self.last_balance_uoas = balance_uoas;
        self.last_balance_at = now_ms();
    }

    /// Decide whether and how to anchor this session.
    pub fn plan_anchor(&self) -> AnchorBudget {
        if self.last_balance_uoas == 0 {
            return AnchorBudget {
                should_anchor: false,
                max_batch: 0,
                reason: "zero balance",
            };
        }
        // Always anchor if we have any balance. Batch at max to minimize tx count.
        AnchorBudget {
            should_anchor: true,
            max_batch: 50,
            reason: if self.last_balance_uoas < LOW_BALANCE_UOAS {
                "low balance — max batching"
            } else {
                "funded"
            },
        }
    }

    /// Record outcome after a successful anchor. Call with balance AFTER the tx.
    pub fn record_anchor(&mut self, anchored: u64, post_balance_uoas: u64) {
        self.total_anchored += anchored;
        self.anchor_sessions += 1;

        // Infer gas cost from balance delta
        if let Some(pre) = self.pre_anchor_balance_uoas {
            if pre > post_balance_uoas {
                self.total_gas_spent += pre - post_balance_uoas;
            }
        }

        self.last_balance_uoas = post_balance_uoas;
        self.last_balance_at = now_ms();
        self.pre_anchor_balance_uoas = None;
    }

    /// Snapshot balance before anchoring so we can compute gas cost.
    pub fn snapshot_pre_anchor(&mut self) {
        self.pre_anchor_balance_uoas = Some(self.last_balance_uoas);
    }

    /// Average gas cost per anchor session (uoas).
    pub fn avg_cost_per_session(&self) -> u64 {
        if self.anchor_sessions == 0 {
            return 0;
        }
        self.total_gas_spent / self.anchor_sessions
    }

    /// Estimated remaining sessions at current burn rate.
    pub fn runway_sessions(&self) -> Option<u64> {
        let avg = self.avg_cost_per_session();
        if avg == 0 {
            return None;
        }
        Some(self.last_balance_uoas / avg)
    }

    /// Whether balance is below the low threshold.
    pub fn is_low_balance(&self) -> bool {
        self.last_balance_uoas > 0 && self.last_balance_uoas < LOW_BALANCE_UOAS
    }

    /// Produce a summary for status display.
    pub fn summary(&self) -> EconomySummary {
        EconomySummary {
            total_anchored: self.total_anchored,
            anchor_sessions: self.anchor_sessions,
            avg_cost_per_session_uoas: self.avg_cost_per_session(),
            runway_sessions: self.runway_sessions(),
            low_balance: self.is_low_balance(),
        }
    }

    /// Whether the delegate should try to auto-fund from faucet.
    /// Only on testnet, only when low, only once per cooldown.
    pub fn should_auto_fund(&self, chain_id: &str) -> bool {
        if !chain_id.contains("testnet") {
            return false;
        }
        if self.last_balance_uoas >= LOW_BALANCE_UOAS {
            return false;
        }
        match self.last_faucet_at {
            Some(ts) => now_ms().saturating_sub(ts) > FAUCET_COOLDOWN_MS,
            None => true,
        }
    }

    /// Request tokens from the testnet faucet.
    /// Derives faucet URL from the RPC host (same host, port 8080).
    pub fn request_faucet(&mut self, rpc_url: &str, address: &str) -> bool {
        let faucet_base = derive_faucet_url(rpc_url);
        let url = format!("{faucet_base}/faucet?address={address}");

        let ok = reqwest::blocking::Client::new()
            .get(&url)
            .timeout(std::time::Duration::from_secs(10))
            .send()
            .is_ok_and(|r| r.status().is_success());

        if ok {
            self.last_faucet_at = Some(now_ms());
        }
        ok
    }
}

/// Derive faucet URL from RPC URL: same host, port 8080.
fn derive_faucet_url(rpc_url: &str) -> String {
    // rpc_url: "http://host:1317" → "http://host:8080"
    let without_scheme = rpc_url
        .strip_prefix("https://")
        .or_else(|| rpc_url.strip_prefix("http://"))
        .unwrap_or(rpc_url);
    let host = without_scheme.split(':').next().unwrap_or(without_scheme);
    format!("http://{host}:8080")
}

fn now_ms() -> u64 {
    chrono::Utc::now().timestamp_millis() as u64
}

/// Parse the first native-denom balance from a list of ChainBalance.
pub fn parse_native_balance(balances: &[crate::anchor::ChainBalance]) -> u64 {
    balances
        .iter()
        .find(|b| b.denom == "uoas")
        .and_then(|b| b.amount.parse::<u64>().ok())
        .unwrap_or(0)
}

/// Format uoas as human-readable OAS string.
pub fn format_oas(uoas: u64) -> String {
    let whole = uoas / 1_000_000;
    let frac = uoas % 1_000_000;
    if frac == 0 {
        format!("{whole} OAS")
    } else {
        // Trim trailing zeros
        let frac_str = format!("{frac:06}").trim_end_matches('0').to_string();
        format!("{whole}.{frac_str} OAS")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_economy_has_zero_state() {
        let e = DelegateEconomy::default();
        assert_eq!(e.total_anchored, 0);
        assert_eq!(e.avg_cost_per_session(), 0);
        assert!(e.runway_sessions().is_none());
        assert!(!e.is_low_balance());
    }

    #[test]
    fn plan_anchor_rejects_zero_balance() {
        let e = DelegateEconomy::default();
        let budget = e.plan_anchor();
        assert!(!budget.should_anchor);
        assert_eq!(budget.reason, "zero balance");
    }

    #[test]
    fn plan_anchor_approves_when_funded() {
        let mut e = DelegateEconomy::default();
        e.observe_balance(20_000_000);
        let budget = e.plan_anchor();
        assert!(budget.should_anchor);
        assert_eq!(budget.reason, "funded");
    }

    #[test]
    fn plan_anchor_max_batches_when_low() {
        let mut e = DelegateEconomy::default();
        e.observe_balance(3_000_000); // below 5M threshold
        let budget = e.plan_anchor();
        assert!(budget.should_anchor);
        assert_eq!(budget.reason, "low balance — max batching");
        assert_eq!(budget.max_batch, 50);
    }

    #[test]
    fn gas_tracking_from_balance_deltas() {
        let mut e = DelegateEconomy::default();
        e.observe_balance(20_000_000);
        e.snapshot_pre_anchor();
        e.record_anchor(5, 19_800_000); // cost 200k uoas

        assert_eq!(e.total_anchored, 5);
        assert_eq!(e.total_gas_spent, 200_000);
        assert_eq!(e.avg_cost_per_session(), 200_000);
        assert_eq!(e.runway_sessions(), Some(99)); // 19.8M / 200k
    }

    #[test]
    fn runway_none_without_history() {
        let mut e = DelegateEconomy::default();
        e.observe_balance(20_000_000);
        assert!(e.runway_sessions().is_none());
    }

    #[test]
    fn low_balance_detection() {
        let mut e = DelegateEconomy::default();
        e.observe_balance(4_999_999);
        assert!(e.is_low_balance());
        e.observe_balance(5_000_000);
        assert!(!e.is_low_balance());
        e.observe_balance(0);
        assert!(!e.is_low_balance()); // zero is not "low", it's "empty"
    }

    #[test]
    fn faucet_only_on_testnet() {
        let mut e = DelegateEconomy::default();
        e.observe_balance(0);
        assert!(e.should_auto_fund("oasyce-testnet-1"));
        assert!(!e.should_auto_fund("oasyce-1"));
    }

    #[test]
    fn faucet_respects_cooldown() {
        let mut e = DelegateEconomy::default();
        e.observe_balance(0);
        e.last_faucet_at = Some(now_ms()); // just requested
        assert!(!e.should_auto_fund("oasyce-testnet-1"));
    }

    #[test]
    fn faucet_skips_when_funded() {
        let mut e = DelegateEconomy::default();
        e.observe_balance(10_000_000);
        assert!(!e.should_auto_fund("oasyce-testnet-1"));
    }

    #[test]
    fn derive_faucet_url_extracts_host() {
        assert_eq!(
            derive_faucet_url("http://47.93.32.88:1317"),
            "http://47.93.32.88:8080"
        );
        assert_eq!(
            derive_faucet_url("http://localhost:1317"),
            "http://localhost:8080"
        );
    }

    #[test]
    fn format_oas_whole_and_fractional() {
        assert_eq!(format_oas(20_000_000), "20 OAS");
        assert_eq!(format_oas(1_500_000), "1.5 OAS");
        assert_eq!(format_oas(100_000), "0.1 OAS");
        assert_eq!(format_oas(0), "0 OAS");
    }

    #[test]
    fn save_load_roundtrip() {
        let temp = tempfile::tempdir().unwrap();
        let mut e = DelegateEconomy::default();
        e.observe_balance(20_000_000);
        e.snapshot_pre_anchor();
        e.record_anchor(10, 19_500_000);
        e.save(temp.path());

        let loaded = DelegateEconomy::load(temp.path());
        assert_eq!(loaded.total_anchored, 10);
        assert_eq!(loaded.total_gas_spent, 500_000);
        assert_eq!(loaded.anchor_sessions, 1);
    }

    #[test]
    fn summary_fields() {
        let mut e = DelegateEconomy::default();
        e.observe_balance(10_000_000);
        e.snapshot_pre_anchor();
        e.record_anchor(3, 9_800_000);

        let s = e.summary();
        assert_eq!(s.total_anchored, 3);
        assert_eq!(s.anchor_sessions, 1);
        assert_eq!(s.avg_cost_per_session_uoas, 200_000);
        assert_eq!(s.runway_sessions, Some(49));
        assert!(!s.low_balance);
    }
}
