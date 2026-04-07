use std::path::Path;
use std::time::Instant;

use crate::responses::ReleaseBaselineCheck;
use thronglets::active_policy::{ActivePolicySet, PolicyStrength};
use thronglets::ambient::{AmbientPolicyState, AmbientPriorProjection};
use thronglets::eval::{
    EvalBaselineComparison, EvalCheckStatus, EvalCheckThresholds, EvalConfig,
    LocalFeedbackSummary, SignalEvalSummary, evaluate_signal_quality,
};
use thronglets::posts::summarize_signal_traces;
use thronglets::presence::summarize_recent_presence;
use thronglets::signals::{Recommendation, Signal, SignalKind, StepCandidate};
use thronglets::storage::TraceStore;
use thronglets::trace::MethodCompliance;
use thronglets::workspace;

pub(crate) fn git_file_history(file_path: &str, max_entries: usize) -> Option<String> {
    use std::path::Path;
    use std::process::Command;

    let path = Path::new(file_path);
    let dir = path.parent()?;

    let output = Command::new("git")
        .args([
            "log",
            "--oneline",
            "--no-decorate",
            &format!("-n{}", max_entries),
            "--format=%ar\t%s",
            "--",
            file_path,
        ])
        .current_dir(dir)
        .output()
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let log = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = log.lines().filter(|l| !l.is_empty()).collect();
    if lines.is_empty() {
        return None;
    }

    let mut result = format!("  git history for {}:", path.file_name()?.to_str()?);
    for line in &lines {
        result.push_str(&format!("\n    {line}"));
    }
    Some(result)
}

