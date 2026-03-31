use serde_json::Value;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn run_bin(args: &[&str], data_dir: &Path) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_thronglets"))
        .args(["--data-dir", data_dir.to_str().unwrap()])
        .args(args)
        .output()
        .expect("failed to run thronglets");
    assert!(
        output.status.success(),
        "command failed: {}\nstderr={}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("stdout should be valid json")
}

fn run_text(args: &[&str], data_dir: &Path) {
    let output = Command::new(env!("CARGO_BIN_EXE_thronglets"))
        .args(["--data-dir", data_dir.to_str().unwrap()])
        .args(args)
        .output()
        .expect("failed to run thronglets");
    assert!(
        output.status.success(),
        "command failed: {}\nstderr={}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn space_snapshot_is_quiet_when_no_recent_activity_exists() {
    let temp = TempDir::new().unwrap();
    let data_dir = temp.path().join("data");

    let data = run_bin(&["space", "--space", "psyche", "--json"], &data_dir);

    assert_eq!(data["schema_version"], "thronglets.space.v2");
    assert_eq!(data["command"], "space");
    assert_eq!(data["data"]["summary"]["status"], "quiet");
    assert_eq!(data["data"]["summary"]["active_sessions"], 0);
    assert_eq!(data["data"]["summary"]["signal_count"], 0);
    assert_eq!(data["data"]["local_feedback"]["positive_24h"], 0);
    assert_eq!(data["data"]["local_feedback"]["negative_24h"], 0);
}

#[test]
fn space_snapshot_surfaces_active_presence_and_signals() {
    let temp = TempDir::new().unwrap();
    let data_dir = temp.path().join("data");

    run_bin(
        &[
            "presence-ping",
            "--space",
            "psyche",
            "--mode",
            "focus",
            "--session-id",
            "codex-psyche-1",
            "--json",
        ],
        &data_dir,
    );
    run_text(
        &[
            "signal-post",
            "--kind",
            "recommend",
            "--space",
            "psyche",
            "--context",
            "shape the psyche roadmap",
            "--message",
            "read the latest plan before editing",
        ],
        &data_dir,
    );

    let data = run_bin(&["space", "--space", "psyche", "--json"], &data_dir);

    let status = data["data"]["summary"]["status"].as_str().unwrap();
    assert!(matches!(status, "active" | "converging"));
    assert_eq!(data["data"]["summary"]["active_sessions"], 1);
    assert_eq!(data["data"]["summary"]["signal_count"], 1);
    assert_eq!(data["data"]["sessions"].as_array().unwrap().len(), 1);
    assert_eq!(data["data"]["signals"].as_array().unwrap().len(), 1);
}
