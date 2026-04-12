//! Pheromone Field — stigmergic collective memory as a unified graph.
//!
//! Traces don't get stored then queried. Traces *excite* field points.
//! The field's state IS the memory. Query the field = perceive the memory.
//!
//! Two layers of intelligence:
//! 1. **Stigmergy** (ant-like): excite → decay → scan. Passive memory.
//! 2. **Self-organization** (slime-mold-like): diffusion + Hebbian coupling.
//!    The field evolves on its own clock via tick(). No agent mediation needed.
//!
//! Diffusion: intensity spreads to neighboring context buckets. Creates
//! continuous gradients. The field discovers context relationships no agent
//! explicitly established.
//!
//! Hebbian coupling: capabilities that fire together wire together.
//! Co-excited capabilities develop associative bonds. scan() surfaces
//! associated capabilities — the field's own "intuition".
//!
//! Architecture: single unified graph behind one lock.
//! Nodes = (capability, context_bucket) → FieldPoint.
//! Edges = capability-level Hebbian couplings (derived from co-excitation).
//! No separate buffers — co-excitation detected from nodes' last_excited timestamps.

use crate::context::ContextHash;
use crate::storage::context_bucket;
use crate::trace::{Outcome, Trace};
use std::collections::HashMap;
use std::sync::RwLock;

// ── Constants ──────────────────────────────────────────────────

/// Half-life in hours. After this many hours without excitation,
/// a field point's intensity halves.
const HALF_LIFE_HOURS: f64 = 48.0;

/// Decay constant: λ = ln(2) / half_life_ms
const DECAY_LAMBDA: f64 = core::f64::consts::LN_2 / (HALF_LIFE_HOURS * 3_600_000.0);

/// Minimum intensity before a field point is pruned (saves memory).
const PRUNE_THRESHOLD: f64 = 0.01;

/// EMA smoothing factor for valence/latency updates.
/// α = 0.1 means new observation has 10% weight.
const EMA_ALPHA: f64 = 0.1;

/// Intensity multiplier for Sigil-attributed traces.
/// Attributed traces deposit slightly more pheromone — the system
/// naturally rewards identity without mandating it.
const ATTRIBUTION_BOOST: f64 = 1.1;

// ── Dynamics Constants ────────────────────────────────────────

/// Fraction of intensity that diffuses to each neighbor per tick.
/// 0.05 = each of ±1 neighbors gets 5%, source keeps 90%.
/// Conservative: total field intensity is conserved (diffusion is zero-sum).
const DIFFUSION_RATE: f64 = 0.05;

/// Time window (ms) for Hebbian co-excitation detection.
/// Capabilities excited within this window of each other form couplings.
const COUPLING_WINDOW_MS: u64 = 60_000;

/// How much a single co-excitation event strengthens a coupling.
const COUPLING_LEARN_RATE: f64 = 0.2;

/// Coupling half-life. Couplings decay slower than field points —
/// associations persist longer than individual memories.
const COUPLING_HALF_LIFE_HOURS: f64 = 168.0;

/// Coupling decay constant.
const COUPLING_DECAY_LAMBDA: f64 =
    core::f64::consts::LN_2 / (COUPLING_HALF_LIFE_HOURS * 3_600_000.0);

/// Minimum coupling weight before pruning.
const COUPLING_PRUNE_THRESHOLD: f64 = 0.05;

// ── Carrying Capacity ───────────────────────────────────────

/// Total pheromone budget for the field. When the field approaches this
/// capacity, new deposits become progressively more expensive — creating
/// natural selection pressure on information quality.
///
/// Calibrated for M1 Pro workloads. Set to f64::MAX to disable.
const FIELD_CAPACITY: f64 = 10_000.0;

// ── Field Point ────────────────────────────────────────────────

/// A single point in the pheromone field.
/// Represents the collective memory of a (capability, context_region) pair.
#[derive(Debug, Clone)]
pub struct FieldPoint {
    /// Signal strength — decays exponentially without excitation.
    /// Each trace increments this by 1.0 (after applying decay).
    pub intensity: f64,

    /// Success rate as exponential moving average [0.0, 1.0].
    /// Positive = mostly succeeds. Negative valence not used;
    /// instead, low valence = high failure rate.
    pub valence: f64,

    /// Latency in ms as exponential moving average.
    pub latency: f64,

    /// Variance accumulator for valence (Welford's online algorithm).
    /// High variance = unstable capability = worth attention.
    pub variance: f64,

    /// Timestamp (Unix ms) of last excitation.
    pub last_excited: u64,

    /// Total excitation count (never decays — lifetime counter).
    pub total_excitations: u64,

    /// Distinct source count (device_identity or node_pubkey).
    /// For corroboration — multi-source is more trustworthy.
    pub source_count: u32,

    /// Track sources as hash set of first 8 bytes of identity.
    sources: Vec<[u8; 8]>,
}

impl FieldPoint {
    fn new(now_ms: u64) -> Self {
        Self {
            intensity: 0.0,
            valence: 0.5, // neutral prior
            latency: 0.0,
            variance: 0.0,
            last_excited: now_ms,
            total_excitations: 0,
            source_count: 0,
            sources: Vec::new(),
        }
    }

    fn effective_decay_lambda(&self) -> f64 {
        // ln(1)=0, ln(10)≈2.3, ln(100)≈4.6 → factor: 1.0, 1.35, 1.69
        let reinforcement_factor =
            1.0 + (self.total_excitations as f64).ln().max(0.0) * 0.15;
        DECAY_LAMBDA / reinforcement_factor
    }

    /// Apply temporal decay to intensity based on elapsed time.
    /// Well-reinforced points (high total_excitations) decay slower —
    /// up to ~2x half-life at ~800 excitations. This creates persistent
    /// landmarks from collectively reinforced knowledge.
    fn decay(&mut self, now_ms: u64) {
        if now_ms <= self.last_excited {
            return;
        }
        let dt = (now_ms - self.last_excited) as f64;
        self.intensity *= (-self.effective_decay_lambda() * dt).exp();
    }

    /// Excite this field point with a new observation.
    fn excite(
        &mut self,
        outcome: Outcome,
        latency_ms: u64,
        now_ms: u64,
        source_id: [u8; 8],
        deposit: f64,
    ) {
        // First, decay existing intensity to current time
        self.decay(now_ms);

        // Increment intensity (the trace deposits pheromone)
        self.intensity += deposit;
        self.last_excited = now_ms;
        self.total_excitations += 1;

        // Update valence (success rate) via EMA
        let outcome_val = if outcome == Outcome::Succeeded {
            1.0
        } else {
            0.0
        };
        let old_valence = self.valence;
        self.valence = self.valence * (1.0 - EMA_ALPHA) + outcome_val * EMA_ALPHA;

        // Update variance via Welford's online method
        let delta = outcome_val - old_valence;
        let delta2 = outcome_val - self.valence;
        self.variance = self.variance * (1.0 - EMA_ALPHA) + (delta * delta2).abs() * EMA_ALPHA;

        // Update latency via EMA
        if latency_ms > 0 {
            if self.latency == 0.0 {
                self.latency = latency_ms as f64;
            } else {
                self.latency = self.latency * (1.0 - EMA_ALPHA) + latency_ms as f64 * EMA_ALPHA;
            }
        }

        // Track unique sources (keep only first 8 bytes for compactness)
        if !self.sources.contains(&source_id) {
            self.sources.push(source_id);
            self.source_count = self.sources.len() as u32;
        }
    }