pub(crate) fn payload_string(payload: &serde_json::Value, key: &str) -> Option<String> {
    payload[key]
        .as_str()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

pub(crate) fn apply_collective_sources(
    candidate: &mut StepCandidate,
    score: &mut i32,
    collective_sources: u32,
) {
    *score += candidate.upgrade_collective_sources(collective_sources);
}

pub(crate) fn explicit_signals(
    store: &TraceStore,
    hook_context: &str,
    context_hash: &[u8; 16],
    space: Option<&str>,
    local_device_identity: &str,
    local_node_pubkey: [u8; 32],
) -> Vec<Signal> {
    let traces = match store.query_signal_traces(context_hash, None, 48, 10, space) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let results = summarize_signal_traces(
        &traces,
        hook_context,
        local_device_identity,
        local_node_pubkey,
        6,
    );

    let mut signals = Vec::new();
    for result in results {
        if result.density_score < 1 || result.context_similarity < 0.85 {
            continue;
        }
        let collective_bonus = if result.promotion_state == "collective" {
            30
        } else {
            0
        };
        let density_bonus = i32::from(result.density_score) * 20;

        match result.kind.as_str() {
            "avoid" => {
                let score = 300 + density_bonus + collective_bonus;
                signals.push(Signal::danger(
                    format!("  ⚠ avoid: {}", result.message),
                    score,
                ));
            }
            "watch" => {
                let score = 200 + density_bonus + collective_bonus;
                signals.push(Signal {
                    kind: SignalKind::History,
                    score,
                    body: format!("  👁 watch: {}", result.message),
                    candidate: None,
                });
            }
            "recommend" => {
                let score = 150 + density_bonus + collective_bonus;
                signals.push(Signal::preparation(
                    format!("  ✦ recommended: {}", result.message),
                    score,
                ));
            }
            _ => {}
        }
    }
    signals
}

pub(crate) fn history_signal_from_projection(prior: AmbientPriorProjection) -> Option<Signal> {
    let (score, body) = match (prior.kind, prior.policy_state) {
        ("success-prior", Some(AmbientPolicyState::StablePath)) => (
            140 + (prior.confidence * 60.0).round() as i32,
            format!(
                "  ✓ stable path: {}",
                strip_success_prior_prefix(&prior.summary)
            ),
        ),
        ("mixed-residue", Some(AmbientPolicyState::PolicyConflict)) => {
            (235, format!("  ! {}", prior.summary))
        }
        ("mixed-residue", Some(AmbientPolicyState::MethodConflict)) => {
            (205, format!("  ~ {}", prior.summary))
        }
        ("mixed-residue", _) => (
            190,
            format!(
                "  ~ unsettled path: {}",
                strip_mixed_residue_prefix(&prior.summary)
            ),
        ),
        _ => return None,
    };
    Some(Signal {
        kind: SignalKind::History,
        score,
        body,
        candidate: None,
    })
}

fn strip_success_prior_prefix(summary: &str) -> &str {
    summary
        .strip_prefix("shared success prior: ")
        .or_else(|| summary.strip_prefix("prior success: "))
        .unwrap_or(summary)
}

fn strip_mixed_residue_prefix(summary: &str) -> &str {
    summary.strip_prefix("mixed residue: ").unwrap_or(summary)
}

pub(crate) fn reinforced_success_threshold(
    feedback_events: &[workspace::RecommendationFeedbackEvent],
    contradictory_failures: u32,
    noncompliant_successes: u32,
) -> u32 {
    let mut reinforced = false;
    let mut contradicted = false;
    for event in feedback_events {
        let relevant_recommendation =
            matches!(event.recommendation_kind.as_str(), "do_next" | "maybe_also");
        let relevant_source = matches!(
            event.source_kind.as_str(),
            "repair" | "preparation" | "adjacency"
        );
        if !(relevant_recommendation && relevant_source) {
            continue;
        }
        if event.positive {
            reinforced = true;
        } else {
            contradicted = true;
        }
    }

    let base_threshold = if reinforced && !contradicted { 2 } else { 3 };
    let contradiction_floor = contradictory_failures
        .saturating_add(noncompliant_successes)
        .saturating_add(2);
    let feedback_floor = if contradicted {
        base_threshold.max(4)
    } else {
        base_threshold
    };
    feedback_floor.max(contradiction_floor)
}

pub(crate) fn active_policy_signal(active_policy: &ActivePolicySet) -> Option<Signal> {
    if active_policy.relevant_rules.is_empty() {
        return None;
    }
    let summaries = active_policy
        .relevant_rules
        .iter()
        .take(2)
        .map(|rule| rule.summary.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    let hard = active_policy
        .relevant_rules
        .iter()
        .any(|rule| rule.strength == PolicyStrength::Hard);
    if hard {
        Some(Signal::danger(
            format!("  ! active policy: {summaries}"),
            340,
        ))
    } else {
        Some(Signal {
            kind: SignalKind::History,
            score: 205,
            body: format!("  ~ active method guidance: {summaries}"),
            candidate: None,
        })
    }
}

pub(crate) fn can_promote_auto_recommend(
    method_compliance: Option<MethodCompliance>,
    hard_policy_active: bool,
    noncompliant_successes: u32,
) -> bool {
    if matches!(method_compliance, Some(MethodCompliance::Noncompliant)) {
        return false;
    }
    if hard_policy_active && noncompliant_successes > 0 {
        return false;
    }
    true
}

pub(crate) fn presence_context_signal(
    store: &TraceStore,
    space: &str,
    current_session_id: Option<&str>,
    local_device_identity: &str,
    local_node_pubkey: [u8; 32],
) -> Option<Signal> {
    let traces = store.query_recent_presence_traces(1, 24).ok()?;
    let active_sessions: Vec<_> = summarize_recent_presence(
        &traces,
        Some(space),
        local_device_identity,
        local_node_pubkey,
        8,
    )
    .into_iter()
    .filter(|session| {
        current_session_id.is_none_or(|sid| session.session_id.as_deref() != Some(sid))
    })
    .collect();

    if active_sessions.is_empty() {
        return None;
    }

    let preview_limit = 2;
    let collective = active_sessions
        .iter()
        .any(|session| session.evidence_scope == "collective");
    let preview = active_sessions
        .iter()
        .take(preview_limit)
        .map(|session| match session.mode.as_deref() {
            Some(mode) => format!("{} ({mode})", session.model_id),
            None => session.model_id.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ");
    let remainder = active_sessions.len().saturating_sub(preview_limit);
    let suffix = if remainder > 0 {
        format!(" +{remainder} more")
    } else {
        String::new()
    };

    Some(Signal {
        kind: SignalKind::History,
        score: if collective { 145 } else { 135 },
        body: format!("  active in space {space}: {preview}{suffix}"),
        candidate: None,
    })
}

pub(crate) fn claim_collective_query(
    candidate: &StepCandidate,
    remaining_queries: &mut usize,
) -> bool {
    if *remaining_queries == 0 || candidate.source_count >= 2 {
        return false;
    }

    *remaining_queries -= 1;
    true
}

pub(crate) fn cached_collective_store<'a>(
    cache: &'a mut Option<TraceStore>,
    dir: &Path,
) -> Option<&'a TraceStore> {
    let db_path = dir.join("traces.db");
    if !db_path.exists() {
        return None;
    }

    if cache.is_none() {
        *cache = TraceStore::open(&db_path).ok();
    }

    cache.as_ref()
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum ProfileStageState {
    Timed(u128),
    Skipped,
}

pub(crate) struct PrehookProfiler {
    enabled: bool,
    started_at: Instant,
    stage_started_at: Instant,
    stages: Vec<(&'static str, ProfileStageState)>,
}

impl PrehookProfiler {
    pub(crate) fn from_env() -> Self {
        let enabled =
            std::env::var_os("THRONGLETS_PROFILE_PREHOOK").is_some_and(|value| value != "0");
        let now = Instant::now();
        Self {
            enabled,
            started_at: now,
            stage_started_at: now,
            stages: Vec::new(),
        }
    }

    pub(crate) fn stage(&mut self, name: &'static str) {
        if !self.enabled {
            return;
        }

        let now = Instant::now();
        self.stages.push((
            name,
            ProfileStageState::Timed(now.duration_since(self.stage_started_at).as_micros()),
        ));
        self.stage_started_at = now;
    }

    pub(crate) fn stage_or_skip(&mut self, name: &'static str, executed: bool) {
        if !self.enabled {
            return;
        }

        if executed {
            self.stage(name);
        } else {
            self.stages.push((name, ProfileStageState::Skipped));
        }
    }

    pub(crate) fn finish(
        &self,
        tool_name: &str,
        recommendations: &[Recommendation],
        stdout_bytes: usize,
        file_guidance_gate: &'static str,
        collective_queries_used: usize,
    ) {
        if !self.enabled {
            return;
        }

        let mut parts = vec![
            format!("tool={tool_name}"),
            format!("emitted={}", recommendations.len()),
            format!("stdout_bytes={stdout_bytes}"),
            format!("output_mode={}", profile_output_mode(recommendations)),
            format!("decision_path={}", profile_decision_path(recommendations)),
            format!(
                "evidence_scope={}",
                profile_evidence_scope(recommendations)
            ),
            format!("file_guidance_gate={file_guidance_gate}"),
            format!("collective_queries_used={collective_queries_used}"),
            format!("total_us={}", self.started_at.elapsed().as_micros()),
        ];
        for (name, state) in &self.stages {
            match state {
                ProfileStageState::Timed(duration_us) => {
                    parts.push(format!("{name}_us={duration_us}"));
                }
                ProfileStageState::Skipped => {
                    parts.push(format!("{name}=skipped"));
                }
            }
        }
        eprintln!("[thronglets:prehook] {}", parts.join(" "));
    }
}

pub(crate) fn profile_output_mode(recommendations: &[Recommendation]) -> &'static str {
    if recommendations.is_empty() {
        "silent"
    } else if recommendations
        .iter()
        .any(|r| r.source_kind == SignalKind::Repair)
    {
        "next-step"
    } else if recommendations
        .iter()
        .any(|r| r.source_kind == SignalKind::Danger)
    {
        "caution"
    } else {
        "context-only"
    }
}

pub(crate) fn profile_decision_path(recommendations: &[Recommendation]) -> &'static str {
    recommendations
        .iter()
        .find(|r| r.source_kind == SignalKind::Repair)
        .or_else(|| recommendations.first())
        .map(|r| match r.source_kind {
            SignalKind::Danger => "danger",
            SignalKind::Repair => "repair",
            SignalKind::Preparation | SignalKind::Adjacency => "legacy",
            SignalKind::History => "history",
        })
        .unwrap_or("none")
}

pub(crate) fn profile_evidence_scope(recommendations: &[Recommendation]) -> &'static str {
    recommendations
        .iter()
        .find_map(|r| r.candidate.as_ref())
        .map(|candidate| match candidate.evidence_scope {
            thronglets::signals::EvidenceScope::Local => "local",
            thronglets::signals::EvidenceScope::Collective => "collective",
        })
        .unwrap_or("none")
}

pub(crate) fn profile_file_guidance_gate(supports_file_guidance: bool) -> &'static str {
    if supports_file_guidance {
        "open"
    } else {
        "na"
    }
}

