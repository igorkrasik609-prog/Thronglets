use serde_json::Value;
use std::path::Path;
use std::process::Command;
use tempfile::TempDir;
use thronglets::network_state::NetworkSnapshot;

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

fn run_bin_raw(args: &[&str], data_dir: &Path) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_thronglets"))
        .args(["--data-dir", data_dir.to_str().unwrap()])
        .args(args)
        .output()
        .expect("failed to run thronglets")
}

#[test]
fn id_json_surfaces_identity_summary() {
    let temp = TempDir::new().unwrap();
    let data_dir = temp.path().join("data");

    let data = run_bin(&["id", "--json"], &data_dir);

    assert_eq!(data["schema_version"], "thronglets.identity.v2");
    assert_eq!(data["command"], "id");
    assert_eq!(data["data"]["summary"]["status"], "healthy");
    assert_eq!(data["data"]["summary"]["binding_source"], "local");
    assert_eq!(data["data"]["summary"]["owner_account"], Value::Null);
    assert_eq!(data["data"]["summary"]["joined_from_device"], Value::Null);
    assert!(
        data["data"]["summary"]["device_identity"]
            .as_str()
            .unwrap()
            .starts_with("oasyce1")
    );
}

#[test]
fn status_json_surfaces_quiet_substrate_activity() {
    let temp = TempDir::new().unwrap();
    let data_dir = temp.path().join("data");

    let status = run_bin(&["status", "--json"], &data_dir);

    assert_eq!(status["command"], "status");
    assert_eq!(status["data"]["summary"]["status"], "local-only");
    assert_eq!(status["data"]["summary"]["network_path_ready"], false);
    assert_eq!(status["data"]["substrate"]["activity"], "quiet");
    assert_eq!(status["data"]["substrate"]["recent_interventions_15m"], 0);
    assert_eq!(status["data"]["network"]["activity"], "offline");
    assert_eq!(status["data"]["network"]["transport_mode"], "offline");
    assert_eq!(status["data"]["network"]["vps_dependency_level"], "offline");
}