    /// Current intensity with decay applied (read-only, doesn't mutate).
    pub fn current_intensity(&self, now_ms: u64) -> f64 {
        if now_ms <= self.last_excited {
            return self.intensity;
        }
        let dt = (now_ms - self.last_excited) as f64;
        self.intensity * (-self.effective_decay_lambda() * dt).exp()
    }

    /// Should this point be pruned?
    pub fn is_dead(&self, now_ms: u64) -> bool {
        self.current_intensity(now_ms) < PRUNE_THRESHOLD
    }
}

// ── Field Key ──────────────────────────────────────────────────

/// Composite key for a field point: (capability, context_bucket).
/// context_bucket is 16-bit, derived from first 2 bytes of SimHash.
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
struct FieldKey {
    capability: String,
    bucket: i64,
}

// ── Graph Edges ──────────────────────────────────────────────

/// Canonical key for a Hebbian edge between two capabilities.
/// cap_a <= cap_b always (avoids duplicates).
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
struct EdgeKey {
    cap_a: String,
    cap_b: String,
}

impl EdgeKey {
    fn new(a: &str, b: &str) -> Self {
        if a <= b {
            Self {
                cap_a: a.to_string(),
                cap_b: b.to_string(),
            }
        } else {
            Self {
                cap_a: b.to_string(),
                cap_b: a.to_string(),
            }
        }
    }

    fn partner(&self, cap: &str) -> Option<&str> {
        if self.cap_a == cap {
            Some(&self.cap_b)
        } else if self.cap_b == cap {
            Some(&self.cap_a)
        } else {
            None
        }
    }
}

/// A Hebbian edge between two capabilities.
/// Strengthens when they co-fire, decays over time.
#[derive(Debug, Clone)]
struct Edge {
    weight: f64,
    last_reinforced: u64,
}

impl Edge {
    fn current_weight(&self, now_ms: u64) -> f64 {
        if now_ms <= self.last_reinforced {
            return self.weight;
        }
        let dt = (now_ms - self.last_reinforced) as f64;
        self.weight * (-COUPLING_DECAY_LAMBDA * dt).exp()
    }

    fn is_dead(&self, now_ms: u64) -> bool {
        self.current_weight(now_ms) < COUPLING_PRUNE_THRESHOLD
    }
}

// ── Result Types ─────────────────────────────────────────────

/// Result of a tick() — the field's autonomous self-evolution step.
#[derive(Debug)]
pub struct TickResult {
    pub diffused: usize,
    pub couplings_reinforced: usize,
    pub couplings_pruned: usize,
    pub points_pruned: usize,
    /// Current field load: total_intensity / FIELD_CAPACITY.
    /// 0.0 = empty, 1.0 = at carrying capacity.
    pub load_factor: f64,
}

impl Default for TickResult {
    fn default() -> Self {
        Self {
            diffused: 0,
            couplings_reinforced: 0,
            couplings_pruned: 0,
            points_pruned: 0,
            load_factor: 0.0,
        }
    }
}

/// Serializable coupling entry for snapshots.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CouplingSnapshotEntry {
    pub cap_a: String,
    pub cap_b: String,
    pub weight: f64,
    pub last_reinforced: u64,
}

/// Result of scanning the field near a context.
#[derive(Debug, Clone)]
pub struct FieldScan {
    pub capability: String,
    pub intensity: f64,
    pub valence: f64,
    pub latency: f64,
    pub variance: f64,
    pub total_excitations: u64,
    pub source_count: u32,
    pub context_similarity: f64,
}

/// Snapshot of the entire field for P2P sync.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FieldSnapshot {
    pub points: Vec<FieldSnapshotEntry>,
    #[serde(default)]
    pub couplings: Vec<CouplingSnapshotEntry>,
    /// Total field intensity at snapshot time. Receivers can use this
    /// to gauge the source field's load factor.
    #[serde(default)]
    pub total_intensity: f64,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FieldSnapshotEntry {
    pub capability: String,
    pub bucket: i64,
    pub intensity: f64,
    pub valence: f64,
    pub latency: f64,
    pub variance: f64,
    pub last_excited: u64,
    pub total_excitations: u64,
    pub source_count: u32,
    #[serde(default)]
    pub source_hashes: Vec<[u8; 8]>,
}

/// Delta for incremental P2P sync.
#[derive(Debug, Clone)]
pub struct FieldDelta {
    pub capability: String,
    pub bucket: i64,
    pub intensity_add: f64,
    pub outcome: Outcome,
    pub latency_ms: u64,
    pub source_id: [u8; 8],
    pub timestamp: u64,
}

/// Semantic-stable effect signals derived from field state.
///
/// These are the field's "hormones" — broadcast signals any external
/// system can consume. The overlay is a projection of field state,
/// not a consumer-specific API.
#[derive(Debug, Clone)]
pub struct FieldOverlay {
    /// How well the field knows this capability in this context [0, 1].
    /// 0 = unknown, 1 = deeply familiar (saturating sigmoid of intensity).
    pub familiarity: f64,

    /// Agreement across observations [0, 1].
    /// High = consistent outcomes. Low = unstable/contradictory.
    pub consensus: f64,

    /// Activity trend [-1, 1].
    /// Positive = recently active (within one half-life).
    /// Negative = decaying (beyond one half-life). Zero = exactly at half-life.
    pub momentum: f64,

    /// Connectedness to other capabilities via Hebbian bonds [0, 1].
    /// High = strongly associated with other capabilities.
    pub coupling: f64,
}

// ── Unified Graph ────────────────────────────────────────────

/// The field's internal state: nodes + edges in a single structure.
/// One lock protects everything. No separate buffers.
struct FieldInner {
    nodes: HashMap<FieldKey, FieldPoint>,
    edges: HashMap<EdgeKey, Edge>,
    /// Running sum of all field point intensities. Maintained incrementally
    /// in excite/prune — never recomputed from scratch. Drives the
    /// carrying capacity mechanism: deposit cost scales with load_factor.
    total_intensity: f64,
}

