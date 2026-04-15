use super::*;

use crate::onboarding_surface::{collect_status_data, render_status_report};
use thronglets::anchor::AnchorClient;
use thronglets::identity::{NodeIdentity, identity_binding_path};
use thronglets::identity_surface::{
    AuthorizationCheckData, authorization_check_data, identity_summary,
};
use thronglets::pulse::PulseEmitter;

pub(crate) fn id(ctx: &FullCtx, json: bool) {
    let data = IdentityIdData {
        summary: identity_summary("healthy", &ctx.binding),
        node_id: ctx.identity.short_id(),
        oasyce_address: ctx.identity.oasyce_address(),
        public_key: hex_encode(&ctx.identity.public_key_bytes()),
        data_dir: ctx.dir.display().to_string(),
    };
    if json {
        print_machine_json_with_schema(IDENTITY_SCHEMA_VERSION, "id", &data);
    } else {
        println!("Thronglets v{}", env!("CARGO_PKG_VERSION"));
        println!("Node ID:         {}", data.node_id);
        println!("Oasyce address:  {}", data.oasyce_address);
        println!("Device identity: {}", data.summary.device_identity);
        println!(
            "Owner account:   {}",
            ctx.binding.owner_account_or_unbound()
        );
        println!("Public key:      {}", data.public_key);
        println!("Binding source:  {}", data.summary.binding_source);
        println!(
            "Joined from:     {}",
            ctx.binding.joined_from_device_or_none()
        );
        println!("Data directory:  {}", data.data_dir);
    }
}

pub(crate) fn authorization_check(ctx: &FullCtx, json: bool) {
    let data: AuthorizationCheckData = authorization_check_data(&ctx.binding);
    if json {
        print_machine_json_with_schema(IDENTITY_SCHEMA_VERSION, "authorization-check", &data);
    } else {
        println!("Authorization:");
        println!(
            "  Meaning:             local execution state is cached here; Oasyce Chain remains the final authorization truth source."
        );
        println!(
            "  Local binding:       {}",
            data.summary.local_binding_status
        );
        println!(
            "  Local source:        {}",
            data.summary.local_binding_source
        );
        println!(
            "  Owner account:       {}",
            data.owner_account.as_deref().unwrap_or("unbound")
        );
        println!("  Device identity:     {}", data.device_identity);
        println!(
            "  Joined from device:  {}",
            data.joined_from_device.as_deref().unwrap_or("none")
        );
        println!("  Final truth source:  {}", data.summary.final_truth_source);
        println!(
            "  Authoritative check: {}",
            data.summary.authoritative_status
        );
        println!("  Execution boundary:  {}", data.summary.execution_boundary);
    }
}

pub(crate) fn owner_bind(ctx: &FullCtx, owner_account: String, json: bool) {
    let binding = ctx
        .binding
        .clone()
        .bind_owner_account(owner_account)
        .expect("failed to update identity binding");
    binding
        .save(&identity_binding_path(&ctx.dir))
        .expect("failed to save identity binding");
    let data = IdentityMutationData {
        summary: identity_summary("updated", &binding),
    };
    if json {
        print_machine_json_with_schema(IDENTITY_SCHEMA_VERSION, "owner-bind", &data);
    } else {
        println!("Owner binding updated:");
        println!("  Owner account:   {}", binding.owner_account_or_unbound());
        println!("  Device identity: {}", binding.device_identity);
        println!("  Source:          {}", binding.binding_source_or_local());
    }
}

pub(crate) async fn anchor(ctx: &FullCtx, rpc: String, chain_id: String, hours: u64) {
    let store = open_store(&ctx.dir);
    let client = AnchorClient::new(&rpc, &chain_id);

    let traces = store
        .unanchored_traces(hours, 500)
        .expect("failed to query unanchored traces");

    if traces.is_empty() {
        println!("No unanchored traces from the last {} hours.", hours);
        return;
    }

    println!(
        "Found {} unanchored traces. Anchoring to {} (chain: {})...",
        traces.len(),
        rpc,
        chain_id
    );

    // Process in batches of 50
    let mut total_anchored: u32 = 0;
    let mut total_skipped: u32 = 0;

    for batch in traces.chunks(50) {
        match client.anchor_batch(&ctx.identity, batch) {
            Ok(result) => {
                if !result.tx_hash.is_empty() {
                    // Mark each trace in this batch as anchored
                    for trace in batch {
                        let _ = store.mark_anchored(
                            &trace.id,
                            0, // block height unknown until chain is live
                            &result.tx_hash,
                        );
                    }
                    println!(
                        "  Batch tx: {}... ({} anchored, {} skipped)",
                        &result.tx_hash[..16],
                        result.anchored,
                        result.skipped
                    );
                }
                total_anchored += result.anchored;
                total_skipped += result.skipped;
            }
            Err(e) => {
                eprintln!("  Batch failed: {}", e);
                total_skipped += batch.len() as u32;
            }
        }
    }

    println!();
    println!("Anchoring complete:");
    println!("  Anchored: {}", total_anchored);
    println!("  Skipped:  {}", total_skipped);
}

