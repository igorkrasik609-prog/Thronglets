//! Lightweight HTTP API for non-MCP agents.
//!
//! Exposes the same trace/query capabilities as MCP, but over HTTP/JSON.
//! This opens Thronglets to Python/LangChain/AutoGen and any HTTP-capable agent.
//!
//! Endpoints:
//! - POST /v1/traces       — record a trace
//! - GET  /v1/query        — query the substrate
//! - GET  /v1/capabilities — list known capabilities
//! - GET  /v1/status       — node status

use crate::context::{simhash, similarity};
use crate::identity::NodeIdentity;
use crate::storage::TraceStore;
use crate::trace::{Outcome, Trace};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tracing::{debug, info};

pub struct HttpContext {
    pub identity: Arc<NodeIdentity>,
    pub store: Arc<TraceStore>,
}

/// Start the HTTP API server on the given port.
pub async fn serve(ctx: Arc<HttpContext>, port: u16) -> std::io::Result<()> {
    let listener = TcpListener::bind(format!("0.0.0.0:{port}")).await?;
    info!(port, "HTTP API listening");

    loop {
        let (mut stream, addr) = listener.accept().await?;
        let ctx = Arc::clone(&ctx);

        tokio::spawn(async move {
            let mut buf = vec![0u8; 65536];
            let n = match stream.read(&mut buf).await {
                Ok(n) if n > 0 => n,
                _ => return,
            };
            let request = String::from_utf8_lossy(&buf[..n]);

            let response = handle_http_request(&ctx, &request);

            let http_response = format!(
                "HTTP/1.1 {} OK\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                if response.contains("\"error\"") { "400" } else { "200" },
                response.len(),
                response,
            );

            let _ = stream.write_all(http_response.as_bytes()).await;
            debug!(%addr, "HTTP request handled");
        });
    }
}

fn handle_http_request(ctx: &HttpContext, raw: &str) -> String {
    // Parse HTTP method and path from first line
    let first_line = raw.lines().next().unwrap_or("");
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    let method = parts.first().copied().unwrap_or("");
    let path = parts.get(1).copied().unwrap_or("");

    // Handle CORS preflight
    if method == "OPTIONS" {
        return "{}".to_string();
    }

    // Extract JSON body (everything after the blank line)
    let body = raw.split("\r\n\r\n").nth(1).unwrap_or("");

    match (method, path) {
        ("POST", "/v1/traces") => handle_post_trace(ctx, body),
        ("GET", p) if p.starts_with("/v1/query") => handle_get_query(ctx, p),
        ("GET", "/v1/capabilities") => handle_get_capabilities(ctx),
        ("GET", "/v1/status") => handle_get_status(ctx),
        _ => json!({"error": "not found", "endpoints": [
            "POST /v1/traces",
            "GET /v1/query?context=...&intent=resolve|evaluate|explore",
            "GET /v1/capabilities",
            "GET /v1/status"
        ]}).to_string(),
    }
}

fn handle_post_trace(ctx: &HttpContext, body: &str) -> String {
    let args: Value = match serde_json::from_str(body) {
        Ok(v) => v,
        Err(e) => return json!({"error": format!("invalid JSON: {e}")}).to_string(),
    };

    let capability = match args["capability"].as_str() {
        Some(s) => s.to_string(),
        None => return json!({"error": "missing field: capability"}).to_string(),
    };

    let outcome = match args["outcome"].as_str().unwrap_or("succeeded") {
        "succeeded" | "success" => Outcome::Succeeded,
        "failed" | "fail" => Outcome::Failed,
        "partial" => Outcome::Partial,
        "timeout" => Outcome::Timeout,
        _ => Outcome::Succeeded,
    };

    let latency_ms = args["latency_ms"].as_u64().unwrap_or(0) as u32;
    let input_size = args["input_size"].as_u64().unwrap_or(0) as u32;
    let context_str = args["context"].as_str().unwrap_or("");
    let model_id = args["model"].as_str().unwrap_or("unknown").to_string();
    let session_id = args["session_id"].as_str().map(String::from);

    let context_hash = simhash(context_str);
    let context_text = if context_str.is_empty() { None } else { Some(context_str.to_string()) };

    let trace = Trace::new(
        capability.clone(),
        outcome,
        latency_ms,
        input_size,
        context_hash,
        context_text,
        session_id,
        model_id,
        ctx.identity.public_key_bytes(),
        |msg| ctx.identity.sign(msg),
    );

    let trace_id_hex: String = trace.id[..8].iter().map(|b| format!("{b:02x}")).collect();

    match ctx.store.insert(&trace) {
        Ok(_) => json!({
            "recorded": true,
            "trace_id": trace_id_hex,
            "capability": capability,
        }).to_string(),
        Err(e) => json!({"error": format!("storage: {e}")}).to_string(),
    }
}