pub(crate) fn strip_check_header(rendered: &str) -> String {
    rendered.lines().skip(1).collect::<Vec<_>>().join("\n")
}

pub(crate) fn run_release_eval_section(
    store: &TraceStore,
    hours: u64,
    max_sessions: usize,
    project_scope: Option<&Path>,
    local_feedback: Option<LocalFeedbackSummary>,
    thresholds: &EvalCheckThresholds,
    baseline: Option<&SignalEvalSummary>,
) -> (&'static str, bool, String, serde_json::Value) {
    match evaluate_signal_quality(
        store,
        hours,
        max_sessions,
        project_scope,
        EvalConfig::default(),
    )
    .expect("failed to evaluate signal quality")
    {
        Some(summary) => {
            let mut summary = summary.with_local_feedback(local_feedback);
            if let Some(baseline) = baseline {
                summary = summary.with_comparison_to_baseline(baseline);
            }
            let check = summary.check(thresholds);
            let (status, rendered) = summary.render_check(thresholds);
            let baseline_check =
                release_baseline_check(summary.comparison_to_baseline.as_ref(), check.status);
            let mut body = strip_check_header(&rendered);
            if let Some(comparison) = summary.comparison_to_baseline.as_ref() {
                if !body.is_empty() {
                    body.push('\n');
                }
                body.push_str(&render_release_baseline_outcome_line(comparison));
                body.push('\n');
                body.push_str(&render_release_baseline_signal_line(comparison));
                if !baseline_check.violations.is_empty() {
                    body.push('\n');
                    body.push_str(&format!(
                        "baseline violations: {}",
                        baseline_check.violations.join("; ")
                    ));
                } else if !baseline_check.notes.is_empty() {
                    body.push('\n');
                    body.push_str(&format!(
                        "baseline notes: {}",
                        baseline_check.notes.join("; ")
                    ));
                }
            }
            let effective_status =
                if matches!(status, EvalCheckStatus::Fail) || baseline_check.status == "FAIL" {
                    "FAIL"
                } else {
                    status.label()
                };
            (
                effective_status,
                effective_status == "FAIL",
                body,
                serde_json::json!({
                    "status": effective_status,
                    "thresholds": thresholds,
                    "summary": summary,
                    "check": check,
                    "baseline_check": baseline_check,
                }),
            )
        }
        None => {
            let notes =
                vec!["not enough recent session history to evaluate signals yet".to_string()];
            (
                "SKIP",
                false,
                "notes: not enough recent session history to evaluate signals yet".to_string(),
                serde_json::json!({
                    "status": "SKIP",
                    "thresholds": thresholds,
                    "summary": serde_json::Value::Null,
                    "check": {
                        "status": "Skip",
                        "violations": Vec::<String>::new(),
                        "notes": notes,
                    },
                    "baseline_check": serde_json::Value::Null,
                }),
            )
        }
    }
}

