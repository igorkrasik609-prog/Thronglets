use crate::context::{simhash, similarity};
use crate::trace::{Outcome, Trace};
use ed25519_dalek::Signature;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::time::{SystemTime, UNIX_EPOCH};

pub const SIGNAL_CAPABILITY_PREFIX: &str = "urn:thronglets:signal:";
pub const SIGNAL_REINFORCEMENT_CAPABILITY_PREFIX: &str = "urn:thronglets:signal-read:";
pub const DEFAULT_SIGNAL_TTL_HOURS: u32 = 72;
pub const DEFAULT_SIGNAL_REINFORCEMENT_TTL_HOURS: u32 = 24;
pub const DEFAULT_RECOMMEND_SIGNAL_TTL_HOURS: u32 = 12;
pub const DEFAULT_AVOID_SIGNAL_TTL_HOURS: u32 = 72;
pub const DEFAULT_WATCH_SIGNAL_TTL_HOURS: u32 = 48;
pub const DEFAULT_INFO_SIGNAL_TTL_HOURS: u32 = 24;
pub const DEFAULT_PSYCHE_STATE_SIGNAL_TTL_HOURS: u32 = 6;
pub const DERIVED_GUIDANCE_EPOCH: &str = "thronglets.derived.v1";
pub const AUTO_DERIVED_SIGNAL_MODEL_ID: &str = "thronglets-auto";
const AUTO_DERIVED_SIGNAL_SOURCE: &str = "derived-guidance";

