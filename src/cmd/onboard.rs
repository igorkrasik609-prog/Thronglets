use super::*;

use std::sync::Arc;

use crate::cli::AdapterArg;
use crate::onboarding_surface::{
    JoinFlowData, ShareFlowData, StartData, collect_status_data, connection_readiness_summary,
    default_share_output_path, export_connection_file, render_join_flow_report,
    render_share_flow_report, render_start_report, summarize_join_flow, summarize_share_flow,
    summarize_start_flow,
};
use thronglets::identity::{ConnectionFile, ConnectionSeedScope, identity_binding_path};
use thronglets::identity_surface::{
    IdentitySummary, authorization_summary, identity_blueprint, identity_summary,
};
use thronglets::network_runtime::attempt_first_connection;

pub(crate) fn start(ctx: &FullCtx, json: bool) {
    let bin = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("thronglets"));
    codesign_if_macos(&bin);
    let hd = home_dir();
    if !json {
        eprint!("detecting AI tools...");
    }
    let report = bootstrap_selected_adapters(AdapterArg::All, &hd, &ctx.dir, &bin)
        .expect("failed to bootstrap adapter plan");
    if !json {
        eprintln!(" done");
    }
    let status = collect_status_data(&hd, &ctx.dir, &ctx.identity, &ctx.binding);
    let data = StartData {
        summary: summarize_start_flow(&report.summary, &status.summary),
        setup: report.summary.clone(),
        readiness: status.summary.clone(),
        identity: status.identity.clone(),
    };
    if json {
        print_machine_json_with_schema(IDENTITY_SCHEMA_VERSION, "start", &data);
    } else {
        render_start_report(&data);
    }
    if !report.summary.healthy {
        std::process::exit(1);
    }
}

pub(crate) async fn share(ctx: &FullCtx, output: Option<PathBuf>, ttl_hours: u32, json: bool) {
    let hd = home_dir();
    let status = collect_status_data(&hd, &ctx.dir, &ctx.identity, &ctx.binding);
    let network_snapshot = thronglets::network_state::NetworkSnapshot::load(&ctx.dir);
    if !status.summary.network_path_ready
        && !network_snapshot.bootstrap_seed_addresses(8).is_empty()
    {
        let _ = attempt_first_connection(
            &ctx.dir,
            &ctx.identity,
            &ctx.binding,
            Arc::new(open_store(&ctx.dir)),
            std::time::Duration::from_secs(12),
        )
        .await;
    }
    let output = output.unwrap_or_else(default_share_output_path);
    let exported = export_connection_file(
        &output,
        ttl_hours,
        false,
        &ctx.binding,
        &ctx.identity,
        &ctx.dir,
    );
    let data = ShareFlowData {
        summary: summarize_share_flow(&exported.summary, &output),
        readiness: exported.summary,
        identity: exported.identity,
        output: exported.output,
        preferred_surface: exported.preferred_surface,
        surfaces: exported.surfaces,
        peer_seed_scope: exported.peer_seed_scope,
        trusted_peer_seed_count: exported.trusted_peer_seed_count,
        peer_seed_count: exported.peer_seed_count,
        ttl_hours: exported.ttl_hours,
    };
    if json {
        print_machine_json_with_schema(IDENTITY_SCHEMA_VERSION, "share", &data);
    } else {
        render_share_flow_report(&data);
    }
}