pub(crate) fn release_baseline_check(
    comparison: Option<&EvalBaselineComparison>,
    eval_status: EvalCheckStatus,
) -> ReleaseBaselineCheck {
    let Some(comparison) = comparison else {
        return ReleaseBaselineCheck {
            status: "SKIP",
            violations: Vec::new(),
            notes: Vec::new(),
        };
    };

    if matches!(eval_status, EvalCheckStatus::Skip) {
        return ReleaseBaselineCheck {
            status: "SKIP",
            violations: Vec::new(),
            notes: vec![
                "baseline comparison inactive because offline eval is still in SKIP".into(),
            ],
        };
    }

    let mut violations = Vec::new();
    if let Some(delta) = comparison.local_retention_delta_tenths_pp
        && delta < -super::RELEASE_MAX_LOCAL_RETENTION_DROP_TENTHS_PP
    {
        violations.push(format!(
            "local edit retention regressed by {}",
            format_release_option_tenths_pp(Some(delta))
        ));
    }
    if comparison.failed_command_rate_delta_tenths_pp
        > super::RELEASE_MAX_FAILED_COMMAND_RATE_RISE_TENTHS_PP
    {
        violations.push(format!(
            "failed command rate regressed by {}",
            format_release_tenths_pp(comparison.failed_command_rate_delta_tenths_pp)
        ));
    }
    if let Some(delta) = comparison.first_successful_change_latency_avg_delta_ms
        && delta > super::RELEASE_MAX_FIRST_CHANGE_LATENCY_RISE_MS
    {
        violations.push(format!(
            "first successful change latency avg regressed by {}",
            format_release_option_ms(Some(delta))
        ));
    }
    if let Some(delta) = comparison.first_successful_change_latency_p50_delta_ms
        && delta > super::RELEASE_MAX_FIRST_CHANGE_LATENCY_RISE_MS
    {
        violations.push(format!(
            "first successful change latency p50 regressed by {}",
            format_release_option_ms(Some(delta))
        ));
    }

    ReleaseBaselineCheck {
        status: if violations.is_empty() {
            "PASS"
        } else {
            "FAIL"
        },
        violations,
        notes: Vec::new(),
    }
}

