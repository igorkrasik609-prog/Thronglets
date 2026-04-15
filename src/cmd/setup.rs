use super::*;

use crate::cli::{AdapterArg, RuntimeArg};
use crate::setup_support::detect_adapter;

pub(crate) fn setup(base: &BaseCtx) {
    let bin = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("thronglets"));
    codesign_if_macos(&bin);
    let hd = home_dir();
    let report = bootstrap_selected_adapters(AdapterArg::All, &hd, &base.dir, &bin)
        .expect("failed to bootstrap adapter plan");
    render_setup_report(&report);
    if !report.summary.healthy {
        std::process::exit(1);
    }
}

pub(crate) fn detect(base: &BaseCtx, agent: AdapterArg, json: bool) {
    let hd = home_dir();
    let detections: Vec<_> = selected_adapters(agent)
        .into_iter()
        .map(|adapter| detect_adapter(&hd, &base.dir, adapter))
        .collect();
    let summary = summarize_detections(detections);
    if json {
        print_machine_json("detect", &summary);
    } else {
        render_detect_report(&summary);
    }
}

pub(crate) fn install_plan_cmd(base: &BaseCtx, agent: AdapterArg, runtime: RuntimeArg, json: bool) {
    let hd = home_dir();
    let mut plans: Vec<_> = selected_adapters(agent)
        .into_iter()
        .map(|adapter| crate::setup_support::install_plan(&hd, &base.dir, adapter))
        .collect();
    filter_generic_runtime_snippets(&mut plans, runtime.key());
    let summary = summarize_install_plans(plans);
    if json {
        print_machine_json("install-plan", &summary);
    } else {
        render_install_plan_report(&summary);
    }
}

pub(crate) fn apply_plan(base: &BaseCtx, agent: AdapterArg, json: bool) {
    let bin = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("thronglets"));
    codesign_if_macos(&bin);
    let hd = home_dir();
    let results =
        apply_selected_adapters(agent, &hd, &base.dir, &bin).expect("failed to apply adapter plan");
    let summary = summarize_apply_results(results);
    if json {
        print_machine_json("apply-plan", &summary);
    } else {
        render_apply_plan_report(&summary);
    }
}

pub(crate) fn doctor(base: &BaseCtx, agent: AdapterArg, json: bool) {
    let hd = home_dir();
    let reports: Vec<_> = selected_adapters(agent)
        .into_iter()
        .map(|adapter| crate::setup_support::doctor_adapter(&hd, &base.dir, adapter))
        .collect();
    let summary = summarize_doctor_reports(agent, reports);
    if json {
        print_machine_json("doctor", &summary);
    } else {
        render_doctor_report(&summary);
    }
    if !summary.summary.healthy {
        std::process::exit(1);
    }
}

pub(crate) fn bootstrap(base: &BaseCtx, agent: AdapterArg, json: bool) {
    let bin = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("thronglets"));
    codesign_if_macos(&bin);
    let hd = home_dir();
    let report = bootstrap_selected_adapters(agent, &hd, &base.dir, &bin)
        .expect("failed to bootstrap adapter plan");
    if json {
        print_machine_json("bootstrap", &report);
    } else {
        render_bootstrap_report(&report);
    }
    if !report.summary.healthy {
        std::process::exit(1);
    }
}

pub(crate) fn clear_restart(base: &BaseCtx, agent: AdapterArg, json: bool) {
    let report =
        clear_selected_restart_state(agent, &base.dir).expect("failed to clear restart state");
    if json {
        print_machine_json("clear-restart", &report);
    } else {
        render_clear_restart_report(&report);
    }
}

pub(crate) fn runtime_ready(base: &BaseCtx, agent: AdapterArg, json: bool) {
    let report =
        mark_selected_runtime_ready(agent, &base.dir).expect("failed to mark runtime ready");
    if json {
        print_machine_json("runtime-ready", &report);
    } else {
        render_runtime_ready_report(&report);
    }
}
