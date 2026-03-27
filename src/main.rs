use std::path::PathBuf;
use std::sync::Arc;
use clap::{Parser, Subcommand};
use thronglets::identity::NodeIdentity;
use thronglets::mcp::McpContext;
use thronglets::network::{NetworkCommand, NetworkConfig, NetworkEvent};
use thronglets::storage::TraceStore;
use thronglets::trace::{Outcome, Trace};
use tracing::info;

#[derive(Parser)]
#[command(name = "thronglets", version, about = "P2P shared memory substrate for AI agents")]
struct Cli {
    /// Data directory (default: ~/.thronglets)
    #[arg(long, global = true)]
    data_dir: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Start the Thronglets node
    Run {
        /// Port to listen on (0 = random)
        #[arg(long, default_value_t = 0)]
        port: u16,

        /// Bootstrap peer multiaddrs
        #[arg(long)]
        bootstrap: Vec<String>,
    },

    /// Show node identity
    Id,

    /// Emit a trace manually
    Emit {
        /// What the trace is about
        about: String,

        /// Tags (comma-separated)
        #[arg(long, default_value = "")]
        tags: String,

        /// Outcome
        #[arg(long, default_value = "succeeded")]
        outcome: String,

        /// Quality 0-100
        #[arg(long, default_value_t = 50)]
        quality: u8,

        /// Latency in ms
        #[arg(long, default_value_t = 0)]
        latency: u32,
    },

    /// Query local aggregate stats
    Query {
        /// Subject to query
        about: String,
    },

    /// Start MCP server for AI agent integration (JSON-RPC over stdio)
    Mcp {
        /// Also start P2P network on this port (0 = random, omit = local only)
        #[arg(long)]
        port: Option<u16>,

        /// Bootstrap peer multiaddrs (only used if --port is set)
        #[arg(long)]
        bootstrap: Vec<String>,
    },

    /// Show connected peers
    Peers,

    /// Show node status and statistics
    Status,
}

fn data_dir(cli_override: &Option<PathBuf>) -> PathBuf {
    cli_override.clone().unwrap_or_else(|| {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        PathBuf::from(home).join(".thronglets")
    })
}

fn load_identity(data_dir: &std::path::Path) -> NodeIdentity {
    NodeIdentity::load_or_generate(&data_dir.join("node.key"))
        .expect("failed to load or generate node identity")
}

fn open_store(data_dir: &std::path::Path) -> TraceStore {
    std::fs::create_dir_all(data_dir).expect("failed to create data directory");
    TraceStore::open(&data_dir.join("traces.db"))
        .expect("failed to open trace store")
}

