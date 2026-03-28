use crate::signals::StepAction;
use crate::storage::TraceStore;
use crate::trace::{Outcome, Trace};
use std::collections::{HashMap, HashSet};

const SESSION_TRACE_LIMIT: usize = 10_000;
const FILE_WINDOW_MS: i64 = 300_000;
const REPAIR_WINDOW_MS: i64 = 600_000;
const LOCAL_HISTORY_GATE_MIN: u32 = 2;
const PATTERN_SUPPORT_MIN: u32 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignalEvalSummary {
    pub sessions_considered: usize,
    pub sessions_scored: usize,
    pub edit_points: usize,
    pub edit_points_with_signal: usize,
    pub repair_opportunities: usize,
    pub repair_predictions: usize,
    pub repair_first_step_hits: usize,
    pub repair_exact_hits: usize,
    pub preparation_predictions: usize,
    pub preparation_hits: usize,
    pub adjacency_predictions: usize,
    pub adjacency_hits: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct SessionEvent {
    tool: String,
    target: Option<String>,
    outcome: Outcome,
    timestamp_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct RepairKey {
    failed_tool: String,
    steps: Vec<StepAction>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PatternChoice<T> {
    value: T,
    support: u32,
    source_count: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PatternStats {
    count: u32,
    sources: HashSet<String>,
}

#[derive(Debug, Default)]
struct SignalTrainingSet {
    file_touch_counts: HashMap<String, u32>,
    repair_patterns: HashMap<RepairKey, PatternStats>,
    preparation_patterns: HashMap<(String, String), PatternStats>,
    adjacency_patterns: HashMap<(String, String), PatternStats>,
}

impl SignalEvalSummary {
    pub fn render(&self) -> String {
        [
            format!("sessions considered: {}", self.sessions_considered),
            format!("sessions scored: {}", self.sessions_scored),
            format!(
                "edit silence rate: {:.1}% ({}/{})",
                percent(self.edit_points.saturating_sub(self.edit_points_with_signal), self.edit_points),
                self.edit_points.saturating_sub(self.edit_points_with_signal),
                self.edit_points,
            ),
            format!(
                "repair coverage: {:.1}% ({}/{})",
                percent(self.repair_predictions, self.repair_opportunities),
                self.repair_predictions,
                self.repair_opportunities,
            ),
            format!(
                "repair first-step precision: {:.1}% ({}/{})",
                percent(self.repair_first_step_hits, self.repair_predictions),
                self.repair_first_step_hits,
                self.repair_predictions,
            ),
            format!(
                "repair exact precision: {:.1}% ({}/{})",
                percent(self.repair_exact_hits, self.repair_predictions),
                self.repair_exact_hits,
                self.repair_predictions,
            ),
            format!(
                "preparation precision: {:.1}% ({}/{})",
                percent(self.preparation_hits, self.preparation_predictions),
                self.preparation_hits,
                self.preparation_predictions,
            ),
            format!(
                "adjacency precision: {:.1}% ({}/{})",
                percent(self.adjacency_hits, self.adjacency_predictions),
                self.adjacency_hits,
                self.adjacency_predictions,
            ),
        ]
        .join("\n")
    }
}

impl PatternStats {
    fn record(&mut self, source_id: &str) {
        self.count += 1;
        self.sources.insert(source_id.to_string());
    }

    fn is_strong(&self) -> bool {
        self.count >= PATTERN_SUPPORT_MIN
    }
}

impl SignalTrainingSet {
    fn observe_session(&mut self, session_id: &str, events: &[SessionEvent]) {
        for event in events {
            if matches!(event.tool.as_str(), "Edit" | "Write") {
                if let Some(target) = event.target.as_ref() {
                    *self.file_touch_counts.entry(target.clone()).or_insert(0) += 1;
                }
            }
        }

        for (idx, event) in events.iter().enumerate() {
            if event.outcome == Outcome::Failed {
                let steps = actual_repair_steps(events, idx);
                if !steps.is_empty() {
                    self.repair_patterns
                        .entry(RepairKey {
                            failed_tool: event.tool.clone(),
                            steps,
                        })
                        .or_insert_with(|| PatternStats {
                            count: 0,
                            sources: HashSet::new(),
                        })
                        .record(session_id);
                }
            }

            if !matches!(event.tool.as_str(), "Edit" | "Write") {
                continue;
            }

            let Some(current_target) = event.target.as_ref() else {
                continue;
            };

            for read_target in actual_preparation_targets(events, idx) {
                self.preparation_patterns
                    .entry((current_target.clone(), read_target))
                    .or_insert_with(|| PatternStats {
                        count: 0,
                        sources: HashSet::new(),
                    })
                    .record(session_id);
            }

            for companion_target in actual_companion_targets(events, idx) {
                self.adjacency_patterns
                    .entry((current_target.clone(), companion_target))
                    .or_insert_with(|| PatternStats {
                        count: 0,
                        sources: HashSet::new(),
                    })
                    .record(session_id);
            }
        }
    }

    fn best_repair(&self, failed_tool: &str) -> Option<PatternChoice<Vec<StepAction>>> {
        self.repair_patterns
            .iter()
            .filter(|(key, stats)| key.failed_tool == failed_tool && stats.is_strong())
            .map(|(key, stats)| PatternChoice {
                value: key.steps.clone(),
                support: stats.count,
                source_count: stats.sources.len(),
            })
            .max_by(|a, b| rank_tuple(a).cmp(&rank_tuple(b)))
    }

    fn best_preparation(&self, edit_target: &str) -> Option<PatternChoice<String>> {
        if self.file_touch_counts.get(edit_target).copied().unwrap_or(0) < LOCAL_HISTORY_GATE_MIN {
            return None;
        }

        self.preparation_patterns
            .iter()
            .filter(|((target, _), stats)| target == edit_target && stats.is_strong())
            .map(|((_, read_target), stats)| PatternChoice {
                value: read_target.clone(),
                support: stats.count,
                source_count: stats.sources.len(),
            })
            .max_by(|a, b| rank_tuple(a).cmp(&rank_tuple(b)))
    }

    fn best_adjacency(&self, edit_target: &str) -> Option<PatternChoice<String>> {
        if self.file_touch_counts.get(edit_target).copied().unwrap_or(0) < LOCAL_HISTORY_GATE_MIN {
            return None;
        }

        self.adjacency_patterns
            .iter()
            .filter(|((target, _), stats)| target == edit_target && stats.is_strong())
            .map(|((_, companion_target), stats)| PatternChoice {
                value: companion_target.clone(),
                support: stats.count,
                source_count: stats.sources.len(),
            })
            .max_by(|a, b| rank_tuple(a).cmp(&rank_tuple(b)))
    }
}

pub fn evaluate_signal_quality(
    store: &TraceStore,
    hours: u64,
    max_sessions: usize,
) -> rusqlite::Result<Option<SignalEvalSummary>> {
    let session_ids = store.recent_session_ids(hours, max_sessions)?;
    if session_ids.len() < 2 {
        return Ok(None);
    }

    let mut sessions = Vec::new();
    for session_id in session_ids {
        let traces = store.query_session(&session_id, SESSION_TRACE_LIMIT)?;
        let events = traces
            .iter()
            .filter_map(trace_to_event)
            .collect::<Vec<_>>();
        if !events.is_empty() {
            sessions.push((session_id, events));
        }
    }

    if sessions.len() < 2 {
        return Ok(None);
    }

    let mut summary = SignalEvalSummary {
        sessions_considered: sessions.len(),
        sessions_scored: 0,
        edit_points: 0,
        edit_points_with_signal: 0,
        repair_opportunities: 0,
        repair_predictions: 0,
        repair_first_step_hits: 0,
        repair_exact_hits: 0,
        preparation_predictions: 0,
        preparation_hits: 0,
        adjacency_predictions: 0,
        adjacency_hits: 0,
    };
    let mut training = SignalTrainingSet::default();

    for (index, (session_id, events)) in sessions.iter().enumerate() {
        if index > 0 {
            summary.sessions_scored += 1;
            score_session(&training, events, &mut summary);
        }
        training.observe_session(session_id, events);
    }

    Ok(Some(summary))
}

fn score_session(training: &SignalTrainingSet, events: &[SessionEvent], summary: &mut SignalEvalSummary) {
    for (idx, event) in events.iter().enumerate() {
        if matches!(event.tool.as_str(), "Edit" | "Write") {
            summary.edit_points += 1;

            let mut emitted_signal = false;
            if let Some(current_target) = event.target.as_deref() {
                if let Some(predicted) = training.best_preparation(current_target) {
                    summary.preparation_predictions += 1;
                    emitted_signal = true;
                    if actual_preparation_targets(events, idx).contains(&predicted.value) {
                        summary.preparation_hits += 1;
                    }
                }

                if let Some(predicted) = training.best_adjacency(current_target) {
                    summary.adjacency_predictions += 1;
                    emitted_signal = true;
                    if actual_companion_targets(events, idx).contains(&predicted.value) {
                        summary.adjacency_hits += 1;
                    }
                }
            }

            if emitted_signal {
                summary.edit_points_with_signal += 1;
            }
        }

        if event.outcome == Outcome::Failed {
            summary.repair_opportunities += 1;
            if let Some(predicted) = training.best_repair(&event.tool) {
                summary.repair_predictions += 1;
                let actual = actual_repair_steps(events, idx);
                if let (Some(predicted_first), Some(actual_first)) =
                    (predicted.value.first(), actual.first())
                {
                    if predicted_first == actual_first {
                        summary.repair_first_step_hits += 1;
                    }
                }
                if predicted.value == actual {
                    summary.repair_exact_hits += 1;
                }
            }
        }
    }
}

fn trace_to_event(trace: &Trace) -> Option<SessionEvent> {
    let tool = trace.capability.strip_prefix("claude-code/")?.to_string();
    Some(SessionEvent {
        target: parse_target(&tool, trace.context_text.as_deref()),
        tool,
        outcome: trace.outcome,
        timestamp_ms: trace.timestamp as i64,
    })
}

fn parse_target(tool: &str, context_text: Option<&str>) -> Option<String> {
    let prefix = match tool {
        "Read" => "read file: ",
        "Edit" => "edit file: ",
        "Write" => "write file: ",
        _ => return None,
    };

    let raw = context_text?.strip_prefix(prefix)?;
    Some(short_target(raw))
}

fn short_target(path: &str) -> String {
    std::path::Path::new(path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or(path)
        .to_string()
}

fn actual_repair_steps(events: &[SessionEvent], failed_index: usize) -> Vec<StepAction> {
    let failed = &events[failed_index];
    let mut steps = Vec::new();
    let mut last_timestamp = failed.timestamp_ms;

    for next in events.iter().skip(failed_index + 1) {
        if next.timestamp_ms - last_timestamp > REPAIR_WINDOW_MS {
            break;
        }
        if next.outcome == Outcome::Failed {
            break;
        }
        steps.push(StepAction::new(next.tool.clone(), next.target.clone()));
        last_timestamp = next.timestamp_ms;
        if steps.len() == 2 {
            break;
        }
    }

    steps
}

fn actual_preparation_targets(events: &[SessionEvent], edit_index: usize) -> HashSet<String> {
    let edit = &events[edit_index];
    let Some(current_target) = edit.target.as_ref() else {
        return HashSet::new();
    };

    let start = edit_index.saturating_sub(5);
    let mut targets = HashSet::new();
    for prev in events.iter().take(edit_index).skip(start) {
        if prev.tool != "Read" {
            continue;
        }
        if edit.timestamp_ms - prev.timestamp_ms > FILE_WINDOW_MS {
            continue;
        }
        if let Some(target) = prev.target.as_ref() {
            if target != current_target {
                targets.insert(target.clone());
            }
        }
    }
    targets
}

fn actual_companion_targets(events: &[SessionEvent], edit_index: usize) -> HashSet<String> {
    let edit = &events[edit_index];
    let Some(current_target) = edit.target.as_ref() else {
        return HashSet::new();
    };

    let start = edit_index.saturating_sub(10);
    let end = (edit_index + 11).min(events.len());
    let mut targets = HashSet::new();
    for (idx, other) in events.iter().enumerate().take(end).skip(start) {
        if idx == edit_index {
            continue;
        }
        if !matches!(other.tool.as_str(), "Edit" | "Write") {
            continue;
        }
        if (other.timestamp_ms - edit.timestamp_ms).abs() > FILE_WINDOW_MS {
            continue;
        }
        if let Some(target) = other.target.as_ref() {
            if target != current_target {
                targets.insert(target.clone());
            }
        }
    }
    targets
}

fn rank_tuple<T>(choice: &PatternChoice<T>) -> (usize, u32) {
    (choice.source_count, choice.support)
}

fn percent(numerator: usize, denominator: usize) -> f64 {
    if denominator == 0 {
        0.0
    } else {
        numerator as f64 / denominator as f64 * 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::simhash;
    use crate::identity::NodeIdentity;

    fn make_trace(
        identity: &NodeIdentity,
        capability: &str,
        outcome: Outcome,
        context: &str,
        session_id: &str,
        timestamp: u64,
    ) -> Trace {
        let mut trace = Trace::new(
            capability.to_string(),
            outcome,
            10,
            1,
            simhash(context),
            Some(context.to_string()),
            Some(session_id.to_string()),
            "test-agent".to_string(),
            identity.public_key_bytes(),
            |msg| identity.sign(msg),
        );
        trace.timestamp = timestamp;
        trace
    }

    #[test]
    fn holdout_eval_scores_repair_and_file_guidance() {
        let store = TraceStore::in_memory().unwrap();
        let identity = NodeIdentity::generate();

        let sessions = ["s1", "s2", "s3"];
        let mut timestamp = chrono::Utc::now().timestamp_millis() as u64 - 10_000;
        for session in sessions {
            for (capability, outcome, context) in [
                ("claude-code/Read", Outcome::Succeeded, "read file: helper.rs"),
                ("claude-code/Edit", Outcome::Succeeded, "edit file: main.rs"),
                ("claude-code/Edit", Outcome::Succeeded, "edit file: helper.rs"),
                ("claude-code/Bash", Outcome::Failed, "bash: cargo test"),
                ("claude-code/Read", Outcome::Succeeded, "read file: Cargo.toml"),
                ("claude-code/Bash", Outcome::Succeeded, "bash: cargo test"),
            ] {
                let trace = make_trace(&identity, capability, outcome, context, session, timestamp);
                store.insert(&trace).unwrap();
                timestamp += 1_000;
            }
            timestamp += 60_000;
        }

        let summary = evaluate_signal_quality(&store, 168, 10)
            .unwrap()
            .expect("expected evaluation summary");

        assert_eq!(summary.sessions_considered, 3);
        assert_eq!(summary.sessions_scored, 2);
        assert!(summary.repair_predictions >= 1);
        assert!(summary.repair_first_step_hits >= 1);
        assert!(summary.preparation_predictions >= 1);
        assert!(summary.preparation_hits >= 1);
        assert!(summary.adjacency_predictions >= 1);
        assert!(summary.adjacency_hits >= 1);
        assert!(summary.render().contains("repair first-step precision"));
    }

    #[test]
    fn returns_none_without_enough_sessions() {
        let store = TraceStore::in_memory().unwrap();
        let identity = NodeIdentity::generate();
        let trace = make_trace(
            &identity,
            "claude-code/Edit",
            Outcome::Succeeded,
            "edit file: main.rs",
            "solo",
            chrono::Utc::now().timestamp_millis() as u64,
        );
        store.insert(&trace).unwrap();

        assert!(evaluate_signal_quality(&store, 168, 10).unwrap().is_none());
    }
}
