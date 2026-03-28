use std::process::Command;

use tempfile::TempDir;
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

#[test]
fn eval_signals_reports_holdout_metrics() {
    let dir = TempDir::new().unwrap();
    let store = TraceStore::open(&dir.path().join("traces.db")).unwrap();
    let identity = NodeIdentity::generate();

    let mut timestamp = chrono::Utc::now().timestamp_millis() as u64 - 10_000;
    for session in ["s1", "s2", "s3"] {
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

    let output = Command::new(env!("CARGO_BIN_EXE_thronglets"))
        .args([
            "--data-dir",
            dir.path().to_str().unwrap(),
            "eval-signals",
            "--hours",
            "168",
            "--max-sessions",
            "10",
        ])
        .output()
        .expect("spawn thronglets eval-signals");

    assert!(
        output.status.success(),
        "eval-signals failed: {}",
        String::from_utf8_lossy(&output.stderr),
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("sessions considered: 3"));
    assert!(stdout.contains("repair first-step precision"));
    assert!(stdout.contains("preparation precision"));
    assert!(stdout.contains("adjacency precision"));
}
