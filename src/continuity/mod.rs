use crate::context::simhash;
use crate::identity::NodeIdentity;
use crate::posts::SignalPostKind;
use crate::posts::{DEFAULT_SIGNAL_TTL_HOURS, SignalTraceConfig, create_signal_trace};
use crate::storage::TraceStore;
use crate::trace::{Outcome, Trace};
use ed25519_dalek::Signature;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};

pub const CONTINUITY_CAPABILITY_PREFIX: &str = "urn:thronglets:continuity:v1:";
pub const EXTERNAL_CONTINUITY_PROVIDER: &str = "thronglets";
pub const EXTERNAL_CONTINUITY_MODE: &str = "optional";
pub const EXTERNAL_CONTINUITY_VERSION: u32 = 1;
pub const CONTINUITY_SUMMARY_HORIZON_HOURS: u32 = 168;
pub const CONTINUITY_RULESET_VERSION: u32 = 1;

const COORDINATION_TTL_HOURS: u32 = 72;
const CONTINUITY_TTL_HOURS: u32 = 168;
const CALIBRATION_TTL_HOURS: u32 = 168;
const STABLE_MIN_AGE_MS: u64 = 2 * 3_600_000;
const STABLE_MIN_TRACE_COUNT: u32 = 2;
const OPEN_LOOP_SIGNAL_MIN_AGE_MS: u64 = 3_600_000;
const OPEN_LOOP_SIGNAL_MIN_TRACE_COUNT: u32 = 2;
const CALIBRATION_REPEATED_FAILURE_THRESHOLD: u32 = 2;

const RULE_RELATION_INFO: &str = "relation-milestone.stable-info";
const RULE_RELATION_WATCH: &str = "relation-milestone.stable-auditable-watch";
const RULE_OPEN_LOOP_WATCH: &str = "open-loop-anchor.repeated-or-aged-watch";
const RULE_CONTINUITY_INFO: &str = "continuity-anchor.stable-auditable-info";
const RULE_WRITEBACK_AVOID: &str = "writeback-calibration.repeated-failure-avoid";

const RULE_RELATION_SUMMARY: &str = "relation-milestone.stable-auditable-summary";
const RULE_OPEN_LOOP_SUMMARY: &str = "open-loop-anchor.stable-auditable-summary";
const RULE_CONTINUITY_SUMMARY: &str = "continuity-anchor.stable-auditable-summary";
const RULE_WRITEBACK_SUMMARY: &str = "writeback-calibration.repeated-failure-summary";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContinuityTaxonomy {
    Coordination,
    Continuity,
    Calibration,
}