#[derive(Debug, Clone)]
pub struct SignalTraceConfig {
    pub model_id: String,
    pub session_id: Option<String>,
    pub owner_account: Option<String>,
    pub device_identity: Option<String>,
    pub agent_id: Option<String>,
    pub sigil_id: Option<String>,
    pub space: Option<String>,
    pub ttl_hours: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalPostKind {
    Recommend,
    Avoid,
    Watch,
    Info,
    PsycheState,
}

impl SignalPostKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Recommend => "recommend",
            Self::Avoid => "avoid",
            Self::Watch => "watch",
            Self::Info => "info",
            Self::PsycheState => "psyche_state",
        }
    }

    pub fn capability(self) -> String {
        format!("{SIGNAL_CAPABILITY_PREFIX}{}", self.as_str())
    }

    pub fn reinforcement_capability(self) -> String {
        format!("{SIGNAL_REINFORCEMENT_CAPABILITY_PREFIX}{}", self.as_str())
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "recommend" => Some(Self::Recommend),
            "avoid" => Some(Self::Avoid),
            "watch" => Some(Self::Watch),
            "info" => Some(Self::Info),
            "psyche_state" => Some(Self::PsycheState),
            _ => None,
        }
    }

    pub fn from_capability(capability: &str) -> Option<(Self, bool)> {
        if let Some(value) = capability.strip_prefix(SIGNAL_CAPABILITY_PREFIX) {
            return Self::parse(value).map(|kind| (kind, false));
        }
        capability
            .strip_prefix(SIGNAL_REINFORCEMENT_CAPABILITY_PREFIX)
            .and_then(Self::parse)
            .map(|kind| (kind, true))
    }

    pub fn default_ttl_hours(self) -> u32 {
        match self {
            Self::Recommend => DEFAULT_RECOMMEND_SIGNAL_TTL_HOURS,
            Self::Avoid => DEFAULT_AVOID_SIGNAL_TTL_HOURS,
            Self::Watch => DEFAULT_WATCH_SIGNAL_TTL_HOURS,
            Self::Info => DEFAULT_INFO_SIGNAL_TTL_HOURS,
            Self::PsycheState => DEFAULT_PSYCHE_STATE_SIGNAL_TTL_HOURS,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SignalTracePayload {
    context: String,
    message: String,
    space: Option<String>,
    expires_at: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    derived_guidance_epoch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    derived_guidance_source: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SignalQueryResult {
    pub kind: String,
    pub message: String,
    pub space: Option<String>,
    pub context_similarity: f64,
    pub total_posts: u64,
    pub reinforcement_count: u32,
    pub source_count: u32,
    pub model_count: u32,
    pub corroboration_tier: String,
    pub density_score: u8,
    pub density_tier: String,
    pub inhibition_penalty: u8,
    pub inhibition_state: String,
    pub latest_timestamp: u64,
    pub expires_at: u64,
    pub contexts: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SignalFeedResult {
    pub kind: String,
    pub message: String,
    pub space: Option<String>,
    pub total_posts: u64,
    pub reinforcement_count: u32,
    pub source_count: u32,
    pub model_count: u32,
    pub corroboration_tier: String,
    pub density_score: u8,
    pub density_tier: String,
    pub inhibition_penalty: u8,
    pub inhibition_state: String,
    pub focus_score: u8,
    pub focus_tier: String,
    pub latest_timestamp: u64,
    pub expires_at: u64,
    pub contexts: Vec<String>,
}

#[derive(Debug, Clone)]
struct DecodedSignalTrace {
    kind: SignalPostKind,
    reinforcement: bool,
    context: String,
    message: String,
    space: Option<String>,
    expires_at: u64,
}

#[derive(Debug)]
struct SignalGroup {
    kind: SignalPostKind,
    message: String,
    space: Option<String>,
    best_similarity: f64,
    total_posts: u64,
    reinforcement_count: u32,
    latest_timestamp: u64,
    expires_at: u64,
    contexts: BTreeSet<String>,
    sources: BTreeSet<String>,
    models: BTreeSet<String>,
}

pub fn is_signal_capability(capability: &str) -> bool {
    capability.starts_with(SIGNAL_CAPABILITY_PREFIX)
        || capability.starts_with(SIGNAL_REINFORCEMENT_CAPABILITY_PREFIX)
}

pub fn create_signal_trace(
    kind: SignalPostKind,
    context: &str,
    message: &str,
    config: SignalTraceConfig,
    node_pubkey: [u8; 32],
    sign_fn: impl FnOnce(&[u8]) -> Signature,
) -> Trace {
    create_signal_trace_at(
        kind,
        context,
        message,
        config,
        now_ms(),
        node_pubkey,
        sign_fn,
    )
}

pub fn create_signal_reinforcement_trace(
    kind: SignalPostKind,
    context: &str,
    message: &str,
    config: SignalTraceConfig,
    node_pubkey: [u8; 32],
    sign_fn: impl FnOnce(&[u8]) -> Signature,
) -> Trace {
    create_signal_reinforcement_trace_at(
        kind,
        context,
        message,
        config,
        now_ms(),
        node_pubkey,
        sign_fn,
    )
}

pub fn create_auto_signal_trace(
    kind: SignalPostKind,
    context: &str,
    message: &str,
    mut config: SignalTraceConfig,
    node_pubkey: [u8; 32],
    sign_fn: impl FnOnce(&[u8]) -> Signature,
) -> Trace {
    config.model_id = AUTO_DERIVED_SIGNAL_MODEL_ID.into();
    create_signal_trace_with_capability_and_meta(
        SignalTraceInput {
            capability: kind.capability(),
            context,
            message,
            now_ms: now_ms(),
            node_pubkey,
            meta: SignalTraceMeta {
                derived_guidance_epoch: Some(DERIVED_GUIDANCE_EPOCH),
                derived_guidance_source: Some(AUTO_DERIVED_SIGNAL_SOURCE),
            },
        },
        config,
        sign_fn,
    )
}

fn create_signal_trace_at(
    kind: SignalPostKind,
    context: &str,
    message: &str,
    config: SignalTraceConfig,
    now_ms: u64,
    node_pubkey: [u8; 32],
    sign_fn: impl FnOnce(&[u8]) -> Signature,
) -> Trace {
    create_signal_trace_with_capability_and_meta(
        SignalTraceInput {
            capability: kind.capability(),
            context,
            message,
            now_ms,
            node_pubkey,
            meta: SignalTraceMeta::default(),
        },
        config,
        sign_fn,
    )
}

fn create_signal_reinforcement_trace_at(
    kind: SignalPostKind,
    context: &str,
    message: &str,
    config: SignalTraceConfig,
    now_ms: u64,
    node_pubkey: [u8; 32],
    sign_fn: impl FnOnce(&[u8]) -> Signature,
) -> Trace {
    create_signal_trace_with_capability_and_meta(
        SignalTraceInput {
            capability: kind.reinforcement_capability(),
            context,
            message,
            now_ms,
            node_pubkey,
            meta: SignalTraceMeta::default(),
        },
        config,
        sign_fn,
    )
}

#[derive(Default)]
struct SignalTraceMeta<'a> {
    derived_guidance_epoch: Option<&'a str>,
    derived_guidance_source: Option<&'a str>,
}

struct SignalTraceInput<'a> {
    capability: String,
    context: &'a str,
    message: &'a str,
    now_ms: u64,
    node_pubkey: [u8; 32],
    meta: SignalTraceMeta<'a>,
}

fn create_signal_trace_with_capability_and_meta(
    input: SignalTraceInput<'_>,
    config: SignalTraceConfig,
    sign_fn: impl FnOnce(&[u8]) -> Signature,
) -> Trace {
    let payload = SignalTracePayload {
        context: input.context.to_string(),
        message: input.message.to_string(),
        space: config.space.clone(),
        expires_at: expires_at_ms(input.now_ms, config.ttl_hours),
        derived_guidance_epoch: input.meta.derived_guidance_epoch.map(ToOwned::to_owned),
        derived_guidance_source: input.meta.derived_guidance_source.map(ToOwned::to_owned),
    };

    let mut trace = Trace::new_with_agent(
        input.capability,
        Outcome::Succeeded,
        0,
        input.message.len().min(u32::MAX as usize) as u32,
        simhash(input.context),
        Some(serde_json::to_string(&payload).expect("signal payload should serialize")),
        config.session_id,
        config.owner_account,
        config.device_identity,
        config.agent_id,
        config.sigil_id,
        config.model_id,
        input.node_pubkey,
        sign_fn,
    );
    trace.timestamp = input.now_ms;
    trace
}

pub fn is_legacy_auto_signal_trace(trace: &Trace) -> bool {
    if !is_signal_capability(&trace.capability) || trace.model_id != AUTO_DERIVED_SIGNAL_MODEL_ID {
        return false;
    }

    let Some(decoded) = decode_signal_payload(trace) else {
        return true;
    };

    !matches!(
        (
            decoded.derived_guidance_epoch.as_deref(),
            decoded.derived_guidance_source.as_deref(),
        ),
        (
            Some(DERIVED_GUIDANCE_EPOCH),
            Some(AUTO_DERIVED_SIGNAL_SOURCE)
        )
    )
}

pub fn summarize_signal_traces(
    traces: &[Trace],
    query_context: &str,
    limit: usize,
) -> Vec<SignalQueryResult> {
    let query_hash = simhash(query_context);
    let now_ms = now_ms();
    let mut groups: HashMap<(SignalPostKind, String, Option<String>), SignalGroup> = HashMap::new();

    for trace in traces {
        if is_legacy_auto_signal_trace(trace) {
            continue;
        }
        let Some(decoded) = decode_signal_trace(trace) else {
            continue;
        };
        if decoded.expires_at <= now_ms {
            continue;
        }

        let similarity_score = similarity(&query_hash, &trace.context_hash);
        let key = (decoded.kind, decoded.message.clone(), decoded.space.clone());
        let entry = groups.entry(key).or_insert_with(|| SignalGroup {
            kind: decoded.kind,
            message: decoded.message.clone(),
            space: decoded.space.clone(),
            best_similarity: similarity_score,
            total_posts: 0,
            reinforcement_count: 0,
            latest_timestamp: trace.timestamp,
            expires_at: decoded.expires_at,
            contexts: BTreeSet::new(),
            sources: BTreeSet::new(),
            models: BTreeSet::new(),
        });
        entry.best_similarity = entry.best_similarity.max(similarity_score);
        entry.latest_timestamp = entry.latest_timestamp.max(trace.timestamp);
        entry.expires_at = entry.expires_at.max(decoded.expires_at);
        if !decoded.context.is_empty() {
            entry.contexts.insert(decoded.context);
        }
        if decoded.reinforcement {
            entry.reinforcement_count = entry.reinforcement_count.saturating_add(1);
        } else {
            entry.total_posts += 1;
            entry.sources.insert(source_key(trace));
            entry.models.insert(trace.model_id.clone());
        }
    }

    let mut results: Vec<_> = groups
        .into_values()
        .filter(|group| group.total_posts > 0)
        .map(|group| {
            let source_count = group.sources.len() as u32;
            let model_count = group.models.len() as u32;
            let freshness_rank =
                signal_freshness_rank(now_ms, group.latest_timestamp, group.expires_at);
            let decay_penalty = signal_decay_penalty(source_count, model_count, freshness_rank);
            let density_score = signal_density_score(
                source_count,
                model_count,
                freshness_rank,
                group.reinforcement_count,
            )
            .saturating_sub(decay_penalty);
            SignalQueryResult {
                kind: group.kind.as_str().to_string(),
                message: group.message,
                space: group.space,
                context_similarity: round2(group.best_similarity),
                total_posts: group.total_posts,
                reinforcement_count: group.reinforcement_count,
                source_count,
                model_count,
                corroboration_tier: signal_corroboration_tier(source_count, model_count)
                    .to_string(),
                density_score,
                density_tier: signal_density_tier(density_score).to_string(),
                inhibition_penalty: 0,
                inhibition_state: "none".into(),
                latest_timestamp: group.latest_timestamp,
                expires_at: group.expires_at,
                contexts: group.contexts.into_iter().take(3).collect(),
            }
        })
        .collect();

    apply_query_avoid_inhibition(&mut results);

    results.sort_by(|a, b| {
        b.context_similarity
            .partial_cmp(&a.context_similarity)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.density_score.cmp(&a.density_score))
            .then_with(|| a.inhibition_penalty.cmp(&b.inhibition_penalty))
            .then_with(|| {
                signal_corroboration_rank(b.source_count, b.model_count)
                    .cmp(&signal_corroboration_rank(a.source_count, a.model_count))
            })
            .then_with(|| b.source_count.cmp(&a.source_count))
            .then_with(|| b.model_count.cmp(&a.model_count))
            .then_with(|| b.total_posts.cmp(&a.total_posts))
            .then_with(|| b.latest_timestamp.cmp(&a.latest_timestamp))
    });
    results.truncate(limit);
    results
}

pub fn summarize_recent_signal_feed(traces: &[Trace], limit: usize) -> Vec<SignalFeedResult> {
    let now_ms = now_ms();
    let mut groups: HashMap<(SignalPostKind, String, Option<String>), SignalGroup> = HashMap::new();

    for trace in traces {
        if is_legacy_auto_signal_trace(trace) {
            continue;
        }
        let Some(decoded) = decode_signal_trace(trace) else {
            continue;
        };
        if decoded.expires_at <= now_ms {
            continue;
        }

        let key = (decoded.kind, decoded.message.clone(), decoded.space.clone());
        let entry = groups.entry(key).or_insert_with(|| SignalGroup {
            kind: decoded.kind,
            message: decoded.message.clone(),
            space: decoded.space.clone(),
            best_similarity: 0.0,
            total_posts: 0,
            reinforcement_count: 0,
            latest_timestamp: trace.timestamp,
            expires_at: decoded.expires_at,
            contexts: BTreeSet::new(),
            sources: BTreeSet::new(),
            models: BTreeSet::new(),
        });
        entry.latest_timestamp = entry.latest_timestamp.max(trace.timestamp);
        entry.expires_at = entry.expires_at.max(decoded.expires_at);
        if !decoded.context.is_empty() {
            entry.contexts.insert(decoded.context);
        }
        if decoded.reinforcement {
            entry.reinforcement_count = entry.reinforcement_count.saturating_add(1);
        } else {
            entry.total_posts += 1;
            entry.sources.insert(source_key(trace));
            entry.models.insert(trace.model_id.clone());
        }
    }

    let mut results: Vec<_> = groups
        .into_values()
        .filter(|group| group.total_posts > 0)
        .map(|group| {
            let source_count = group.sources.len() as u32;
            let model_count = group.models.len() as u32;
            let freshness_rank =
                signal_freshness_rank(now_ms, group.latest_timestamp, group.expires_at);
            let decay_penalty = signal_decay_penalty(source_count, model_count, freshness_rank);
            let density_score = signal_density_score(
                source_count,
                model_count,
                freshness_rank,
                group.reinforcement_count,
            )
            .saturating_sub(decay_penalty);
            let focus_score = signal_focus_score(source_count, model_count, freshness_rank)
                .saturating_sub(decay_penalty);
            SignalFeedResult {
                kind: group.kind.as_str().to_string(),
                message: group.message,
                space: group.space,
                total_posts: group.total_posts,
                reinforcement_count: group.reinforcement_count,
                source_count,
                model_count,
                corroboration_tier: signal_corroboration_tier(source_count, model_count)
                    .to_string(),
                density_score,
                density_tier: signal_density_tier(density_score).to_string(),
                inhibition_penalty: 0,
                inhibition_state: "none".into(),
                focus_score,
                focus_tier: signal_focus_tier(focus_score).to_string(),
                latest_timestamp: group.latest_timestamp,
                expires_at: group.expires_at,
                contexts: group.contexts.into_iter().take(3).collect(),
            }
        })
        .collect();

    apply_feed_avoid_inhibition(&mut results);

    results.sort_by(|a, b| {
        b.focus_score
            .cmp(&a.focus_score)
            .then_with(|| b.density_score.cmp(&a.density_score))
            .then_with(|| a.inhibition_penalty.cmp(&b.inhibition_penalty))
            .then_with(|| b.source_count.cmp(&a.source_count))
            .then_with(|| b.model_count.cmp(&a.model_count))
            .then_with(|| b.latest_timestamp.cmp(&a.latest_timestamp))
            .then_with(|| b.total_posts.cmp(&a.total_posts))
    });
    results.truncate(limit);
    results
}

pub fn filter_signal_feed_results(
    results: Vec<SignalFeedResult>,
    min_sources: u32,
) -> Vec<SignalFeedResult> {
    let filtered: Vec<_> = results
        .into_iter()
        .filter(|result| result.source_count >= min_sources)
        .collect();

    let has_dense = filtered.iter().any(|result| {
        signal_density_tier(result.density_score) == "promoted"
            || signal_density_tier(result.density_score) == "dominant"
    });
    if !has_dense {
        return filtered;
    }

    let mut dense: Vec<_> = filtered
        .iter()
        .filter(|result| {
            let tier = signal_density_tier(result.density_score);
            tier == "promoted" || tier == "dominant"
        })
        .cloned()
        .collect();
    let mut background: Vec<_> = filtered
        .iter()
        .filter(|result| {
            let tier = signal_density_tier(result.density_score);
            tier != "promoted" && tier != "dominant"
        })
        .cloned()
        .collect();
    dense.append(&mut background);
    dense
}

pub fn create_query_reinforcement_traces(
    results: &[SignalQueryResult],
    query_context: &str,
    config: SignalTraceConfig,
    node_pubkey: [u8; 32],
    sign_fn: impl Fn(&[u8]) -> Signature,
) -> Vec<Trace> {
    results
        .iter()
        .filter(|result| {
            let tier = result.density_tier.as_str();
            tier == "promoted" || tier == "dominant"
        })
        .take(3)
        .filter_map(|result| {
            let kind = SignalPostKind::parse(&result.kind)?;
            let mut config = config.clone();
            config.space = result.space.clone();
            Some(create_signal_reinforcement_trace(
                kind,
                preferred_reinforcement_context(&result.contexts, query_context),
                &result.message,
                config,
                node_pubkey,
                |msg| sign_fn(msg),
            ))
        })
        .collect()
}

pub fn create_feed_reinforcement_traces(
    results: &[SignalFeedResult],
    config: SignalTraceConfig,
    node_pubkey: [u8; 32],
    sign_fn: impl Fn(&[u8]) -> Signature,
) -> Vec<Trace> {
    results
        .iter()
        .filter(|result| {
            let tier = result.density_tier.as_str();
            tier == "promoted" || tier == "dominant"
        })
        .take(3)
        .filter_map(|result| {
            let kind = SignalPostKind::parse(&result.kind)?;
            let mut config = config.clone();
            config.space = result.space.clone();
            Some(create_signal_reinforcement_trace(
                kind,
                preferred_reinforcement_context(&result.contexts, &result.message),
                &result.message,
                config,
                node_pubkey,
                |msg| sign_fn(msg),
            ))
        })
        .collect()
}

fn decode_signal_trace(trace: &Trace) -> Option<DecodedSignalTrace> {
    let (kind, reinforcement) = SignalPostKind::from_capability(&trace.capability)?;
    let payload = decode_signal_payload(trace)?;
    Some(DecodedSignalTrace {
        kind,
        reinforcement,
        context: payload.context,
        message: payload.message,
        space: payload.space,
        expires_at: payload.expires_at,
    })
}

fn decode_signal_payload(trace: &Trace) -> Option<SignalTracePayload> {
    serde_json::from_str(trace.context_text.as_deref()?).ok()
}

pub fn expires_at_ms(now_ms: u64, ttl_hours: u32) -> u64 {
    now_ms.saturating_add((ttl_hours as u64).saturating_mul(60 * 60 * 1000))
}

fn source_key(trace: &Trace) -> String {
    let node = trace.device_identity.clone().unwrap_or_else(|| {
        trace
            .node_pubkey
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    });
    match trace.session_id.as_deref() {
        Some(session_id) => format!("{node}:{session_id}"),
        None => node,
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn signal_corroboration_tier(source_count: u32, model_count: u32) -> &'static str {
    match (source_count > 1, model_count > 1) {
        (_, true) => "multi_model",
        (true, false) => "repeated_source",
        (false, false) => "single_source",
    }
}

fn signal_corroboration_rank(source_count: u32, model_count: u32) -> u8 {
    match signal_corroboration_tier(source_count, model_count) {
        "multi_model" => 2,
        "repeated_source" => 1,
        _ => 0,
    }
}

fn signal_freshness_rank(now_ms: u64, latest_timestamp: u64, expires_at: u64) -> u8 {
    let total_lifetime = expires_at.saturating_sub(latest_timestamp).max(1);
    let remaining_lifetime = expires_at.saturating_sub(now_ms);
    let freshness_ratio = remaining_lifetime as f64 / total_lifetime as f64;
    if freshness_ratio >= 0.66 {
        2
    } else if freshness_ratio >= 0.33 {
        1
    } else {
        0
    }
}

fn signal_decay_penalty(source_count: u32, model_count: u32, freshness_rank: u8) -> u8 {
    match (
        signal_corroboration_rank(source_count, model_count),
        freshness_rank,
    ) {
        (0, 0) => 2,
        (0, 1) | (1, 0) => 1,
        _ => 0,
    }
}

fn signal_focus_score(source_count: u32, model_count: u32, freshness_rank: u8) -> u8 {
    let breadth = match source_count {
        0 | 1 => 0u8,
        2 => 1,
        _ => 2,
    };
    breadth + signal_corroboration_rank(source_count, model_count) + freshness_rank
}

fn signal_density_score(
    source_count: u32,
    model_count: u32,
    freshness_rank: u8,
    reinforcement_count: u32,
) -> u8 {
    let resonance = match source_count {
        0 | 1 => 0u8,
        2 => 2,
        _ => 3,
    };
    resonance
        + signal_corroboration_rank(source_count, model_count)
        + freshness_rank
        + reinforcement_count.min(2) as u8
}

fn preferred_reinforcement_context<'a>(contexts: &'a [String], fallback: &'a str) -> &'a str {
    contexts
        .iter()
        .find(|context| !context.is_empty())
        .map(String::as_str)
        .unwrap_or(fallback)
}

fn signal_density_tier(density_score: u8) -> &'static str {
    if density_score >= 5 {
        "dominant"
    } else if density_score >= 3 {
        "promoted"
    } else if density_score >= 1 {
        "candidate"
    } else {
        "sparse"
    }
}

fn signal_focus_tier(focus_score: u8) -> &'static str {
    if focus_score >= 5 {
        "primary"
    } else if focus_score >= 3 {
        "secondary"
    } else {
        "background"
    }
}

