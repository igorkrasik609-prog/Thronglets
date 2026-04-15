use super::*;

use thronglets::active_policy::{compile_active_policy, method_compliance_from_payload};
use thronglets::context::{simhash, similarity as context_similarity};
use thronglets::contracts::{
    GIT_HISTORY_MAX_ENTRIES, PREHOOK_HEADER, PREHOOK_MAX_COLLECTIVE_QUERIES, PREHOOK_MAX_HINTS,
};
use thronglets::posts::SignalPostKind;
use thronglets::presence::{
    DEFAULT_PRESENCE_TTL_MINUTES, PresenceTraceConfig, create_presence_trace,
};
use thronglets::signals::{Signal, SignalKind, select as select_signals};
use thronglets::trace::{Outcome, Trace};

pub(crate) fn hook(ctx: &FullCtx) {
    // Read a generic post-tool hook payload from stdin.
    let hook_debug = std::env::var("THRONGLETS_HOOK_DEBUG").is_ok();

    let mut input = String::new();
    if std::io::Read::read_to_string(&mut std::io::stdin(), &mut input).is_err() {
        if hook_debug {
            eprintln!("[thronglets:hook] stdin read failed");
        }
        std::process::exit(0);
    }

    if hook_debug {
        eprintln!(
            "[thronglets:hook] stdin ({} bytes): {}",
            input.len(),
            &input[..input.len().min(200)]
        );
    }

    let payload: serde_json::Value = match serde_json::from_str(&input) {
        Ok(v) => v,
        Err(e) => {
            if hook_debug {
                eprintln!("[thronglets:hook] JSON parse error: {e}");
            }
            std::process::exit(0);
        }
    };

    let tool_name = payload["tool_name"].as_str().unwrap_or("");

    // Skip thronglets' own MCP calls to avoid recursion
    if tool_name.starts_with("mcp__thronglets") {
        std::process::exit(0);
    }

    // Skip empty tool names
    if tool_name.is_empty() {
        std::process::exit(0);
    }

    let agent_source = payload["agent_source"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("claude-code");
    auto_clear_restart_from_agent_source(&ctx.dir, agent_source);
    let current_space = derive_space(&payload);
    let current_mode = payload_string(&payload, "mode");

    // Map tool to capability URI
    let capability = if tool_name.starts_with("mcp__") {
        // MCP tools: mcp__server__tool -> mcp:server/tool
        tool_name.replacen("mcp__", "mcp:", 1).replace("__", "/")
    } else {
        format!("{agent_source}/{tool_name}")
    };

    // Determine outcome from tool_response
    let tool_response = &payload["tool_response"];
    let outcome = if tool_response.is_null() || tool_response.is_string() {
        // String response = success (Read, Grep, etc.)
        Outcome::Succeeded
    } else if let Some(obj) = tool_response.as_object() {
        if obj.contains_key("error") || obj.get("success") == Some(&serde_json::Value::Bool(false))
        {
            Outcome::Failed
        } else {
            Outcome::Succeeded
        }
    } else {
        Outcome::Succeeded
    };

    // Build context from tool_input
    let context_text = thronglets::context::build_hook_context(tool_name, &payload["tool_input"]);
    let active_policy = compile_active_policy(&payload, &payload["tool_input"]);
    let method_compliance = method_compliance_from_payload(&payload, &active_policy);

    // Input size = rough byte length of tool_input
    let input_size = payload["tool_input"].to_string().len() as u32;

    // Session ID from the calling agent runtime
    let session_id = payload["session_id"].as_str().map(String::from);

    // Model from environment or default
    let model = payload["model"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .map(ToOwned::to_owned)
        .or_else(|| std::env::var("CLAUDE_MODEL").ok())
        .unwrap_or_else(|| agent_source.to_string());

    // Load workspace once for both strategy inference and state update
    let mut ws = load_workspace_state(&ctx.dir);
    let enriched_context = if let Some(strategy) = ws.infer_strategy() {
        format!("[{strategy}] {context_text}")
    } else {
        context_text.clone()
    };

    let store = open_store(&ctx.dir);
    let ctx_hash = simhash(&enriched_context);
    let is_error = matches!(outcome, Outcome::Failed);
    let trace = Trace::new_with_agent_compliance(
        capability.clone(),
        outcome,
        0, // latency not available from hook
        input_size,
        ctx_hash,
        Some(enriched_context),
        session_id.clone(),
        ctx.binding.owner_account.clone(),
        Some(ctx.binding.device_identity.clone()),
        None,
        None,
        method_compliance,
        model,
        ctx.identity.public_key_bytes(),
        |msg| ctx.identity.sign(msg),
    );
    match store.insert_with_space(&trace, current_space.as_deref()) {
        Ok(_) => {
            if hook_debug {
                eprintln!("[thronglets:hook] recorded {capability}");
            }
        }
        Err(e) => {
            if hook_debug {
                eprintln!("[thronglets:hook] store insert failed: {e}");
            }
        }
    }

    // Psyche -> Thronglets bridge: ingest sparse exports as continuity/signals
    if tool_name.starts_with("mcp__psyche__") {
        let ingested = bridge_psyche_exports(
            tool_response,
            &store,
            &ctx.identity,
            &ctx.binding,
            session_id.as_deref(),
            &trace.model_id,
            current_space.as_deref(),
        );
        if hook_debug && ingested > 0 {
            eprintln!("[thronglets:hook] bridged {ingested} psyche exports");
        }
    }

    if current_space.is_some() || current_mode.is_some() {
        let presence = create_presence_trace(
            PresenceTraceConfig {
                model_id: trace.model_id.clone(),
                session_id: session_id.clone(),
                owner_account: ctx.binding.owner_account.clone(),
                device_identity: Some(ctx.binding.device_identity.clone()),
                space: current_space.clone(),
                mode: current_mode,
                sigil_id: None,
                capability: None,
                ttl_minutes: DEFAULT_PRESENCE_TTL_MINUTES,
            },
            ctx.identity.public_key_bytes(),
            |msg| ctx.identity.sign(msg),
        );
        let _ = store.insert(&presence);
    }
    let outcome_str = if is_error { "failed" } else { "succeeded" };

    // Track file interactions
    let file_path = workspace::extract_file_path(tool_name, &payload["tool_input"]);
    if let Some(ref fp) = file_path {
        ws.record_file(fp.clone(), tool_name, context_text.clone(), outcome_str);
    }

    // Track tool call sequence (for decision context)
    ws.record_action(
        tool_name,
        file_path.clone(),
        outcome_str,
        session_id.as_deref(),
    );
    let feedback_events = ws.resolve_recommendation_feedback(
        session_id.as_deref(),
        current_space.as_deref(),
        tool_name,
        file_path.as_deref(),
        outcome_str,
    );

    // Feedback -> trace: make signal evaluation visible to the substrate
    for event in &feedback_events {
        let polarity = if event.positive {
            "positive"
        } else {
            "negative"
        };
        let feedback_context = format!(
            "feedback:{} {} {}",
            polarity, event.recommendation_kind, event.source_kind,
        );
        let feedback_trace = Trace::new_with_identity(
            "urn:thronglets:signal:feedback".into(),
            if event.positive {
                Outcome::Succeeded
            } else {
                Outcome::Failed
            },
            0,
            0,
            simhash(&feedback_context),
            Some(feedback_context),
            session_id.clone(),
            ctx.binding.owner_account.clone(),
            Some(ctx.binding.device_identity.clone()),
            "thronglets-feedback".into(),
            ctx.identity.public_key_bytes(),
            |msg| ctx.identity.sign(msg),
        );
        let _ = store.insert_with_space(&feedback_trace, current_space.as_deref());
    }

    // Track pending feedback for Edit/Write
    if matches!(tool_name, "Edit" | "Write")
        && let Some(fp) = file_path
    {
        ws.add_pending_feedback(fp, tool_name);
    }

    // Resolve pending feedback (check git status for previous edits)
    ws.resolve_feedback();

    // Record errors for trace history (signals derived lazily by Prehook)
    if is_error && let Some(err) = workspace::extract_error(&payload["tool_response"]) {
        ws.record_error(tool_name, context_text.clone(), err, current_space.clone());
    }

    // Track session
    if let Some(sid) = &session_id {
        ws.track_session(sid, &capability, is_error);
    }

    ws.save(&ctx.dir);
}

pub(crate) fn prehook(ctx: &FullCtx) {
    let mut profiler = PrehookProfiler::from_env();
    let hook_debug = std::env::var("THRONGLETS_HOOK_DEBUG").is_ok();

    // Read a generic pre-tool hook payload from stdin.
    let mut input = String::new();
    if std::io::Read::read_to_string(&mut std::io::stdin(), &mut input).is_err() {
        if hook_debug {
            eprintln!("[thronglets:prehook] stdin read failed");
        }
        std::process::exit(0);
    }

    let payload: serde_json::Value = match serde_json::from_str(&input) {
        Ok(v) => v,
        Err(e) => {
            if hook_debug {
                eprintln!("[thronglets:prehook] JSON parse error: {e}");
            }
            std::process::exit(0);
        }
    };

    let tool_name = payload["tool_name"].as_str().unwrap_or("");
    let current_space = derive_space(&payload);
    let current_session_id = payload_string(&payload, "session_id");
    let agent_source = payload["agent_source"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or("claude-code");
    auto_clear_restart_from_agent_source(&ctx.dir, agent_source);

    // Skip thronglets' own calls and empty names
    if tool_name.starts_with("mcp__thronglets") || tool_name.is_empty() {
        std::process::exit(0);
    }

    // -- Pheromone model: silence is normal. Signal only on anomaly. --
    let mut signals: Vec<Signal> = Vec::new();
    let mut ws = load_workspace_state(&ctx.dir);
    let current_file = workspace::extract_file_path(tool_name, &payload["tool_input"]);
    let hook_context = thronglets::context::build_hook_context(tool_name, &payload["tool_input"]);
    let supports_file_guidance = matches!(tool_name, "Edit" | "Write") && current_file.is_some();
    profiler.stage("workspace");

    let mut collective_store: Option<thronglets::storage::TraceStore> = None;
    let mut collective_queries_remaining = PREHOOK_MAX_COLLECTIVE_QUERIES;

    let mut has_recent_tool_error = false;
    let active_policy = compile_active_policy(&payload, &payload["tool_input"]);

    if let Some(signal) = active_policy_signal(&active_policy) {
        signals.push(signal);
    }

    // -- Danger pheromone: low edit retention --
    if let Some(retention_warning) = ws.retention_warning(current_file.as_deref()) {
        let score = retention_warning.score
            + ws.recommendation_score_adjustment(SignalKind::Danger, current_space.as_deref());
        signals.push(Signal::danger(retention_warning.body, score));
    }

    // -- Alarm pheromone: recent errors with this tool --
    let now_ms = chrono::Utc::now().timestamp_millis();
    let ctx_hash = simhash(&hook_context);
    if let Some(recent_error) = ws.recent_errors.iter().find(|e| {
        // Scoped errors only match their own space; unscoped (legacy) errors are always visible
        if let Some(ref es) = e.space {
            if current_space.as_deref() != Some(es.as_str()) {
                return false;
            }
        }
        let age_ms = now_ms - e.timestamp_ms;
        if e.tool == tool_name && age_ms < 3_600_000 {
            return true; // same tool, last hour -- always fire
        }
        // Context-similar error within 7 days -- experiential recall
        if age_ms < 604_800_000 && !hook_context.is_empty() {
            let e_hash = e.context_hash.unwrap_or_else(|| simhash(&e.context));
            return context_similarity(&ctx_hash, &e_hash) >= 0.75;
        }
        false
    }) {
        let signal = {
            let e = recent_error;
            let snippet = if e.error_snippet.len() > 80 {
                format!("{}...", &e.error_snippet[..80])
            } else {
                e.error_snippet.clone()
            };
            let age_ms = now_ms - e.timestamp_ms;
            let score = if age_ms < 3_600_000 { 360 } else { 290 };
            Signal {
                kind: SignalKind::History,
                score,
                body: format!("  \u{26a0} past error: {snippet}"),
                candidate: None,
            }
        };
        has_recent_tool_error = true;
        signals.push(signal);
    }
    profiler.stage("danger");

    let explicit_signals_checked = !hook_context.is_empty();
    if explicit_signals_checked
        && let Some(store) = cached_collective_store(&mut collective_store, &ctx.dir)
    {
        for mut sig in explicit_signals(
            store,
            &hook_context,
            &ctx_hash,
            current_space.as_deref(),
            &ctx.binding.device_identity,
            ctx.identity.public_key_bytes(),
        ) {
            sig.score += ws.recommendation_score_adjustment(sig.kind, current_space.as_deref());
            signals.push(sig);
        }
    }
    profiler.stage_or_skip("explicit_signals", explicit_signals_checked);

    // -- Experience pheromone: past failures from trace history --
    let has_danger = signals.iter().any(|s| matches!(s.kind, SignalKind::Danger));
    let experience_checked = explicit_signals_checked && !has_danger;
    if experience_checked
        && let Some(store) = cached_collective_store(&mut collective_store, &ctx.dir)
        && let Ok(failures) =
            store.query_similar_failed_traces(&ctx_hash, 48, 168, 5, current_space.as_deref())
        && !failures.is_empty()
    {
        let count = failures.len();
        let snippet: String = failures[0]
            .context_text
            .as_deref()
            .unwrap_or("unknown")
            .chars()
            .take(80)
            .collect();
        let mut score = 280 + (count as i32).min(5) * 20;
        score += ws.recommendation_score_adjustment(SignalKind::History, current_space.as_deref());
        signals.push(Signal {
            kind: SignalKind::History,
            score,
            body: format!(
                "  \u{26a0} risk residue: {count} similar failure session(s) ({snippet})"
            ),
            candidate: None,
        });
    }
    profiler.stage_or_skip("experience", experience_checked);

    // -- Conflict prior: mixed outcomes mean the environment has not
    // yet settled on a stable path.
    let history_prior_checked = explicit_signals_checked && !has_danger;
    if history_prior_checked
        && let Some(store) = cached_collective_store(&mut collective_store, &ctx.dir)
    {
        for mut prior in thronglets::ambient::host_history_priors_for_context(
            store,
            &ctx_hash,
            current_space.as_deref(),
            None,
            &active_policy.relevant_rules,
        )
        .into_iter()
        .filter_map(history_signal_from_projection)
        {
            prior.score +=
                ws.recommendation_score_adjustment(SignalKind::History, current_space.as_deref());
            signals.push(prior);
        }
    }
    profiler.stage_or_skip("history_priors", history_prior_checked);

    if has_recent_tool_error
        && let Some(repair_hint) = ws
            .repair_trajectory_hint(tool_name)
            .or_else(|| ws.repair_hints(tool_name))
    {
        let mut repair_hint = repair_hint;
        repair_hint.score +=
            ws.recommendation_score_adjustment(SignalKind::Repair, current_space.as_deref());
        if claim_collective_query(&repair_hint.candidate, &mut collective_queries_remaining)
            && let Some(store) = cached_collective_store(&mut collective_store, &ctx.dir)
            && let Ok(collective_sources) = store.count_repair_sources(
                tool_name,
                &repair_hint.candidate.steps,
                168,
                current_space.as_deref(),
            )
        {
            apply_collective_sources(
                &mut repair_hint.candidate,
                &mut repair_hint.score,
                collective_sources,
            );
        }

        signals.push(Signal::repair_candidate(
            repair_hint.body,
            repair_hint.score,
            repair_hint.candidate,
        ));
    }
    profiler.stage("repair");

    // Co-edit signals: files that are frequently edited together.
    if supports_file_guidance
        && let Some(current_file) = current_file.as_deref()
        && let Some(store) = cached_collective_store(&mut collective_store, &ctx.dir)
    {
        for sig in co_edit_signals(
            store,
            current_file,
            &ws.recent_actions,
            current_session_id.as_deref(),
            current_space.as_deref(),
        ) {
            signals.push(sig);
        }
    }
    profiler.stage("co_edit");

    let presence_checked =
        current_space.is_some() && (!supports_file_guidance || !signals.is_empty());
    if presence_checked
        && let Some(store) = cached_collective_store(&mut collective_store, &ctx.dir)
        && let Some(space) = current_space.as_deref()
        && let Some(presence_signal) = presence_context_signal(
            store,
            space,
            current_session_id.as_deref(),
            &ctx.binding.device_identity,
            ctx.identity.public_key_bytes(),
        )
    {
        signals.push(presence_signal);
    }
    profiler.stage_or_skip("presence", presence_checked);

    // -- Field fallback: abstract patterns from Level 2-3 --
    let has_strong_signal = signals
        .iter()
        .any(|s| !matches!(s.kind, SignalKind::History) || s.score >= 300);
    let field_fallback_checked = !has_strong_signal && !hook_context.is_empty();
    if field_fallback_checked {
        let scan_request = thronglets::pheromone_socket::ScanRequest {
            context_hash: ctx_hash,
            space: current_space.clone(),
            file_path: current_file.clone(),
            limit: 3,
        };

        // Try live socket first (hot field, ~1ms), fall back to disk (stale, ~10ms)
        let scans: Vec<thronglets::pheromone::FieldScan> =
            thronglets::pheromone_socket::query(&ctx.dir, &scan_request).unwrap_or_else(|| {
                let field_path = ctx.dir.join("pheromone-field.v1.json");
                if field_path.exists()
                    && let Ok(data) = std::fs::read_to_string(&field_path)
                    && let Ok(snapshot) =
                        serde_json::from_str::<thronglets::pheromone::FieldSnapshot>(&data)
                {
                    let field = thronglets::pheromone::PheromoneField::new();
                    field.restore(&snapshot);
                    field.scan_with_fallback(
                        &ctx_hash,
                        current_space.as_deref(),
                        current_file.as_deref(),
                        3,
                    )
                } else {
                    Vec::new()
                }
            });

        for scan in scans {
            if scan.intensity > 0.1 {
                let level_tag = match scan.level {
                    thronglets::pheromone::AbstractionLevel::Project => "project",
                    thronglets::pheromone::AbstractionLevel::Typed => "pattern",
                    thronglets::pheromone::AbstractionLevel::Universal => "universal",
                    _ => continue,
                };
                let score = 220 + (scan.intensity * 40.0).round() as i32;
                signals.push(Signal {
                    kind: SignalKind::History,
                    score,
                    body: format!(
                        "  \u{1f30a} {level_tag}: {} ({:.0}% success across projects)",
                        scan.capability,
                        scan.valence * 100.0,
                    ),
                    candidate: None,
                });
            }
        }
    }
    profiler.stage_or_skip("field_fallback", field_fallback_checked);

    // History is a fallback when we don't already know a likely next move.
    let has_higher_priority_signal = signals
        .iter()
        .any(|signal| signal.kind != SignalKind::History);
    let mut git_checked = false;
    if !has_higher_priority_signal && supports_file_guidance {
        git_checked = true;
        if let Some(git_hints) = current_file
            .as_ref()
            .and_then(|fp| git_file_history(fp, GIT_HISTORY_MAX_ENTRIES))
        {
            signals.push(Signal::history(git_hints));
        }
    }
    profiler.stage_or_skip("git", git_checked);

    // Guardrail: prehook stays short and category-stable.
    let recommendations = ws.suppress_duplicate_recommendations(
        current_session_id.as_deref(),
        current_space.as_deref(),
        select_signals(signals, PREHOOK_MAX_HINTS),
    );
    if !recommendations.is_empty() {
        ws.record_intervention(
            tool_name,
            recommendations
                .iter()
                .map(|recommendation| recommendation.source_kind.as_str().to_string())
                .collect(),
        );
        ws.record_recommendation_emissions(
            tool_name,
            current_session_id.as_deref(),
            current_space.as_deref(),
            &recommendations,
        );
        ws.save(&ctx.dir);
    }
    profiler.stage("select");

    // Output: only when there's something worth saying
    let mut stdout_bytes = 0;
    if !recommendations.is_empty() {
        stdout_bytes += PREHOOK_HEADER.len() + 1;
        println!("{PREHOOK_HEADER}");
        for recommendation in &recommendations {
            let rendered = recommendation.render();
            stdout_bytes += rendered.len() + 1;
            println!("{rendered}");
        }
    }
    profiler.finish(
        tool_name,
        &recommendations,
        stdout_bytes,
        profile_file_guidance_gate(supports_file_guidance),
        PREHOOK_MAX_COLLECTIVE_QUERIES - collective_queries_remaining,
    );
    // Normal state -> complete silence. Zero tokens.
}

pub(crate) fn ingest(
    ctx: &FullCtx,
    session: Option<String>,
    model: String,
    space: Option<String>,
    json: bool,
) {
    let mut input = String::new();
    if std::io::Read::read_to_string(&mut std::io::stdin(), &mut input).is_err() {
        std::process::exit(0);
    }
    let payload: serde_json::Value = match serde_json::from_str(&input) {
        Ok(v) => v,
        Err(_) => std::process::exit(0),
    };
    let store = open_store(&ctx.dir);
    let ingested = bridge_psyche_exports(
        &payload,
        &store,
        &ctx.identity,
        &ctx.binding,
        session.as_deref(),
        &model,
        space.as_deref(),
    );
    if json {
        println!("{}", serde_json::json!({ "ingested": ingested }));
    } else if ingested > 0 {
        eprintln!("[thronglets:ingest] bridged {ingested} psyche exports");
    }
}

pub(crate) fn lifecycle_hook(ctx: &FullCtx, event: String) {
    // Read lifecycle hook payload from stdin.
    let mut input = String::new();
    if std::io::Read::read_to_string(&mut std::io::stdin(), &mut input).is_err() {
        std::process::exit(0);
    }

    let payload: serde_json::Value = match serde_json::from_str(&input) {
        Ok(v) => v,
        Err(_) => std::process::exit(0),
    };

    let session_id = payload_string(&payload, "session_id");
    let current_space = derive_space(&payload);

    let store = open_store(&ctx.dir);
    let mut ws = load_workspace_state(&ctx.dir);

    match event.as_str() {
        "session-start" => {
            let source = payload["source"].as_str().unwrap_or("startup");
            let model = payload["model"]
                .as_str()
                .unwrap_or("claude-code")
                .to_string();

            // Record lifecycle trace
            let lifecycle_ctx = format!("session:{} source:{}", source, model);
            let trace = Trace::new_with_identity(
                "urn:thronglets:lifecycle:session-start".into(),
                Outcome::Succeeded,
                0,
                0,
                simhash(&lifecycle_ctx),
                Some(lifecycle_ctx),
                session_id.clone(),
                ctx.binding.owner_account.clone(),
                Some(ctx.binding.device_identity.clone()),
                model.clone(),
                ctx.identity.public_key_bytes(),
                |msg| ctx.identity.sign(msg),
            );
            let _ = store.insert_with_space(&trace, current_space.as_deref());

            // Emit presence
            let presence = create_presence_trace(
                PresenceTraceConfig {
                    model_id: model,
                    session_id: session_id.clone(),
                    owner_account: ctx.binding.owner_account.clone(),
                    device_identity: Some(ctx.binding.device_identity.clone()),
                    space: current_space.clone(),
                    mode: payload_string(&payload, "mode"),
                    sigil_id: None,
                    capability: None,
                    ttl_minutes: DEFAULT_PRESENCE_TTL_MINUTES,
                },
                ctx.identity.public_key_bytes(),
                |msg| ctx.identity.sign(msg),
            );
            let _ = store.insert(&presence);

            // Initialize session in workspace
            if let Some(sid) = &session_id {
                ws.track_session(sid, "lifecycle/session-start", false);
            }

            // -- Session briefing: experience as background knowledge --
            let mut notes: Vec<String> = Vec::new();

            // Corrections first (recommend signals) -- what works
            if let Ok(recs) = store.query_recent_signal_traces(
                168,
                Some(SignalPostKind::Recommend),
                5,
                current_space.as_deref(),
            ) {
                for sig in &recs {
                    if let Some(msg) = sig
                        .context_text
                        .as_deref()
                        .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
                        .and_then(|v| v["message"].as_str().map(String::from))
                    {
                        let short: String = msg.chars().take(120).collect();
                        notes.push(format!("- {short}"));
                    }
                }
            }

            // Known failures (avoid signals) -- what doesn't work
            if let Ok(avoids) = store.query_recent_signal_traces(
                48,
                Some(SignalPostKind::Avoid),
                3,
                current_space.as_deref(),
            ) {
                for sig in &avoids {
                    if let Some((lifecycle_ctx, msg)) = sig
                        .context_text
                        .as_deref()
                        .and_then(|j| serde_json::from_str::<serde_json::Value>(j).ok())
                        .and_then(|v| {
                            let c = v["context"].as_str()?.to_string();
                            let m = v["message"].as_str()?.to_string();
                            Some((c, m))
                        })
                    {
                        let short_ctx: String = lifecycle_ctx.chars().take(40).collect();
                        let short_msg: String = msg.chars().take(60).collect();
                        notes.push(format!("- {short_ctx}: {short_msg}"));
                    }
                }
            }

            // Deduplicate and limit
            notes.dedup();
            notes.truncate(5);

            if !notes.is_empty() {
                let briefing = format!("Environment notes:\n{}", notes.join("\n"));
                let output = serde_json::json!({ "additionalContext": briefing });
                println!("{}", output);
            }

            ws.save(&ctx.dir);
        }

        "session-end" => {
            let lifecycle_ctx = format!(
                "session-end source:{}",
                payload["source"].as_str().unwrap_or("end"),
            );
            let trace = Trace::new_with_identity(
                "urn:thronglets:lifecycle:session-end".into(),
                Outcome::Succeeded,
                0,
                0,
                simhash(&lifecycle_ctx),
                Some(lifecycle_ctx),
                session_id.clone(),
                ctx.binding.owner_account.clone(),
                Some(ctx.binding.device_identity.clone()),
                "thronglets-lifecycle".into(),
                ctx.identity.public_key_bytes(),
                |msg| ctx.identity.sign(msg),
            );
            let _ = store.insert_with_space(&trace, current_space.as_deref());

            ws.save(&ctx.dir);
        }

        "subagent-start" => {
            let agent_type = payload["agent_type"].as_str().unwrap_or("unknown");
            let agent_id = payload_string(&payload, "agent_id").unwrap_or_else(|| "anon".into());
            let lifecycle_ctx = format!("subagent-start type:{} id:{}", agent_type, agent_id);
            let trace = Trace::new_with_identity(
                "urn:thronglets:lifecycle:subagent-start".into(),
                Outcome::Succeeded,
                0,
                0,
                simhash(&lifecycle_ctx),
                Some(lifecycle_ctx),
                session_id.clone(),
                ctx.binding.owner_account.clone(),
                Some(ctx.binding.device_identity.clone()),
                "thronglets-lifecycle".into(),
                ctx.identity.public_key_bytes(),
                |msg| ctx.identity.sign(msg),
            );
            let _ = store.insert_with_space(&trace, current_space.as_deref());
        }

        "subagent-stop" => {
            let agent_type = payload["agent_type"].as_str().unwrap_or("unknown");
            let agent_id = payload_string(&payload, "agent_id").unwrap_or_else(|| "anon".into());
            // Extract a summary fingerprint from last_assistant_message if present
            let summary: String = payload["last_assistant_message"]
                .as_str()
                .map(|m| m.chars().take(200).collect())
                .unwrap_or_default();
            let lifecycle_ctx = if summary.is_empty() {
                format!("subagent-stop type:{} id:{}", agent_type, agent_id)
            } else {
                format!(
                    "subagent-stop type:{} id:{} summary:{}",
                    agent_type, agent_id, summary
                )
            };
            let trace = Trace::new_with_identity(
                "urn:thronglets:lifecycle:subagent-stop".into(),
                Outcome::Succeeded,
                0,
                0,
                simhash(&lifecycle_ctx),
                Some(lifecycle_ctx),
                session_id.clone(),
                ctx.binding.owner_account.clone(),
                Some(ctx.binding.device_identity.clone()),
                "thronglets-lifecycle".into(),
                ctx.identity.public_key_bytes(),
                |msg| ctx.identity.sign(msg),
            );
            let _ = store.insert_with_space(&trace, current_space.as_deref());
        }

        _ => {} // Unknown event -- silent exit
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use thronglets::active_policy::{ActivePolicySet, PolicyStrength};
    use thronglets::ambient::ambient_priors_for_context;
    use thronglets::identity::NodeIdentity;
    use thronglets::posts::{
        DEFAULT_SIGNAL_TTL_HOURS, SignalTraceConfig, create_auto_signal_trace, create_signal_trace,
    };
    use thronglets::storage::TraceStore;
    use thronglets::trace::MethodCompliance;

    #[test]
    fn explicit_signals_surfaces_all_kinds() {
        let local_identity = NodeIdentity::generate();
        let store = TraceStore::in_memory().unwrap();

        // Insert avoid signal
        let avoid = create_signal_trace(
            SignalPostKind::Avoid,
            "edit file: src/main.rs",
            "skip the generated lockfile",
            SignalTraceConfig {
                model_id: "codex".into(),
                session_id: Some("local-a".into()),
                owner_account: None,
                device_identity: Some(local_identity.device_identity()),
                agent_id: None,
                sigil_id: None,
                space: None,
                ttl_hours: DEFAULT_SIGNAL_TTL_HOURS,
            },
            local_identity.public_key_bytes(),
            |msg| local_identity.sign(msg),
        );
        store.insert(&avoid).unwrap();

        // Insert watch signal
        let watch = create_auto_signal_trace(
            SignalPostKind::Watch,
            "bash: cargo test",
            "Bash errors \u{2192} Edit (2 sessions)",
            SignalTraceConfig {
                model_id: "ignored".into(),
                session_id: Some("local-b".into()),
                owner_account: None,
                device_identity: Some(local_identity.device_identity()),
                agent_id: None,
                sigil_id: None,
                space: None,
                ttl_hours: 48,
            },
            local_identity.public_key_bytes(),
            |msg| local_identity.sign(msg),
        );
        store.insert(&watch).unwrap();

        // Insert recommend signal
        let recommend = create_auto_signal_trace(
            SignalPostKind::Recommend,
            "bash: Run full test suite",
            "convergent: 4 sessions did this",
            SignalTraceConfig {
                model_id: "ignored".into(),
                session_id: Some("local-c".into()),
                owner_account: None,
                device_identity: Some(local_identity.device_identity()),
                agent_id: None,
                sigil_id: None,
                space: None,
                ttl_hours: 168,
            },
            local_identity.public_key_bytes(),
            |msg| local_identity.sign(msg),
        );
        store.insert(&recommend).unwrap();

        // Query avoid
        let ctx = "edit file: src/main.rs";
        let avoid_signals = explicit_signals(
            &store,
            ctx,
            &simhash(ctx),
            None,
            &local_identity.device_identity(),
            local_identity.public_key_bytes(),
        );
        assert!(!avoid_signals.is_empty());
        assert_eq!(avoid_signals[0].kind, SignalKind::Danger);
        assert!(avoid_signals[0].body.contains("avoid"));

        // Query watch
        let ctx = "bash: cargo test";
        let watch_signals = explicit_signals(
            &store,
            ctx,
            &simhash(ctx),
            None,
            &local_identity.device_identity(),
            local_identity.public_key_bytes(),
        );
        assert!(!watch_signals.is_empty());
        assert!(watch_signals.iter().any(|s| s.body.contains("watch")));

        // Query recommend
        let ctx = "bash: Run full test suite";
        let rec_signals = explicit_signals(
            &store,
            ctx,
            &simhash(ctx),
            None,
            &local_identity.device_identity(),
            local_identity.public_key_bytes(),
        );
        assert!(!rec_signals.is_empty());
        assert!(rec_signals.iter().any(|s| s.body.contains("recommended")));
    }

    #[test]
    fn explicit_signals_respects_space() {
        let local_identity = NodeIdentity::generate();
        let store = TraceStore::in_memory().unwrap();

        let trace = create_signal_trace(
            SignalPostKind::Avoid,
            "edit file: src/main.rs",
            "skip the generated lockfile",
            SignalTraceConfig {
                model_id: "codex".into(),
                session_id: Some("session-local".into()),
                owner_account: None,
                device_identity: Some(local_identity.device_identity()),
                agent_id: None,
                sigil_id: None,
                space: Some("other-space".into()),
                ttl_hours: DEFAULT_SIGNAL_TTL_HOURS,
            },
            local_identity.public_key_bytes(),
            |msg| local_identity.sign(msg),
        );
        store.insert(&trace).unwrap();

        // Wrong space -> empty
        let ctx = "edit file: src/main.rs";
        let wrong_space = explicit_signals(
            &store,
            ctx,
            &simhash(ctx),
            Some("psyche"),
            &local_identity.device_identity(),
            local_identity.public_key_bytes(),
        );
        assert!(wrong_space.is_empty());

        // Right space -> found
        let right_space = explicit_signals(
            &store,
            ctx,
            &simhash(ctx),
            Some("other-space"),
            &local_identity.device_identity(),
            local_identity.public_key_bytes(),
        );
        assert!(!right_space.is_empty());
        assert!(right_space[0].body.contains("skip the generated lockfile"));
    }

    #[test]
    fn history_prior_signals_surface_stable_path_after_three_compliant_sessions() {
        let store = TraceStore::in_memory().unwrap();
        let identity = NodeIdentity::generate();
        let ctx = "bash: cargo test --workspace";
        for idx in 0..3 {
            let trace = Trace::new_with_agent_compliance(
                "tool:Bash".into(),
                Outcome::Succeeded,
                0,
                1,
                simhash(ctx),
                Some(ctx.into()),
                Some(format!("session-{idx}")),
                None,
                Some(identity.device_identity()),
                None,
                None,
                Some(MethodCompliance::Compliant),
                "codex".into(),
                identity.public_key_bytes(),
                |msg| identity.sign(msg),
            );
            store.insert(&trace).unwrap();
        }

        let prior = thronglets::ambient::host_history_priors_for_context(
            &store,
            &simhash(ctx),
            None,
            None,
            &ActivePolicySet::default().relevant_rules,
        )
        .into_iter()
        .find_map(history_signal_from_projection)
        .unwrap();
        assert_eq!(prior.kind, SignalKind::History);
        assert!(prior.body.contains("stable path"));
        assert!(
            prior
                .body
                .contains("3 compliant session(s) crossed this context")
        );
    }

    #[test]
    fn history_prior_signals_stay_quiet_below_threshold() {
        let store = TraceStore::in_memory().unwrap();
        let identity = NodeIdentity::generate();
        let ctx = "bash: cargo test --workspace";
        for idx in 0..2 {
            let trace = Trace::new_with_agent(
                "tool:Bash".into(),
                Outcome::Succeeded,
                0,
                1,
                simhash(ctx),
                Some(ctx.into()),
                Some(format!("session-{idx}")),
                None,
                Some(identity.device_identity()),
                None,
                None,
                "codex".into(),
                identity.public_key_bytes(),
                |msg| identity.sign(msg),
            );
            store.insert(&trace).unwrap();
        }

        let signals: Vec<_> = thronglets::ambient::host_history_priors_for_context(
            &store,
            &simhash(ctx),
            None,
            None,
            &ActivePolicySet::default().relevant_rules,
        )
        .into_iter()
        .filter_map(history_signal_from_projection)
        .collect();
        assert!(
            signals
                .iter()
                .all(|signal| !signal.body.contains("stable path"))
        );
    }

    #[test]
    fn history_prior_signals_wait_for_clear_margin_over_recent_failures() {
        let store = TraceStore::in_memory().unwrap();
        let identity = NodeIdentity::generate();
        let ctx = "bash: cargo test --workspace";
        for idx in 0..3 {
            let trace = Trace::new_with_agent(
                "tool:Bash".into(),
                Outcome::Succeeded,
                0,
                1,
                simhash(ctx),
                Some(ctx.into()),
                Some(format!("success-{idx}")),
                None,
                Some(identity.device_identity()),
                None,
                None,
                "codex".into(),
                identity.public_key_bytes(),
                |msg| identity.sign(msg),
            );
            store.insert(&trace).unwrap();
        }
        for idx in 0..2 {
            let failed = Trace::new_with_agent(
                "tool:Bash".into(),
                Outcome::Failed,
                0,
                1,
                simhash(ctx),
                Some(ctx.into()),
                Some(format!("failed-{idx}")),
                None,
                Some(identity.device_identity()),
                None,
                None,
                "codex".into(),
                identity.public_key_bytes(),
                |msg| identity.sign(msg),
            );
            store.insert(&failed).unwrap();
        }

        let signals: Vec<_> = thronglets::ambient::host_history_priors_for_context(
            &store,
            &simhash(ctx),
            None,
            None,
            &ActivePolicySet::default().relevant_rules,
        )
        .into_iter()
        .filter_map(history_signal_from_projection)
        .collect();
        assert!(
            signals
                .iter()
                .all(|signal| !signal.body.contains("stable path"))
        );
    }

    #[test]
    fn history_prior_signals_surface_unsettled_path_when_outcomes_conflict() {
        let store = TraceStore::in_memory().unwrap();
        let identity = NodeIdentity::generate();
        let ctx = "bash: cargo test --workspace";
        for idx in 0..2 {
            let success = Trace::new_with_agent(
                "tool:Bash".into(),
                Outcome::Succeeded,
                0,
                1,
                simhash(ctx),
                Some(ctx.into()),
                Some(format!("success-{idx}")),
                None,
                Some(identity.device_identity()),
                None,
                None,
                "codex".into(),
                identity.public_key_bytes(),
                |msg| identity.sign(msg),
            );
            store.insert(&success).unwrap();

            let failed = Trace::new_with_agent(
                "tool:Bash".into(),
                Outcome::Failed,
                0,
                1,
                simhash(ctx),
                Some(ctx.into()),
                Some(format!("failed-{idx}")),
                None,
                Some(identity.device_identity()),
                None,
                None,
                "codex".into(),
                identity.public_key_bytes(),
                |msg| identity.sign(msg),
            );
            store.insert(&failed).unwrap();
        }

        let prior = thronglets::ambient::host_history_priors_for_context(
            &store,
            &simhash(ctx),
            None,
            None,
            &ActivePolicySet::default().relevant_rules,
        )
        .into_iter()
        .find_map(history_signal_from_projection)
        .unwrap();
        assert_eq!(prior.kind, SignalKind::History);
        assert!(prior.body.contains("unsettled path"));
    }

    #[test]
    fn history_prior_signals_stay_quiet_when_one_side_clearly_dominates() {
        let store = TraceStore::in_memory().unwrap();
        let identity = NodeIdentity::generate();
        let ctx = "bash: cargo test --workspace";
        for idx in 0..5 {
            let success = Trace::new_with_agent(
                "tool:Bash".into(),
                Outcome::Succeeded,
                0,
                1,
                simhash(ctx),
                Some(ctx.into()),
                Some(format!("success-{idx}")),
                None,
                Some(identity.device_identity()),
                None,
                None,
                "codex".into(),
                identity.public_key_bytes(),
                |msg| identity.sign(msg),
            );
            store.insert(&success).unwrap();
        }
        let failed = Trace::new_with_agent(
            "tool:Bash".into(),
            Outcome::Failed,
            0,
            1,
            simhash(ctx),
            Some(ctx.into()),
            Some("failed-0".into()),
            None,
            Some(identity.device_identity()),
            None,
            None,
            "codex".into(),
            identity.public_key_bytes(),
            |msg| identity.sign(msg),
        );
        store.insert(&failed).unwrap();

        assert!(
            thronglets::ambient::host_history_priors_for_context(
                &store,
                &simhash(ctx),
                None,
                None,
                &ActivePolicySet::default().relevant_rules,
            )
            .into_iter()
            .find_map(history_signal_from_projection)
            .is_none()
        );
    }

    #[test]
    fn ambient_priors_surface_conflict_and_success_without_new_ontology() {
        let store = TraceStore::in_memory().unwrap();
        let identity = NodeIdentity::generate();
        let mixed_ctx = "deploy thronglets service after reviewing recent failures";
        let stable_ctx = "rotate provider endpoint after stable repair path";
        for idx in 0..3 {
            let success = Trace::new_with_agent_compliance(
                "tool:Bash".into(),
                Outcome::Succeeded,
                0,
                1,
                simhash(mixed_ctx),
                Some(mixed_ctx.into()),
                Some(format!("success-{idx}")),
                None,
                Some(identity.device_identity()),
                None,
                None,
                Some(MethodCompliance::Compliant),
                "codex".into(),
                identity.public_key_bytes(),
                |msg| identity.sign(msg),
            );
            store.insert(&success).unwrap();
        }
        for idx in 0..2 {
            let failed = Trace::new_with_agent(
                "tool:Bash".into(),
                Outcome::Failed,
                0,
                1,
                simhash(mixed_ctx),
                Some(mixed_ctx.into()),
                Some(format!("failed-{idx}")),
                None,
                Some(identity.device_identity()),
                None,
                None,
                "codex".into(),
                identity.public_key_bytes(),
                |msg| identity.sign(msg),
            );
            store.insert(&failed).unwrap();
        }
        for idx in 0..5 {
            let success = Trace::new_with_agent_compliance(
                "tool:Bash".into(),
                Outcome::Succeeded,
                0,
                1,
                simhash(stable_ctx),
                Some(stable_ctx.into()),
                Some(format!("stable-success-{idx}")),
                None,
                Some(identity.device_identity()),
                None,
                None,
                Some(MethodCompliance::Compliant),
                "codex".into(),
                identity.public_key_bytes(),
                |msg| identity.sign(msg),
            );
            store.insert(&success).unwrap();
        }

        let mixed_priors = ambient_priors_for_context(&store, &simhash(mixed_ctx), None, None, 3);
        assert!(!mixed_priors.is_empty());
        assert!(
            mixed_priors
                .iter()
                .any(|prior| prior.summary.contains("mixed residue"))
        );
        assert!(
            mixed_priors
                .iter()
                .any(|prior| prior.kind == "mixed-residue")
        );

        let stable_priors = ambient_priors_for_context(&store, &simhash(stable_ctx), None, None, 3);
        assert!(!stable_priors.is_empty());
        assert!(
            stable_priors
                .iter()
                .any(|prior| prior.summary.contains("crossed this context"))
        );
        assert!(
            stable_priors
                .iter()
                .any(|prior| prior.kind == "success-prior")
        );
        assert!(
            mixed_priors
                .iter()
                .all(|prior| prior.provider == "thronglets")
        );
        assert!(
            stable_priors
                .iter()
                .all(|prior| prior.provider == "thronglets")
        );
    }

    #[test]
    fn ambient_priors_limit_results_and_sort_by_confidence() {
        let store = TraceStore::in_memory().unwrap();
        let identity = NodeIdentity::generate();
        let ctx = "investigate repeated frontend regression path";
        for idx in 0..4 {
            let success = Trace::new_with_agent(
                "tool:Edit".into(),
                Outcome::Succeeded,
                0,
                1,
                simhash(ctx),
                Some(ctx.into()),
                Some(format!("success-{idx}")),
                None,
                Some(identity.device_identity()),
                None,
                None,
                "codex".into(),
                identity.public_key_bytes(),
                |msg| identity.sign(msg),
            );
            store.insert(&success).unwrap();
        }
        for idx in 0..3 {
            let failed = Trace::new_with_agent(
                "tool:Edit".into(),
                Outcome::Failed,
                0,
                1,
                simhash(ctx),
                Some(ctx.into()),
                Some(format!("failed-{idx}")),
                None,
                Some(identity.device_identity()),
                None,
                None,
                "codex".into(),
                identity.public_key_bytes(),
                |msg| identity.sign(msg),
            );
            store.insert(&failed).unwrap();
        }

        let priors = ambient_priors_for_context(&store, &simhash(ctx), None, None, 2);
        assert_eq!(priors.len(), 2);
        assert!(priors[0].confidence >= priors[1].confidence);
    }

    #[test]
    fn ambient_priors_bias_toward_current_goal_without_new_ontology() {
        let store = TraceStore::in_memory().unwrap();
        let identity = NodeIdentity::generate();
        let ctx = "repair deployment after repeated endpoint failures but with one previously stable path";
        for idx in 0..5 {
            let success = Trace::new_with_agent_compliance(
                "tool:Bash".into(),
                Outcome::Succeeded,
                0,
                1,
                simhash(ctx),
                Some(ctx.into()),
                Some(format!("success-{idx}")),
                None,
                Some(identity.device_identity()),
                None,
                None,
                Some(MethodCompliance::Compliant),
                "codex".into(),
                identity.public_key_bytes(),
                |msg| identity.sign(msg),
            );
            store.insert(&success).unwrap();
        }
        for idx in 0..3 {
            let failed = Trace::new_with_agent(
                "tool:Bash".into(),
                Outcome::Failed,
                0,
                1,
                simhash(ctx),
                Some(ctx.into()),
                Some(format!("failed-{idx}")),
                None,
                Some(identity.device_identity()),
                None,
                None,
                "codex".into(),
                identity.public_key_bytes(),
                |msg| identity.sign(msg),
            );
            store.insert(&failed).unwrap();
        }

        let build_priors = ambient_priors_for_context(
            &store,
            &simhash(ctx),
            None,
            Some(thronglets::ambient::AmbientTurnGoal::Build),
            3,
        );
        let explore_priors = ambient_priors_for_context(
            &store,
            &simhash(ctx),
            None,
            Some(thronglets::ambient::AmbientTurnGoal::Explore),
            3,
        );
        let repair_priors = ambient_priors_for_context(
            &store,
            &simhash(ctx),
            None,
            Some(thronglets::ambient::AmbientTurnGoal::Repair),
            3,
        );

        let build_success = build_priors
            .iter()
            .find(|prior| prior.kind == "success-prior")
            .unwrap();
        let repair_success = repair_priors
            .iter()
            .find(|prior| prior.kind == "success-prior")
            .unwrap();
        let build_failure = build_priors
            .iter()
            .find(|prior| prior.kind == "failure-residue")
            .unwrap();
        let repair_failure = repair_priors
            .iter()
            .find(|prior| prior.kind == "failure-residue")
            .unwrap();

        assert!(
            explore_priors
                .iter()
                .all(|prior| prior.kind != "success-prior")
        );
        assert!(build_success.confidence > repair_success.confidence);
        assert!(build_success.confidence > 0.8);
        assert!(repair_failure.confidence > build_failure.confidence);
        assert!(repair_priors.iter().all(|prior| prior.goal.is_some()));
    }

    #[test]
    fn ambient_priors_keep_explore_success_soft_and_non_exclusive() {
        let store = TraceStore::in_memory().unwrap();
        let identity = NodeIdentity::generate();
        let ctx = "investigate a non-consensus optimization route with one stable prior path";
        for idx in 0..5 {
            let success = Trace::new_with_agent_compliance(
                "tool:Edit".into(),
                Outcome::Succeeded,
                0,
                1,
                simhash(ctx),
                Some(ctx.into()),
                Some(format!("success-{idx}")),
                None,
                Some(identity.device_identity()),
                None,
                None,
                Some(MethodCompliance::Compliant),
                "codex".into(),
                identity.public_key_bytes(),
                |msg| identity.sign(msg),
            );
            store.insert(&success).unwrap();
        }

        let explore_priors = ambient_priors_for_context(
            &store,
            &simhash(ctx),
            None,
            Some(thronglets::ambient::AmbientTurnGoal::Explore),
            3,
        );
        let build_priors = ambient_priors_for_context(
            &store,
            &simhash(ctx),
            None,
            Some(thronglets::ambient::AmbientTurnGoal::Build),
            3,
        );

        let explore_success = explore_priors
            .iter()
            .find(|prior| prior.kind == "success-prior")
            .unwrap();
        let build_success = build_priors
            .iter()
            .find(|prior| prior.kind == "success-prior")
            .unwrap();

        assert!(explore_success.confidence <= 0.68);
        assert!(build_success.confidence > explore_success.confidence);
        assert!(explore_success.summary.contains("non-exclusive baseline"));
    }

    #[test]
    fn hard_current_turn_policy_surfaces_as_danger_signal() {
        let active_policy = ActivePolicySet {
            all_rules: vec![thronglets::active_policy::ActivePolicyRule {
                id: "task:reuse-components".into(),
                strength: PolicyStrength::Hard,
                scope: thronglets::active_policy::PolicyScope::Task,
                summary: "reuse existing shared components".into(),
            }],
            relevant_rules: vec![thronglets::active_policy::ActivePolicyRule {
                id: "task:reuse-components".into(),
                strength: PolicyStrength::Hard,
                scope: thronglets::active_policy::PolicyScope::Task,
                summary: "reuse existing shared components".into(),
            }],
        };

        let signal = active_policy_signal(&active_policy).unwrap();
        assert_eq!(signal.kind, SignalKind::History);
        assert!(signal.body.contains("reuse existing shared components"));
    }
}
