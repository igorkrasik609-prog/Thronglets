//! Integration test: two Thronglets nodes connect over loopback bootstrap,
//! one emits a trace, the other receives it via gossipsub.
//! This proves the core P2P loop works without relying on flaky mDNS timing.

use std::time::Duration;
use tempfile::TempDir;
use thronglets::context::simhash;
use thronglets::identity::{
    ConnectionFile, ConnectionSeedScope, DEFAULT_CONNECTION_FILE_TTL_HOURS, IdentityBinding,
    NodeIdentity,
};
use thronglets::network::{NetworkCommand, NetworkConfig, NetworkEvent};
use thronglets::network_runtime::attempt_first_connection;
use thronglets::network_state::NetworkSnapshot;
use thronglets::storage::TraceStore;
use thronglets::trace::{Outcome, Trace};

fn free_loopback_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind ephemeral port");
    listener.local_addr().expect("local addr").port()
}

async fn wait_for_peer_connection(
    event_rx: &mut tokio::sync::mpsc::Receiver<NetworkEvent>,
    timeout: Duration,
) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    while tokio::time::Instant::now() < deadline {
        tokio::select! {
            Some(event) = event_rx.recv() => {
                if matches!(event, NetworkEvent::PeerConnected { .. }) {
                    return true;
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }
    false
}

#[tokio::test]
async fn two_nodes_sync_trace_via_loopback_bootstrap() {
    // Initialize tracing for test output
    let _ = tracing_subscriber::fmt()
        .with_env_filter("thronglets=debug")
        .with_test_writer()
        .try_init();

    let port_a = free_loopback_port();
    let port_b = free_loopback_port();

    // --- Node A ---
    let id_a = NodeIdentity::generate();
    let mut secret_a = id_a.secret_key_bytes();
    let keypair_a =
        libp2p::identity::Keypair::ed25519_from_bytes(&mut secret_a).expect("keypair A");
    let peer_id_a = libp2p::PeerId::from(keypair_a.public());
    let config_a = NetworkConfig {
        listen_port: port_a,
        ..Default::default()
    };
    let (cmd_tx_a, mut event_rx_a) = thronglets::network::start(keypair_a, config_a)
        .await
        .expect("start node A");

    // --- Node B ---
    let id_b = NodeIdentity::generate();
    let mut secret_b = id_b.secret_key_bytes();
    let keypair_b =
        libp2p::identity::Keypair::ed25519_from_bytes(&mut secret_b).expect("keypair B");
    let bootstrap_a: libp2p::Multiaddr = format!("/ip4/127.0.0.1/tcp/{port_a}/p2p/{peer_id_a}")
        .parse()
        .expect("bootstrap addr");
    let config_b = NetworkConfig {
        listen_port: port_b,
        bootstrap_peers: vec![bootstrap_a],
        trusted_peers: Vec::new(),
        known_peers: Vec::new(),
    };
    let (cmd_tx_b, mut event_rx_b) = thronglets::network::start(keypair_b, config_b)
        .await
        .expect("start node B");

    // Wait for the loopback bootstrap dial to establish on both sides.
    let discovery_timeout = Duration::from_secs(10);
    let mut a_found_b = false;
    let mut b_found_a = false;

    let discover_deadline = tokio::time::Instant::now() + discovery_timeout;
    while tokio::time::Instant::now() < discover_deadline && !(a_found_b && b_found_a) {
        tokio::select! {
            Some(event) = event_rx_a.recv() => {
                if matches!(event, NetworkEvent::PeerConnected { .. }) {
                    a_found_b = true;
                }
            }
            Some(event) = event_rx_b.recv() => {
                if matches!(event, NetworkEvent::PeerConnected { .. }) {
                    b_found_a = true;
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }

    assert!(
        a_found_b,
        "Node A should observe the bootstrap connection from Node B"
    );
    assert!(b_found_a, "Node B should connect to Node A via bootstrap");

    // Give gossipsub mesh time to form
    tokio::time::sleep(Duration::from_secs(2)).await;

    // --- Node A emits a trace ---
    let trace = Trace::new(
        "test-capability/v1".into(),
        Outcome::Succeeded,
        42,
        1000,
        simhash("integration test for P2P trace sync"),
        Some("integration test for P2P trace sync".to_string()),
        None,
        "test-model".into(),
        id_a.public_key_bytes(),
        |msg| id_a.sign(msg),
    );
    let trace_id = trace.id;

    cmd_tx_a
        .send(NetworkCommand::PublishTrace {
            trace: Box::new(trace),
            space: None,
        })
        .await
        .expect("send publish command");

    // --- Node B should receive the trace ---
    let receive_timeout = Duration::from_secs(10);
    let mut received_trace: Option<Box<Trace>> = None;

    let receive_deadline = tokio::time::Instant::now() + receive_timeout;
    while tokio::time::Instant::now() < receive_deadline && received_trace.is_none() {
        tokio::select! {
            Some(event) = event_rx_b.recv() => {
                if let NetworkEvent::TraceReceived { trace: t, .. } = event {
                    received_trace = Some(t);
                }
            }
            _ = tokio::time::sleep(Duration::from_millis(100)) => {}
        }
    }

    let received = received_trace.expect("Node B should receive the trace from Node A");

    // Verify the received trace
    assert_eq!(received.id, trace_id, "Trace ID should match");
    assert_eq!(received.capability, "test-capability/v1");
    assert_eq!(received.outcome, Outcome::Succeeded);
    assert_eq!(received.latency_ms, 42);
    assert_eq!(received.input_size, 1000);
    assert_eq!(received.model_id, "test-model");
    assert!(
        received.verify(),
        "Received trace signature should be valid"
    );
    assert!(received.verify_id(), "Received trace ID should be valid");

    // Store it and check aggregation
    let store = TraceStore::in_memory().unwrap();
    assert!(store.insert(&received).unwrap(), "Should store the trace");
    let stats = store.aggregate("test-capability/v1").unwrap().unwrap();
    assert_eq!(stats.total_traces, 1);
    assert!((stats.success_rate - 1.0).abs() < 0.001);

    // Cleanup
    drop(cmd_tx_a);
    drop(cmd_tx_b);
}

#[tokio::test]
async fn node_reconnects_via_known_peer_without_bootstrap() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("thronglets=debug")
        .with_test_writer()
        .try_init();

    let port_a = free_loopback_port();
    let port_b = free_loopback_port();

    let id_a = NodeIdentity::generate();
    let mut secret_a = id_a.secret_key_bytes();
    let keypair_a =
        libp2p::identity::Keypair::ed25519_from_bytes(&mut secret_a).expect("keypair A");
    let peer_id_a = libp2p::PeerId::from(keypair_a.public());
    let config_a = NetworkConfig {
        listen_port: port_a,
        ..Default::default()
    };
    let (cmd_tx_a, mut event_rx_a) = thronglets::network::start(keypair_a, config_a)
        .await
        .expect("start node A");

    let id_b = NodeIdentity::generate();
    let mut secret_b = id_b.secret_key_bytes();
    let keypair_b =
        libp2p::identity::Keypair::ed25519_from_bytes(&mut secret_b).expect("keypair B");
    let bootstrap_a: libp2p::Multiaddr = format!("/ip4/127.0.0.1/tcp/{port_a}/p2p/{peer_id_a}")
        .parse()
        .expect("bootstrap addr");
    let config_b = NetworkConfig {
        listen_port: port_b,
        bootstrap_peers: vec![bootstrap_a.clone()],
        trusted_peers: Vec::new(),
        known_peers: Vec::new(),
    };
    let (cmd_tx_b, mut event_rx_b) = thronglets::network::start(keypair_b, config_b)
        .await
        .expect("start node B");

    assert!(
        wait_for_peer_connection(&mut event_rx_a, Duration::from_secs(10)).await,
        "Node A should observe the bootstrap connection from Node B"
    );
    assert!(
        wait_for_peer_connection(&mut event_rx_b, Duration::from_secs(10)).await,
        "Node B should connect to Node A via bootstrap"
    );

    drop(cmd_tx_b);
    tokio::time::sleep(Duration::from_secs(1)).await;

    let id_b_restarted = NodeIdentity::generate();
    let mut secret_b_restarted = id_b_restarted.secret_key_bytes();
    let keypair_b_restarted =
        libp2p::identity::Keypair::ed25519_from_bytes(&mut secret_b_restarted)
            .expect("keypair B restarted");
    let config_b_restarted = NetworkConfig {
        listen_port: port_b,
        bootstrap_peers: Vec::new(),
        trusted_peers: Vec::new(),
        known_peers: vec![bootstrap_a],
    };
    let (_cmd_tx_b_restarted, mut event_rx_b_restarted) =
        thronglets::network::start(keypair_b_restarted, config_b_restarted)
            .await
            .expect("restart node B");

    assert!(
        wait_for_peer_connection(&mut event_rx_b_restarted, Duration::from_secs(10)).await,
        "Restarted node B should reconnect to Node A using known peers without bootstrap"
    );

    drop(cmd_tx_a);
}

#[tokio::test]
async fn node_connects_via_trusted_peer_without_bootstrap() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("thronglets=debug")
        .with_test_writer()
        .try_init();

    let port_a = free_loopback_port();
    let port_b = free_loopback_port();

    let id_a = NodeIdentity::generate();
    let mut secret_a = id_a.secret_key_bytes();
    let keypair_a =
        libp2p::identity::Keypair::ed25519_from_bytes(&mut secret_a).expect("keypair A");
    let peer_id_a = libp2p::PeerId::from(keypair_a.public());
    let config_a = NetworkConfig {
        listen_port: port_a,
        ..Default::default()
    };
    let (cmd_tx_a, mut event_rx_a) = thronglets::network::start(keypair_a, config_a)
        .await
        .expect("start node A");

    let id_b = NodeIdentity::generate();
    let mut secret_b = id_b.secret_key_bytes();
    let keypair_b =
        libp2p::identity::Keypair::ed25519_from_bytes(&mut secret_b).expect("keypair B");
    let trusted_a: libp2p::Multiaddr = format!("/ip4/127.0.0.1/tcp/{port_a}/p2p/{peer_id_a}")
        .parse()
        .expect("trusted addr");
    let config_b = NetworkConfig {
        listen_port: port_b,
        bootstrap_peers: Vec::new(),
        trusted_peers: vec![trusted_a],
        known_peers: Vec::new(),
    };
    let (_cmd_tx_b, mut event_rx_b) = thronglets::network::start(keypair_b, config_b)
        .await
        .expect("start node B");

    assert!(
        wait_for_peer_connection(&mut event_rx_a, Duration::from_secs(10)).await,
        "Node A should observe the trusted direct connection from Node B"
    );
    assert!(
        wait_for_peer_connection(&mut event_rx_b, Duration::from_secs(10)).await,
        "Node B should connect to Node A via trusted peer seeds without bootstrap"
    );

    drop(cmd_tx_a);
}

#[tokio::test]
async fn secondary_node_recovers_via_signed_connection_file_without_bootstrap() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("thronglets=debug")
        .with_test_writer()
        .try_init();

    let temp = TempDir::new().unwrap();
    let connection_file_path = temp.path().join("owner.connection.json");

    let port_a = free_loopback_port();
    let port_b = free_loopback_port();

    let id_a = NodeIdentity::generate();
    let mut secret_a = id_a.secret_key_bytes();
    let keypair_a =
        libp2p::identity::Keypair::ed25519_from_bytes(&mut secret_a).expect("keypair A");
    let peer_id_a = libp2p::PeerId::from(keypair_a.public());
    let config_a = NetworkConfig {
        listen_port: port_a,
        ..Default::default()
    };
    let (cmd_tx_a, mut event_rx_a) = thronglets::network::start(keypair_a, config_a)
        .await
        .expect("start node A");

    let binding_a = IdentityBinding::new(id_a.device_identity())
        .bind_owner_account("oasyce1owner".into())
        .expect("bind owner");
    let trusted_a: libp2p::Multiaddr = format!("/ip4/127.0.0.1/tcp/{port_a}/p2p/{peer_id_a}")
        .parse()
        .expect("trusted addr");
    let connection = ConnectionFile::from_binding(
        &binding_a,
        &id_a,
        DEFAULT_CONNECTION_FILE_TTL_HOURS,
        ConnectionSeedScope::Trusted,
        vec![trusted_a.to_string()],
    )
    .expect("create signed connection file");
    connection
        .save(&connection_file_path)
        .expect("save connection file");
    let loaded_connection =
        ConnectionFile::load(&connection_file_path).expect("load connection file");
    assert_eq!(
        loaded_connection.peer_seed_scope,
        ConnectionSeedScope::Trusted
    );

    let id_b = NodeIdentity::generate();
    let mut secret_b = id_b.secret_key_bytes();
    let keypair_b =
        libp2p::identity::Keypair::ed25519_from_bytes(&mut secret_b).expect("keypair B");
    let mut secondary_snapshot = NetworkSnapshot::begin(0);
    secondary_snapshot.merge_trusted_peer_seeds(loaded_connection.peer_seeds.clone());
    let config_b = NetworkConfig {
        listen_port: port_b,
        bootstrap_peers: Vec::new(),
        trusted_peers: secondary_snapshot
            .trusted_peer_seed_addresses(8)
            .into_iter()
            .filter_map(|address| address.parse().ok())
            .collect(),
        known_peers: secondary_snapshot
            .remembered_peer_addresses(16)
            .into_iter()
            .filter_map(|address| address.parse().ok())
            .collect(),
    };
    let (_cmd_tx_b, mut event_rx_b) = thronglets::network::start(keypair_b, config_b)
        .await
        .expect("start node B");

    assert!(
        wait_for_peer_connection(&mut event_rx_a, Duration::from_secs(10)).await,
        "Node A should observe the trusted direct connection from secondary node"
    );
    assert!(
        wait_for_peer_connection(&mut event_rx_b, Duration::from_secs(10)).await,
        "Secondary node should connect to primary via signed connection file without bootstrap"
    );

    drop(cmd_tx_a);
}

#[tokio::test]
async fn first_connection_attempt_promotes_same_owner_path_to_trusted_seed() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("thronglets=debug")
        .with_test_writer()
        .try_init();

    let temp = TempDir::new().unwrap();
    let data_dir = temp.path().join("secondary-data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let port_a = free_loopback_port();

    let id_a = NodeIdentity::generate();
    let mut secret_a = id_a.secret_key_bytes();
    let keypair_a =
        libp2p::identity::Keypair::ed25519_from_bytes(&mut secret_a).expect("keypair A");
    let peer_id_a = libp2p::PeerId::from(keypair_a.public());
    let config_a = NetworkConfig {
        listen_port: port_a,
        ..Default::default()
    };
    let (cmd_tx_a, mut event_rx_a) = thronglets::network::start(keypair_a, config_a)
        .await
        .expect("start node A");

    let id_b = NodeIdentity::generate();
    let binding_b = IdentityBinding::new(id_b.device_identity())
        .joined_via_connection(Some("oasyce1owner".into()), id_a.device_identity())
        .unwrap();

    let remembered_a: libp2p::Multiaddr = format!("/ip4/127.0.0.1/tcp/{port_a}/p2p/{peer_id_a}")
        .parse()
        .expect("remembered addr");
    let mut snapshot = NetworkSnapshot::begin(0);
    snapshot.merge_peer_seeds([remembered_a.to_string()]);
    snapshot.save(&data_dir);

    let result = attempt_first_connection(
        &data_dir,
        &id_b,
        &binding_b,
        std::sync::Arc::new(TraceStore::open(&data_dir.join("traces.db")).unwrap()),
        Duration::from_secs(10),
    )
    .await
    .expect("attempt first connection");

    assert!(result.connected_once);
    assert!(result.trusted_same_owner_ready);
    assert!(
        wait_for_peer_connection(&mut event_rx_a, Duration::from_secs(2)).await,
        "Primary node should observe the attempted same-owner connection"
    );

    let snapshot = NetworkSnapshot::load(&data_dir);
    assert_eq!(
        snapshot.peer_count, 0,
        "ephemeral probe should not leave fake live peers behind"
    );
    let trusted = snapshot.trusted_peer_seed_addresses(8);
    assert!(
        trusted.contains(&remembered_a.to_string()),
        "trusted seeds should contain the remembered same-owner path"
    );
    assert!(!trusted.is_empty());

    drop(cmd_tx_a);
}

#[tokio::test]
async fn first_connection_attempt_can_use_persisted_bootstrap_seed() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter("thronglets=debug")
        .with_test_writer()
        .try_init();

    let temp = TempDir::new().unwrap();
    let data_dir = temp.path().join("secondary-data");
    std::fs::create_dir_all(&data_dir).unwrap();

    let port_a = free_loopback_port();

    let id_a = NodeIdentity::generate();
    let mut secret_a = id_a.secret_key_bytes();
    let keypair_a =
        libp2p::identity::Keypair::ed25519_from_bytes(&mut secret_a).expect("keypair A");
    let peer_id_a = libp2p::PeerId::from(keypair_a.public());
    let config_a = NetworkConfig {
        listen_port: port_a,
        ..Default::default()
    };
    let (cmd_tx_a, mut event_rx_a) = thronglets::network::start(keypair_a, config_a)
        .await
        .expect("start node A");

    let id_b = NodeIdentity::generate();
    let binding_b = IdentityBinding::new(id_b.device_identity());
    let bootstrap_a: libp2p::Multiaddr = format!("/ip4/127.0.0.1/tcp/{port_a}/p2p/{peer_id_a}")
        .parse()
        .expect("bootstrap addr");

    let mut snapshot = NetworkSnapshot::begin(0);
    snapshot.remember_bootstrap_seeds([bootstrap_a.to_string()]);
    snapshot.save(&data_dir);

    let result = attempt_first_connection(
        &data_dir,
        &id_b,
        &binding_b,
        std::sync::Arc::new(TraceStore::open(&data_dir.join("traces.db")).unwrap()),
        Duration::from_secs(10),
    )
    .await
    .expect("attempt first connection");

    assert!(result.connected_once);
    assert!(!result.trusted_same_owner_ready);
    assert!(
        wait_for_peer_connection(&mut event_rx_a, Duration::from_secs(2)).await,
        "Primary node should observe the connection that came from persisted bootstrap memory"
    );

    let snapshot = NetworkSnapshot::load(&data_dir);
    assert_eq!(snapshot.peer_count, 0);
    assert_eq!(
        snapshot.bootstrap_seed_addresses(8),
        vec![bootstrap_a.to_string()]
    );
    let remembered = snapshot.remembered_peer_addresses(8);
    assert!(
        remembered
            .iter()
            .any(|addr| addr.contains(&peer_id_a.to_string())),
        "first connection attempt should learn a reusable remembered peer path for the connected bootstrap peer; remembered={remembered:?}"
    );

    drop(cmd_tx_a);
}
