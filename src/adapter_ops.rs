use std::path::Path;

use crate::responses::*;
use crate::setup_support::{
    AdapterApplyResult, AdapterDetection, AdapterDoctor, AdapterKind, AdapterPlan,
    clear_restart_pending, detect_adapter, doctor_adapter, install_claude, install_codex,
    install_cursor, install_openclaw, install_plan, set_restart_pending,
};

pub(crate) fn collect_restart_commands(
    commands: impl IntoIterator<Item = Option<String>>,
) -> Vec<String> {
    let mut values: Vec<_> = commands.into_iter().flatten().collect();
    values.sort();
    values.dedup();
    values
}

pub(crate) fn selected_adapters(target: crate::cli::AdapterArg) -> Vec<AdapterKind> {
    [
        AdapterKind::Claude,
        AdapterKind::Codex,
        AdapterKind::Cursor,
        AdapterKind::OpenClaw,
        AdapterKind::Generic,
    ]
    .into_iter()
    .filter(|adapter| target.includes(*adapter))
    .collect()
}

pub(crate) fn selected_known_adapters(target: crate::cli::AdapterArg) -> Vec<AdapterKind> {
    selected_adapters(target)
        .into_iter()
        .filter(|adapter| !matches!(adapter, AdapterKind::Generic))
        .collect()
}

pub(crate) fn selected_restart_adapters(target: crate::cli::AdapterArg) -> Vec<AdapterKind> {
    selected_known_adapters(target)
        .into_iter()
        .filter(|adapter| {
            matches!(
                adapter,
                AdapterKind::Codex | AdapterKind::Cursor | AdapterKind::OpenClaw
            )
        })
        .collect()
}

pub(crate) fn summarize_doctor_reports(
    target: crate::cli::AdapterArg,
    reports: Vec<AdapterDoctor>,
) -> DoctorData {
    let healthy = !doctor_should_fail(target, &reports);
    let restart_pending = reports.iter().any(|report| report.restart_pending);
    let restart_commands =
        collect_restart_commands(reports.iter().map(|report| report.restart_command.clone()));
    let mut next_steps: Vec<_> = reports
        .iter()
        .filter(|report| doctor_report_requires_action(target, report))
        .flat_map(|report| report.remediation.iter().cloned())
        .collect();
    next_steps.sort();
    next_steps.dedup();

    DoctorData {
        summary: DoctorSummary {
            status: if !healthy {
                "needs-fix"
            } else if restart_pending {
                "restart-pending"
            } else {
                "healthy"
            },
            healthy,
            restart_pending,
            restart_commands,
            next_steps,
        },
        reports,
    }
}

pub(crate) fn summarize_detections(detections: Vec<AdapterDetection>) -> DetectData {
    let detected_agents = detections
        .iter()
        .filter(|detection| detection.present)
        .map(|detection| detection.agent.clone())
        .collect();
    let recommended_agents = detections
        .iter()
        .filter(|detection| detection.present && detection.apply_by_default)
        .map(|detection| detection.agent.clone())
        .collect();

    DetectData {
        summary: DetectSummary {
            status: "ready",
            detected_agents,
            recommended_agents,
        },
        detections,
    }
}

pub(crate) fn summarize_install_plans(plans: Vec<AdapterPlan>) -> InstallPlanData {
    let restart_required = plans.iter().any(|plan| plan.requires_restart);
    let restart_commands =
        collect_restart_commands(plans.iter().map(|plan| plan.restart_command.clone()));
    let mut next_steps: Vec<_> = plans
        .iter()
        .filter_map(|plan| plan.apply_command.clone())
        .collect();
    next_steps.sort();
    next_steps.dedup();

    InstallPlanData {
        summary: PlanSummary {
            status: "planned",
            restart_required,
            restart_commands,
            next_steps,
        },
        plans,
    }
}

pub(crate) fn summarize_apply_results(results: Vec<AdapterApplyResult>) -> ApplyPlanData {
    let restart_required = results.iter().any(|result| result.requires_restart);
    let restart_commands =
        collect_restart_commands(results.iter().map(|result| result.restart_command.clone()));
    let mut next_steps = Vec::new();
    if restart_required {
        next_steps.push("Restart the targeted agent so the new integration is loaded.".into());
    }

    ApplyPlanData {
        summary: ApplySummary {
            status: "applied",
            restart_required,
            restart_commands,
            next_steps,
        },
        results,
    }
}

pub(crate) fn summarize_clear_restart_results(
    results: Vec<ClearRestartResult>,
) -> ClearRestartData {
    let cleared_agents: Vec<_> = results
        .iter()
        .filter(|result| result.cleared)
        .map(|result| result.agent.clone())
        .collect();
    let mut next_steps = Vec::new();
    if cleared_agents.is_empty() {
        next_steps
            .push("Run `thronglets doctor --agent <adapter>` to confirm current status.".into());
    }

    ClearRestartData {
        summary: ClearRestartSummary {
            status: if cleared_agents.is_empty() {
                "already-clear"
            } else {
                "cleared"
            },
            cleared_agents,
            next_steps,
        },
        results,
    }
}