#[test]
fn connection_join_json_preserves_secondary_device_and_owner_binding() {
    let temp = TempDir::new().unwrap();
    let primary_dir = temp.path().join("primary");
    let secondary_dir = temp.path().join("secondary");
    let connection_file = temp.path().join("device.connection.json");

    let owner = run_bin(
        &["owner-bind", "--owner-account", "oasyce1owner", "--json"],
        &primary_dir,
    );
    assert_eq!(owner["data"]["summary"]["status"], "updated");

    let exported = run_bin(
        &[
            "connection-export",
            "--output",
            connection_file.to_str().unwrap(),
            "--json",
        ],
        &primary_dir,
    );
    let primary_device = exported["data"]["identity"]["device_identity"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(exported["command"], "connection-export");
    assert_eq!(exported["data"]["identity"]["owner_account"], "oasyce1owner");
    assert_eq!(exported["data"]["signed_by_device"], primary_device);
    assert_eq!(exported["data"]["peer_seed_scope"], "remembered");
    assert_eq!(exported["data"]["trusted_peer_seed_count"], 0);
    assert_eq!(exported["data"]["peer_seed_count"], 0);
    assert_eq!(exported["data"]["ttl_hours"], 24);
    assert!(exported["data"]["expires_at"].as_u64().unwrap() > 0);

    let joined = run_bin(
        &[
            "connection-join",
            "--file",
            connection_file.to_str().unwrap(),
            "--json",
        ],
        &secondary_dir,
    );
    let secondary_device = joined["data"]["identity"]["device_identity"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(joined["schema_version"], "thronglets.identity.v2");
    assert_eq!(joined["command"], "connection-join");
    assert_eq!(joined["data"]["summary"]["status"], "identity-only");
    assert_eq!(joined["data"]["summary"]["network_path_ready"], false);
    assert_eq!(joined["data"]["identity"]["owner_account"], "oasyce1owner");
    assert_eq!(
        joined["data"]["identity"]["binding_source"],
        "connection_file"
    );
    assert_eq!(
        joined["data"]["identity"]["joined_from_device"],
        primary_device.as_str()
    );
    assert_eq!(joined["data"]["signature_verified"], true);
    assert_eq!(joined["data"]["peer_seed_scope"], "remembered");
    assert_eq!(joined["data"]["imported_trusted_peer_seed_count"], 0);
    assert_eq!(joined["data"]["imported_peer_seed_count"], 0);
    assert_ne!(secondary_device, primary_device);

    let status = run_bin(&["status", "--json"], &secondary_dir);
    assert_eq!(status["command"], "status");
    assert_eq!(status["data"]["summary"]["status"], "identity-only");
    assert_eq!(status["data"]["identity"]["owner_account"], "oasyce1owner");
    assert_eq!(
        status["data"]["identity"]["binding_source"],
        "connection_file"
    );
    assert_eq!(
        status["data"]["identity"]["joined_from_device"],
        primary_device.as_str()
    );
    assert_eq!(
        status["data"]["identity"]["device_identity"],
        secondary_device.as_str()
    );
}

#[test]
fn connection_join_imports_peer_seeds_into_local_snapshot() {
    let temp = TempDir::new().unwrap();
    let primary_dir = temp.path().join("primary");
    let secondary_dir = temp.path().join("secondary");
    let connection_file = temp.path().join("device.connection.json");

    run_bin(
        &["owner-bind", "--owner-account", "oasyce1owner", "--json"],
        &primary_dir,
    );

    let mut snapshot = NetworkSnapshot::begin(1);
    snapshot.observe_peer_address("12D3KooWAlpha", "/ip4/10.0.0.1/tcp/4001");
    snapshot.merge_peer_seeds(["/ip4/10.0.0.9/tcp/4001".to_string()]);
    snapshot.save(&primary_dir);

    let exported = run_bin(
        &[
            "connection-export",
            "--output",
            connection_file.to_str().unwrap(),
            "--json",
        ],
        &primary_dir,
    );
    assert_eq!(exported["data"]["peer_seed_scope"], "remembered");
    assert_eq!(exported["data"]["trusted_peer_seed_count"], 0);
    assert_eq!(exported["data"]["peer_seed_count"], 2);

    let joined = run_bin(
        &[
            "connection-join",
            "--file",
            connection_file.to_str().unwrap(),
            "--json",
        ],
        &secondary_dir,
    );
    assert_eq!(joined["data"]["summary"]["status"], "identity-plus-peer-seeds");
    assert_eq!(joined["data"]["summary"]["network_path_ready"], true);
    assert_eq!(joined["data"]["peer_seed_scope"], "remembered");
    assert_eq!(joined["data"]["imported_trusted_peer_seed_count"], 0);
    assert_eq!(joined["data"]["imported_peer_seed_count"], 2);

    let status = run_bin(&["status", "--json"], &secondary_dir);
    assert_eq!(status["data"]["summary"]["status"], "network-paths-ready");
    assert_eq!(status["data"]["network"]["trusted_peer_seed_count"], 0);
    assert_eq!(status["data"]["network"]["peer_seed_count"], 2);
}

#[test]
fn connection_export_prefers_trusted_peer_seeds() {
    let temp = TempDir::new().unwrap();
    let primary_dir = temp.path().join("primary");
    let connection_file = temp.path().join("device.connection.json");

    run_bin(
        &["owner-bind", "--owner-account", "oasyce1owner", "--json"],
        &primary_dir,
    );

    let mut snapshot = NetworkSnapshot::begin(1);
    snapshot.observe_peer_address("12D3KooWAlpha", "/ip4/10.0.0.1/tcp/4001");
    snapshot.merge_peer_seeds(["/ip4/10.0.0.9/tcp/4001".to_string()]);
    snapshot.merge_trusted_peer_seeds(["/ip4/10.0.0.8/tcp/4001".to_string()]);
    snapshot.save(&primary_dir);

    let exported = run_bin(
        &[
            "connection-export",
            "--output",
            connection_file.to_str().unwrap(),
            "--json",
        ],
        &primary_dir,
    );
    assert_eq!(exported["data"]["peer_seed_scope"], "trusted");
    assert_eq!(exported["data"]["trusted_peer_seed_count"], 1);
    assert_eq!(exported["data"]["peer_seed_count"], 1);

    let inspected = run_bin(
        &[
            "connection-inspect",
            "--file",
            connection_file.to_str().unwrap(),
            "--json",
        ],
        &primary_dir,
    );
    assert_eq!(inspected["data"]["summary"]["status"], "trusted-same-owner-ready");
    assert_eq!(inspected["data"]["peer_seed_scope"], "trusted");
    assert_eq!(inspected["data"]["trusted_peer_seed_count"], 1);
    assert_eq!(inspected["data"]["peer_seed_count"], 1);
}

#[test]
fn expired_connection_file_join_fails() {
    let temp = TempDir::new().unwrap();
    let primary_dir = temp.path().join("primary");
    let secondary_dir = temp.path().join("secondary");
    let connection_file = temp.path().join("expired.connection.json");

    run_bin(
        &["owner-bind", "--owner-account", "oasyce1owner", "--json"],
        &primary_dir,
    );
    let exported = run_bin(
        &[
            "connection-export",
            "--output",
            connection_file.to_str().unwrap(),
            "--ttl-hours",
            "0",
            "--json",
        ],
        &primary_dir,
    );
    assert_eq!(exported["data"]["ttl_hours"], 0);

    let output = run_bin_raw(
        &[
            "connection-join",
            "--file",
            connection_file.to_str().unwrap(),
            "--json",
        ],
        &secondary_dir,
    );
    assert!(
        !output.status.success(),
        "expired join unexpectedly succeeded"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("connection file has expired"));
}

#[test]
fn connection_export_without_owner_binding_still_exports_network_pairing_file() {
    let temp = TempDir::new().unwrap();
    let primary_dir = temp.path().join("primary");
    let secondary_dir = temp.path().join("secondary");
    let connection_file = temp.path().join("device.connection.json");

    let exported = run_bin(
        &[
            "connection-export",
            "--output",
            connection_file.to_str().unwrap(),
            "--json",
        ],
        &primary_dir,
    );
    assert_eq!(exported["command"], "connection-export");
    assert_eq!(exported["data"]["summary"]["status"], "identity-only");
    assert_eq!(exported["data"]["identity"]["owner_account"], Value::Null);
    assert_eq!(exported["data"]["peer_seed_scope"], "remembered");
    assert_eq!(exported["data"]["peer_seed_count"], 0);

    let inspected = run_bin(
        &[
            "connection-inspect",
            "--file",
            connection_file.to_str().unwrap(),
            "--json",
        ],
        &primary_dir,
    );
    assert_eq!(inspected["data"]["summary"]["status"], "identity-only");
    assert_eq!(inspected["data"]["identity"]["owner_account"], Value::Null);
    assert_eq!(inspected["data"]["signature_verified"], true);

    let joined = run_bin(
        &[
            "connection-join",
            "--file",
            connection_file.to_str().unwrap(),
            "--json",
        ],
        &secondary_dir,
    );
    assert_eq!(joined["data"]["summary"]["status"], "identity-only");
    assert_eq!(joined["data"]["identity"]["owner_account"], Value::Null);
    assert_eq!(
        joined["data"]["identity"]["binding_source"],
        "connection_file"
    );
}

#[test]
fn owner_bind_after_ownerless_connection_join_preserves_join_origin() {
    let temp = TempDir::new().unwrap();
    let primary_dir = temp.path().join("primary");
    let secondary_dir = temp.path().join("secondary");
    let connection_file = temp.path().join("device.connection.json");

    run_bin(
        &[
            "connection-export",
            "--output",
            connection_file.to_str().unwrap(),
            "--json",
        ],
        &primary_dir,
    );
    let joined = run_bin(
        &[
            "connection-join",
            "--file",
            connection_file.to_str().unwrap(),
            "--json",
        ],
        &secondary_dir,
    );
    let primary_device = joined["data"]["identity"]["joined_from_device"]
        .as_str()
        .unwrap()
        .to_string();

    let bound = run_bin(
        &[
            "owner-bind",
            "--owner-account",
            "oasyce1owner",
            "--json",
        ],
        &secondary_dir,
    );
    assert_eq!(bound["data"]["summary"]["owner_account"], "oasyce1owner");
    assert_eq!(
        bound["data"]["summary"]["binding_source"],
        "connection_file"
    );
    assert_eq!(
        bound["data"]["summary"]["joined_from_device"],
        primary_device.as_str()
    );
}

#[test]
fn connection_inspect_json_surfaces_verified_metadata() {
    let temp = TempDir::new().unwrap();
    let primary_dir = temp.path().join("primary");
    let connection_file = temp.path().join("device.connection.json");

    run_bin(
        &["owner-bind", "--owner-account", "oasyce1owner", "--json"],
        &primary_dir,
    );
    run_bin(
        &[
            "connection-export",
            "--output",
            connection_file.to_str().unwrap(),
            "--ttl-hours",
            "12",
            "--json",
        ],
        &primary_dir,
    );

    let inspected = run_bin(
        &[
            "connection-inspect",
            "--file",
            connection_file.to_str().unwrap(),
            "--json",
        ],
        &primary_dir,
    );
    assert_eq!(inspected["command"], "connection-inspect");
    assert_eq!(inspected["data"]["summary"]["status"], "identity-only");
    assert_eq!(
        inspected["data"]["identity"]["owner_account"],
        "oasyce1owner"
    );
    assert_eq!(inspected["data"]["signature_verified"], true);
    assert_eq!(inspected["data"]["peer_seed_count"], 0);
    assert_eq!(inspected["data"]["trusted_peer_seed_count"], 0);
    assert_eq!(inspected["data"]["ttl_hours"], 12);
    assert!(inspected["data"]["expires_at"].as_u64().unwrap() > 0);
}

#[test]
fn owner_bind_rejects_conflicting_owner() {
    let temp = TempDir::new().unwrap();
    let data_dir = temp.path().join("data");

    run_bin(
        &["owner-bind", "--owner-account", "oasyce1owner", "--json"],
        &data_dir,
    );
    let output = run_bin_raw(
        &["owner-bind", "--owner-account", "oasyce1other", "--json"],
        &data_dir,
    );
    assert!(
        !output.status.success(),
        "conflicting owner-bind unexpectedly succeeded"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("refusing to overwrite"));
}

#[test]
fn connection_join_rejects_conflicting_owner_binding() {
    let temp = TempDir::new().unwrap();
    let primary_dir = temp.path().join("primary");
    let secondary_dir = temp.path().join("secondary");
    let connection_file = temp.path().join("device.connection.json");

    run_bin(
        &["owner-bind", "--owner-account", "oasyce1owner", "--json"],
        &primary_dir,
    );
    run_bin(
        &["owner-bind", "--owner-account", "oasyce1other", "--json"],
        &secondary_dir,
    );
    run_bin(
        &[
            "connection-export",
            "--output",
            connection_file.to_str().unwrap(),
            "--json",
        ],
        &primary_dir,
    );

    let output = run_bin_raw(
        &[
            "connection-join",
            "--file",
            connection_file.to_str().unwrap(),
            "--json",
        ],
        &secondary_dir,
    );
    assert!(
        !output.status.success(),
        "conflicting connection-join unexpectedly succeeded"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("refusing to overwrite"));
}