impl FieldInner {
    fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            edges: HashMap::new(),
            total_intensity: 0.0,
        }
    }

    fn current_total_intensity(&self, now_ms: u64) -> f64 {
        self.nodes
            .values()
            .map(|point| point.current_intensity(now_ms))
            .sum()
    }

    /// Current load factor: total_intensity / FIELD_CAPACITY.
    /// 0.0 = empty field, 1.0 = at capacity, >1.0 = over capacity.
    fn load_factor(&self, now_ms: u64) -> f64 {
        self.current_total_intensity(now_ms) / FIELD_CAPACITY
    }

    /// Diffuse intensity from each field point to neighboring buckets.
    /// Conservative: total intensity is preserved (source loses what neighbors gain).
    fn diffuse(&mut self, now_ms: u64) -> usize {
        // Phase 1: collect diffusion operations (read-only)
        let mut ops: Vec<(FieldKey, f64, f64, f64)> = Vec::new();
        let mut sources: Vec<(FieldKey, f64)> = Vec::new();

        for (key, point) in &self.nodes {
            let intensity = point.current_intensity(now_ms);
            if intensity < PRUNE_THRESHOLD * 10.0 {
                continue;
            }
            let diffuse_amount = intensity * DIFFUSION_RATE;

            for offset in [-1i64, 1] {
                let neighbor_bucket = key.bucket + offset;
                if !(0..=65535).contains(&neighbor_bucket) {
                    continue;
                }
                let neighbor_key = FieldKey {
                    capability: key.capability.clone(),
                    bucket: neighbor_bucket,
                };
                ops.push((neighbor_key, diffuse_amount, point.valence, point.latency));
            }
            sources.push((key.clone(), diffuse_amount * 2.0));
        }

        let diffused = ops.len();

        // Phase 2: apply — source loses, neighbors gain
        for (key, amount) in sources {
            if let Some(point) = self.nodes.get_mut(&key) {
                point.decay(now_ms);
                point.intensity = (point.intensity - amount).max(0.0);
                point.last_excited = now_ms;
            }
        }

        for (key, amount, valence, latency) in ops {
            let point = self
                .nodes
                .entry(key)
                .or_insert_with(|| FieldPoint::new(now_ms));
            point.decay(now_ms);
            let total = point.intensity + amount;
            if total > 0.0 {
                point.valence = (point.valence * point.intensity + valence * amount) / total;
                point.latency = (point.latency * point.intensity + latency * amount) / total;
            }
            point.intensity = total;
            point.last_excited = now_ms;
        }

        diffused
    }

    /// Excite a node. Pure field-point update, no coupling detection.
    /// O(1) per call — used by hydrate_from_store to avoid O(n) scan.
    ///
    /// Three evolutionary pressures shape the effective deposit:
    /// 1. **Outcome weighting**: successful traces deposit more pheromone
    /// 2. **Corroboration bonus**: multi-source points receive stronger deposits
    /// 3. **Carrying capacity**: deposit cost increases quadratically with field load
    fn excite_node(&mut self, trace: &Trace) -> FieldDelta {
        let bucket = context_bucket(&trace.context_hash);
        let key = FieldKey {
            capability: trace.capability.clone(),
            bucket,
        };
        let now_ms = trace.timestamp;
        let source_id = source_fingerprint(trace);

        // ── Outcome weighting ──
        // Successful patterns physically deposit more pheromone.
        // Failed traces still leave a mark (0.1) — the field remembers
        // attempts, but success dominates the landscape.
        let outcome_weight = match trace.outcome {
            Outcome::Succeeded => 1.0,
            Outcome::Partial => 0.5,
            Outcome::Failed => 0.1,
            Outcome::Timeout => 0.2,
        };

        // Attribution boost (existing: Sigil identity rewarded)
        let attribution = if trace.is_attributed() {
            ATTRIBUTION_BOOST
        } else {
            1.0
        };

        let base_deposit = outcome_weight * attribution;

        // ── Carrying capacity ──
        // Compute load factor BEFORE getting mutable entry reference.
        // Deposit cost increases quadratically as field fills.
        // At 0% load: cost_multiplier = 1.0 (no penalty)
        // At 50% load: cost_multiplier = 1.25
        // At 100% load: cost_multiplier = 2.0
        // At 200% load: cost_multiplier = 5.0
        self.total_intensity = self.current_total_intensity(now_ms);
        let load = self.load_factor(now_ms);
        let cost_multiplier = 1.0 + load * load;

        // ── Corroboration bonus ──
        // Multi-source points receive stronger deposits. Information
        // reinforced by multiple independent agents is more valuable.
        // Log scaling: 2 sources = 1.07x, 10 = 1.23x
        // Read source_count before acquiring mutable entry.
        let prior_source_count = self
            .nodes
            .get(&key)
            .map(|p| p.source_count)
            .unwrap_or(0);

        let corroboration = if prior_source_count > 1 {
            1.0 + (prior_source_count as f64).ln() * 0.1
        } else {
            1.0
        };

        let effective_deposit = base_deposit * corroboration / cost_multiplier;

        let point = self
            .nodes
            .entry(key)
            .or_insert_with(|| FieldPoint::new(now_ms));

        // Apply decay on existing point, then deposit
        point.excite(
            trace.outcome,
            trace.latency_ms as u64,
            now_ms,
            source_id,
            effective_deposit,
        );

        // Maintain running total
        self.total_intensity += effective_deposit;

        FieldDelta {
            capability: trace.capability.clone(),
            bucket,
            intensity_add: effective_deposit,
            outcome: trace.outcome,
            latency_ms: trace.latency_ms as u64,
            source_id,
            timestamp: now_ms,
        }
    }
}

// ── Pheromone Field ──────────────────────────────────────────

/// The pheromone field. A self-organizing signal network.
///
/// Two modes of operation:
/// - **Passive** (ant-like): agents excite() and scan(). The field remembers.
/// - **Active** (slime-mold-like): tick() runs internal dynamics. The field evolves.
///
/// Internal dynamics:
/// - Diffusion: intensity flows to neighboring context buckets
/// - Hebbian coupling: co-excited capabilities form associative bonds
/// - Decay: everything fades without reinforcement
///
/// RwLock: concurrent readers, exclusive writers.
pub struct PheromoneField {
    inner: RwLock<FieldInner>,
}

impl PheromoneField {
    pub fn new() -> Self {
        Self {
            inner: RwLock::new(FieldInner::new()),
        }
    }

    /// Excite the field with a trace. This is the WRITE operation.
    /// No INSERT. The trace modifies the field's state and is gone.
    ///
    /// Also detects Hebbian co-excitations: scans nodes' last_excited
    /// to find capabilities within the coupling window. O(capabilities).
    pub fn excite(&self, trace: &Trace) -> FieldDelta {
        let mut inner = self.inner.write().unwrap();
        let delta = inner.excite_node(trace);

        // Hebbian: detect co-excitations from field state.
        // For each OTHER capability, find max(last_excited) across its nodes.
        let now_ms = trace.timestamp;
        let co_excited: Vec<String> = {
            let mut max_ts: HashMap<String, u64> = HashMap::new();
            for (k, p) in &inner.nodes {
                if k.capability == trace.capability {
                    continue;
                }
                let ts = max_ts.entry(k.capability.clone()).or_insert(0);
                *ts = (*ts).max(p.last_excited);
            }
            max_ts
                .into_iter()
                .filter(|(_, ts)| now_ms.saturating_sub(*ts) < COUPLING_WINDOW_MS)
                .map(|(cap, _)| cap)
                .collect()
        };

        for cap in &co_excited {
            let edge_key = EdgeKey::new(&trace.capability, cap);
            let edge = inner.edges.entry(edge_key).or_insert(Edge {
                weight: 0.0,
                last_reinforced: now_ms,
            });
            let dt = now_ms.saturating_sub(edge.last_reinforced) as f64;
            edge.weight *= (-COUPLING_DECAY_LAMBDA * dt).exp();
            edge.weight += COUPLING_LEARN_RATE;
            edge.last_reinforced = now_ms;
        }

        delta
    }