pub(crate) fn summarize_runtime_ready_results(
    results: Vec<RuntimeReadyResult>,
) -> RuntimeReadyData {
    let ready_agents: Vec<_> = results
        .iter()
        .filter(|result| result.ready)
        .map(|result| result.agent.clone())
        .collect();
    let mut next_steps = Vec::new();
    if ready_agents.is_empty() {
        next_steps
            .push("Run `thronglets doctor --agent <adapter>` to confirm current status.".into());
    }

    RuntimeReadyData {
        summary: RuntimeReadySummary {
            status: if ready_agents.is_empty() {
                "already-ready"
            } else {
                "ready"
            },
            ready_agents,
            next_steps,
        },
        results,
    }
}

pub(crate) fn apply_selected_adapters(
    target: crate::cli::AdapterArg,
    home_dir: &Path,
    data_dir: &Path,
    bin_path: &Path,
) -> std::io::Result<Vec<AdapterApplyResult>> {
    let mut results = Vec::new();

    for agent in selected_known_adapters(target) {
        match agent {
            AdapterKind::Claude => {
                let result = install_claude(home_dir, data_dir, bin_path)?;
                let mut changed = Vec::new();
                if result.added_post_hook {
                    changed.push("installed PostToolUse hook".into());
                }
                if result.added_pre_hook {
                    changed.push("installed PreToolUse hook".into());
                }
                if result.added_lifecycle_hooks > 0 {
                    changed.push(format!(
                        "installed {} lifecycle hooks (SessionStart/End, SubagentStart/Stop)",
                        result.added_lifecycle_hooks,
                    ));
                }
                if result.mcp_hotloaded {
                    changed.push("MCP server hot-loaded into current session".into());
                }
                if changed.is_empty() {
                    changed.push("hooks already present".into());
                }
                results.push(AdapterApplyResult {
                    agent: agent.key().into(),
                    applied: true,
                    changed,
                    requires_restart: false,
                    restart_command: None,
                    paths: vec![result.settings_path.display().to_string()],
                    note: None,
                });
            }
            AdapterKind::Codex => {
                let force = !matches!(target, crate::cli::AdapterArg::All);
                if let Some(result) = install_codex(home_dir, data_dir, bin_path, force)? {
                    set_restart_pending(data_dir, agent, true)?;
                    let mut changed = Vec::new();
                    if result.created_config {
                        changed.push("created Codex config".into());
                    }
                    if result.updated_server {
                        changed.push("installed Thronglets MCP server".into());
                    }
                    if result.updated_agents_memory {
                        changed.push("updated managed AGENTS block".into());
                    }
                    if changed.is_empty() {
                        changed.push("config already present".into());
                    }
                    results.push(AdapterApplyResult {
                        agent: agent.key().into(),
                        applied: true,
                        changed,
                        requires_restart: true,
                        restart_command: Some("Restart Codex".into()),
                        paths: vec![
                            result.config_path.display().to_string(),
                            result.agents_path.display().to_string(),
                        ],
                        note: Some("Restart Codex to load the MCP server.".into()),
                    });
                } else {
                    results.push(AdapterApplyResult {
                        agent: agent.key().into(),
                        applied: false,
                        changed: vec![],
                        requires_restart: false,
                        restart_command: None,
                        paths: vec![],
                        note: Some("Codex not detected; skipped in all-adapters mode.".into()),
                    });
                }
            }
            AdapterKind::Cursor => {
                let force = !matches!(target, crate::cli::AdapterArg::All);
                if let Some(result) = install_cursor(home_dir, data_dir, bin_path, force)? {
                    set_restart_pending(data_dir, agent, true)?;
                    let mut changed = Vec::new();
                    if result.created_config {
                        changed.push("created Cursor MCP config".into());
                    }
                    if result.updated_server {
                        changed.push("installed Thronglets MCP server".into());
                    }
                    if changed.is_empty() {
                        changed.push("config already present".into());
                    }
                    results.push(AdapterApplyResult {
                        agent: agent.key().into(),
                        applied: true,
                        changed,
                        requires_restart: true,
                        restart_command: Some("Restart Cursor".into()),
                        paths: vec![result.config_path.display().to_string()],
                        note: Some("Restart Cursor to load the MCP server.".into()),
                    });
                } else {
                    results.push(AdapterApplyResult {
                        agent: agent.key().into(),
                        applied: false,
                        changed: vec![],
                        requires_restart: false,
                        restart_command: None,
                        paths: vec![],
                        note: Some("Cursor not detected; skipped in all-adapters mode.".into()),
                    });
                }
            }
            AdapterKind::OpenClaw => {
                let force = !matches!(target, crate::cli::AdapterArg::All);
                if let Some(result) = install_openclaw(home_dir, data_dir, bin_path, true, force)? {
                    set_restart_pending(data_dir, agent, true)?;
                    let mut changed = Vec::new();
                    if result.created_config {
                        changed.push("created OpenClaw config".into());
                    } else {
                        changed.push("updated OpenClaw plugin config".into());
                    }
                    changed.push("wrote local plugin assets".into());
                    results.push(AdapterApplyResult {
                        agent: agent.key().into(),
                        applied: true,
                        changed,
                        requires_restart: true,
                        restart_command: Some("openclaw gateway restart".into()),
                        paths: vec![
                            result.config_path.display().to_string(),
                            result.plugin_dir.display().to_string(),
                        ],
                        note: Some(if result.restarted_gateway {
                            "Requested OpenClaw gateway restart.".into()
                        } else {
                            "OpenClaw gateway restart may still be required.".into()
                        }),
                    });
                } else {
                    results.push(AdapterApplyResult {
                        agent: agent.key().into(),
                        applied: false,
                        changed: vec![],
                        requires_restart: false,
                        restart_command: None,
                        paths: vec![],
                        note: Some("OpenClaw not detected; skipped in all-adapters mode.".into()),
                    });
                }
            }
            AdapterKind::Generic => {}
        }
    }

    Ok(results)
}