impl ContinuityTaxonomy {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Coordination => "coordination",
            Self::Continuity => "continuity",
            Self::Calibration => "calibration",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ContinuityEvent {
    RelationMilestone,
    WritebackCalibration,
    ContinuityAnchor,
    OpenLoopAnchor,
}

impl ContinuityEvent {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RelationMilestone => "relation-milestone",
            Self::WritebackCalibration => "writeback-calibration",
            Self::ContinuityAnchor => "continuity-anchor",
            Self::OpenLoopAnchor => "open-loop-anchor",
        }
    }

    fn expected_taxonomy(self) -> ContinuityTaxonomy {
        match self {
            Self::RelationMilestone | Self::OpenLoopAnchor => ContinuityTaxonomy::Coordination,
            Self::ContinuityAnchor => ContinuityTaxonomy::Continuity,
            Self::WritebackCalibration => ContinuityTaxonomy::Calibration,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExternalContinuityInput {
    pub provider: String,
    pub mode: String,
    pub version: u32,
    pub taxonomy: ContinuityTaxonomy,
    pub event: ContinuityEvent,
    pub summary: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub space: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audit_ref: Option<String>,
}

impl ExternalContinuityInput {
    pub fn validate(&self) -> Result<(), String> {
        if self.provider != EXTERNAL_CONTINUITY_PROVIDER {
            return Err(format!(
                "external_continuity.provider must be `{EXTERNAL_CONTINUITY_PROVIDER}`"
            ));
        }
        if self.mode != EXTERNAL_CONTINUITY_MODE {
            return Err(format!(
                "external_continuity.mode must be `{EXTERNAL_CONTINUITY_MODE}`"
            ));
        }
        if self.version != EXTERNAL_CONTINUITY_VERSION {
            return Err(format!(
                "external_continuity.version must be {}",
                EXTERNAL_CONTINUITY_VERSION
            ));
        }
        if self.summary.trim().is_empty() {
            return Err("external_continuity.summary must not be empty".into());
        }
        if self.taxonomy != self.event.expected_taxonomy() {
            return Err(format!(
                "external_continuity.taxonomy must be `{}` for `{}`",
                self.event.expected_taxonomy().as_str(),
                self.event.as_str()
            ));
        }
        Ok(())
    }

    pub fn ttl_hours(&self) -> u32 {
        retention_hours_for_taxonomy(self.taxonomy)
    }

    pub fn capability(&self) -> String {
        format!(
            "{CONTINUITY_CAPABILITY_PREFIX}{}:{}",
            self.taxonomy.as_str(),
            self.event.as_str()
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ContinuityTracePayload {
    provider: String,
    mode: String,
    version: u32,
    taxonomy: ContinuityTaxonomy,
    event: ContinuityEvent,
    summary: String,
    space: Option<String>,
    audit_ref: Option<String>,
    expires_at: u64,
}

#[derive(Debug, Clone)]
pub struct DecodedContinuityTrace {
    pub taxonomy: ContinuityTaxonomy,
    pub event: ContinuityEvent,
    pub summary: String,
    pub space: Option<String>,
    pub audit_ref: Option<String>,
    pub expires_at: u64,
    pub timestamp: u64,
    pub outcome: Outcome,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct ContinuityDerivedSignal {
    pub kind: String,
    pub message: String,
    pub space: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContinuityTraceSummary {
    pub taxonomy: String,
    pub event: String,
    pub summary: String,
    pub space: Option<String>,
    pub audit_ref: Option<String>,
    pub trace_count: u32,
    pub session_count: u32,
    pub failed_count: u32,
    pub latest_timestamp: u64,
    pub expires_at: u64,
    pub local_only: bool,
    pub derived_signal: Option<ContinuityDerivedSignal>,
    pub net_summary_candidate: bool,
    pub runtime: ContinuityRuntimeDisposition,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContinuityNetSummaryCandidate {
    pub taxonomy: String,
    pub event: String,
    pub summary: String,
    pub space: Option<String>,
    pub audit_ref: Option<String>,
    pub trace_count: u32,
    pub session_count: u32,
    pub latest_timestamp: u64,
    pub reason: String,
    pub trigger_rule: &'static str,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContinuityRuleSet {
    pub version: u32,
    pub retention_hours: ContinuityRetentionHours,
    pub stable_min_age_hours: u32,
    pub stable_min_trace_count: u32,
    pub open_loop_watch_min_age_hours: u32,
    pub open_loop_watch_min_trace_count: u32,
    pub calibration_repeated_failure_threshold: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContinuityRetentionHours {
    pub coordination: u32,
    pub continuity: u32,
    pub calibration: u32,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContinuityRuntimeDisposition {
    pub state: &'static str,
    pub local_retention_hours: u32,
    pub stable_evidence: bool,
    pub auditable_evidence: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub derived_signal_rule: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary_candidate_rule: Option<&'static str>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContinuitySnapshotSummary {
    pub trace_count: usize,
    pub derived_signal_count: usize,
    pub net_summary_candidate_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContinuitySpaceData {
    pub rules: ContinuityRuleSet,
    pub summary: ContinuitySnapshotSummary,
    pub traces: Vec<ContinuityTraceSummary>,
    pub net_summary_candidates: Vec<ContinuityNetSummaryCandidate>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ContinuityRecordData {
    pub local_only_raw: bool,
    pub taxonomy: String,
    pub event: String,
    pub space: Option<String>,
    pub expires_at: u64,
    pub derived_signal: Option<ContinuityDerivedSignal>,
    pub net_summary_candidate: bool,
    pub runtime: ContinuityRuntimeDisposition,
}

#[derive(Debug, Clone, Serialize)]
pub struct ExternalContinuityRecordResult {
    pub trace_id: String,
    pub capability: String,
    pub external_continuity: Option<ContinuityRecordData>,
}

#[derive(Debug, Clone)]
struct ContinuityGroup {
    taxonomy: ContinuityTaxonomy,
    event: ContinuityEvent,
    summary: String,
    space: Option<String>,
    audit_ref: Option<String>,
    trace_count: u32,
    failed_count: u32,
    earliest_timestamp: u64,
    latest_timestamp: u64,
    expires_at: u64,
    sessions: BTreeSet<String>,
}

pub fn is_continuity_capability(capability: &str) -> bool {
    capability.starts_with(CONTINUITY_CAPABILITY_PREFIX)
}

#[allow(clippy::too_many_arguments)]
pub fn create_external_continuity_trace(
    input: &ExternalContinuityInput,
    outcome: Outcome,
    model_id: String,
    session_id: Option<String>,
    owner_account: Option<String>,
    device_identity: Option<String>,
    node_pubkey: [u8; 32],
    sign_fn: impl FnOnce(&[u8]) -> Signature,
) -> Trace {
    let now_ms = now_ms();
    let payload = ContinuityTracePayload {
        provider: input.provider.clone(),
        mode: input.mode.clone(),
        version: input.version,
        taxonomy: input.taxonomy,
        event: input.event,
        summary: input.summary.trim().to_string(),
        space: input.space.clone(),
        audit_ref: input.audit_ref.clone(),
        expires_at: expires_at_ms(now_ms, input.ttl_hours()),
    };
    let context_for_hash = format!(
        "{} {} {}",
        payload.taxonomy.as_str(),
        payload.space.clone().unwrap_or_default(),
        payload.summary
    );
    let mut trace = Trace::new_with_identity(
        input.capability(),
        outcome,
        0,
        payload.summary.len().min(u32::MAX as usize) as u32,
        simhash(&context_for_hash),
        Some(serde_json::to_string(&payload).expect("continuity payload should serialize")),
        session_id,
        owner_account,
        device_identity,
        model_id,
        node_pubkey,
        sign_fn,
    );
    trace.timestamp = now_ms;
    trace
}

pub fn decode_continuity_trace(trace: &Trace) -> Option<DecodedContinuityTrace> {
    let capability = trace
        .capability
        .strip_prefix(CONTINUITY_CAPABILITY_PREFIX)?;
    let (taxonomy_str, event_str) = capability.split_once(':')?;
    let payload: ContinuityTracePayload =
        serde_json::from_str(trace.context_text.as_deref()?).ok()?;
    if taxonomy_str != payload.taxonomy.as_str() || event_str != payload.event.as_str() {
        return None;
    }
    Some(DecodedContinuityTrace {
        taxonomy: payload.taxonomy,
        event: payload.event,
        summary: payload.summary,
        space: payload.space,
        audit_ref: payload.audit_ref,
        expires_at: payload.expires_at,
        timestamp: trace.timestamp,
        outcome: trace.outcome,
        session_id: trace.session_id.clone(),
    })
}

pub fn summarize_recent_continuity(
    traces: &[Trace],
    space: Option<&str>,
    limit: usize,
) -> ContinuitySpaceData {
    let now_ms = now_ms();
    let mut groups: HashMap<
        (ContinuityTaxonomy, ContinuityEvent, String, Option<String>),
        ContinuityGroup,
    > = HashMap::new();

    for trace in traces {
        let Some(decoded) = decode_continuity_trace(trace) else {
            continue;
        };
        if decoded.expires_at <= now_ms {
            continue;
        }
        if !matches_space(decoded.space.as_deref(), space) {
            continue;
        }
        let key = (
            decoded.taxonomy,
            decoded.event,
            decoded.summary.clone(),
            decoded.space.clone(),
        );
        let entry = groups.entry(key).or_insert_with(|| ContinuityGroup {
            taxonomy: decoded.taxonomy,
            event: decoded.event,
            summary: decoded.summary.clone(),
            space: decoded.space.clone(),
            audit_ref: decoded.audit_ref.clone(),
            trace_count: 0,
            failed_count: 0,
            earliest_timestamp: decoded.timestamp,
            latest_timestamp: decoded.timestamp,
            expires_at: decoded.expires_at,
            sessions: BTreeSet::new(),
        });
        entry.trace_count = entry.trace_count.saturating_add(1);
        if matches!(
            decoded.outcome,
            Outcome::Failed | Outcome::Partial | Outcome::Timeout
        ) {
            entry.failed_count = entry.failed_count.saturating_add(1);
        }
        entry.earliest_timestamp = entry.earliest_timestamp.min(decoded.timestamp);
        entry.latest_timestamp = entry.latest_timestamp.max(decoded.timestamp);
        entry.expires_at = entry.expires_at.max(decoded.expires_at);
        if entry.audit_ref.is_none() {
            entry.audit_ref = decoded.audit_ref.clone();
        }
        if let Some(session_id) = decoded.session_id {
            entry.sessions.insert(session_id);
        }
    }

    let mut trace_summaries: Vec<ContinuityTraceSummary> = groups
        .values()
        .map(|group| {
            let derived_signal =
                derived_signal_for_group(group, now_ms).map(|(_, kind, message)| {
                    ContinuityDerivedSignal {
                        kind: kind.as_str().to_string(),
                        message,
                        space: group.space.clone(),
                    }
                });
            let net_summary_candidate = summary_candidate_rule_for_group(group, now_ms).is_some();
            ContinuityTraceSummary {
                taxonomy: group.taxonomy.as_str().to_string(),
                event: group.event.as_str().to_string(),
                summary: group.summary.clone(),
                space: group.space.clone(),
                audit_ref: group.audit_ref.clone(),
                trace_count: group.trace_count,
                session_count: group.sessions.len() as u32,
                failed_count: group.failed_count,
                latest_timestamp: group.latest_timestamp,
                expires_at: group.expires_at,
                local_only: true,
                derived_signal,
                net_summary_candidate,
                runtime: runtime_disposition_for_group(group, now_ms),
            }
        })
        .collect();

    trace_summaries.sort_by(|a, b| {
        b.net_summary_candidate
            .cmp(&a.net_summary_candidate)
            .then_with(|| b.derived_signal.is_some().cmp(&a.derived_signal.is_some()))
            .then_with(|| b.latest_timestamp.cmp(&a.latest_timestamp))
            .then_with(|| b.trace_count.cmp(&a.trace_count))
    });

    let net_summary_candidates: Vec<_> = groups
        .values()
        .filter_map(|group| {
            let trigger_rule = summary_candidate_rule_for_group(group, now_ms)?;
            Some(ContinuityNetSummaryCandidate {
                taxonomy: group.taxonomy.as_str().to_string(),
                event: group.event.as_str().to_string(),
                summary: group.summary.clone(),
                space: group.space.clone(),
                audit_ref: group.audit_ref.clone(),
                trace_count: group.trace_count,
                session_count: group.sessions.len() as u32,
                latest_timestamp: group.latest_timestamp,
                reason: net_summary_reason(group, now_ms),
                trigger_rule,
            })
        })
        .collect();

    let derived_signal_count = trace_summaries
        .iter()
        .filter(|trace| trace.derived_signal.is_some())
        .count();
    trace_summaries.truncate(limit);

    ContinuitySpaceData {
        rules: continuity_rule_set(),
        summary: ContinuitySnapshotSummary {
            trace_count: trace_summaries.len(),
            derived_signal_count,
            net_summary_candidate_count: net_summary_candidates.len(),
        },
        traces: trace_summaries,
        net_summary_candidates,
    }
}

pub fn continuity_record_data(
    trace: &Trace,
    snapshot: &ContinuitySpaceData,
) -> Option<ContinuityRecordData> {
    let decoded = decode_continuity_trace(trace)?;
    let matching = snapshot.traces.iter().find(|item| {
        item.taxonomy == decoded.taxonomy.as_str()
            && item.event == decoded.event.as_str()
            && item.summary == decoded.summary
            && item.space == decoded.space
    })?;
    Some(ContinuityRecordData {
        local_only_raw: true,
        taxonomy: matching.taxonomy.clone(),
        event: matching.event.clone(),
        space: matching.space.clone(),
        expires_at: matching.expires_at,
        derived_signal: matching.derived_signal.clone(),
        net_summary_candidate: matching.net_summary_candidate,
        runtime: matching.runtime.clone(),
    })
}

pub fn derived_signal_kind(data: &ContinuityRecordData) -> Option<SignalPostKind> {
    data.derived_signal
        .as_ref()
        .and_then(|signal| SignalPostKind::parse(&signal.kind))
}

pub struct ExternalContinuityRecordConfig {
    pub owner_account: Option<String>,
    pub device_identity: String,
    pub outcome: Outcome,
    pub model_id: String,
    pub session_id: Option<String>,
}

pub fn record_external_continuity(
    store: &TraceStore,
    identity: &NodeIdentity,
    input: &ExternalContinuityInput,
    config: ExternalContinuityRecordConfig,
) -> Result<ExternalContinuityRecordResult, String> {
    input.validate()?;

    let trace = create_external_continuity_trace(
        input,
        config.outcome,
        config.model_id,
        config.session_id,
        config.owner_account.clone(),
        Some(config.device_identity.clone()),
        identity.public_key_bytes(),
        |msg| identity.sign(msg),
    );
    let trace_id_hex: String = trace.id[..8].iter().map(|b| format!("{b:02x}")).collect();

    store.insert(&trace).map_err(|e| format!("storage: {e}"))?;
    // Raw external continuity traces are local-only evidence. Mark them as
    // published immediately so they never enter the regular P2P gossip path.
    store
        .mark_published(&[trace.id])
        .map_err(|e| format!("storage: {e}"))?;

    let continuity_traces = store
        .query_recent_continuity_traces(CONTINUITY_SUMMARY_HORIZON_HOURS, 200)
        .map_err(|e| format!("storage: {e}"))?;
    let continuity = summarize_recent_continuity(&continuity_traces, input.space.as_deref(), 200);
    let continuity_data = continuity_record_data(&trace, &continuity);

    if let Some(data) = &continuity_data
        && let Some(kind) = derived_signal_kind(data)
        && let Some(signal) = &data.derived_signal
    {
        let signal_trace = create_signal_trace(
            kind,
            &signal.message,
            &signal.message,
            SignalTraceConfig {
                model_id: "thronglets-continuity".into(),
                session_id: trace.session_id.clone(),
                owner_account: config.owner_account,
                device_identity: Some(config.device_identity),
                agent_id: None,
                sigil_id: None,
                space: signal.space.clone(),
                ttl_hours: DEFAULT_SIGNAL_TTL_HOURS,
            },
            identity.public_key_bytes(),
            |msg| identity.sign(msg),
        );
        store
            .insert(&signal_trace)
            .map_err(|e| format!("storage: {e}"))?;
    }

    Ok(ExternalContinuityRecordResult {
        trace_id: trace_id_hex,
        capability: trace.capability,
        external_continuity: continuity_data,
    })
}

fn derived_signal_for_group(
    group: &ContinuityGroup,
    now_ms: u64,
) -> Option<(&'static str, SignalPostKind, String)> {
    group.space.as_ref()?;
    let age_ms = now_ms.saturating_sub(group.earliest_timestamp);
    let stable = has_stable_evidence(group, now_ms);
    match group.event {
        ContinuityEvent::RelationMilestone => {
            if stable {
                let auditable = is_auditable(group);
                let (rule, kind) = if auditable {
                    (RULE_RELATION_WATCH, SignalPostKind::Watch)
                } else {
                    (RULE_RELATION_INFO, SignalPostKind::Info)
                };
                Some((rule, kind, group.summary.clone()))
            } else {
                None
            }
        }
        ContinuityEvent::OpenLoopAnchor => {
            if group.trace_count >= OPEN_LOOP_SIGNAL_MIN_TRACE_COUNT
                || age_ms >= OPEN_LOOP_SIGNAL_MIN_AGE_MS
            {
                Some((
                    RULE_OPEN_LOOP_WATCH,
                    SignalPostKind::Watch,
                    group.summary.clone(),
                ))
            } else {
                None
            }
        }
        ContinuityEvent::ContinuityAnchor => {
            if stable && is_auditable(group) {
                Some((
                    RULE_CONTINUITY_INFO,
                    SignalPostKind::Info,
                    group.summary.clone(),
                ))
            } else {
                None
            }
        }
        ContinuityEvent::WritebackCalibration => {
            if group.failed_count >= CALIBRATION_REPEATED_FAILURE_THRESHOLD && stable {
                Some((
                    RULE_WRITEBACK_AVOID,
                    SignalPostKind::Avoid,
                    group.summary.clone(),
                ))
            } else {
                None
            }
        }
    }
}

fn summary_candidate_rule_for_group(group: &ContinuityGroup, now_ms: u64) -> Option<&'static str> {
    if !has_stable_evidence(group, now_ms) || !is_auditable(group) {
        return None;
    }
    match group.event {
        ContinuityEvent::WritebackCalibration => (group.failed_count
            >= CALIBRATION_REPEATED_FAILURE_THRESHOLD)
            .then_some(RULE_WRITEBACK_SUMMARY),
        ContinuityEvent::RelationMilestone => Some(RULE_RELATION_SUMMARY),
        ContinuityEvent::ContinuityAnchor => Some(RULE_CONTINUITY_SUMMARY),
        ContinuityEvent::OpenLoopAnchor => Some(RULE_OPEN_LOOP_SUMMARY),
    }
}

fn net_summary_reason(group: &ContinuityGroup, now_ms: u64) -> String {
    let age_hours = now_ms
        .saturating_sub(group.earliest_timestamp)
        .saturating_div(3_600_000);
    match group.event {
        ContinuityEvent::WritebackCalibration => format!(
            "repeated calibration failures stayed external for ~{age_hours}h and are auditable"
        ),
        ContinuityEvent::RelationMilestone => format!(
            "coordination boundary stayed stable for ~{age_hours}h and is externally auditable"
        ),
        ContinuityEvent::ContinuityAnchor => format!(
            "continuity evidence remained stable for ~{age_hours}h and is externally auditable"
        ),
        ContinuityEvent::OpenLoopAnchor => format!(
            "the open loop remained unresolved for ~{age_hours}h and is externally auditable"
        ),
    }
}

fn has_stable_evidence(group: &ContinuityGroup, now_ms: u64) -> bool {
    group.trace_count >= STABLE_MIN_TRACE_COUNT
        || now_ms.saturating_sub(group.earliest_timestamp) >= STABLE_MIN_AGE_MS
}

fn is_auditable(group: &ContinuityGroup) -> bool {
    group.audit_ref.is_some() || group.sessions.len() >= 2
}

fn matches_space(candidate: Option<&str>, space: Option<&str>) -> bool {
    match space {
        Some(space) => candidate == Some(space),
        None => true,
    }
}

fn continuity_rule_set() -> ContinuityRuleSet {
    ContinuityRuleSet {
        version: CONTINUITY_RULESET_VERSION,
        retention_hours: ContinuityRetentionHours {
            coordination: COORDINATION_TTL_HOURS,
            continuity: CONTINUITY_TTL_HOURS,
            calibration: CALIBRATION_TTL_HOURS,
        },
        stable_min_age_hours: (STABLE_MIN_AGE_MS / 3_600_000) as u32,
        stable_min_trace_count: STABLE_MIN_TRACE_COUNT,
        open_loop_watch_min_age_hours: (OPEN_LOOP_SIGNAL_MIN_AGE_MS / 3_600_000) as u32,
        open_loop_watch_min_trace_count: OPEN_LOOP_SIGNAL_MIN_TRACE_COUNT,
        calibration_repeated_failure_threshold: CALIBRATION_REPEATED_FAILURE_THRESHOLD,
    }
}

fn retention_hours_for_taxonomy(taxonomy: ContinuityTaxonomy) -> u32 {
    match taxonomy {
        ContinuityTaxonomy::Coordination => COORDINATION_TTL_HOURS,
        ContinuityTaxonomy::Continuity => CONTINUITY_TTL_HOURS,
        ContinuityTaxonomy::Calibration => CALIBRATION_TTL_HOURS,
    }
}

fn runtime_disposition_for_group(
    group: &ContinuityGroup,
    now_ms: u64,
) -> ContinuityRuntimeDisposition {
    let derived_signal_rule = derived_signal_for_group(group, now_ms).map(|(rule, _, _)| rule);
    let summary_candidate_rule = summary_candidate_rule_for_group(group, now_ms);
    let state = if summary_candidate_rule.is_some() {
        "summary-candidate"
    } else if derived_signal_rule.is_some() {
        "derived-signal"
    } else {
        "local-only"
    };
    ContinuityRuntimeDisposition {
        state,
        local_retention_hours: retention_hours_for_taxonomy(group.taxonomy),
        stable_evidence: has_stable_evidence(group, now_ms),
        auditable_evidence: is_auditable(group),
        derived_signal_rule,
        summary_candidate_rule,
    }
}

fn expires_at_ms(now_ms: u64, ttl_hours: u32) -> u64 {
    now_ms.saturating_add((ttl_hours as u64) * 3_600_000)
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock should be after unix epoch")
        .as_millis() as u64
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::NodeIdentity;

    fn make_trace(
        input: ExternalContinuityInput,
        outcome: Outcome,
        session_id: Option<&str>,
        timestamp: u64,
    ) -> Trace {
        let identity = NodeIdentity::generate();
        let mut trace = create_external_continuity_trace(
            &input,
            outcome,
            "psyche".into(),
            session_id.map(str::to_string),
            None,
            Some(identity.device_identity()),
            identity.public_key_bytes(),
            |msg| identity.sign(msg),
        );
        trace.timestamp = timestamp;
        let ttl_hours = input.ttl_hours();
        let payload = ContinuityTracePayload {
            provider: input.provider,
            mode: input.mode,
            version: input.version,
            taxonomy: input.taxonomy,
            event: input.event,
            summary: input.summary.clone(),
            space: input.space.clone(),
            audit_ref: input.audit_ref.clone(),
            expires_at: expires_at_ms(timestamp, ttl_hours),
        };
        trace.context_text = Some(serde_json::to_string(&payload).unwrap());
        trace
    }

    #[test]
    fn writeback_calibration_only_derives_avoid_after_repeated_failures() {
        let now = now_ms();
        let input = ExternalContinuityInput {
            provider: EXTERNAL_CONTINUITY_PROVIDER.into(),
            mode: EXTERNAL_CONTINUITY_MODE.into(),
            version: EXTERNAL_CONTINUITY_VERSION,
            taxonomy: ContinuityTaxonomy::Calibration,
            event: ContinuityEvent::WritebackCalibration,
            summary: "writeback drift keeps reopening the same loop".into(),
            space: Some("psyche".into()),
            audit_ref: Some("audit-1".into()),
        };
        let one = make_trace(
            input.clone(),
            Outcome::Failed,
            Some("s1"),
            now - STABLE_MIN_AGE_MS,
        );
        let summary = summarize_recent_continuity(std::slice::from_ref(&one), Some("psyche"), 10);
        assert_eq!(summary.summary.derived_signal_count, 0);

        let two = make_trace(input, Outcome::Failed, Some("s2"), now - 1000);
        let summary = summarize_recent_continuity(&[one, two], Some("psyche"), 10);
        let derived = summary.traces[0].derived_signal.as_ref().unwrap();
        assert_eq!(derived.kind, "avoid");
        assert_eq!(summary.traces[0].runtime.state, "summary-candidate");
        assert_eq!(
            summary.traces[0].runtime.derived_signal_rule,
            Some(RULE_WRITEBACK_AVOID)
        );
        assert_eq!(
            summary.traces[0].runtime.summary_candidate_rule,
            Some(RULE_WRITEBACK_SUMMARY)
        );
    }

    #[test]
    fn stable_auditable_continuity_anchor_becomes_net_candidate() {
        let now = now_ms();
        let input = ExternalContinuityInput {
            provider: EXTERNAL_CONTINUITY_PROVIDER.into(),
            mode: EXTERNAL_CONTINUITY_MODE.into(),
            version: EXTERNAL_CONTINUITY_VERSION,
            taxonomy: ContinuityTaxonomy::Continuity,
            event: ContinuityEvent::ContinuityAnchor,
            summary: "continuity stayed externally legible across handoff".into(),
            space: Some("psyche".into()),
            audit_ref: Some("anchor-1".into()),
        };
        let one = make_trace(
            input.clone(),
            Outcome::Succeeded,
            Some("s1"),
            now - STABLE_MIN_AGE_MS,
        );
        let two = make_trace(input, Outcome::Succeeded, Some("s2"), now - 1000);
        let summary = summarize_recent_continuity(&[one, two], Some("psyche"), 10);
        assert_eq!(summary.summary.net_summary_candidate_count, 1);
        assert_eq!(summary.rules.version, CONTINUITY_RULESET_VERSION);
        assert_eq!(
            summary.rules.retention_hours.coordination,
            COORDINATION_TTL_HOURS
        );
        assert_eq!(
            summary.rules.retention_hours.continuity,
            CONTINUITY_TTL_HOURS
        );
        assert_eq!(
            summary.rules.retention_hours.calibration,
            CALIBRATION_TTL_HOURS
        );
        assert_eq!(
            summary.traces[0].derived_signal.as_ref().unwrap().kind,
            "info"
        );
        assert_eq!(summary.traces[0].runtime.state, "summary-candidate");
        assert_eq!(
            summary.net_summary_candidates[0].trigger_rule,
            RULE_CONTINUITY_SUMMARY
        );
    }

    #[test]
    fn durable_relation_milestone_without_audit_becomes_info_not_watch() {
        let now = now_ms();
        let input = ExternalContinuityInput {
            provider: EXTERNAL_CONTINUITY_PROVIDER.into(),
            mode: EXTERNAL_CONTINUITY_MODE.into(),
            version: EXTERNAL_CONTINUITY_VERSION,
            taxonomy: ContinuityTaxonomy::Coordination,
            event: ContinuityEvent::RelationMilestone,
            summary: "relation boundary shifted to familiar".into(),
            space: Some("psyche".into()),
            audit_ref: None,
        };
        let one = make_trace(
            input.clone(),
            Outcome::Succeeded,
            Some("s1"),
            now - STABLE_MIN_AGE_MS,
        );
        let summary = summarize_recent_continuity(std::slice::from_ref(&one), Some("psyche"), 10);
        let derived = summary.traces[0].derived_signal.as_ref().unwrap();
        assert_eq!(derived.kind, "info");
        assert!(!summary.traces[0].net_summary_candidate);
        assert_eq!(summary.traces[0].runtime.state, "derived-signal");
        assert_eq!(
            summary.traces[0].runtime.derived_signal_rule,
            Some(RULE_RELATION_INFO)
        );
    }

    #[test]
    fn short_lived_open_loop_stays_local_only_without_signal() {
        let now = now_ms();
        let input = ExternalContinuityInput {
            provider: EXTERNAL_CONTINUITY_PROVIDER.into(),
            mode: EXTERNAL_CONTINUITY_MODE.into(),
            version: EXTERNAL_CONTINUITY_VERSION,
            taxonomy: ContinuityTaxonomy::Coordination,
            event: ContinuityEvent::OpenLoopAnchor,
            summary: "an open loop remains but is still too fresh".into(),
            space: Some("psyche".into()),
            audit_ref: None,
        };
        let one = make_trace(input, Outcome::Succeeded, Some("s1"), now - 1000);
        let summary = summarize_recent_continuity(std::slice::from_ref(&one), Some("psyche"), 10);
        assert!(summary.traces[0].derived_signal.is_none());
        assert!(!summary.traces[0].net_summary_candidate);
        assert!(summary.traces[0].local_only);
    }

    #[test]
    fn recording_external_continuity_marks_raw_trace_as_local_only() {
        let store = TraceStore::in_memory().unwrap();
        let identity = NodeIdentity::generate();
        let input = ExternalContinuityInput {
            provider: EXTERNAL_CONTINUITY_PROVIDER.into(),
            mode: EXTERNAL_CONTINUITY_MODE.into(),
            version: EXTERNAL_CONTINUITY_VERSION,
            taxonomy: ContinuityTaxonomy::Continuity,
            event: ContinuityEvent::ContinuityAnchor,
            summary: "continuity stayed externally legible across handoff".into(),
            space: Some("psyche".into()),
            audit_ref: Some("anchor-42".into()),
        };

        let result = record_external_continuity(
            &store,
            &identity,
            &input,
            ExternalContinuityRecordConfig {
                owner_account: None,
                device_identity: identity.device_identity(),
                outcome: Outcome::Succeeded,
                model_id: "psyche".into(),
                session_id: Some("s1".into()),
            },
        )
        .unwrap();

        assert!(result.external_continuity.as_ref().unwrap().local_only_raw);
        assert_eq!(
            result.external_continuity.as_ref().unwrap().runtime.state,
            "local-only"
        );
        assert_eq!(store.unpublished_traces(10).unwrap().len(), 0);
    }
}