fn parse_outcome(s: &str) -> Outcome {
    match s {
        "succeeded" | "success" | "ok" => Outcome::Succeeded,
        "failed" | "fail" | "error" => Outcome::Failed,
        "partial" => Outcome::Partial,
        "timeout" => Outcome::Timeout,
        _ => {
            eprintln!("Unknown outcome '{}', defaulting to 'succeeded'", s);
            Outcome::Succeeded
        }
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "thronglets=info".parse().unwrap()),
        )
        .init();

    let cli = Cli::parse();
    let dir = data_dir(&cli.data_dir);
    let identity = load_identity(&dir);

    match cli.command {
        Commands::Id => {
            println!("Thronglets v{}", env!("CARGO_PKG_VERSION"));
            println!("Node ID:         {}", identity.short_id());
            println!("Oasyce address:  {}", identity.oasyce_address());
            println!("Public key:      {}", hex_encode(&identity.public_key_bytes()));
            println!("Data directory:  {}", dir.display());
        }

        Commands::Emit { about, tags, outcome, quality, latency } => {
            let store = open_store(&dir);
            let tag_list: Vec<String> = tags.split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            let outcome = parse_outcome(&outcome);
            let trace = Trace::new(
                about.clone(),
                tag_list,
                outcome,
                latency,
                quality,
                identity.public_key_bytes(),
                |msg| identity.sign(msg),
            );
            store.insert(&trace).expect("failed to insert trace");
            println!("Trace emitted:");
            println!("  ID:      {}", hex_encode(&trace.id[..8]));
            println!("  About:   {}", about);
            println!("  Outcome: {:?}", outcome);
            println!("  Quality: {}", quality);
        }

        Commands::Query { about } => {
            let store = open_store(&dir);
            match store.aggregate(&about).expect("query failed") {
                Some(stats) => {
                    println!("Aggregate for '{}':", about);
                    println!("  Total traces:  {}", stats.total_traces);
                    println!("  Success rate:  {:.1}%", stats.success_rate * 100.0);
                    println!("  Avg latency:   {:.0}ms", stats.avg_latency_ms);
                    println!("  Avg quality:   {:.1}", stats.avg_quality);
                }
                None => {
                    println!("No traces found for '{}'", about);
                }
            }
        }

        Commands::Run { port, bootstrap } => {
            let store = open_store(&dir);

            // Convert ed25519-dalek key to libp2p identity
            let libp2p_keypair = libp2p::identity::Keypair::ed25519_from_bytes(
                &mut identity.secret_key_bytes()
            ).expect("failed to create libp2p keypair");

            let bootstrap_addrs: Vec<libp2p::Multiaddr> = bootstrap.iter()
                .filter_map(|s| s.parse().ok())
                .collect();

            let config = NetworkConfig {
                listen_port: port,
                bootstrap_peers: bootstrap_addrs,
            };

            let (cmd_tx, mut event_rx) = thronglets::network::start(libp2p_keypair, config)
                .await
                .expect("failed to start network");

            info!("Node {} running. Press Ctrl+C to stop.", identity.short_id());

            // Periodic timers
            let mut evaporation_interval = tokio::time::interval(std::time::Duration::from_secs(3600));
            evaporation_interval.tick().await;
            let mut dht_publish_interval = tokio::time::interval(std::time::Duration::from_secs(300));
            dht_publish_interval.tick().await;

            // Main event loop: handle network events
            loop {
                tokio::select! {
                    Some(event) = event_rx.recv() => {
                        match event {
                            NetworkEvent::PeerConnected(peer) => {
                                info!(%peer, "Peer connected");
                            }
                            NetworkEvent::PeerDisconnected(peer) => {
                                info!(%peer, "Peer disconnected");
                            }
                            NetworkEvent::TraceReceived(trace) => {
                                match store.insert(&trace) {
                                    Ok(true) => {
                                        info!(
                                            about = %trace.about,
                                            outcome = ?trace.outcome,
                                            "Stored new trace from network"
                                        );
                                    }
                                    Ok(false) => {
                                        // Duplicate, already have it
                                    }
                                    Err(e) => {
                                        tracing::warn!(%e, "Failed to store received trace");
                                    }
                                }
                            }
                        }
                    }
                    _ = evaporation_interval.tick() => {
                        match store.evaporate(None) {
                            Ok(n) if n > 0 => info!(deleted = n, "Evaporated expired traces"),
                            Ok(_) => {}
                            Err(e) => tracing::warn!(%e, "Evaporation failed"),
                        }
                    }
                    _ = dht_publish_interval.tick() => {
                        // Publish aggregate summaries for all known subjects to DHT
                        if let Ok(subjects) = store.distinct_subjects(100) {
                            for about in subjects {
                                if let Ok(Some(stats)) = store.aggregate(&about) {
                                    let _ = cmd_tx.send(NetworkCommand::PublishSummary {
                                        about,
                                        stats,
                                    }).await;
                                }
                            }
                        }
                    }
                    _ = tokio::signal::ctrl_c() => {
                        info!("Shutting down...");
                        break;
                    }
                }
            }

            drop(cmd_tx);
        }

        Commands::Mcp { port, bootstrap } => {
            let store = open_store(&dir);
            let store = Arc::new(store);

            let network_tx = if let Some(p) = port {
                let libp2p_keypair = libp2p::identity::Keypair::ed25519_from_bytes(
                    &mut identity.secret_key_bytes()
                ).expect("failed to create libp2p keypair");

                let bootstrap_addrs: Vec<libp2p::Multiaddr> = bootstrap.iter()
                    .filter_map(|s| s.parse().ok())
                    .collect();

                let config = NetworkConfig {
                    listen_port: p,
                    bootstrap_peers: bootstrap_addrs,
                };

                let (cmd_tx, mut event_rx) = thronglets::network::start(libp2p_keypair, config)
                    .await
                    .expect("failed to start network");

                // Spawn event handler for network events + periodic evaporation
                let store_bg = Arc::clone(&store);
                tokio::spawn(async move {
                    let mut evaporation_interval = tokio::time::interval(std::time::Duration::from_secs(3600));
                    evaporation_interval.tick().await; // consume the immediate first tick

                    loop {
                        tokio::select! {
                            event = event_rx.recv() => {
                                match event {
                                    Some(NetworkEvent::PeerConnected(peer)) => {
                                        info!(%peer, "Peer connected");
                                    }
                                    Some(NetworkEvent::PeerDisconnected(peer)) => {
                                        info!(%peer, "Peer disconnected");
                                    }
                                    Some(NetworkEvent::TraceReceived(trace)) => {
                                        match store_bg.insert(&trace) {
                                            Ok(true) => {
                                                info!(about = %trace.about, "Stored trace from network");
                                            }
                                            Ok(false) => {}
                                            Err(e) => {
                                                tracing::warn!(%e, "Failed to store received trace");
                                            }
                                        }
                                    }
                                    None => break, // channel closed
                                }
                            }
                            _ = evaporation_interval.tick() => {
                                match store_bg.evaporate(None) {
                                    Ok(n) if n > 0 => info!(deleted = n, "Evaporated expired traces"),
                                    Ok(_) => {}
                                    Err(e) => tracing::warn!(%e, "Evaporation failed"),
                                }
                            }
                        }
                    }
                });

                Some(cmd_tx)
            } else {
                None
            };

            let ctx = Arc::new(McpContext {
                identity: Arc::new(identity),
                store,
                network_tx,
            });

            thronglets::mcp::serve_stdio(ctx).await;
        }

        Commands::Peers => {
            println!("The 'peers' command requires a running node.");
            println!("Use 'thronglets run' to start a node, then peers are logged to console.");
        }

        Commands::Status => {
            let store = open_store(&dir);
            let trace_count = store.count().unwrap_or(0);
            let subject_count = store.distinct_subjects(1000)
                .map(|s| s.len())
                .unwrap_or(0);
            let db_path = dir.join("traces.db");
            let db_size = std::fs::metadata(&db_path)
                .map(|m| m.len())
                .unwrap_or(0);

            let size_display = if db_size >= 1_048_576 {
                format!("{:.1} MB", db_size as f64 / 1_048_576.0)
            } else if db_size >= 1024 {
                format!("{:.1} KB", db_size as f64 / 1024.0)
            } else {
                format!("{} B", db_size)
            };

            println!("Thronglets v{}", env!("CARGO_PKG_VERSION"));
            println!();
            println!("  Node ID:           {}", identity.short_id());
            println!("  Oasyce address:    {}", identity.oasyce_address());
            println!("  Data directory:    {}", dir.display());
            println!();
            println!("  Trace count:       {}", trace_count);
            println!("  Distinct subjects: {}", subject_count);
            println!("  Database size:     {}", size_display);
        }
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