fn apply_query_avoid_inhibition(results: &mut [SignalQueryResult]) {
    let inhibition_penalty = results
        .iter()
        .filter(|result| result.kind == SignalPostKind::Avoid.as_str())
        .filter(|result| {
            let tier = result.density_tier.as_str();
            tier == "promoted" || tier == "dominant"
        })
        .map(|result| density_inhibition_penalty(&result.density_tier))
        .max()
        .unwrap_or(0);
    if inhibition_penalty == 0 {
        return;
    }

    for result in results {
        if result.kind == SignalPostKind::Avoid.as_str() {
            continue;
        }
        if result.context_similarity < 0.85 {
            continue;
        }
        result.inhibition_penalty = inhibition_penalty;
        result.inhibition_state = inhibition_state_label(inhibition_penalty).into();
    }
}

fn apply_feed_avoid_inhibition(results: &mut [SignalFeedResult]) {
    let avoid_results: Vec<_> = results
        .iter()
        .filter(|result| result.kind == SignalPostKind::Avoid.as_str())
        .filter(|result| {
            let tier = result.density_tier.as_str();
            tier == "promoted" || tier == "dominant"
        })
        .map(|result| {
            (
                result.contexts.clone(),
                density_inhibition_penalty(&result.density_tier),
            )
        })
        .collect();

    if avoid_results.is_empty() {
        return;
    }

    for result in results {
        if result.kind == SignalPostKind::Avoid.as_str() {
            continue;
        }
        let inhibition_penalty = avoid_results
            .iter()
            .filter(|(contexts, _)| contexts_overlap(contexts, &result.contexts))
            .map(|(_, penalty)| *penalty)
            .max()
            .unwrap_or(0);
        if inhibition_penalty == 0 {
            continue;
        }
        result.inhibition_penalty = inhibition_penalty;
        result.inhibition_state = inhibition_state_label(inhibition_penalty).into();
    }
}