pub(crate) async fn join(ctx: &FullCtx, file: Option<PathBuf>, json: bool) {
    let file = file.unwrap_or_else(default_share_output_path);
    let bin = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("thronglets"));
    let hd = home_dir();
    let report = bootstrap_selected_adapters(AdapterArg::All, &hd, &ctx.dir, &bin)
        .expect("failed to bootstrap adapter plan");
    let connection = ConnectionFile::load(&file).expect("failed to read connection file");
    let inspect_readiness = connection_readiness_summary(
        connection.peer_seed_scope.clone(),
        connection.peer_seeds.len(),
        "export",
    );
    let binding = ctx
        .binding
        .clone()
        .joined_via_connection(
            connection.owner_account.clone(),
            connection.oasyce_delegate_policy.clone(),
            connection.primary_device_identity.clone(),
        )
        .expect("failed to update identity binding");
    binding
        .save(&identity_binding_path(&ctx.dir))
        .expect("failed to save identity binding");
    let mut network_snapshot = thronglets::network_state::NetworkSnapshot::load(&ctx.dir);
    match connection.peer_seed_scope {
        ConnectionSeedScope::Trusted => {
            network_snapshot.merge_trusted_peer_seeds(connection.peer_seeds.clone());
        }
        ConnectionSeedScope::Remembered => {
            network_snapshot.merge_peer_seeds(connection.peer_seeds.clone());
        }
    }
    network_snapshot.save(&ctx.dir);
    let mut status = collect_status_data(&hd, &ctx.dir, &ctx.identity, &binding);
    if report.summary.healthy
        && !report.summary.restart_required
        && !report.summary.restart_pending
        && status.summary.network_path_ready
        && !status.summary.connected
        && !status.summary.trusted_same_owner_ready
    {
        let _ = attempt_first_connection(
            &ctx.dir,
            &ctx.identity,
            &binding,
            Arc::new(open_store(&ctx.dir)),
            std::time::Duration::from_secs(12),
        )
        .await;
        status = collect_status_data(&hd, &ctx.dir, &ctx.identity, &binding);
    }
    let data = JoinFlowData {
        summary: summarize_join_flow(&report.summary, &status.summary),
        setup: report.summary.clone(),
        inspect: inspect_readiness,
        readiness: status.summary.clone(),
        identity: status.identity.clone(),
        file: file.display().to_string(),
    };
    if json {
        print_machine_json_with_schema(IDENTITY_SCHEMA_VERSION, "join", &data);
    } else {
        render_join_flow_report(&data);
    }
    if !report.summary.healthy {
        std::process::exit(1);
    }
}

pub(crate) fn connection_export(
    ctx: &FullCtx,
    output: PathBuf,
    ttl_hours: u32,
    json: bool,
    include_oasyce_surface: bool,
) {
    let data = export_connection_file(
        &output,
        ttl_hours,
        include_oasyce_surface,
        &ctx.binding,
        &ctx.identity,
        &ctx.dir,
    );
    if json {
        print_machine_json_with_schema(IDENTITY_SCHEMA_VERSION, "connection-export", &data);
    } else {
        println!("Connection file: {}", data.summary.status);
        println!("  Meaning:            {}", data.summary.detail);
        println!("  Output:             {}", data.output);
        println!(
            "  Owner account:      {}",
            data.identity.owner_account.as_deref().unwrap_or("unbound")
        );
        println!("  Primary device:     {}", data.identity.device_identity);
        println!("  Signed by device:   {}", data.signed_by_device);
        println!("  Seed scope:         {}", data.peer_seed_scope);
        println!("  Trusted seeds:      {}", data.trusted_peer_seed_count);
        println!("  Peer seeds:         {}", data.peer_seed_count);
        println!("  Expires in:         {}h", data.ttl_hours);
        if let Some(preferred) = &data.preferred_surface
            && let Some(surface) = data.surfaces.get(preferred)
        {
            println!("  Preferred surface:  {}", preferred);
            println!("  AI join:            {}", surface.join.argv.join(" "));
        }
        if let Some(step) = &data.summary.next_step {
            println!("  Next:               {step}");
        }
    }
}