fn handle_get_query(ctx: &HttpContext, path: &str) -> String {
    // Parse query string from path
    let query_str = path.split('?').nth(1).unwrap_or("");
    let params: HashMap<&str, &str> = query_str
        .split('&')
        .filter_map(|p| p.split_once('='))
        .collect();

    let context_str = params.get("context").copied().unwrap_or("");
    let intent = params.get("intent").copied().unwrap_or("explore");
    let capability = params.get("capability").copied().unwrap_or("");
    let limit: usize = params.get("limit")
        .and_then(|s| s.parse().ok())
        .unwrap_or(10);

    match intent {
        "resolve" => {
            let context_hash = simhash(context_str);
            let traces = match ctx.store.query_similar(&context_hash, 48, limit * 10) {
                Ok(t) => t,
                Err(e) => return json!({"error": format!("query: {e}")}).to_string(),
            };

            let mut cap_groups: HashMap<&str, Vec<&Trace>> = HashMap::new();
            for t in &traces {
                cap_groups.entry(&t.capability).or_default().push(t);
            }

            let mut capabilities: Vec<Value> = cap_groups.iter().map(|(cap, group)| {
                let total = group.len() as u64;
                let successes = group.iter().filter(|t| matches!(t.outcome, Outcome::Succeeded)).count() as f64;
                let success_rate = if total > 0 { successes / total as f64 } else { 0.0 };
                let best_sim = group.iter()
                    .map(|t| similarity(&context_hash, &t.context_hash))
                    .fold(0.0_f64, f64::max);
                let samples: Vec<&str> = group.iter()
                    .filter_map(|t| t.context_text.as_deref())
                    .take(3)
                    .collect();

                json!({
                    "capability": cap,
                    "context_similarity": round2(best_sim),
                    "success_rate": round2(success_rate),
                    "total_traces": total,
                    "context_samples": samples,
                })
            }).collect();

            capabilities.sort_by(|a, b| {
                b["context_similarity"].as_f64().unwrap_or(0.0)
                    .partial_cmp(&a["context_similarity"].as_f64().unwrap_or(0.0))
                    .unwrap_or(std::cmp::Ordering::Equal)
            });
            capabilities.truncate(limit);

            json!({"capabilities": capabilities}).to_string()
        }
        "evaluate" => {
            if capability.is_empty() {
                return json!({"error": "evaluate requires ?capability="}).to_string();
            }
            match ctx.store.aggregate(capability) {
                Ok(Some(stats)) => json!({
                    "capability": capability,
                    "stats": {
                        "total_traces": stats.total_traces,
                        "success_rate": round2(stats.success_rate),
                        "p50_latency_ms": stats.p50_latency_ms,
                        "p95_latency_ms": stats.p95_latency_ms,
                        "confidence": round2(stats.confidence),
                    }
                }).to_string(),
                Ok(None) => json!({"capability": capability, "stats": null}).to_string(),
                Err(e) => json!({"error": format!("query: {e}")}).to_string(),
            }
        }
        _ => handle_get_capabilities(ctx),
    }
}

fn handle_get_capabilities(ctx: &HttpContext) -> String {
    let caps = ctx.store.distinct_capabilities(100).unwrap_or_default();
    let mut result = Vec::new();
    for cap in &caps {
        if let Ok(Some(stats)) = ctx.store.aggregate(cap) {
            result.push(json!({
                "capability": cap,
                "total_traces": stats.total_traces,
                "success_rate": round2(stats.success_rate),
                "p50_latency_ms": stats.p50_latency_ms,
                "confidence": round2(stats.confidence),
            }));
        }
    }
    json!({"capabilities": result}).to_string()
}

fn handle_get_status(ctx: &HttpContext) -> String {
    let trace_count = ctx.store.count().unwrap_or(0);
    let cap_count = ctx.store.distinct_capabilities(1000)
        .map(|s| s.len())
        .unwrap_or(0);

    json!({
        "version": env!("CARGO_PKG_VERSION"),
        "node_id": hex_encode(&ctx.identity.public_key_bytes()[..4]),
        "trace_count": trace_count,
        "capabilities": cap_count,
    }).to_string()
}

fn round2(v: f64) -> f64 {
    (v * 100.0).round() / 100.0
}

fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
