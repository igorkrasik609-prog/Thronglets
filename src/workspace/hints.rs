//! Read-only hint generation methods for WorkspaceState.
//!
//! These methods query workspace state to produce signals for prehook injection.
//! They never mutate state.

use crate::signals::{StepAction, StepCandidate};

use super::{
    DangerHint, PendingFeedback, RecentError, RecentFile, RepairHint, WorkspaceState,
};

struct RepairTrajectoryPattern {
    weighted_support: f64,
    count: u32,
    last_seen_ms: i64,
    steps: Vec<StepAction>,
    source_ids: Vec<String>,
}

impl WorkspaceState {
    /// Generate feedback hints for prehook injection.
    /// Shows retention rate and specific file feedback.
    pub fn feedback_hints(&self, current_file: Option<&str>) -> Option<String> {
        let resolved: Vec<&PendingFeedback> = self
            .pending_feedback
            .iter()
            .filter(|p| p.resolved)
            .collect();

        if resolved.is_empty() {
            return None;
        }

        let mut lines: Vec<String> = Vec::new();

        let committed = resolved
            .iter()
            .filter(|p| p.outcome.as_deref() == Some("committed"))
            .count();
        let reverted = resolved
            .iter()
            .filter(|p| p.outcome.as_deref() == Some("reverted"))
            .count();
        let total = committed + reverted;
        if total >= 3 {
            let rate = (committed as f64 / total as f64 * 100.0).round();
            lines.push(format!(
                "  edit retention: {rate}% ({committed}/{total} committed)"
            ));
        }

        if let Some(file) = current_file {
            let file_fb: Vec<_> = resolved.iter().filter(|p| p.file_path == file).collect();
            if !file_fb.is_empty() {
                let file_committed = file_fb
                    .iter()
                    .filter(|p| p.outcome.as_deref() == Some("committed"))
                    .count();
                let fname = std::path::Path::new(file)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or(file);
                lines.push(format!(
                    "  {fname}: {file_committed}/{} edits committed",
                    file_fb.len()
                ));
            }
        }

        if lines.is_empty() {
            None
        } else {
            Some(lines.join("\n"))
        }
    }

    /// Emit a localized retention warning only when the current file has
    /// enough poor outcomes to be decision-relevant.
    pub fn retention_warning(&self, current_file: Option<&str>) -> Option<DangerHint> {
        let file = current_file?;
        let file_feedback: Vec<&PendingFeedback> = self
            .pending_feedback
            .iter()
            .filter(|p| p.resolved && p.file_path == file)
            .filter(|p| matches!(p.outcome.as_deref(), Some("committed" | "reverted")))
            .collect();

        let total = file_feedback.len();
        if total < 2 {
            return None;
        }

        let committed = file_feedback
            .iter()
            .filter(|p| p.outcome.as_deref() == Some("committed"))
            .count();
        let rate = committed as f64 / total as f64 * 100.0;
        if rate >= 50.0 {
            return None;
        }

        let fname = std::path::Path::new(file)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(file);

        Some(DangerHint {
            body: format!("  ⚠ low retention for {fname}: {committed}/{total} edits committed"),
            score: 340,
        })
    }