fn density_inhibition_penalty(density_tier: &str) -> u8 {
    match density_tier {
        "dominant" => 2,
        "promoted" => 1,
        _ => 0,
    }
}

fn inhibition_state_label(inhibition_penalty: u8) -> &'static str {
    match inhibition_penalty {
        2 => "strong",
        1 => "moderate",
        _ => "none",
    }
}

fn contexts_overlap(left: &[String], right: &[String]) -> bool {
    left.iter().any(|left_context| {
        right
            .iter()
            .any(|right_context| right_context == left_context)
    })
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::NodeIdentity;

    fn signal_config(identity: &NodeIdentity, model: &str, session_id: &str) -> SignalTraceConfig {
        SignalTraceConfig {
            model_id: model.into(),
            session_id: Some(session_id.into()),
            owner_account: None,
            device_identity: Some(identity.device_identity()),
            agent_id: None,
            sigil_id: None,
            space: None,
            ttl_hours: DEFAULT_SIGNAL_TTL_HOURS,
        }
    }

    #[test]
    fn summarize_signal_posts_groups_by_kind_and_message() {
        let identity = NodeIdentity::generate();
        let trace_a = create_signal_trace(
            SignalPostKind::Avoid,
            "fix flaky ci workflow",
            "skip the generated lockfile",
            signal_config(&identity, "codex", "session-a"),
            identity.public_key_bytes(),
            |msg| identity.sign(msg),
        );
        std::thread::sleep(std::time::Duration::from_millis(2));
        let trace_b = create_signal_trace(
            SignalPostKind::Avoid,
            "repair flaky ci pipeline",
            "skip the generated lockfile",
            signal_config(&identity, "openclaw", "session-b"),
            identity.public_key_bytes(),
            |msg| identity.sign(msg),
        );
        let trace_b_timestamp = trace_b.timestamp;

        let results = summarize_signal_traces(&[trace_a, trace_b], "fix flaky ci workflow", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].kind, "avoid");
        assert_eq!(results[0].message, "skip the generated lockfile");
        assert_eq!(results[0].total_posts, 2);
        assert_eq!(results[0].source_count, 2);
        assert_eq!(results[0].model_count, 2);
        assert_eq!(results[0].corroboration_tier, "multi_model");
        assert_eq!(results[0].density_tier, "dominant");
        assert!(results[0].expires_at >= trace_b_timestamp);
    }

    #[test]
    fn summarize_signal_posts_keeps_identical_messages_separate_across_spaces() {
        let identity = NodeIdentity::generate();
        let mut psyche_config = signal_config(&identity, "codex", "session-a");
        psyche_config.space = Some("psyche".into());
        let mut thronglets_config = signal_config(&identity, "codex", "session-b");
        thronglets_config.space = Some("thronglets".into());

        let psyche = create_signal_trace(
            SignalPostKind::Recommend,
            "repair parser regressions",
            "fix parser before UI cleanup",
            psyche_config,
            identity.public_key_bytes(),
            |msg| identity.sign(msg),
        );
        let thronglets = create_signal_trace(
            SignalPostKind::Recommend,
            "repair parser regressions",
            "fix parser before UI cleanup",
            thronglets_config,
            identity.public_key_bytes(),
            |msg| identity.sign(msg),
        );

        // Same message in different spaces → two separate signal groups
        let results = summarize_signal_traces(
            &[psyche.clone(), thronglets.clone()],
            "repair parser regressions",
            10,
        );
        assert_eq!(results.len(), 2);
        let spaces: Vec<_> = results.iter().map(|r| r.space.as_deref()).collect();
        assert!(spaces.contains(&Some("psyche")));
        assert!(spaces.contains(&Some("thronglets")));

        // When SQL pre-filters to only psyche traces, only one group remains
        let psyche_only = summarize_signal_traces(&[psyche], "repair parser regressions", 10);
        assert_eq!(psyche_only.len(), 1);
        assert_eq!(psyche_only[0].space.as_deref(), Some("psyche"));
    }

    #[test]
    fn summarize_signal_posts_ignores_expired_entries() {
        let identity = NodeIdentity::generate();
        let base_now = now_ms();
        let expired = create_signal_trace_at(
            SignalPostKind::Watch,
            "ship the current branch",
            "run release-check before push",
            SignalTraceConfig {
                model_id: "codex".into(),
                session_id: Some("session-a".into()),
                owner_account: None,
                device_identity: Some(identity.device_identity()),
                agent_id: None,
                sigil_id: None,
                space: None,
                ttl_hours: 1,
            },
            base_now.saturating_sub(3 * 60 * 60 * 1000),
            identity.public_key_bytes(),
            |msg| identity.sign(msg),
        );
        let fresh = create_signal_trace_at(
            SignalPostKind::Watch,
            "ship the current branch",
            "run release-check before push",
            SignalTraceConfig {
                model_id: "openclaw".into(),
                session_id: Some("session-b".into()),
                owner_account: None,
                device_identity: Some(identity.device_identity()),
                agent_id: None,
                sigil_id: None,
                space: None,
                ttl_hours: 24,
            },
            base_now,
            identity.public_key_bytes(),
            |msg| identity.sign(msg),
        );

        let results = summarize_signal_traces(&[expired, fresh], "ship the current branch", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].kind, "watch");
        assert_eq!(results[0].total_posts, 1);
    }

    #[test]
    fn summarize_signal_posts_ignores_legacy_auto_signals() {
        let identity = NodeIdentity::generate();
        let legacy = create_signal_trace(
            SignalPostKind::Recommend,
            "repair release flow",
            "stable path: stale derived guidance",
            signal_config(&identity, AUTO_DERIVED_SIGNAL_MODEL_ID, "legacy-auto"),
            identity.public_key_bytes(),
            |msg| identity.sign(msg),
        );
        assert!(is_legacy_auto_signal_trace(&legacy));

        let current = create_auto_signal_trace(
            SignalPostKind::Recommend,
            "repair release flow",
            "stable path: current derived guidance",
            signal_config(&identity, "ignored", "current-auto"),
            identity.public_key_bytes(),
            |msg| identity.sign(msg),
        );
        assert!(!is_legacy_auto_signal_trace(&current));

        let results = summarize_signal_traces(&[legacy, current], "repair release flow", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].message, "stable path: current derived guidance");
    }

    #[test]
    fn summarize_signal_posts_multi_source_increases_density() {
        let identity_a = NodeIdentity::generate();
        let identity_b = NodeIdentity::generate();

        let trace_a = create_signal_trace(
            SignalPostKind::Recommend,
            "repair release flow",
            "run release-check before push",
            signal_config(&identity_a, "codex", "session-a"),
            identity_a.public_key_bytes(),
            |msg| identity_a.sign(msg),
        );
        let trace_b = create_signal_trace(
            SignalPostKind::Recommend,
            "repair release flow",
            "run release-check before push",
            signal_config(&identity_b, "openclaw", "session-b"),
            identity_b.public_key_bytes(),
            |msg| identity_b.sign(msg),
        );

        let results = summarize_signal_traces(&[trace_a, trace_b], "repair release flow", 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].source_count, 2);
        assert_eq!(results[0].model_count, 2);
        assert_eq!(results[0].corroboration_tier, "multi_model");
        assert_eq!(results[0].density_tier, "dominant");
    }

    #[test]
    fn summarize_signal_posts_reinforcement_increases_density_without_new_sources() {
        let identity = NodeIdentity::generate();
        let signal = create_signal_trace(
            SignalPostKind::Recommend,
            "repair release flow",
            "run release-check before push",
            signal_config(&identity, "codex", "session-a"),
            identity.public_key_bytes(),
            |msg| identity.sign(msg),
        );
        let reinforcement_a = create_signal_reinforcement_trace(
            SignalPostKind::Recommend,
            "repair release flow",
            "run release-check before push",
            SignalTraceConfig {
                model_id: "thronglets-query".into(),
                session_id: None,
                owner_account: None,
                device_identity: Some(identity.device_identity()),
                agent_id: None,
                sigil_id: None,
                space: None,
                ttl_hours: DEFAULT_SIGNAL_REINFORCEMENT_TTL_HOURS,
            },
            identity.public_key_bytes(),
            |msg| identity.sign(msg),
        );
        let reinforcement_b = create_signal_reinforcement_trace(
            SignalPostKind::Recommend,
            "repair release flow",
            "run release-check before push",
            SignalTraceConfig {
                model_id: "thronglets-feed".into(),
                session_id: None,
                owner_account: None,
                device_identity: Some(identity.device_identity()),
                agent_id: None,
                sigil_id: None,
                space: None,
                ttl_hours: DEFAULT_SIGNAL_REINFORCEMENT_TTL_HOURS,
            },
            identity.public_key_bytes(),
            |msg| identity.sign(msg),
        );

        let results = summarize_signal_traces(
            &[signal, reinforcement_a, reinforcement_b],
            "repair release flow",
            10,
        );
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].total_posts, 1);
        assert_eq!(results[0].reinforcement_count, 2);
        assert_eq!(results[0].source_count, 1);
        assert_eq!(results[0].model_count, 1);
        assert_eq!(results[0].density_tier, "promoted");
    }

    #[test]
    fn summarize_signal_posts_aging_single_source_reinforcement_evaporates() {
        let identity = NodeIdentity::generate();
        let now = now_ms();
        let stale_now = now.saturating_sub(50 * 60 * 1000);
        let signal = create_signal_trace_at(
            SignalPostKind::Recommend,
            "repair release flow",
            "run release-check before push",
            SignalTraceConfig {
                model_id: "codex".into(),
                session_id: None,
                owner_account: None,
                device_identity: Some(identity.device_identity()),
                agent_id: None,
                sigil_id: None,
                space: None,
                ttl_hours: 1,
            },
            stale_now,
            identity.public_key_bytes(),
            |msg| identity.sign(msg),
        );
        let reinforcement_a = create_signal_reinforcement_trace_at(
            SignalPostKind::Recommend,
            "repair release flow",
            "run release-check before push",
            SignalTraceConfig {
                model_id: "thronglets-query".into(),
                session_id: None,
                owner_account: None,
                device_identity: Some(identity.device_identity()),
                agent_id: None,
                sigil_id: None,
                space: None,
                ttl_hours: 1,
            },
            stale_now,
            identity.public_key_bytes(),
            |msg| identity.sign(msg),
        );
        let reinforcement_b = create_signal_reinforcement_trace_at(
            SignalPostKind::Recommend,
            "repair release flow",
            "run release-check before push",
            SignalTraceConfig {
                model_id: "thronglets-feed".into(),
                session_id: None,
                owner_account: None,
                device_identity: Some(identity.device_identity()),
                agent_id: None,
                sigil_id: None,
                space: None,
                ttl_hours: 1,
            },
            stale_now,
            identity.public_key_bytes(),
            |msg| identity.sign(msg),
        );

        let results = summarize_signal_traces(
            &[signal, reinforcement_a, reinforcement_b],
            "repair release flow",
            10,
        );
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].density_tier, "sparse");
    }

    #[test]
    fn summarize_signal_posts_ignores_reinforcement_without_base_signal() {
        let identity = NodeIdentity::generate();
        let reinforcement = create_signal_reinforcement_trace(
            SignalPostKind::Watch,
            "ship the current branch",
            "run release-check before push",
            SignalTraceConfig {
                model_id: "thronglets-query".into(),
                session_id: None,
                owner_account: None,
                device_identity: Some(identity.device_identity()),
                agent_id: None,
                sigil_id: None,
                space: None,
                ttl_hours: DEFAULT_SIGNAL_REINFORCEMENT_TTL_HOURS,
            },
            identity.public_key_bytes(),
            |msg| identity.sign(msg),
        );

        let results = summarize_signal_traces(&[reinforcement], "ship the current branch", 10);
        assert!(results.is_empty());
    }

    #[test]
    fn summarize_signal_posts_promoted_avoid_inhibits_competing_recommendation() {
        let remote_a = NodeIdentity::generate();
        let remote_b = NodeIdentity::generate();

        let recommend = create_signal_trace(
            SignalPostKind::Recommend,
            "repair release flow",
            "run release-check before push",
            signal_config(&remote_a, "codex", "remote-a"),
            remote_a.public_key_bytes(),
            |msg| remote_a.sign(msg),
        );
        let avoid_a = create_signal_trace(
            SignalPostKind::Avoid,
            "repair release flow",
            "skip the generated lockfile",
            signal_config(&remote_a, "openclaw", "remote-b"),
            remote_a.public_key_bytes(),
            |msg| remote_a.sign(msg),
        );
        let avoid_b = create_signal_trace(
            SignalPostKind::Avoid,
            "repair release flow",
            "skip the generated lockfile",
            signal_config(&remote_b, "claude", "remote-c"),
            remote_b.public_key_bytes(),
            |msg| remote_b.sign(msg),
        );

        let results =
            summarize_signal_traces(&[recommend, avoid_a, avoid_b], "repair release flow", 10);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].kind, "avoid");
        assert_eq!(results[0].inhibition_state, "none");
        assert_eq!(results[1].kind, "recommend");
        assert_eq!(results[1].inhibition_state, "strong");
        assert_eq!(results[1].inhibition_penalty, 2);
    }

    #[test]
    fn summarize_recent_signal_feed_prioritizes_multi_source_support() {
        let local_identity = NodeIdentity::generate();
        let remote_a = NodeIdentity::generate();
        let remote_b = NodeIdentity::generate();

        let local_signal = create_signal_trace(
            SignalPostKind::Info,
            "repair release flow",
            "update changelog before tagging",
            signal_config(&local_identity, "codex", "local-1"),
            local_identity.public_key_bytes(),
            |msg| local_identity.sign(msg),
        );
        let collective_a = create_signal_trace(
            SignalPostKind::Recommend,
            "repair release flow",
            "run release-check before push",
            signal_config(&remote_a, "openclaw", "remote-a"),
            remote_a.public_key_bytes(),
            |msg| remote_a.sign(msg),
        );
        let collective_b = create_signal_trace(
            SignalPostKind::Recommend,
            "ship the current branch",
            "run release-check before push",
            signal_config(&remote_b, "claude", "remote-b"),
            remote_b.public_key_bytes(),
            |msg| remote_b.sign(msg),
        );

        let results = summarize_recent_signal_feed(&[local_signal, collective_a, collective_b], 10);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].message, "run release-check before push");
        assert_eq!(results[0].source_count, 2);
        assert_eq!(results[0].model_count, 2);
        assert_eq!(results[0].corroboration_tier, "multi_model");
        assert_eq!(results[0].density_tier, "dominant");
        assert_eq!(results[0].focus_tier, "primary");
    }

    #[test]
    fn summarize_recent_signal_feed_filters_to_requested_space() {
        let identity = NodeIdentity::generate();
        let mut psyche_config = signal_config(&identity, "codex", "session-a");
        psyche_config.space = Some("psyche".into());
        let mut core_config = signal_config(&identity, "codex", "session-b");
        core_config.space = Some("core".into());

        let _psyche = create_signal_trace(
            SignalPostKind::Watch,
            "repair parser regressions",
            "watch the parser panic path",
            psyche_config,
            identity.public_key_bytes(),
            |msg| identity.sign(msg),
        );
        let core = create_signal_trace(
            SignalPostKind::Watch,
            "repair parser regressions",
            "watch the parser panic path",
            core_config,
            identity.public_key_bytes(),
            |msg| identity.sign(msg),
        );

        // SQL-level space filter would return only core traces
        let results = summarize_recent_signal_feed(&[core], 10);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].space.as_deref(), Some("core"));
    }

    #[test]
    fn summarize_recent_signal_feed_prefers_multi_model_support_when_counts_tie() {
        let remote_a = NodeIdentity::generate();
        let remote_b = NodeIdentity::generate();
        let remote_c = NodeIdentity::generate();
        let remote_d = NodeIdentity::generate();
        let base_now = now_ms();

        let multi_model_a = create_signal_trace_at(
            SignalPostKind::Recommend,
            "repair release flow",
            "run release-check before push",
            signal_config(&remote_a, "codex", "remote-a"),
            base_now,
            remote_a.public_key_bytes(),
            |msg| remote_a.sign(msg),
        );
        let multi_model_b = create_signal_trace_at(
            SignalPostKind::Recommend,
            "repair release flow",
            "run release-check before push",
            signal_config(&remote_b, "openclaw", "remote-b"),
            base_now,
            remote_b.public_key_bytes(),
            |msg| remote_b.sign(msg),
        );
        let single_model_a = create_signal_trace_at(
            SignalPostKind::Watch,
            "repair release flow",
            "rerun the targeted test first",
            signal_config(&remote_c, "codex", "remote-c"),
            base_now,
            remote_c.public_key_bytes(),
            |msg| remote_c.sign(msg),
        );
        let single_model_b = create_signal_trace_at(
            SignalPostKind::Watch,
            "repair release flow",
            "rerun the targeted test first",
            signal_config(&remote_d, "codex", "remote-d"),
            base_now,
            remote_d.public_key_bytes(),
            |msg| remote_d.sign(msg),
        );

        let results = summarize_recent_signal_feed(
            &[single_model_a, single_model_b, multi_model_a, multi_model_b],
            10,
        );
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].message, "run release-check before push");
        assert_eq!(results[0].source_count, 2);
        assert_eq!(results[0].model_count, 2);
        assert_eq!(results[0].corroboration_tier, "multi_model");
        assert_eq!(results[0].density_tier, "dominant");
        assert_eq!(results[0].focus_tier, "primary");
        assert_eq!(results[1].message, "rerun the targeted test first");
        assert_eq!(results[1].source_count, 2);
        assert_eq!(results[1].model_count, 1);
        assert_eq!(results[1].corroboration_tier, "repeated_source");
        assert_eq!(results[1].density_tier, "dominant");
        // 2 sources + 1 model + fresh: breadth(1) + corr(1) + fresh(2) = 4 → "secondary"
        assert_eq!(results[1].focus_tier, "secondary");
    }

    #[test]
    fn summarize_recent_signal_feed_prefers_multi_model_over_repeated_source() {
        let local_identity = NodeIdentity::generate();
        let remote_a = NodeIdentity::generate();
        let remote_b = NodeIdentity::generate();
        let remote_c = NodeIdentity::generate();
        let remote_d = NodeIdentity::generate();
        let base_now = now_ms();

        let multi_model_a = create_signal_trace_at(
            SignalPostKind::Recommend,
            "repair release flow",
            "run release-check before push",
            signal_config(&remote_a, "codex", "remote-a"),
            base_now,
            remote_a.public_key_bytes(),
            |msg| remote_a.sign(msg),
        );
        let multi_model_b = create_signal_trace_at(
            SignalPostKind::Recommend,
            "repair release flow",
            "run release-check before push",
            signal_config(&remote_b, "openclaw", "remote-b"),
            base_now,
            remote_b.public_key_bytes(),
            |msg| remote_b.sign(msg),
        );
        let repeated_a = create_signal_trace_at(
            SignalPostKind::Watch,
            "repair release flow",
            "rerun the targeted test first",
            signal_config(&remote_c, "codex", "remote-c"),
            base_now,
            remote_c.public_key_bytes(),
            |msg| remote_c.sign(msg),
        );
        let repeated_b = create_signal_trace_at(
            SignalPostKind::Watch,
            "repair release flow",
            "rerun the targeted test first",
            signal_config(&remote_d, "codex", "remote-d"),
            base_now,
            remote_d.public_key_bytes(),
            |msg| remote_d.sign(msg),
        );
        let repeated_c = create_signal_trace_at(
            SignalPostKind::Watch,
            "repair release flow",
            "rerun the targeted test first",
            signal_config(&local_identity, "codex", "local-1"),
            base_now,
            local_identity.public_key_bytes(),
            |msg| local_identity.sign(msg),
        );

        let results = summarize_recent_signal_feed(
            &[
                repeated_a,
                repeated_b,
                repeated_c,
                multi_model_a,
                multi_model_b,
            ],
            10,
        );
        assert_eq!(results.len(), 2);
        // With pure density formula, 3 sources (repeated) outranks 2 sources (multi_model)
        // because resonance(3)=3 + corr(1) + fresh(2) = 6 vs resonance(2)=2 + corr(2) + fresh(2) = 6
        // tie-broken by source_count: 3 > 2
        assert_eq!(results[0].message, "rerun the targeted test first");
        assert_eq!(results[0].corroboration_tier, "repeated_source");
        assert_eq!(results[0].density_tier, "dominant");
        assert_eq!(results[1].message, "run release-check before push");
        assert_eq!(results[1].corroboration_tier, "multi_model");
        assert_eq!(results[1].density_tier, "dominant");
        assert!(results[0].source_count > results[1].source_count);
    }

    #[test]
    fn summarize_recent_signal_feed_prefers_fresher_signal_when_support_is_close() {
        let remote_a = NodeIdentity::generate();
        let remote_b = NodeIdentity::generate();
        let remote_c = NodeIdentity::generate();
        let remote_d = NodeIdentity::generate();
        let remote_e = NodeIdentity::generate();
        let base_now = now_ms();
        let old_now = base_now.saturating_sub(50 * 60 * 60 * 1000);

        let old_a = create_signal_trace_at(
            SignalPostKind::Recommend,
            "repair release flow",
            "run release-check before push",
            signal_config(&remote_a, "codex", "remote-a"),
            old_now,
            remote_a.public_key_bytes(),
            |msg| remote_a.sign(msg),
        );
        let old_b = create_signal_trace_at(
            SignalPostKind::Recommend,
            "repair release flow",
            "run release-check before push",
            signal_config(&remote_b, "openclaw", "remote-b"),
            old_now,
            remote_b.public_key_bytes(),
            |msg| remote_b.sign(msg),
        );
        let old_c = create_signal_trace_at(
            SignalPostKind::Recommend,
            "repair release flow",
            "run release-check before push",
            signal_config(&remote_e, "claude", "remote-old-c"),
            old_now,
            remote_e.public_key_bytes(),
            |msg| remote_e.sign(msg),
        );

        let fresh_a = create_signal_trace_at(
            SignalPostKind::Watch,
            "repair release flow",
            "rerun the targeted test first",
            signal_config(&remote_c, "codex", "remote-c"),
            base_now,
            remote_c.public_key_bytes(),
            |msg| remote_c.sign(msg),
        );
        let fresh_b = create_signal_trace_at(
            SignalPostKind::Watch,
            "repair release flow",
            "rerun the targeted test first",
            signal_config(&remote_d, "openclaw", "remote-d"),
            base_now,
            remote_d.public_key_bytes(),
            |msg| remote_d.sign(msg),
        );
        let fresh_c = create_signal_trace_at(
            SignalPostKind::Watch,
            "repair release flow",
            "rerun the targeted test first",
            signal_config(&remote_e, "claude", "remote-e"),
            base_now,
            remote_e.public_key_bytes(),
            |msg| remote_e.sign(msg),
        );

        let results =
            summarize_recent_signal_feed(&[old_a, old_b, old_c, fresh_a, fresh_b, fresh_c], 10);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].message, "rerun the targeted test first");
        assert_eq!(results[0].corroboration_tier, "multi_model");
        assert_eq!(results[0].density_tier, "dominant");
        assert_eq!(results[0].focus_tier, "primary");
        assert_eq!(results[1].message, "run release-check before push");
        assert_eq!(results[1].corroboration_tier, "multi_model");
        assert_eq!(results[1].density_tier, "dominant");
        assert_eq!(results[1].focus_tier, "secondary");
        assert_eq!(results[0].source_count, results[1].source_count);
    }

    #[test]
    fn summarize_recent_signal_feed_inhibits_competing_signal_with_shared_context() {
        let remote_a = NodeIdentity::generate();
        let remote_b = NodeIdentity::generate();
        let remote_c = NodeIdentity::generate();

        let recommend = create_signal_trace(
            SignalPostKind::Recommend,
            "repair release flow",
            "run release-check before push",
            signal_config(&remote_a, "codex", "remote-a"),
            remote_a.public_key_bytes(),
            |msg| remote_a.sign(msg),
        );
        let avoid_a = create_signal_trace(
            SignalPostKind::Avoid,
            "repair release flow",
            "skip the generated lockfile",
            signal_config(&remote_b, "openclaw", "remote-b"),
            remote_b.public_key_bytes(),
            |msg| remote_b.sign(msg),
        );
        let avoid_b = create_signal_trace(
            SignalPostKind::Avoid,
            "repair release flow",
            "skip the generated lockfile",
            signal_config(&remote_c, "claude", "remote-c"),
            remote_c.public_key_bytes(),
            |msg| remote_c.sign(msg),
        );

        let results = summarize_recent_signal_feed(&[recommend, avoid_a, avoid_b], 10);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].kind, "avoid");
        assert_eq!(results[1].kind, "recommend");
        assert_eq!(results[1].inhibition_state, "strong");
        assert_eq!(results[1].inhibition_penalty, 2);
    }

    #[test]
    fn filter_signal_feed_results_by_min_sources() {
        let results = vec![
            SignalFeedResult {
                kind: "recommend".into(),
                message: "single source".into(),
                space: None,
                total_posts: 1,
                reinforcement_count: 0,
                source_count: 1,
                model_count: 1,
                corroboration_tier: "single_source".into(),
                density_score: 0,
                density_tier: "sparse".into(),
                inhibition_penalty: 0,
                inhibition_state: "none".into(),
                focus_score: 0,
                focus_tier: "background".into(),
                latest_timestamp: 1,
                expires_at: 2,
                contexts: vec!["ctx".into()],
            },
            SignalFeedResult {
                kind: "recommend".into(),
                message: "multi source".into(),
                space: None,
                total_posts: 2,
                reinforcement_count: 0,
                source_count: 2,
                model_count: 2,
                corroboration_tier: "multi_model".into(),
                density_score: 6,
                density_tier: "dominant".into(),
                inhibition_penalty: 0,
                inhibition_state: "none".into(),
                focus_score: 6,
                focus_tier: "primary".into(),
                latest_timestamp: 2,
                expires_at: 3,
                contexts: vec!["ctx".into()],
            },
        ];

        let filtered = filter_signal_feed_results(results, 2);
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].message, "multi source");
    }

    #[test]
    fn signal_corroboration_tier_distinguishes_single_repeated_and_multi_model() {
        assert_eq!(signal_corroboration_tier(1, 1), "single_source");
        assert_eq!(signal_corroboration_tier(2, 1), "repeated_source");
        assert_eq!(signal_corroboration_tier(1, 2), "multi_model");
        assert_eq!(signal_corroboration_tier(3, 2), "multi_model");
    }

    #[test]
    fn signal_corroboration_rank_prefers_multi_model() {
        assert!(signal_corroboration_rank(1, 2) > signal_corroboration_rank(3, 1));
        assert!(signal_corroboration_rank(2, 1) > signal_corroboration_rank(1, 1));
    }

    #[test]
    fn signal_freshness_rank_prefers_newer_signals() {
        let now = now_ms();
        let recent_latest = now.saturating_sub(6 * 60 * 60 * 1000);
        let stale_latest = now.saturating_sub(50 * 60 * 60 * 1000);
        let expires = now.saturating_add(22 * 60 * 60 * 1000);
        assert!(
            signal_freshness_rank(now, recent_latest, now + 72 * 60 * 60 * 1000)
                > signal_freshness_rank(now, stale_latest, stale_latest + 72 * 60 * 60 * 1000)
        );
        assert!(signal_freshness_rank(now, now, expires) >= 1);
    }

    #[test]
    fn signal_decay_penalty_hits_weak_aging_signals_first() {
        assert_eq!(signal_decay_penalty(1, 1, 0), 2);
        assert_eq!(signal_decay_penalty(1, 1, 1), 1);
        assert_eq!(signal_decay_penalty(2, 1, 0), 1);
        assert_eq!(signal_decay_penalty(1, 2, 0), 0);
    }

    #[test]
    fn signal_focus_tier_prefers_multi_source_multi_model() {
        assert_eq!(signal_focus_tier(signal_focus_score(3, 2, 2)), "primary");
        assert_eq!(signal_focus_tier(signal_focus_score(2, 1, 1)), "secondary");
        assert_eq!(signal_focus_tier(signal_focus_score(1, 1, 0)), "background");
    }

    #[test]
    fn signal_density_tier_moves_from_sparse_to_dominant() {
        // 1 source, 1 model, stale, no reinforcement → resonance=0, corr=0, fresh=0, reinf=0 = 0
        assert_eq!(
            signal_density_tier(signal_density_score(1, 1, 0, 0)),
            "sparse"
        );
        // 1 source, 1 model, fresh, no reinforcement → 0+0+2+0 = 2
        assert_eq!(
            signal_density_tier(signal_density_score(1, 1, 2, 0)),
            "candidate"
        );
        // 2 sources, 2 models, stale, no reinforcement → 2+2+0+0 = 4
        assert_eq!(
            signal_density_tier(signal_density_score(2, 2, 0, 0)),
            "promoted"
        );
        // 2 sources, 2 models, fresh, no reinforcement → 2+2+2+0 = 6
        assert_eq!(
            signal_density_tier(signal_density_score(2, 2, 2, 0)),
            "dominant"
        );
    }
}
