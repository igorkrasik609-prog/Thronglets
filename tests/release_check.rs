use std::io::Write;
use std::process::{Command, Output, Stdio};

use thronglets::context::simhash;
use thronglets::identity::NodeIdentity;
use thronglets::storage::TraceStore;
use thronglets::trace::{Outcome, Trace};

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

fn sparse_profile_input() -> String {
    let mut lines = Vec::new();
    lines.push("[thronglets:prehook] tool=Edit emitted=2 stdout_bytes=40 output_mode=next-step decision_path=repair evidence_scope=collective file_guidance_gate=open collective_queries_used=0 total_us=300".to_string());
    lines.push("[thronglets:prehook] tool=Edit emitted=1 stdout_bytes=30 output_mode=context-only decision_path=history evidence_scope=none file_guidance_gate=closed collective_queries_used=0 total_us=200".to_string());
    for _ in 0..8 {
        lines.push("[thronglets:prehook] tool=Bash emitted=0 stdout_bytes=0 output_mode=silent decision_path=none evidence_scope=none file_guidance_gate=na collective_queries_used=0 total_us=100".to_string());
    }
    format!("{}\n", lines.join("\n"))
}

fn run_release_check(data_dir: &str, args: &[&str], input: &str) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_thronglets"))
        .args(["--data-dir", data_dir, "release-check"])
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn thronglets release-check");

    {
        let mut stdin = child.stdin.take().expect("stdin available");
        stdin.write_all(input.as_bytes()).expect("write stdin");
    }

    child.wait_with_output().expect("wait for thronglets")
}

#[test]
fn release_check_passes_with_good_profile_and_skips_thin_eval() {
    let dir = tempfile::TempDir::new().unwrap();

    let output = run_release_check(dir.path().to_str().unwrap(), &[], &sparse_profile_input());

    assert!(
        output.status.success(),
        "release-check failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("PASS"));
    assert!(stdout.contains("profile: PASS"));
    assert!(stdout.contains("eval: SKIP"));
    assert!(stdout.contains("not enough recent session history"));
}

#[test]
fn release_check_fails_when_eval_finds_noisy_adjacency() {
    let dir = tempfile::TempDir::new().unwrap();
    let store = TraceStore::open(&dir.path().join("traces.db")).unwrap();
    let identity = NodeIdentity::generate();

    let mut timestamp = chrono::Utc::now().timestamp_millis() as u64 - 10_000;
    for session in ["s1", "s2", "s3", "s4", "s5", "s6", "s7"] {
        let companion = if matches!(session, "s1" | "s2") {
            "helper.rs"
        } else {
            "other.rs"
        };
        let events = [
            (
                "claude-code/Edit",
                Outcome::Succeeded,
                "edit file: main.rs".to_string(),
            ),
            (
                "claude-code/Edit",
                Outcome::Succeeded,
                format!("edit file: {companion}"),
            ),
        ];
        for (capability, outcome, context) in events {
            let trace = make_trace(&identity, capability, outcome, &context, session, timestamp);
            store.insert(&trace).unwrap();
            timestamp += 1_000;
        }
        timestamp += 60_000;
    }

    let output = run_release_check(
        dir.path().to_str().unwrap(),
        &["--global"],
        &sparse_profile_input(),
    );

    assert!(
        !output.status.success(),
        "release-check unexpectedly passed: {}",
        String::from_utf8_lossy(&output.stdout)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("FAIL"));
    assert!(stdout.contains("profile: PASS"));
    assert!(stdout.contains("eval: FAIL"));
    assert!(stdout.contains("violations:"));
    assert!(stdout.contains("adjacency precision"));
    assert!(stdout.contains("10.0%"));
}

#[test]
fn release_check_can_require_profile_samples() {
    let dir = tempfile::TempDir::new().unwrap();

    let output = run_release_check(
        dir.path().to_str().unwrap(),
        &["--require-profile-samples"],
        "",
    );

    assert!(
        !output.status.success(),
        "release-check unexpectedly passed"
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("FAIL"));
    assert!(stdout.contains("profile: FAIL"));
    assert!(stdout.contains("no prehook profile samples found"));
}