pub(crate) async fn pulse(ctx: &FullCtx, sigil_id: String, rpc: String, chain_id: String) {
    let store = open_store(&ctx.dir);
    let emitter = PulseEmitter::new(&sigil_id, &rpc, &chain_id);
    let identity =
        NodeIdentity::load_or_generate(&ctx.dir.join("node.key")).expect("failed to load identity");

    println!("Aggregating dimensions...");
    let dims = emitter.aggregate_dimensions(&store, &identity);
    for (name, alive) in &dims {
        println!("  {name}: {}", if *alive { "alive" } else { "silent" });
    }

    // Run blocking HTTP in spawn_blocking to avoid tokio runtime conflict
    let result = tokio::task::spawn_blocking(move || emitter.emit(&identity, &store))
        .await
        .expect("pulse task panicked");

    match result {
        Ok(result) => {
            println!(
                "Pulse sent: {} (dimensions: {})",
                result.tx_hash,
                result.dimensions.join(", ")
            );
        }
        Err(thronglets::pulse::PulseError::NoDimensions) => {
            println!("All dimensions silent \u{2014} no pulse sent.");
        }
        Err(e) => {
            eprintln!("Pulse failed: {e}");
            std::process::exit(1);
        }
    }
}

pub(crate) fn peers(ctx: &FullCtx, json: bool, limit: usize) {
    let snapshot = thronglets::network_state::NetworkSnapshot::load(&ctx.dir);
    let status = snapshot.to_status();
    let peers: Vec<_> = snapshot.peers.into_iter().take(limit).collect();
    let data = PeersData {
        summary: PeersSummary {
            status: status.activity,
            connected_peers: status.peer_count,
            known_peers: status.known_peer_count,
            trusted_peer_seed_count: status.trusted_peer_seed_count,
            peer_seed_count: status.peer_seed_count,
            bootstrap_targets: status.bootstrap_targets,
            vps_dependency_level: status.vps_dependency_level,
        },
        peers,
    };
    if json {
        print_machine_json_with_schema(NETWORK_SCHEMA_VERSION, "peers", &data);
    } else if data.peers.is_empty() {
        println!("No peers observed yet.");
        println!(
            "Network status: {} ({})",
            status.activity, status.vps_dependency_level
        );
    } else {
        println!(
            "Known peers: {} ({} currently connected, {} trusted seeds, {} total seeds, dependency {})",
            data.summary.known_peers,
            data.summary.connected_peers,
            data.summary.trusted_peer_seed_count,
            data.summary.peer_seed_count,
            data.summary.vps_dependency_level
        );
        let now = chrono::Utc::now().timestamp_millis();
        for peer in &data.peers {
            let age_s = (now.saturating_sub(peer.last_seen_at_ms)) / 1000;
            let state = if peer.connected { "connected" } else { "seen" };
            println!("  {state}: {} ({age_s}s ago)", peer.peer_id);
            if !peer.addresses.is_empty() {
                println!("    addrs: {}", peer.addresses.join(", "));
            }
        }
    }
}

pub(crate) fn net_check(ctx: &FullCtx, json: bool, bootstrap_offline: bool) {
    let status = thronglets::network_state::NetworkSnapshot::load(&ctx.dir).to_status();
    let data = summarize_net_check_for_scenario(&status, bootstrap_offline);
    if json {
        print_machine_json_with_schema(NETWORK_SCHEMA_VERSION, "net-check", &data);
    } else {
        render_net_check(&data);
    }
}

pub(crate) fn status(ctx: &FullCtx, json: bool) {
    let hd = home_dir();
    let data = collect_status_data(&hd, &ctx.dir, &ctx.identity, &ctx.binding);
    if json {
        print_machine_json_with_schema(IDENTITY_SCHEMA_VERSION, "status", &data);
    } else {
        render_status_report(&data, ctx.binding.owner_account_or_unbound());
    }
}