pub(crate) fn render_release_baseline_outcome_line(comparison: &EvalBaselineComparison) -> String {
    format!(
        "vs baseline ({} scored): retention {}, failed cmds {}, first change avg {}, p50 {}",
        comparison.baseline_sessions_scored,
        format_release_option_tenths_pp(comparison.local_retention_delta_tenths_pp),
        format_release_tenths_pp(comparison.failed_command_rate_delta_tenths_pp),
        format_release_option_ms(comparison.first_successful_change_latency_avg_delta_ms),
        format_release_option_ms(comparison.first_successful_change_latency_p50_delta_ms),
    )
}

pub(crate) fn render_release_baseline_signal_line(comparison: &EvalBaselineComparison) -> String {
    format!(
        "vs baseline signals: silence {}, repair cov {}, repair step {}, repair exact {}, prep {}, adj {}",
        format_release_tenths_pp(comparison.edit_silence_rate_delta_tenths_pp),
        format_release_tenths_pp(comparison.repair_coverage_delta_tenths_pp),
        format_release_tenths_pp(comparison.repair_first_step_precision_delta_tenths_pp),
        format_release_tenths_pp(comparison.repair_exact_precision_delta_tenths_pp),
        format_release_tenths_pp(comparison.preparation_precision_delta_tenths_pp),
        format_release_tenths_pp(comparison.adjacency_precision_delta_tenths_pp),
    )
}

pub(crate) fn format_release_tenths_pp(delta: i32) -> String {
    format!("{:+}.{}pp", delta / 10, delta.abs() % 10)
}

pub(crate) fn format_release_option_tenths_pp(delta: Option<i32>) -> String {
    delta
        .map(format_release_tenths_pp)
        .unwrap_or_else(|| "n/a".into())
}

pub(crate) fn format_release_option_ms(delta: Option<i64>) -> String {
    match delta {
        Some(delta) => format!("{delta:+}ms"),
        None => "n/a".into(),
    }
}

pub(crate) fn run_release_doctor_section(
    home_dir: &Path,
    data_dir: &Path,
) -> (&'static str, bool, String, serde_json::Value) {
    let reports: Vec<_> = crate::adapter_ops::selected_adapters(super::AdapterArg::All)
        .into_iter()
        .map(|adapter| {
            crate::setup_support::doctor_adapter(home_dir, data_dir, adapter)
        })
        .collect();
    let summary =
        crate::adapter_ops::summarize_doctor_reports(super::AdapterArg::All, reports);
    let status = if summary.summary.status == "healthy" {
        "PASS"
    } else {
        "FAIL"
    };
    let mut lines = vec![format!("status: {}", summary.summary.status)];
    if summary.summary.restart_pending {
        lines.push("restart pending: yes".into());
    }
    for step in &summary.summary.next_steps {
        lines.push(format!("next: {step}"));
    }

    (
        status,
        status == "FAIL",
        lines.join("\n"),
        serde_json::json!({
            "status": status,
            "summary": summary.summary,
            "reports": summary.reports,
        }),
    )
}

pub(crate) fn load_eval_baseline(
    path: &Path,
) -> Result<SignalEvalSummary, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|err| format!("read baseline {}: {err}", path.display()))?;
    serde_json::from_str(&raw).map_err(|err| format!("parse baseline {}: {err}", path.display()))
}

pub(crate) fn print_release_section(name: &str, status: &str, body: &str) {
    println!("{name}: {status}");
    for line in body.lines() {
        println!("  {line}");
    }
}

pub(crate) fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