pub(crate) fn bootstrap_selected_adapters(
    target: crate::cli::AdapterArg,
    home_dir: &Path,
    data_dir: &Path,
    bin_path: &Path,
) -> std::io::Result<BootstrapData> {
    let detections: Vec<_> = selected_adapters(target)
        .into_iter()
        .map(|adapter| detect_adapter(home_dir, data_dir, adapter))
        .collect();
    let plans: Vec<_> = selected_adapters(target)
        .into_iter()
        .map(|adapter| install_plan(home_dir, data_dir, adapter))
        .collect();
    let applied = apply_selected_adapters(target, home_dir, data_dir, bin_path)?;
    let doctor_reports: Vec<_> = selected_adapters(target)
        .into_iter()
        .map(|adapter| doctor_adapter(home_dir, data_dir, adapter))
        .collect();
    let doctor_summary = summarize_doctor_reports(target, doctor_reports);
    let healthy = doctor_summary.summary.healthy;
    let restart_pending = doctor_summary.summary.restart_pending;
    let restart_required = applied.iter().any(|result| result.requires_restart);
    let mut next_steps = doctor_summary.summary.next_steps.clone();
    let restart_commands =
        collect_restart_commands(applied.iter().map(|result| result.restart_command.clone()));
    if restart_required {
        next_steps.push("Restart the targeted agent so the new integration is loaded.".into());
    }
    next_steps.sort();
    next_steps.dedup();

    Ok(BootstrapData {
        summary: BootstrapSummary {
            status: if !healthy {
                "needs-fix"
            } else if restart_pending {
                "restart-pending"
            } else {
                "healthy"
            },
            healthy,
            restart_pending,
            restart_required,
            restart_commands,
            next_steps,
        },
        detections,
        plans,
        results: applied,
        reports: doctor_summary.reports,
    })
}

pub(crate) fn clear_selected_restart_state(
    target: crate::cli::AdapterArg,
    data_dir: &Path,
) -> std::io::Result<ClearRestartData> {
    let mut results = Vec::new();

    for agent in selected_restart_adapters(target) {
        let cleared = clear_restart_pending(data_dir, agent)?;
        results.push(ClearRestartResult {
            agent: agent.key().into(),
            cleared,
            note: if cleared {
                Some("Cleared persisted restart-pending state.".into())
            } else {
                Some("No persisted restart-pending state was present.".into())
            },
        });
    }

    Ok(summarize_clear_restart_results(results))
}

pub(crate) fn mark_selected_runtime_ready(
    target: crate::cli::AdapterArg,
    data_dir: &Path,
) -> std::io::Result<RuntimeReadyData> {
    let mut results = Vec::new();

    for agent in selected_restart_adapters(target) {
        let ready = clear_restart_pending(data_dir, agent)?;
        results.push(RuntimeReadyResult {
            agent: agent.key().into(),
            ready,
            note: if ready {
                Some("Marked runtime as ready after a successful reload.".into())
            } else {
                Some("No persisted restart-pending state was present.".into())
            },
        });
    }

    Ok(summarize_runtime_ready_results(results))
}

pub(crate) fn doctor_should_fail(
    target: crate::cli::AdapterArg,
    reports: &[AdapterDoctor],
) -> bool {
    reports.iter().any(|report| match target {
        crate::cli::AdapterArg::All => report.present && !report.healthy,
        _ => report.agent != AdapterKind::Generic.key() && !report.healthy,
    })
}

pub(crate) fn doctor_report_requires_action(
    target: crate::cli::AdapterArg,
    report: &AdapterDoctor,
) -> bool {
    match target {
        crate::cli::AdapterArg::All => {
            report.present && (!report.healthy || report.restart_pending)
        }
        _ => {
            report.agent != AdapterKind::Generic.key()
                && (!report.healthy || report.restart_pending)
        }
    }
}
