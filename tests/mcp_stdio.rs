//! Integration test: verify the MCP server works end-to-end over stdio.
//! Spawns `thronglets mcp` as a subprocess, sends JSON-RPC messages,
//! and verifies responses match the MCP protocol.

use serde_json::{json, Value};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

/// Send a JSON-RPC request and read the response line.
async fn rpc_call(
    stdin: &mut tokio::process::ChildStdin,
    stdout: &mut tokio::io::Lines<BufReader<tokio::process::ChildStdout>>,
    request: Value,
) -> Value {
    let mut line = serde_json::to_string(&request).unwrap();
    line.push('\n');
    stdin.write_all(line.as_bytes()).await.unwrap();
    stdin.flush().await.unwrap();

    let resp_line = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        stdout.next_line(),
    )
    .await
    .expect("timeout waiting for MCP response")
    .expect("IO error reading response")
    .expect("unexpected EOF");

    serde_json::from_str(&resp_line).expect("response should be valid JSON")
}

/// Send a notification (no response expected).
async fn rpc_notify(
    stdin: &mut tokio::process::ChildStdin,
    request: Value,
) {
    let mut line = serde_json::to_string(&request).unwrap();
    line.push('\n');
    stdin.write_all(line.as_bytes()).await.unwrap();
    stdin.flush().await.unwrap();
}

#[tokio::test]
async fn mcp_full_protocol_handshake() {
    // Build the binary first
    let status = std::process::Command::new("cargo")
        .args(["build"])
        .status()
        .expect("cargo build failed");
    assert!(status.success(), "cargo build must succeed");

    // Use a temp dir so we don't pollute the real data dir
    let tmp_dir = tempfile::tempdir().unwrap();

    let mut child = Command::new("cargo")
        .args(["run", "--", "--data-dir", tmp_dir.path().to_str().unwrap(), "mcp"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn thronglets mcp");

    let mut stdin = child.stdin.take().unwrap();
    let stdout_reader = BufReader::new(child.stdout.take().unwrap());
    let mut lines = stdout_reader.lines();

    // 1. Initialize
    let resp = rpc_call(&mut stdin, &mut lines, json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    })).await;

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 1);
    assert_eq!(resp["result"]["serverInfo"]["name"], "thronglets");
    assert!(resp["result"]["protocolVersion"].as_str().is_some());

    // 2. Send initialized notification (no response expected)
    rpc_notify(&mut stdin, json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    })).await;

    // 3. List tools
    let resp = rpc_call(&mut stdin, &mut lines, json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list"
    })).await;

    let tools = resp["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 3);
    let tool_names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(tool_names.contains(&"trace_emit"));
    assert!(tool_names.contains(&"collective_query"));
    assert!(tool_names.contains(&"discover"));

    // 4. Emit a trace
    let resp = rpc_call(&mut stdin, &mut lines, json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "trace_emit",
            "arguments": {
                "about": "mcp-test/integration",
                "tags": ["test", "integration"],
                "outcome": "succeeded",
                "quality": 95,
                "latency_ms": 42
            }
        }
    })).await;

    assert!(resp["error"].is_null(), "trace_emit should succeed: {resp}");
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("mcp-test/integration"));

    // 5. Query collective intelligence
    let resp = rpc_call(&mut stdin, &mut lines, json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {
            "name": "collective_query",
            "arguments": { "about": "mcp-test/integration" }
        }
    })).await;

    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("100.0%"), "should show 100% success rate");
    assert!(text.contains("Total traces: 1"));

    // 6. Discover by tags
    let resp = rpc_call(&mut stdin, &mut lines, json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "tools/call",
        "params": {
            "name": "discover",
            "arguments": { "tags": ["integration"] }
        }
    })).await;

    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.contains("mcp-test/integration"));

    // 7. Unknown method returns error
    let resp = rpc_call(&mut stdin, &mut lines, json!({
        "jsonrpc": "2.0",
        "id": 99,
        "method": "nonexistent"
    })).await;

    assert_eq!(resp["error"]["code"], -32601);

    // Cleanup
    drop(stdin);
    let _ = child.kill().await;
}