    /// Infer current strategy from recent tool call sequence.
    /// Returns a short label like "analyze-modify", "debug-cycle", "explore".
    pub fn infer_strategy(&self) -> Option<String> {
        if self.recent_actions.len() < 3 {
            return None;
        }

        let recent: Vec<&str> = self
            .recent_actions
            .iter()
            .take(8)
            .map(|a| a.tool.as_str())
            .collect();

        let reads = recent.iter().filter(|t| **t == "Read").count();
        let edits = recent
            .iter()
            .filter(|t| **t == "Edit" || **t == "Write")
            .count();
        let bashes = recent.iter().filter(|t| **t == "Bash").count();
        let greps = recent
            .iter()
            .filter(|t| **t == "Grep" || **t == "Glob")
            .count();
        let agents = recent.iter().filter(|t| **t == "Agent").count();

        if bashes >= 2 && edits >= 1 {
            return Some("build-fix-cycle".to_string());
        }
        if greps >= 2 && reads >= 1 && edits == 0 {
            return Some("codebase-exploration".to_string());
        }
        if reads >= 2 && edits >= 1 && bashes == 0 {
            return Some("analyze-modify".to_string());
        }
        if edits >= 3 {
            let unique_files: std::collections::HashSet<_> = self
                .recent_actions
                .iter()
                .take(8)
                .filter(|a| a.tool == "Edit" || a.tool == "Write")
                .filter_map(|a| a.file_path.as_deref())
                .collect();
            if unique_files.len() >= 3 {
                return Some("multi-file-refactor".to_string());
            }
        }
        if agents >= 1 {
            return Some("delegated-research".to_string());
        }
        if reads >= 3 && edits == 0 {
            return Some("code-review".to_string());
        }

        None
    }

    /// Suggest likely repair paths after failures of the current tool.
    pub fn repair_hints(&self, current_tool: &str) -> Option<RepairHint> {
        let now = chrono::Utc::now().timestamp_millis();
        let repairs: Vec<_> = self
            .repair_patterns
            .iter()
            .filter(|p| p.error_tool == current_tool && (now - p.last_seen_ms) < 86_400_000)
            .collect();

        if repairs.is_empty() {
            return None;
        }

        let best = repairs
            .into_iter()
            .filter_map(|p| {
                let age_ms = now - p.last_seen_ms;
                let weighted_support = p.count as f64 * Self::repair_recency_weight(age_ms);
                let source_count = Self::source_count(&p.source_ids);
                let (confidence, score) = Self::repair_confidence(weighted_support, p.count)?;
                let step = match &p.repair_target {
                    Some(target) => format!("{} {}", p.repair_tool, target),
                    None => p.repair_tool.clone(),
                };
                let candidate = StepCandidate::single(
                    p.repair_tool.clone(),
                    p.repair_target.clone(),
                    confidence,
                    p.count,
                    source_count,
                );
                Some((
                    weighted_support + source_count as f64 * 0.1,
                    p.last_seen_ms,
                    RepairHint {
                        body: format!(
                            "  repair path after {current_tool} failure: {step} ({confidence}, {}x)",
                            p.count
                        ),
                        score: score + Self::independence_bonus(source_count),
                        candidate,
                    },
                ))
            })
            .max_by(|a, b| {
                a.0.partial_cmp(&b.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.1.cmp(&b.1))
            })?;

        Some(best.2)
    }

