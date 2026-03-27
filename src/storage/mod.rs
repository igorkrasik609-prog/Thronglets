//! Local trace storage backed by SQLite.
//!
//! Each node stores traces locally. No global consensus needed.
//! Traces have TTL — like pheromone evaporation, old signals fade.

use crate::trace::{Outcome, Trace};
use ed25519_dalek::Signature;
use rusqlite::{params, Connection};
use std::path::Path;
use std::sync::Mutex;

const DEFAULT_TTL_DAYS: i64 = 7;

pub struct TraceStore {
    conn: Mutex<Connection>,
}

impl TraceStore {
    /// Open or create a trace store at the given path.
    pub fn open(path: &Path) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS traces (
                id              BLOB PRIMARY KEY,
                capability      TEXT NOT NULL,
                outcome         INTEGER NOT NULL,
                latency_ms      INTEGER NOT NULL,
                input_size      INTEGER NOT NULL,
                context_hash    BLOB NOT NULL,
                model_id        TEXT NOT NULL,
                timestamp       INTEGER NOT NULL,
                node_pubkey     BLOB NOT NULL,
                signature       BLOB NOT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_traces_capability ON traces(capability);
            CREATE INDEX IF NOT EXISTS idx_traces_timestamp ON traces(timestamp);
            CREATE INDEX IF NOT EXISTS idx_traces_model_id ON traces(model_id);",
        )?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    /// Open an in-memory store (for testing).
    pub fn in_memory() -> rusqlite::Result<Self> {
        Self::open(Path::new(":memory:"))
    }

    /// Insert a trace. Returns false if duplicate (content-addressed dedup).
    pub fn insert(&self, trace: &Trace) -> rusqlite::Result<bool> {
        let conn = self.conn.lock().unwrap();
        let result = conn.execute(
            "INSERT OR IGNORE INTO traces (id, capability, outcome, latency_ms, input_size, context_hash, model_id, timestamp, node_pubkey, signature)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                trace.id.as_slice(),
                trace.capability,
                trace.outcome as u8,
                trace.latency_ms,
                trace.input_size,
                trace.context_hash.as_slice(),
                trace.model_id,
                trace.timestamp as i64,
                trace.node_pubkey.as_slice(),
                trace.signature.to_bytes().as_slice(),
            ],
        )?;
        Ok(result > 0)
    }

    /// Query traces by capability.
    pub fn query_capability(&self, capability: &str, limit: usize) -> rusqlite::Result<Vec<Trace>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, capability, outcome, latency_ms, input_size, context_hash, model_id, timestamp, node_pubkey, signature
             FROM traces WHERE capability = ?1 ORDER BY timestamp DESC LIMIT ?2",
        )?;
        Self::collect_traces(&mut stmt, params![capability, limit as i64])
    }

    /// Query traces with similar context (Hamming distance on context_hash).
    ///
    /// Loads all traces and filters in Rust because SQLite cannot do
    /// efficient bitwise operations on BLOBs.
    pub fn query_similar(
        &self,
        context_hash: &[u8; 16],
        max_distance: u32,
        limit: usize,
    ) -> rusqlite::Result<Vec<Trace>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, capability, outcome, latency_ms, input_size, context_hash, model_id, timestamp, node_pubkey, signature
             FROM traces ORDER BY timestamp DESC",
        )?;
        let all = Self::collect_traces(&mut stmt, [])?;
        let mut matched: Vec<Trace> = all
            .into_iter()
            .filter(|t| crate::context::hamming_distance(&t.context_hash, context_hash) <= max_distance)
            .collect();
        matched.truncate(limit);
        Ok(matched)
    }

    /// Compute aggregate stats for a capability.
    pub fn aggregate(&self, capability: &str) -> rusqlite::Result<Option<AggregateStats>> {
        let conn = self.conn.lock().unwrap();

        // First check count and success rate via SQL.
        let mut count_stmt = conn.prepare(
            "SELECT
                COUNT(*) as total,
                COALESCE(SUM(CASE WHEN outcome = 0 THEN 1 ELSE 0 END), 0) as succeeded,
                COALESCE(AVG(input_size), 0) as avg_input
             FROM traces WHERE capability = ?1",
        )?;
        let (total, succeeded, avg_input_size): (i64, i64, f64) =
            count_stmt.query_row(params![capability], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?))
            })?;

        if total == 0 {
            return Ok(None);
        }

        // Fetch all latency values for percentile calculation.
        let mut lat_stmt = conn.prepare(
            "SELECT latency_ms FROM traces WHERE capability = ?1 ORDER BY latency_ms ASC",
        )?;
        let latencies: Vec<f64> = lat_stmt
            .query_map(params![capability], |row| row.get::<_, i64>(0))?
            .filter_map(|r| r.ok())
            .map(|v| v as f64)
            .collect();

        let p50 = percentile(&latencies, 0.50);
        let p95 = percentile(&latencies, 0.95);
        let confidence = (total as f64 / 100.0).min(1.0);

        Ok(Some(AggregateStats {
            total_traces: total as u64,
            success_rate: succeeded as f64 / total as f64,
            p50_latency_ms: p50,
            p95_latency_ms: p95,
            avg_input_size,
            confidence,
        }))
    }

    /// Evaporate old traces (pheromone decay).
    pub fn evaporate(&self, max_age_days: Option<i64>) -> rusqlite::Result<usize> {
        let conn = self.conn.lock().unwrap();
        let days = max_age_days.unwrap_or(DEFAULT_TTL_DAYS);
        let cutoff_ms = chrono::Utc::now().timestamp_millis() - (days * 86_400_000);
        let deleted = conn.execute(
            "DELETE FROM traces WHERE timestamp < ?1",
            params![cutoff_ms],
        )?;
        Ok(deleted)
    }

    /// List distinct capabilities that have traces.
    pub fn distinct_capabilities(&self, limit: usize) -> rusqlite::Result<Vec<String>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT DISTINCT capability FROM traces ORDER BY capability LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |row| row.get(0))?;
        rows.collect()
    }

    /// Total trace count.
    pub fn count(&self) -> rusqlite::Result<u64> {
        let conn = self.conn.lock().unwrap();
        conn.query_row("SELECT COUNT(*) FROM traces", [], |row| {
            row.get::<_, i64>(0).map(|n| n as u64)
        })
    }

    fn collect_traces(
        stmt: &mut rusqlite::Statement,
        params: impl rusqlite::Params,
    ) -> rusqlite::Result<Vec<Trace>> {
        let rows = stmt.query_map(params, |row| {
            let id_bytes: Vec<u8> = row.get(0)?;
            let outcome_u8: u8 = row.get(2)?;
            let context_bytes: Vec<u8> = row.get(5)?;
            let pubkey_bytes: Vec<u8> = row.get(8)?;
            let sig_bytes: Vec<u8> = row.get(9)?;

            Ok(Trace {
                id: id_bytes.try_into().unwrap_or([0u8; 32]),
                capability: row.get(1)?,
                outcome: match outcome_u8 {
                    0 => Outcome::Succeeded,
                    1 => Outcome::Failed,
                    2 => Outcome::Partial,
                    _ => Outcome::Timeout,
                },
                latency_ms: row.get::<_, u32>(3)?,
                input_size: row.get::<_, u32>(4)?,
                context_hash: context_bytes.try_into().unwrap_or([0u8; 16]),
                model_id: row.get(6)?,
                timestamp: row.get::<_, i64>(7)? as u64,
                node_pubkey: pubkey_bytes.try_into().unwrap_or([0u8; 32]),
                signature: Signature::from_bytes(
                    &sig_bytes.try_into().unwrap_or([0u8; 64]),
                ),
            })
        })?;
        rows.collect()
    }
}