pub(crate) fn connection_inspect(_ctx: &FullCtx, file: PathBuf, json: bool) {
    let connection = ConnectionFile::load(&file).expect("failed to read connection file");
    let inspected_binding = thronglets::identity::IdentityBinding {
        schema_version: "thronglets.identity.v1".into(),
        owner_account: connection.owner_account.clone(),
        oasyce_delegate_policy: connection.oasyce_delegate_policy.clone(),
        device_identity: connection.primary_device_identity.clone(),
        binding_source: Some("connection_file".into()),
        joined_from_device: None,
        updated_at: connection.exported_at,
    };
    let identity = IdentitySummary {
        status: "valid",
        owner_account: connection.owner_account.clone(),
        device_identity: connection.primary_device_identity.clone(),
        binding_source: "connection_file".into(),
        joined_from_device: None,
        identity_model: identity_blueprint(
            connection.owner_account.clone(),
            connection.primary_device_identity.clone(),
        ),
        authorization: authorization_summary(&inspected_binding),
    };
    let inspected_trusted_peer_seed_count = match connection.peer_seed_scope {
        ConnectionSeedScope::Trusted => connection.peer_seeds.len(),
        ConnectionSeedScope::Remembered => 0,
    };
    let peer_seed_scope = connection.peer_seed_scope_label();
    let readiness = connection_readiness_summary(
        connection.peer_seed_scope.clone(),
        connection.peer_seeds.len(),
        "export",
    );
    let data = ConnectionInspectData {
        summary: readiness,
        identity,
        file: file.display().to_string(),
        preferred_surface: connection.effective_preferred_surface(),
        surfaces: connection.effective_surfaces(),
        primary_device_pubkey: connection.primary_device_pubkey.clone(),
        peer_seed_scope,
        trusted_peer_seed_count: inspected_trusted_peer_seed_count,
        peer_seed_count: connection.peer_seeds.len(),
        exported_at: connection.exported_at,
        expires_at: connection.expires_at,
        ttl_hours: connection.ttl_hours(),
        signature_verified: true,
    };
    if json {
        print_machine_json_with_schema(IDENTITY_SCHEMA_VERSION, "connection-inspect", &data);
    } else {
        println!("Connection file: {}", data.summary.status);
        println!("  Meaning:            {}", data.summary.detail);
        println!("  File:               {}", data.file);
        println!(
            "  Owner account:      {}",
            data.identity.owner_account.as_deref().unwrap_or("unbound")
        );
        println!("  Primary device:     {}", data.identity.device_identity);
        println!("  Signature verified: yes");
        println!("  Seed scope:         {}", data.peer_seed_scope);
        println!("  Trusted seeds:      {}", data.trusted_peer_seed_count);
        println!("  Peer seeds:         {}", data.peer_seed_count);
        println!("  Expires in:         {}h", data.ttl_hours);
        if let Some(preferred) = &data.preferred_surface
            && let Some(surface) = data.surfaces.get(preferred)
        {
            println!("  Preferred surface:  {}", preferred);
            println!("  AI join:            {}", surface.join.argv.join(" "));
        }
        if let Some(step) = &data.summary.next_step {
            println!("  Next:               {step}");
        }
    }
}

pub(crate) fn connection_join(ctx: &FullCtx, file: PathBuf, json: bool) {
    let connection = ConnectionFile::load(&file).expect("failed to read connection file");
    let binding = ctx
        .binding
        .clone()
        .joined_via_connection(
            connection.owner_account.clone(),
            connection.oasyce_delegate_policy.clone(),
            connection.primary_device_identity.clone(),
        )
        .expect("failed to update identity binding");
    binding
        .save(&identity_binding_path(&ctx.dir))
        .expect("failed to save identity binding");
    let mut network_snapshot = thronglets::network_state::NetworkSnapshot::load(&ctx.dir);
    match connection.peer_seed_scope {
        ConnectionSeedScope::Trusted => {
            network_snapshot.merge_trusted_peer_seeds(connection.peer_seeds.clone());
        }
        ConnectionSeedScope::Remembered => {
            network_snapshot.merge_peer_seeds(connection.peer_seeds.clone());
        }
    }
    network_snapshot.save(&ctx.dir);
    let imported_trusted_peer_seed_count = match connection.peer_seed_scope {
        ConnectionSeedScope::Trusted => connection.peer_seeds.len(),
        ConnectionSeedScope::Remembered => 0,
    };
    let peer_seed_scope = connection.peer_seed_scope_label();
    let readiness = connection_readiness_summary(
        connection.peer_seed_scope.clone(),
        connection.peer_seeds.len(),
        "export",
    );
    let data = ConnectionJoinData {
        summary: readiness,
        identity: identity_summary("joined", &binding),
        file: file.display().to_string(),
        signature_verified: true,
        peer_seed_scope,
        imported_trusted_peer_seed_count,
        imported_peer_seed_count: connection.peer_seeds.len(),
        source_expires_at: connection.expires_at,
    };
    if json {
        print_machine_json_with_schema(IDENTITY_SCHEMA_VERSION, "connection-join", &data);
    } else {
        println!("Connection join: {}", data.summary.status);
        println!("  Meaning:            {}", data.summary.detail);
        println!("  File:               {}", data.file);
        println!(
            "  Owner account:      {}",
            data.identity.owner_account.as_deref().unwrap_or("unbound")
        );
        println!("  Device identity:    {}", data.identity.device_identity);
        println!(
            "  Joined from device: {}",
            connection.primary_device_identity
        );
        println!("  Signature verified: yes");
        println!("  Seed scope:         {}", data.peer_seed_scope);
        println!(
            "  Imported trusted seeds: {}",
            data.imported_trusted_peer_seed_count
        );
        println!("  Imported peer seeds: {}", data.imported_peer_seed_count);
        if let Some(step) = &data.summary.next_step {
            println!("  Next:               {step}");
        }
    }
}