    /// Suggest a short multi-step repair trajectory after failures of the current tool.
    /// Uses only recent local actions, so it stays cheap enough for prehook.
    pub fn repair_trajectory_hint(&self, current_tool: &str) -> Option<RepairHint> {
        use std::collections::HashMap;

        if self.recent_actions.len() < 2 {
            return None;
        }

        let now = chrono::Utc::now().timestamp_millis();
        let actions: Vec<_> = self.recent_actions.iter().rev().collect();
        let mut patterns: HashMap<String, RepairTrajectoryPattern> = HashMap::new();

        for (i, action) in actions.iter().enumerate() {
            if action.tool != current_tool || action.outcome != "failed" {
                continue;
            }

            let mut steps: Vec<StepAction> = Vec::new();
            let mut last_ts = action.timestamp_ms;
            for next in actions.iter().skip(i + 1) {
                if next.timestamp_ms - last_ts > 600_000 {
                    break;
                }
                if next.outcome == "failed" {
                    break;
                }
                steps.push(Self::step_action(&next.tool, next.file_path.as_deref()));
                last_ts = next.timestamp_ms;
                if steps.len() == 2 {
                    break;
                }
            }

            if steps.is_empty() {
                continue;
            }

            let key = steps
                .iter()
                .map(StepAction::render)
                .collect::<Vec<_>>()
                .join(" -> ");
            let age_ms = now - action.timestamp_ms;
            let weight = Self::repair_recency_weight(age_ms);
            let entry = patterns.entry(key).or_insert(RepairTrajectoryPattern {
                weighted_support: 0.0,
                count: 0,
                last_seen_ms: action.timestamp_ms,
                steps: steps.clone(),
                source_ids: Vec::new(),
            });
            entry.weighted_support += weight;
            entry.count += 1;
            entry.last_seen_ms = entry.last_seen_ms.max(action.timestamp_ms);
            Self::push_unique_source(&mut entry.source_ids, action.session_id.as_deref());
        }

        let best = patterns
            .into_iter()
            .filter_map(|(trajectory, pattern)| {
                let source_count = Self::source_count(&pattern.source_ids);
                let weighted_support = pattern.weighted_support;
                let count = pattern.count;
                let (confidence, score) = Self::repair_confidence(weighted_support, count)?;
                let candidate =
                    StepCandidate::sequence(pattern.steps, confidence, count, source_count);
                Some((
                    weighted_support + source_count as f64 * 0.1,
                    pattern.last_seen_ms,
                    RepairHint {
                        body: format!(
                            "  repair trajectory after {current_tool} failure: {trajectory} ({confidence}, {count}x)",
                        ),
                        score: score + Self::independence_bonus(source_count),
                        candidate,
                    },
                ))
            })
            .max_by(|a, b| {
                a.0.partial_cmp(&b.0)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.1.cmp(&b.1))
            })?;