    /// Scan the field near a context hash. Returns all capabilities
    /// with non-trivial intensity in the neighborhood.
    ///
    /// This replaces: query_similar → group by capability → aggregate.
    /// All in O(n) where n = number of live field points, not trace count.
    pub fn scan(
        &self,
        context_hash: &ContextHash,
        bucket_radius: i64,
        limit: usize,
    ) -> Vec<FieldScan> {
        let target_bucket = context_bucket(context_hash);
        let bucket_lo = (target_bucket - bucket_radius).max(0);
        let bucket_hi = (target_bucket + bucket_radius).min(65535);
        let now_ms = chrono::Utc::now().timestamp_millis() as u64;

        let inner = self.inner.read().unwrap();

        // Phase 1: collect all live points in the bucket range, grouped by capability
        let mut cap_map: HashMap<String, FieldScan> = HashMap::new();

        for (key, point) in &inner.nodes {
            if key.bucket < bucket_lo || key.bucket > bucket_hi {
                continue;
            }
            let intensity = point.current_intensity(now_ms);
            if intensity < PRUNE_THRESHOLD {
                continue;
            }

            let bucket_dist = (key.bucket - target_bucket).unsigned_abs();
            let similarity = 1.0 - (bucket_dist as f64 / 65536.0).min(1.0);

            let entry = cap_map
                .entry(key.capability.clone())
                .or_insert_with(|| FieldScan {
                    capability: key.capability.clone(),
                    intensity: 0.0,
                    valence: 0.0,
                    latency: 0.0,
                    variance: 0.0,
                    total_excitations: 0,
                    source_count: 0,
                    context_similarity: 0.0,
                });

            let w = intensity;
            let old_w = entry.intensity;
            let total_w = old_w + w;
            if total_w > 0.0 {
                entry.valence = (entry.valence * old_w + point.valence * w) / total_w;
                entry.latency = (entry.latency * old_w + point.latency * w) / total_w;
                entry.variance = (entry.variance * old_w + point.variance * w) / total_w;
                entry.context_similarity =
                    (entry.context_similarity * old_w + similarity * w) / total_w;
            }
            entry.intensity = total_w;
            entry.total_excitations += point.total_excitations;
            entry.source_count = entry.source_count.max(point.source_count);
        }

        // Phase 2: surface Hebbian-coupled capabilities
        let primary_data: Vec<(String, f64, f64, f64)> = cap_map
            .values()
            .map(|s| {
                (
                    s.capability.clone(),
                    s.intensity,
                    s.valence,
                    s.context_similarity,
                )
            })
            .collect();

        for (primary_cap, primary_intensity, primary_valence, primary_sim) in &primary_data {
            for (key, edge) in &inner.edges {
                let Some(partner) = key.partner(primary_cap) else {
                    continue;
                };
                if cap_map.contains_key(partner) {
                    continue;
                }
                let cw = edge.current_weight(now_ms);
                if cw < COUPLING_PRUNE_THRESHOLD {
                    continue;
                }
                cap_map.insert(
                    partner.to_string(),
                    FieldScan {
                        capability: partner.to_string(),
                        intensity: primary_intensity * cw,
                        valence: *primary_valence,
                        latency: 0.0,
                        variance: 0.0,
                        total_excitations: 0,
                        source_count: 0,
                        context_similarity: primary_sim * cw,
                    },
                );
            }
        }

        let mut results: Vec<FieldScan> = cap_map.into_values().collect();
        results.sort_by(|a, b| {
            b.intensity
                .partial_cmp(&a.intensity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);
        results
    }

    /// Get field stats for a specific capability (all buckets).
    /// Replaces: aggregate(capability).
    pub fn aggregate(&self, capability: &str) -> Option<FieldScan> {
        let now_ms = chrono::Utc::now().timestamp_millis() as u64;
        let inner = self.inner.read().unwrap();

        let mut total_intensity = 0.0;
        let mut weighted_valence = 0.0;
        let mut weighted_latency = 0.0;
        let mut weighted_variance = 0.0;
        let mut total_excitations = 0u64;
        let mut max_source_count = 0u32;

        for (key, point) in &inner.nodes {
            if key.capability != capability {
                continue;
            }
            let intensity = point.current_intensity(now_ms);
            if intensity < PRUNE_THRESHOLD {
                continue;
            }
            weighted_valence += point.valence * intensity;
            weighted_latency += point.latency * intensity;
            weighted_variance += point.variance * intensity;
            total_intensity += intensity;
            total_excitations += point.total_excitations;
            max_source_count = max_source_count.max(point.source_count);
        }

        if total_intensity < PRUNE_THRESHOLD {
            return None;
        }

        Some(FieldScan {
            capability: capability.to_string(),
            intensity: total_intensity,
            valence: weighted_valence / total_intensity,
            latency: weighted_latency / total_intensity,
            variance: weighted_variance / total_intensity,
            total_excitations,
            source_count: max_source_count,
            context_similarity: 1.0,
        })
    }

    /// Prune dead field points and edges. Returns number of pruned points.
    pub fn prune(&self) -> usize {
        let now_ms = chrono::Utc::now().timestamp_millis() as u64;
        let mut inner = self.inner.write().unwrap();
        inner.edges.retain(|_, e| !e.is_dead(now_ms));
        let before = inner.nodes.len();
        inner.nodes.retain(|_, p| !p.is_dead(now_ms));
        inner.total_intensity = inner.current_total_intensity(now_ms);
        before - inner.nodes.len()
    }

    /// Autonomous self-evolution step. The field's own clock.
    ///
    /// Call this periodically (e.g. every 5 minutes). The field:
    /// 1. Diffuses intensity to neighboring buckets (slime-mold tube flow)
    /// 2. Prunes dead edges (Hebbian couplings)
    /// 3. Prunes dead field points
    ///
    /// Single lock for the entire tick — atomic self-evolution.
    pub fn tick(&self) -> TickResult {
        let now_ms = chrono::Utc::now().timestamp_millis() as u64;
        let mut inner = self.inner.write().unwrap();

        let diffused = inner.diffuse(now_ms);

        let before_edges = inner.edges.len();
        inner.edges.retain(|_, e| !e.is_dead(now_ms));
        let couplings_pruned = before_edges - inner.edges.len();

        let before_nodes = inner.nodes.len();
        inner.nodes.retain(|_, p| !p.is_dead(now_ms));
        let points_pruned = before_nodes - inner.nodes.len();
        inner.total_intensity = inner.current_total_intensity(now_ms);

        TickResult {
            diffused,
            couplings_reinforced: 0,
            couplings_pruned,
            points_pruned,
            load_factor: inner.load_factor(now_ms),
        }
    }

    /// Number of Hebbian edges (for diagnostics).
    pub fn coupling_count(&self) -> usize {
        self.inner.read().unwrap().edges.len()
    }

    /// Number of live field points (for diagnostics).
    pub fn len(&self) -> usize {
        self.inner.read().unwrap().nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Current field load factor: total_intensity / FIELD_CAPACITY.
    /// 0.0 = empty, 1.0 = at carrying capacity, >1.0 = over capacity.
    pub fn load_factor(&self) -> f64 {
        let now_ms = chrono::Utc::now().timestamp_millis() as u64;
        self.inner.read().unwrap().load_factor(now_ms)
    }

    /// Total pheromone intensity across all field points.
    pub fn total_intensity(&self) -> f64 {
        let now_ms = chrono::Utc::now().timestamp_millis() as u64;
        self.inner.read().unwrap().current_total_intensity(now_ms)
    }

    /// Snapshot the entire field for P2P sync or persistence.
    pub fn snapshot(&self) -> FieldSnapshot {
        let now_ms = chrono::Utc::now().timestamp_millis() as u64;
        let inner = self.inner.read().unwrap();

        let points = inner
            .nodes
            .iter()
            .filter(|(_, p)| !p.is_dead(now_ms))
            .map(|(key, point)| FieldSnapshotEntry {
                capability: key.capability.clone(),
                bucket: key.bucket,
                intensity: point.intensity,
                valence: point.valence,
                latency: point.latency,
                variance: point.variance,
                last_excited: point.last_excited,
                total_excitations: point.total_excitations,
                source_count: point.source_count,
                source_hashes: point.sources.clone(),
            })
            .collect();

        let couplings = inner
            .edges
            .iter()
            .filter(|(_, e)| !e.is_dead(now_ms))
            .map(|(key, e)| CouplingSnapshotEntry {
                cap_a: key.cap_a.clone(),
                cap_b: key.cap_b.clone(),
                weight: e.current_weight(now_ms),
                last_reinforced: e.last_reinforced,
            })
            .collect();

        FieldSnapshot {
            points,
            couplings,
            total_intensity: inner.current_total_intensity(now_ms),
        }
    }

    /// Restore field from a snapshot (e.g., on startup from disk).
    pub fn restore(&self, snapshot: &FieldSnapshot) {
        let mut inner = self.inner.write().unwrap();
        let now_ms = chrono::Utc::now().timestamp_millis() as u64;
        inner.nodes.clear();
        inner.edges.clear();
        inner.total_intensity = 0.0;

        for entry in &snapshot.points {
            let key = FieldKey {
                capability: entry.capability.clone(),
                bucket: entry.bucket,
            };
            inner.nodes.insert(
                key,
                FieldPoint {
                    intensity: entry.intensity,
                    valence: entry.valence,
                    latency: entry.latency,
                    variance: entry.variance,
                    last_excited: entry.last_excited,
                    total_excitations: entry.total_excitations,
                    source_count: entry
                        .source_count
                        .max(entry.source_hashes.len() as u32),
                    sources: entry.source_hashes.clone(),
                },
            );
        }

        for entry in &snapshot.couplings {
            let key = EdgeKey::new(&entry.cap_a, &entry.cap_b);
            let edge = inner.edges.entry(key).or_insert(Edge {
                weight: 0.0,
                last_reinforced: entry.last_reinforced,
            });
            edge.weight = edge.weight.max(entry.weight);
            edge.last_reinforced = edge.last_reinforced.max(entry.last_reinforced);
        }

        inner.total_intensity = inner.current_total_intensity(now_ms);
    }

    /// Apply a delta from P2P sync. CRDT-friendly: addition is commutative.
    pub fn apply_delta(&self, delta: &FieldDelta) {
        let key = FieldKey {
            capability: delta.capability.clone(),
            bucket: delta.bucket,
        };
        let mut inner = self.inner.write().unwrap();
        let point = inner
            .nodes
            .entry(key)
            .or_insert_with(|| FieldPoint::new(delta.timestamp));
        point.excite(
            delta.outcome,
            delta.latency_ms,
            delta.timestamp,
            delta.source_id,
            delta.intensity_add,
        );
        inner.total_intensity += delta.intensity_add;
    }

    /// List all capabilities with their total intensity (for explore intent).
    pub fn capabilities(&self, limit: usize) -> Vec<FieldScan> {
        let now_ms = chrono::Utc::now().timestamp_millis() as u64;
        let inner = self.inner.read().unwrap();

        let mut cap_map: HashMap<&str, (f64, f64, f64, u64, u32)> = HashMap::new();

        for (key, point) in &inner.nodes {
            let intensity = point.current_intensity(now_ms);
            if intensity < PRUNE_THRESHOLD {
                continue;
            }
            let entry = cap_map
                .entry(&key.capability)
                .or_insert((0.0, 0.0, 0.0, 0, 0));
            entry.0 += intensity;
            entry.1 += point.valence * intensity;
            entry.2 += point.latency * intensity;
            entry.3 += point.total_excitations;
            entry.4 = entry.4.max(point.source_count);
        }

        let mut results: Vec<FieldScan> = cap_map
            .into_iter()
            .map(|(cap, (intensity, wv, wl, exc, sc))| FieldScan {
                capability: cap.to_string(),
                intensity,
                valence: if intensity > 0.0 { wv / intensity } else { 0.5 },
                latency: if intensity > 0.0 { wl / intensity } else { 0.0 },
                variance: 0.0,
                total_excitations: exc,
                source_count: sc,
                context_similarity: 0.0,
            })
            .collect();
        results.sort_by(|a, b| {
            b.intensity
                .partial_cmp(&a.intensity)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        results.truncate(limit);
        results
    }
}

impl PheromoneField {
    /// Project field state into semantic-stable effect signals.
    ///
    /// Pure query — does not modify field state.
    /// Returns zero overlay if capability/context not found in field.
    pub fn overlay(&self, context_hash: &ContextHash, capability: &str) -> FieldOverlay {
        let now_ms = chrono::Utc::now().timestamp_millis() as u64;
        let inner = self.inner.read().unwrap();
        let bucket = context_bucket(context_hash);
        let key = FieldKey {
            capability: capability.to_string(),
            bucket,
        };

        // Node-derived signals
        let (familiarity, consensus, momentum) = match inner.nodes.get(&key) {
            Some(point) => {
                let intensity = point.current_intensity(now_ms);

                // Familiarity: sigmoid saturation of intensity.
                // intensity=1 → 0.39, intensity=3 → 0.78, intensity=5 → 0.92
                let fam = 1.0 - (-intensity * 0.5_f64).exp();

                // Consensus: inverse variance. variance=0 → 1.0, variance≥1 → 0.0
                let con = (1.0 - point.variance).max(0.0);

                // Momentum: linear from +1 (just excited) to -1 (2× half-life ago).
                // Crosses zero at exactly one half-life.
                let age_hours = (now_ms.saturating_sub(point.last_excited)) as f64 / 3_600_000.0;
                let mom = (1.0 - age_hours / HALF_LIFE_HOURS).clamp(-1.0, 1.0);

                (fam, con, mom)
            }
            None => (0.0, 0.0, 0.0),
        };

        // Edge-derived coupling: average weight of live edges involving this capability.
        let coupling = {
            let mut total_weight = 0.0;
            let mut count = 0u32;
            for (edge_key, edge) in &inner.edges {
                if edge_key.cap_a == capability || edge_key.cap_b == capability {
                    let w = edge.current_weight(now_ms);
                    if w > COUPLING_PRUNE_THRESHOLD {
                        total_weight += w;
                        count += 1;
                    }
                }
            }
            if count > 0 {
                (total_weight / count as f64).min(1.0)
            } else {
                0.0
            }
        };

        FieldOverlay {
            familiarity,
            consensus,
            momentum,
            coupling,
        }
    }
}

impl PheromoneField {
    /// Hydrate the field from existing traces in the store.
    /// Called once on startup to warm the field from cold storage.
    ///
    /// Uses excite_node() directly — no co-excitation scan per trace.
    /// Historical traces have original timestamps (spread across hours/days),
    /// so they'd never be within the 60s coupling window anyway.
    /// O(traces) instead of O(traces × nodes).
    /// Clear all field state (nodes + edges). Used for data reset.
    pub fn clear(&self) {
        let mut inner = self.inner.write().unwrap();
        inner.nodes.clear();
        inner.edges.clear();
        inner.total_intensity = 0.0;
    }

    pub fn hydrate_from_store(&self, store: &crate::storage::TraceStore) {
        let caps = match store.distinct_capabilities(500) {
            Ok(c) => c,
            Err(_) => return,
        };
        let mut count = 0u64;
        for cap in &caps {
            if crate::posts::is_signal_capability(cap)
                || crate::presence::is_presence_capability(cap)
                || crate::continuity::is_continuity_capability(cap)
            {
                continue;
            }
            let mut traces = match store.query_capability(cap, 200) {
                Ok(t) => t,
                Err(_) => continue,
            };
            // query_capability returns DESC (newest first).
            // EMA gives most weight to last-processed observation.
            // Reverse so newest traces dominate the final valence.
            traces.reverse();
            // Lock per capability batch — no coupling overhead
            {
                let mut inner = self.inner.write().unwrap();
                for trace in &traces {
                    inner.excite_node(trace);
                    count += 1;
                }
            }
        }
        if count > 0 {
            tracing::info!(
                traces = count,
                points = self.len(),
                "Hydrated pheromone field from store"
            );
        }
    }
}

impl Default for PheromoneField {
    fn default() -> Self {
        Self::new()
    }
}

// ── Helpers ────────────────────────────────────────────────────

/// Extract an 8-byte source fingerprint from a trace for tracking unique sources.
fn source_fingerprint(trace: &Trace) -> [u8; 8] {
    if let Some(ref di) = trace.device_identity {
        let bytes = di.as_bytes();
        let mut fp = [0u8; 8];
        for (i, b) in bytes.iter().take(8).enumerate() {
            fp[i] = *b;
        }
        fp
    } else {
        let mut fp = [0u8; 8];
        fp.copy_from_slice(&trace.node_pubkey[..8]);
        fp
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::simhash;
    use crate::trace::Trace;
    use ed25519_dalek::{Signer, SigningKey};

    fn make_trace(capability: &str, context: &str, outcome: Outcome, latency_ms: u32) -> Trace {
        make_trace_with_seed(capability, context, outcome, latency_ms, 1)
    }

    fn make_trace_with_seed(
        capability: &str,
        context: &str,
        outcome: Outcome,
        latency_ms: u32,
        seed: u8,
    ) -> Trace {
        let key = SigningKey::from_bytes(&[seed; 32]);
        Trace::new(
            capability.to_string(),
            outcome,
            latency_ms,
            0,
            simhash(context),
            Some(context.to_string()),
            None,
            "test-model".to_string(),
            key.verifying_key().to_bytes(),
            |bytes| key.sign(bytes),
        )
    }

    fn make_attributed_trace(
        capability: &str,
        context: &str,
        outcome: Outcome,
        latency_ms: u32,
    ) -> Trace {
        use crate::trace::TraceConfig;
        let key = SigningKey::from_bytes(&[1u8; 32]);
        TraceConfig::for_sigil("SIG_test", capability, outcome, "test-model")
            .context_raw(simhash(context), Some(context.to_string()))
            .latency_ms(latency_ms)
            .sign(key.verifying_key().to_bytes(), |bytes| key.sign(bytes))
    }

    #[test]
    fn excite_and_scan() {
        let field = PheromoneField::new();
        let t1 = make_trace("claude-code/Bash", "git status", Outcome::Succeeded, 100);
        let t2 = make_trace("claude-code/Bash", "git status", Outcome::Succeeded, 200);
        let t3 = make_trace("claude-code/Bash", "git status", Outcome::Failed, 5000);

        field.excite(&t1);
        field.excite(&t2);
        field.excite(&t3);

        let hash = simhash("git status");
        let results = field.scan(&hash, 1, 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].capability, "claude-code/Bash");
        assert!(results[0].intensity > 2.0);
        assert!(results[0].total_excitations == 3);
    }

    #[test]
    fn aggregate_capability() {
        let field = PheromoneField::new();
        for _ in 0..10 {
            let t = make_trace("claude-code/Edit", "edit file", Outcome::Succeeded, 50);
            field.excite(&t);
        }
        let t = make_trace("claude-code/Edit", "edit file", Outcome::Failed, 1000);
        field.excite(&t);

        let agg = field.aggregate("claude-code/Edit").unwrap();
        assert_eq!(agg.total_excitations, 11);
        assert!(agg.valence > 0.5);
    }

    #[test]
    fn attributed_traces_get_intensity_boost() {
        let field = PheromoneField::new();
        let anon = make_trace("anon-cap", "same context", Outcome::Succeeded, 100);
        let attr = make_attributed_trace("attr-cap", "same context", Outcome::Succeeded, 100);

        field.excite(&anon);
        field.excite(&attr);

        let anon_agg = field.aggregate("anon-cap").unwrap();
        let attr_agg = field.aggregate("attr-cap").unwrap();

        // Attributed trace should have higher intensity (1.1x)
        assert!(
            attr_agg.intensity > anon_agg.intensity,
            "attributed ({}) should have higher intensity than anonymous ({})",
            attr_agg.intensity,
            anon_agg.intensity
        );
    }

    #[test]
    fn decay_reduces_intensity() {
        let field = PheromoneField::new();
        let t = make_trace("test/cap", "some context", Outcome::Succeeded, 100);
        field.excite(&t);

        // Simulate time passing
        {
            let mut inner = field.inner.write().unwrap();
            for point in inner.nodes.values_mut() {
                point.last_excited -= 72 * 3_600_000;
            }
        }

        let hash = simhash("some context");
        let results = field.scan(&hash, 1, 10);
        assert!(results.is_empty() || results[0].intensity < 0.5);
    }

    #[test]
    fn prune_removes_dead_points() {
        let field = PheromoneField::new();
        let t = make_trace("test/cap", "context", Outcome::Succeeded, 100);
        field.excite(&t);
        assert_eq!(field.len(), 1);

        {
            let mut inner = field.inner.write().unwrap();
            for point in inner.nodes.values_mut() {
                point.last_excited -= 30 * 24 * 3_600_000;
            }
        }

        let pruned = field.prune();
        assert_eq!(pruned, 1);
        assert_eq!(field.len(), 0);
    }

    #[test]
    fn snapshot_and_restore() {
        let field = PheromoneField::new();
        for i in 0..5 {
            let ctx = format!("context {}", i);
            let t = make_trace("cap/test", &ctx, Outcome::Succeeded, 100);
            field.excite(&t);
        }

        let snapshot = field.snapshot();
        assert!(!snapshot.points.is_empty());

        let field2 = PheromoneField::new();
        field2.restore(&snapshot);
        assert_eq!(field2.len(), field.len());
    }

    #[test]
    fn diffusion_spreads_to_neighbors() {
        let field = PheromoneField::new();
        let t = make_trace("cap/diffuse", "exact context", Outcome::Succeeded, 100);
        field.excite(&t);

        assert_eq!(field.len(), 1);

        let result = field.tick();
        assert!(result.diffused > 0);
        assert!(
            field.len() >= 2,
            "diffusion should create neighbor points, got {}",
            field.len()
        );

        let inner = field.inner.read().unwrap();
        let mut intensities: Vec<f64> = inner.nodes.values().map(|p| p.intensity).collect();
        intensities.sort_by(|a, b| b.partial_cmp(a).unwrap());
        assert!(
            intensities[0] > intensities[1],
            "source should be stronger than neighbors"
        );
    }

    #[test]
    fn diffusion_conserves_total_intensity() {
        let field = PheromoneField::new();
        let t = make_trace(
            "cap/conserve",
            "some mid-range context",
            Outcome::Succeeded,
            100,
        );
        field.excite(&t);

        let total_before: f64 = {
            let inner = field.inner.read().unwrap();
            inner.nodes.values().map(|p| p.intensity).sum()
        };

        field.tick();

        let total_after: f64 = {
            let inner = field.inner.read().unwrap();
            inner.nodes.values().map(|p| p.intensity).sum()
        };

        assert!(
            (total_after - total_before).abs() < 0.1,
            "diffusion should conserve intensity: before={total_before:.4}, after={total_after:.4}"
        );
    }

    #[test]
    fn hebbian_coupling_forms_on_coexcitation() {
        let field = PheromoneField::new();

        let t1 = make_trace("cap/alpha", "task context", Outcome::Succeeded, 100);
        let t2 = make_trace("cap/beta", "task context", Outcome::Succeeded, 200);
        field.excite(&t1);
        field.excite(&t2);

        assert_eq!(
            field.coupling_count(),
            1,
            "co-excitation should create a coupling"
        );

        let inner = field.inner.read().unwrap();
        let key = EdgeKey::new("cap/alpha", "cap/beta");
        let edge = inner.edges.get(&key).expect("edge should exist");
        assert!(
            (edge.weight - COUPLING_LEARN_RATE).abs() < 0.01,
            "edge weight should be ~{COUPLING_LEARN_RATE}, got {}",
            edge.weight
        );
    }

    #[test]
    fn hebbian_coupling_surfaces_in_scan() {
        let field = PheromoneField::new();

        // Build strong presence for primary in the search context
        for _ in 0..5 {
            let t = make_trace("cap/primary", "search context", Outcome::Succeeded, 100);
            field.excite(&t);
        }

        // Excite associated in a DISTANT context — gives it a last_excited
        // timestamp for coupling detection, but its field point is far from
        // the search context's bucket, so it won't appear in scan directly.
        let t_assoc = make_trace(
            "cap/associated",
            "completely unrelated distant topic xyz 999",
            Outcome::Succeeded,
            100,
        );
        field.excite(&t_assoc);

        // Excite primary again — detects co-excitation with associated
        let t_primary = make_trace("cap/primary", "search context", Outcome::Succeeded, 100);
        field.excite(&t_primary);

        assert!(field.coupling_count() >= 1, "coupling should have formed");

        // Scan for primary's context — associated should surface via coupling
        let hash = simhash("search context");
        let results = field.scan(&hash, 1, 10);

        let caps: Vec<&str> = results.iter().map(|r| r.capability.as_str()).collect();
        assert!(
            caps.contains(&"cap/primary"),
            "primary should be in results"
        );
        assert!(
            caps.contains(&"cap/associated"),
            "coupled capability should surface in scan results, got: {:?}",
            caps
        );

        let primary_i = results
            .iter()
            .find(|r| r.capability == "cap/primary")
            .unwrap()
            .intensity;
        let assoc_i = results
            .iter()
            .find(|r| r.capability == "cap/associated")
            .unwrap()
            .intensity;
        assert!(
            primary_i > assoc_i,
            "associated ({assoc_i:.2}) should rank below primary ({primary_i:.2})"
        );
    }

    #[test]
    fn tick_prunes_dead_couplings() {
        let field = PheromoneField::new();
        let t1 = make_trace("cap/a", "ctx", Outcome::Succeeded, 100);
        let t2 = make_trace("cap/b", "ctx", Outcome::Succeeded, 100);
        field.excite(&t1);
        field.excite(&t2);
        assert_eq!(field.coupling_count(), 1);

        {
            let mut inner = field.inner.write().unwrap();
            for e in inner.edges.values_mut() {
                e.last_reinforced -= 365 * 24 * 3_600_000;
            }
        }

        let result = field.tick();
        assert_eq!(result.couplings_pruned, 1);
        assert_eq!(field.coupling_count(), 0);
    }

    #[test]
    fn snapshot_preserves_couplings() {
        let field = PheromoneField::new();
        let t1 = make_trace("cap/x", "ctx", Outcome::Succeeded, 100);
        let t2 = make_trace("cap/y", "ctx", Outcome::Succeeded, 100);
        field.excite(&t1);
        field.excite(&t2);

        let snapshot = field.snapshot();
        assert_eq!(snapshot.couplings.len(), 1);

        let field2 = PheromoneField::new();
        field2.restore(&snapshot);
        assert_eq!(field2.coupling_count(), 1);
    }

    #[test]
    fn snapshot_restore_preserves_source_fingerprints() {
        let field = PheromoneField::new();
        let t1 = make_trace_with_seed("cap/src", "ctx", Outcome::Succeeded, 100, 1);
        let t2 = make_trace_with_seed("cap/src", "ctx", Outcome::Succeeded, 100, 2);
        field.excite(&t1);
        field.excite(&t2);

        let snapshot = field.snapshot();
        assert_eq!(snapshot.points[0].source_hashes.len(), 2);

        let restored = PheromoneField::new();
        restored.restore(&snapshot);
        restored.excite(&make_trace_with_seed(
            "cap/src",
            "ctx",
            Outcome::Succeeded,
            100,
            1,
        ));

        let inner = restored.inner.read().unwrap();
        let point = inner
            .nodes
            .values()
            .find(|point| point.total_excitations >= 3)
            .expect("expected restored point");
        assert_eq!(
            point.source_count, 2,
            "restored source hashes should dedupe repeated sources"
        );
        assert_eq!(point.sources.len(), 2);
    }

    #[test]
    fn delta_sync() {
        let field_a = PheromoneField::new();
        let t = make_trace("cap/sync", "sync test", Outcome::Succeeded, 100);
        let delta = field_a.excite(&t);

        let field_b = PheromoneField::new();
        field_b.apply_delta(&delta);

        let agg_a = field_a.aggregate("cap/sync");
        let agg_b = field_b.aggregate("cap/sync");
        assert!(agg_a.is_some());
        assert!(agg_b.is_some());
        assert_eq!(
            agg_a.unwrap().total_excitations,
            agg_b.unwrap().total_excitations
        );
    }

    // ── Overlay tests ────────────────────────────────────────

    #[test]
    fn overlay_empty_field() {
        let field = PheromoneField::new();
        let hash = simhash("unknown context");
        let o = field.overlay(&hash, "nonexistent/cap");
        assert_eq!(o.familiarity, 0.0);
        assert_eq!(o.consensus, 0.0);
        assert_eq!(o.momentum, 0.0);
        assert_eq!(o.coupling, 0.0);
    }

    #[test]
    fn overlay_after_excitation() {
        let field = PheromoneField::new();
        let t = make_trace("cap/overlay", "test context", Outcome::Succeeded, 100);
        field.excite(&t);

        let hash = simhash("test context");
        let o = field.overlay(&hash, "cap/overlay");
        assert!(o.familiarity > 0.0, "familiarity={}", o.familiarity);
        assert!(o.consensus > 0.0, "consensus={}", o.consensus);
        assert!(o.momentum > 0.0, "momentum={}", o.momentum);
        assert_eq!(o.coupling, 0.0, "no edges yet");
    }

    #[test]
    fn overlay_familiarity_grows_with_intensity() {
        let field = PheromoneField::new();
        let hash = simhash("ctx");
        let t = make_trace("cap/fam", "ctx", Outcome::Succeeded, 100);
        field.excite(&t);
        let fam1 = field.overlay(&hash, "cap/fam").familiarity;

        for _ in 0..5 {
            let t = make_trace("cap/fam", "ctx", Outcome::Succeeded, 100);
            field.excite(&t);
        }
        let fam6 = field.overlay(&hash, "cap/fam").familiarity;
        assert!(fam6 > fam1, "familiarity should grow: {fam1} < {fam6}");
        assert!(fam6 < 1.0, "should not saturate at 6 excitations");
    }

    #[test]
    fn overlay_consensus_drops_with_mixed_outcomes() {
        let field = PheromoneField::new();
        let hash = simhash("ctx");

        // All successes → high consensus
        for _ in 0..5 {
            let t = make_trace("cap/con", "ctx", Outcome::Succeeded, 100);
            field.excite(&t);
        }
        let con_pure = field.overlay(&hash, "cap/con").consensus;

        // Mix in failures → variance rises → consensus drops
        for _ in 0..5 {
            let t = make_trace("cap/con", "ctx", Outcome::Failed, 100);
            field.excite(&t);
        }
        let con_mixed = field.overlay(&hash, "cap/con").consensus;
        assert!(
            con_mixed < con_pure,
            "consensus should drop: {con_pure} > {con_mixed}"
        );
    }

    #[test]
    fn overlay_momentum_decays_with_age() {
        let field = PheromoneField::new();
        let hash = simhash("ctx");
        let t = make_trace("cap/mom", "ctx", Outcome::Succeeded, 100);
        field.excite(&t);

        let mom_fresh = field.overlay(&hash, "cap/mom").momentum;

        // Age the point by one half-life
        {
            let mut inner = field.inner.write().unwrap();
            for p in inner.nodes.values_mut() {
                p.last_excited -= (HALF_LIFE_HOURS * 3_600_000.0) as u64;
            }
        }

        let mom_aged = field.overlay(&hash, "cap/mom").momentum;
        assert!(
            mom_fresh > mom_aged,
            "momentum should decay: {mom_fresh} > {mom_aged}"
        );
        assert!(
            mom_aged.abs() < 0.05,
            "at half-life, momentum ≈ 0: {mom_aged}"
        );
    }

    #[test]
    fn overlay_coupling_from_coexcitation() {
        let field = PheromoneField::new();
        let hash = simhash("task ctx");

        let t1 = make_trace("cap/co1", "task ctx", Outcome::Succeeded, 100);
        let t2 = make_trace("cap/co2", "task ctx", Outcome::Succeeded, 100);
        field.excite(&t1);
        field.excite(&t2);

        let o = field.overlay(&hash, "cap/co1");
        assert!(
            o.coupling > 0.0,
            "coupling should be positive after co-excitation: {}",
            o.coupling
        );
    }

    #[test]
    fn read_side_intensity_matches_mutating_decay() {
        let now_ms = chrono::Utc::now().timestamp_millis() as u64;
        let mut point = FieldPoint::new(now_ms);
        point.intensity = 5.0;
        point.total_excitations = 256;
        point.last_excited = now_ms - 72 * 3_600_000;

        let read_only = point.current_intensity(now_ms);
        let mut mutated = point.clone();
        mutated.decay(now_ms);

        assert!(
            (read_only - mutated.intensity).abs() < 1e-9,
            "read-side decay ({read_only}) should match mutating decay ({})",
            mutated.intensity
        );
    }

    #[test]
    fn field_point_current_intensity_stays_finite_at_boundaries() {
        let now_ms = chrono::Utc::now().timestamp_millis() as u64;
        let mut point = FieldPoint::new(now_ms);

        point.intensity = 0.0;
        assert_eq!(point.current_intensity(now_ms), 0.0);

        point.intensity = f64::MAX / 4.0;
        assert!(point.current_intensity(now_ms).is_finite());

        point.intensity = -3.5;
        assert!(point.current_intensity(now_ms).is_finite());
    }

    #[test]
    fn snapshot_restore_preserves_current_intensity() {
        let field = PheromoneField::new();
        let t = make_trace("cap/recover", "ctx", Outcome::Succeeded, 100);
        field.excite(&t);

        {
            let mut inner = field.inner.write().unwrap();
            for point in inner.nodes.values_mut() {
                point.last_excited -= (HALF_LIFE_HOURS * 3_600_000.0) as u64;
            }
        }

        let before = field.aggregate("cap/recover").unwrap().intensity;
        let snapshot = field.snapshot();

        let restored = PheromoneField::new();
        restored.restore(&snapshot);
        let after = restored.aggregate("cap/recover").unwrap().intensity;

        assert!(
            (before - after).abs() < 0.01,
            "restore should preserve current intensity: before={before}, after={after}"
        );
    }

    #[test]
    fn clear_resets_total_intensity() {
        let field = PheromoneField::new();
        field.excite(&make_trace("cap/clear", "ctx", Outcome::Succeeded, 100));
        assert!(field.total_intensity() > 0.0);

        field.clear();

        assert!(field.is_empty());
        assert_eq!(field.total_intensity(), 0.0);
        assert_eq!(field.load_factor(), 0.0);
    }
}