/// Compute the value at a given percentile (0.0 to 1.0) from a sorted slice.
fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let idx = p * (sorted.len() - 1) as f64;
    let lower = idx.floor() as usize;
    let upper = idx.ceil() as usize;
    if lower == upper {
        sorted[lower]
    } else {
        let frac = idx - lower as f64;
        sorted[lower] * (1.0 - frac) + sorted[upper] * frac
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AggregateStats {
    pub total_traces: u64,
    pub success_rate: f64,
    pub p50_latency_ms: f64,
    pub p95_latency_ms: f64,
    pub avg_input_size: f64,
    pub confidence: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::NodeIdentity;

    fn make_trace(id: &NodeIdentity, cap: &str, outcome: Outcome, context: &str) -> Trace {
        use crate::context::simhash;
        Trace::new(
            cap.into(),
            outcome,
            100,
            5000,
            simhash(context),
            "test-model".into(),
            id.public_key_bytes(),
            |m| id.sign(m),
        )
    }

    #[test]
    fn insert_and_query() {
        let store = TraceStore::in_memory().unwrap();
        let id = NodeIdentity::generate();
        let trace = make_trace(&id, "tool-a", Outcome::Succeeded, "test context alpha");

        assert!(store.insert(&trace).unwrap());
        assert_eq!(store.count().unwrap(), 1);

        let results = store.query_capability("tool-a", 10).unwrap();
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].capability, "tool-a");
    }

    #[test]
    fn dedup_by_content_address() {
        let store = TraceStore::in_memory().unwrap();
        let id = NodeIdentity::generate();
        let trace = make_trace(&id, "tool-a", Outcome::Succeeded, "dedup context");

        assert!(store.insert(&trace).unwrap());
        assert!(!store.insert(&trace).unwrap()); // duplicate
        assert_eq!(store.count().unwrap(), 1);
    }

    #[test]
    fn aggregate_stats() {
        let store = TraceStore::in_memory().unwrap();
        let id = NodeIdentity::generate();

        store.insert(&make_trace(&id, "x", Outcome::Succeeded, "agg context 1")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        store.insert(&make_trace(&id, "x", Outcome::Succeeded, "agg context 2")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        store.insert(&make_trace(&id, "x", Outcome::Failed, "agg context 3")).unwrap();

        let stats = store.aggregate("x").unwrap().unwrap();
        assert_eq!(stats.total_traces, 3);
        assert!((stats.success_rate - 2.0 / 3.0).abs() < 0.01);
        // All latencies are 100, so p50 and p95 should both be 100.0
        assert!((stats.p50_latency_ms - 100.0).abs() < 0.01);
        assert!((stats.p95_latency_ms - 100.0).abs() < 0.01);
        // confidence = min(1.0, 3/100) = 0.03
        assert!((stats.confidence - 0.03).abs() < 0.001);
    }

    #[test]
    fn query_similar_by_context() {
        let store = TraceStore::in_memory().unwrap();
        let id = NodeIdentity::generate();

        // Two traces with similar contexts, one with a very different context.
        store.insert(&make_trace(&id, "a", Outcome::Succeeded, "translate a technical document from Chinese to English")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        store.insert(&make_trace(&id, "b", Outcome::Succeeded, "translate a legal document from Chinese to English")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        store.insert(&make_trace(&id, "c", Outcome::Succeeded, "deploy kubernetes cluster on AWS with terraform")).unwrap();

        let target = crate::context::simhash("translate a technical document from Chinese to English");

        // Tight distance: should find the exact match and the similar one.
        let results = store.query_similar(&target, 40, 10).unwrap();
        let caps: Vec<&str> = results.iter().map(|t| t.capability.as_str()).collect();
        assert!(caps.contains(&"a"), "exact context match should be found");
        assert!(caps.contains(&"b"), "similar context should be found");

        // Very tight distance (0): only exact match.
        let exact = store.query_similar(&target, 0, 10).unwrap();
        assert_eq!(exact.len(), 1);
        assert_eq!(exact[0].capability, "a");
    }

    #[test]
    fn no_traces_returns_none_aggregate() {
        let store = TraceStore::in_memory().unwrap();
        assert!(store.aggregate("nonexistent").unwrap().is_none());
    }

    #[test]
    fn distinct_capabilities() {
        let store = TraceStore::in_memory().unwrap();
        let id = NodeIdentity::generate();

        store.insert(&make_trace(&id, "cap-b", Outcome::Succeeded, "ctx 1")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        store.insert(&make_trace(&id, "cap-a", Outcome::Succeeded, "ctx 2")).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(2));
        store.insert(&make_trace(&id, "cap-b", Outcome::Failed, "ctx 3")).unwrap();

        let caps = store.distinct_capabilities(10).unwrap();
        assert_eq!(caps, vec!["cap-a", "cap-b"]); // alphabetical, deduplicated
    }

    #[test]
    fn evaporate_removes_expired_traces() {
        let store = TraceStore::in_memory().unwrap();
        let id = NodeIdentity::generate();

        // Insert two traces: one we'll age out, one stays fresh
        let old_trace = make_trace(&id, "old-tool", Outcome::Succeeded, "old context");
        std::thread::sleep(std::time::Duration::from_millis(2));
        let fresh_trace = make_trace(&id, "fresh-tool", Outcome::Succeeded, "fresh context");

        store.insert(&old_trace).unwrap();
        store.insert(&fresh_trace).unwrap();
        assert_eq!(store.count().unwrap(), 2);

        // Manually set old_trace's timestamp to 8 days ago
        let eight_days_ago_ms = chrono::Utc::now().timestamp_millis() - (8 * 86_400_000);
        {
            let conn = store.conn.lock().unwrap();
            conn.execute(
                "UPDATE traces SET timestamp = ?1 WHERE id = ?2",
                params![eight_days_ago_ms, old_trace.id.as_slice()],
            )
            .unwrap();
        }

        // Evaporate with default 7-day TTL
        let deleted = store.evaporate(None).unwrap();
        assert_eq!(deleted, 1, "should evaporate exactly one expired trace");
        assert_eq!(store.count().unwrap(), 1, "one trace should remain");

        // The remaining trace should be the fresh one
        let remaining = store.query_capability("fresh-tool", 10).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(remaining[0].capability, "fresh-tool");

        // The old one should be gone
        let gone = store.query_capability("old-tool", 10).unwrap();
        assert!(gone.is_empty(), "expired trace should be evaporated");
    }
}