        Some(best.2)
    }

    /// Generate decision context hints for a file operation.
    /// Shows: (1) what was read before previous edits of this file, (2) files co-edited with this file.
    pub fn decision_hints(&self, tool_name: &str, current_file: Option<&str>) -> Option<String> {
        let file = current_file?;
        if !matches!(tool_name, "Edit" | "Write" | "Read") {
            return None;
        }

        let _now = chrono::Utc::now().timestamp_millis();
        let mut lines: Vec<String> = Vec::new();

        // 1. Co-edit pattern: files edited within 5 minutes of editing this file
        if matches!(tool_name, "Edit" | "Write") {
            let mut co_edits: std::collections::HashMap<String, u32> =
                std::collections::HashMap::new();

            let actions: Vec<_> = self.recent_actions.iter().collect();
            for (i, action) in actions.iter().enumerate() {
                if action.file_path.as_deref() != Some(file) {
                    continue;
                }
                if !matches!(action.tool.as_str(), "Edit" | "Write") {
                    continue;
                }

                let start = i.saturating_sub(10);
                let end = (i + 10).min(actions.len());
                for (j, other) in actions.iter().enumerate().take(end).skip(start) {
                    if j == i {
                        continue;
                    }
                    if !matches!(other.tool.as_str(), "Edit" | "Write") {
                        continue;
                    }
                    if let Some(ref other_path) = other.file_path
                        && other_path != file
                        && (other.timestamp_ms - action.timestamp_ms).abs() < 300_000
                    {
                        let short = Self::file_name(other_path);
                        *co_edits.entry(short.to_string()).or_insert(0) += 1;
                    }
                }
            }

            if !co_edits.is_empty() {
                let mut sorted: Vec<_> = co_edits.into_iter().collect();
                sorted.sort_by(|a, b| b.1.cmp(&a.1));
                let top: Vec<String> = sorted
                    .iter()
                    .take(3)
                    .map(|(name, count)| format!("{name} ({count}x)"))
                    .collect();
                let fname = Self::file_name(file);
                lines.push(format!("  co-edited with {fname}: {}", top.join(", ")));
            }
        }

        // 2. Preparation pattern: what was read before previous edits of this file
        if matches!(tool_name, "Edit" | "Write") {
            let actions: Vec<_> = self.recent_actions.iter().collect();
            let mut prep_files: std::collections::HashMap<String, u32> =
                std::collections::HashMap::new();

            for (i, action) in actions.iter().enumerate() {
                if action.file_path.as_deref() != Some(file) {
                    continue;
                }
                if !matches!(action.tool.as_str(), "Edit" | "Write") {
                    continue;
                }

                let start = i + 1;
                let end = (i + 6).min(actions.len());
                for prev in actions.iter().take(end).skip(start) {
                    if prev.tool != "Read" {
                        continue;
                    }
                    if let Some(ref read_path) = prev.file_path
                        && read_path != file
                    {
                        let short = Self::file_name(read_path);
                        *prep_files.entry(short.to_string()).or_insert(0) += 1;
                    }
                }
            }

            if !prep_files.is_empty() {
                let mut sorted: Vec<_> = prep_files.into_iter().collect();
                sorted.sort_by(|a, b| b.1.cmp(&a.1));
                let top: Vec<String> = sorted
                    .iter()
                    .take(3)
                    .map(|(name, count)| format!("{name} ({count}x)"))
                    .collect();
                lines.push(format!("  prep reads before editing: {}", top.join(", ")));
            }
        }

        if lines.is_empty() {
            None
        } else {
            Some(lines.join("\n"))
        }
    }

    /// Generate context hints for prehook injection.
    /// Returns None if workspace is empty or stale (>24h).
    pub fn context_hints(&self, current_tool: &str, current_file: Option<&str>) -> Option<String> {
        let now = chrono::Utc::now().timestamp_millis();
        let age_hours = (now - self.updated_ms) as f64 / 3_600_000.0;

        if self.updated_ms == 0 || age_hours > 24.0 {
            return None;
        }

        let mut lines: Vec<String> = Vec::new();

        if let Some(file) = current_file {
            let file_history: Vec<&RecentFile> = self
                .recent_files
                .iter()
                .filter(|f| f.path == file)
                .take(3)
                .collect();

            if !file_history.is_empty() {
                lines.push(format!("  file history for {file}:"));
                for f in &file_history {
                    let age = Self::age_str(now, f.timestamp_ms);
                    lines.push(format!(
                        "    {age}: {action} — {ctx} [{outcome}]",
                        action = f.action,
                        ctx = f.context,
                        outcome = f.outcome
                    ));
                }
            }
        }

        let recent_tool_errors: Vec<&RecentError> = self
            .recent_errors
            .iter()
            .filter(|e| e.tool == current_tool && (now - e.timestamp_ms) < 3_600_000)
            .take(2)
            .collect();

        if !recent_tool_errors.is_empty() {
            lines.push(format!("  recent {current_tool} errors:"));
            for e in &recent_tool_errors {
                let age = Self::age_str(now, e.timestamp_ms);
                let snippet = if e.error_snippet.len() > 120 {
                    format!("{}...", &e.error_snippet[..120])
                } else {
                    e.error_snippet.clone()
                };
                lines.push(format!("    {age}: {snippet}"));
            }
        }

        if let Some(prev) = self.sessions.front() {
            let session_age_h = (now - prev.last_seen_ms) as f64 / 3_600_000.0;
            if session_age_h > 0.08 && session_age_h < 24.0 {
                let caps = prev.top_capabilities.join(", ");
                lines.push(format!(
                    "  previous session ({:.0}h ago): {} tool calls, {} errors, used: {caps}",
                    session_age_h, prev.tool_count, prev.error_count
                ));
            }
        }

        if lines.is_empty() {
            None
        } else {
            Some(lines.join("\n"))
        }
    }

    pub(crate) fn age_str(now_ms: i64, then_ms: i64) -> String {
        let diff_s = (now_ms - then_ms) / 1000;
        if diff_s < 60 {
            format!("{diff_s}s ago")
        } else if diff_s < 3600 {
            format!("{}m ago", diff_s / 60)
        } else if diff_s < 86400 {
            format!("{}h ago", diff_s / 3600)
        } else {
            format!("{}d ago", diff_s / 86400)
        }
    }
}
