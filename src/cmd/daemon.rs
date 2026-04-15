use super::*;

use std::sync::Arc;

use crate::cli::AdapterArg;
use thronglets::mcp::McpContext;
use thronglets::network_runtime::{
    NetworkRuntimeOptions, NetworkRuntimeRequest, start_network_runtime,
};
use thronglets::pheromone::PheromoneField;
use tracing::info;

pub(crate) async fn run(ctx: FullCtx, port: u16, bootstrap: Vec<String>) {
    let store = Arc::new(open_store(&ctx.dir));
    let field = Arc::new(PheromoneField::new());
    field.hydrate_from_store(&store);
    let command_tx = start_network_runtime(NetworkRuntimeRequest {
        data_dir: &ctx.dir,
        identity: &ctx.identity,
        binding: &ctx.binding,
        store: Arc::clone(&store),
        field: Some(Arc::clone(&field)),
        listen_port: port,
        bootstrap: &bootstrap,
        options: NetworkRuntimeOptions::node(),
    })
    .await
    .expect("failed to start network");

    // Background pulse emitter (fail-open: no-op if env vars missing)
    maybe_spawn_pulse(&ctx.dir, &store);

    // Field socket: prehook queries the live field via IPC
    let _field_socket = thronglets::pheromone_socket::start_listener(Arc::clone(&field), &ctx.dir);

    info!(
        "Node {} running. Press Ctrl+C to stop.",
        ctx.identity.short_id()
    );

    tokio::signal::ctrl_c()
        .await
        .expect("failed to wait for shutdown signal");
    info!("Shutting down...");
    let _ = command_tx
        .send(thronglets::network::NetworkCommand::Shutdown)
        .await;
}

pub(crate) async fn mcp(
    ctx: FullCtx,
    port: Option<u16>,
    bootstrap: Vec<String>,
    local: bool,
    agent: Option<AdapterArg>,
) {
    let store = Arc::new(open_store(&ctx.dir));
    let field = Arc::new(PheromoneField::new());

    // Restore pheromone field from disk if available
    let field_path = ctx.dir.join("pheromone-field.v1.json");
    let restored_from_disk = if field_path.exists()
        && let Ok(data) = std::fs::read_to_string(&field_path)
        && let Ok(snapshot) = serde_json::from_str(&data)
    {
        field.restore(&snapshot);
        tracing::info!(points = field.len(), "Restored pheromone field from disk");
        true
    } else {
        false
    };

    if !restored_from_disk {
        field.hydrate_from_store(&store);
    }

    if let Some(adapter) = agent.and_then(AdapterArg::as_kind) {
        let _ =
            crate::setup_support::auto_clear_restart_pending_on_runtime_contact(&ctx.dir, adapter);
    }

    // Auto-join P2P network unless --local is specified.
    let network_tx = if !local {
        let p2p_port = port.unwrap_or(0);
        Some(
            start_network_runtime(NetworkRuntimeRequest {
                data_dir: &ctx.dir,
                identity: &ctx.identity,
                binding: &ctx.binding,
                store: Arc::clone(&store),
                field: Some(Arc::clone(&field)),
                listen_port: p2p_port,
                bootstrap: &bootstrap,
                options: NetworkRuntimeOptions::participant(),
            })
            .await
            .expect("failed to start network"),
        )
    } else {
        None
    };

    // Non-blocking update check (background thread, never fails)
    thronglets::update::check_for_update();

    // Background pulse emitter (fail-open: no-op if env vars missing)
    maybe_spawn_pulse(&ctx.dir, &store);

    // Field socket: prehook queries the live field via IPC instead of loading stale JSON
    let _field_socket = thronglets::pheromone_socket::start_listener(Arc::clone(&field), &ctx.dir);

    let mcp_ctx = Arc::new(McpContext {
        identity: Arc::new(ctx.identity),
        binding: Arc::new(ctx.binding),
        store,
        field: Arc::clone(&field),
        network_tx,
    });

    thronglets::mcp::serve_stdio(mcp_ctx).await;

    // Persist pheromone field on shutdown
    let snapshot = field.snapshot();
    if snapshot.points.is_empty() {
        let _ = std::fs::remove_file(&field_path);
    } else if let Ok(data) = serde_json::to_string(&snapshot) {
        let _ = std::fs::write(&field_path, data);
    }
}

pub(crate) async fn serve(
    ctx: FullCtx,
    port: u16,
    p2p_port: u16,
    bootstrap: Vec<String>,
    local: bool,
) {
    let store = Arc::new(open_store(&ctx.dir));
    let field = Arc::new(PheromoneField::new());

    // Restore pheromone field from disk if available
    let field_path = ctx.dir.join("pheromone-field.v1.json");
    let restored_from_disk = if field_path.exists()
        && let Ok(data) = std::fs::read_to_string(&field_path)
        && let Ok(snapshot) = serde_json::from_str(&data)
    {
        field.restore(&snapshot);
        tracing::info!(points = field.len(), "Restored pheromone field from disk");
        true
    } else {
        false
    };
    if !restored_from_disk {
        field.hydrate_from_store(&store);
    }

    // Auto-join P2P network unless --local is specified.
    let _network_tx = if !local {
        Some(
            start_network_runtime(NetworkRuntimeRequest {
                data_dir: &ctx.dir,
                identity: &ctx.identity,
                binding: &ctx.binding,
                store: Arc::clone(&store),
                field: Some(Arc::clone(&field)),
                listen_port: p2p_port,
                bootstrap: &bootstrap,
                options: NetworkRuntimeOptions::participant(),
            })
            .await
            .expect("failed to start network"),
        )
    } else {
        None
    };

    // Non-blocking update check (background thread, never fails)
    thronglets::update::check_for_update();

    // Background pulse emitter (fail-open: no-op if env vars missing)
    maybe_spawn_pulse(&ctx.dir, &store);

    // Field socket: prehook queries the live field via IPC
    let _field_socket = thronglets::pheromone_socket::start_listener(Arc::clone(&field), &ctx.dir);

    let http_ctx = Arc::new(thronglets::http::HttpContext {
        identity: Arc::new(ctx.identity),
        binding: Arc::new(ctx.binding),
        store,
        data_dir: ctx.dir.clone(),
    });
    println!("Thronglets HTTP API on http://0.0.0.0:{port}");
    if !local {
        println!("  P2P network joined (port {p2p_port}, 0 = random)");
    }
    println!("  POST /v1/traces       \u{2014} record a trace");
    println!("  POST /v1/presence     \u{2014} leave a lightweight session presence heartbeat");
    println!("  POST /v1/signals      \u{2014} leave an explicit short signal");
    println!("  GET  /v1/presence/feed \u{2014} show recent active sessions in a space");
    println!("  GET  /v1/signals      \u{2014} query explicit short signals");
    println!("  GET  /v1/signals/feed \u{2014} show recent converging explicit signals");
    println!("  GET  /v1/query        \u{2014} query the substrate");
    println!("  GET  /v1/capabilities \u{2014} list capabilities");
    println!("  GET  /v1/status       \u{2014} node status");
    thronglets::http::serve(http_ctx, port)
        .await
        .expect("HTTP server failed");

    // Persist pheromone field on shutdown
    let snapshot = field.snapshot();
    if snapshot.points.is_empty() {
        let _ = std::fs::remove_file(&field_path);
    } else if let Ok(data) = serde_json::to_string(&snapshot) {
        let _ = std::fs::write(&field_path, data);
    }
}
