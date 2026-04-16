//! Integration test: verify the MCP server works end-to-end over stdio.
//! Spawns `thronglets mcp` as a subprocess, sends JSON-RPC messages,
//! and verifies responses match the MCP protocol.

use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use tempfile::TempDir;

const STARTUP_RPC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);
const RPC_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

enum StdoutEvent {
    Line(String),
    Eof,
    Error(String),
}

struct McpHarness {
    child: Child,
    stdin: ChildStdin,
    stdout: Receiver<StdoutEvent>,
    reader: Option<thread::JoinHandle<()>>,
}

impl McpHarness {
    fn call(&mut self, request: Value) -> Value {
        rpc_call_with_timeout(&mut self.stdin, &self.stdout, request, RPC_TIMEOUT)
    }

    fn call_startup(&mut self, request: Value) -> Value {
        rpc_call_with_timeout(&mut self.stdin, &self.stdout, request, STARTUP_RPC_TIMEOUT)
    }

    fn notify(&mut self, request: Value) {
        rpc_notify(&mut self.stdin, request);
    }

    fn shutdown(mut self) {
        drop(self.stdin);
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

/// Send a JSON-RPC request and read the response line.
fn rpc_call_with_timeout(
    stdin: &mut ChildStdin,
    stdout: &Receiver<StdoutEvent>,
    request: Value,
    timeout: std::time::Duration,
) -> Value {
    let method = request["method"].as_str().unwrap_or("unknown").to_string();
    let mut line = serde_json::to_string(&request).unwrap();
    line.push('\n');
    stdin.write_all(line.as_bytes()).unwrap();
    stdin.flush().unwrap();

    let event = stdout
        .recv_timeout(timeout)
        .unwrap_or_else(|_| panic!("timeout waiting for MCP response to {method}"));

    match event {
        StdoutEvent::Line(resp_line) => {
            serde_json::from_str(&resp_line).expect("response should be valid JSON")
        }
        StdoutEvent::Eof => panic!("unexpected EOF waiting for MCP response to {method}"),
        StdoutEvent::Error(err) => panic!("IO error reading MCP response to {method}: {err}"),
    }
}

/// Send a notification (no response expected).
fn rpc_notify(stdin: &mut ChildStdin, request: Value) {
    let mut line = serde_json::to_string(&request).unwrap();
    line.push('\n');
    stdin.write_all(line.as_bytes()).unwrap();
    stdin.flush().unwrap();
}

fn spawn_mcp_stdio(tmp_dir: &TempDir) -> McpHarness {
    let mut child = Command::new(env!("CARGO_BIN_EXE_thronglets"))
        .args([
            "--data-dir",
            tmp_dir.path().to_str().unwrap(),
            "mcp",
            "--local",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn thronglets mcp");
    let stdin = child.stdin.take().expect("stdin should be piped");
    let stdout = child.stdout.take().expect("stdout should be piped");
    let (tx, rx) = mpsc::channel();
    let reader = thread::spawn(move || {
        let reader = BufReader::new(stdout);
        for line in reader.lines() {
            match line {
                Ok(line) => {
                    if tx.send(StdoutEvent::Line(line)).is_err() {
                        return;
                    }
                }
                Err(err) => {
                    let _ = tx.send(StdoutEvent::Error(err.to_string()));
                    return;
                }
            }
        }
        let _ = tx.send(StdoutEvent::Eof);
    });

    McpHarness {
        child,
        stdin,
        stdout: rx,
        reader: Some(reader),
    }
}

#[test]
fn mcp_stdio_smoke() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let mut harness = spawn_mcp_stdio(&tmp_dir);

    let resp = harness.call_startup(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    }));

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 1);
    assert_eq!(resp["result"]["serverInfo"]["name"], "thronglets");
    assert!(resp["result"]["protocolVersion"].as_str().is_some());

    harness.notify(json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    }));

    let resp = harness.call(json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list"
    }));

    let tools = resp["result"]["tools"].as_array().unwrap();
    assert!(
        tools.len() >= 9,
        "expected at least 9 tools after ambient prior surface expansion"
    );
    let tool_names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(tool_names.contains(&"trace_record"));
    assert!(tool_names.contains(&"substrate_query"));
    assert!(tool_names.contains(&"trace_anchor"));
    assert!(tool_names.contains(&"signal_post"));
    assert!(tool_names.contains(&"signal_feed"));
    assert!(tool_names.contains(&"presence_ping"));
    assert!(tool_names.contains(&"presence_feed"));
    assert!(tool_names.contains(&"authorization_check"));
    assert!(tool_names.contains(&"ambient_priors"));

    harness.shutdown();
}

#[test]
#[ignore = "slow e2e MCP handshake"]
fn mcp_full_protocol_handshake_e2e() {
    let tmp_dir = tempfile::tempdir().unwrap();
    let mut harness = spawn_mcp_stdio(&tmp_dir);

    let resp = harness.call_startup(json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {}
    }));

    assert_eq!(resp["jsonrpc"], "2.0");
    assert_eq!(resp["id"], 1);
    assert_eq!(resp["result"]["serverInfo"]["name"], "thronglets");
    assert!(resp["result"]["protocolVersion"].as_str().is_some());

    harness.notify(json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    }));

    let resp = harness.call(json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list"
    }));
    let tools = resp["result"]["tools"].as_array().unwrap();
    assert!(
        tools.len() >= 9,
        "expected at least 9 tools after ambient prior surface expansion"
    );

    let resp = harness.call(json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "trace_record",
            "arguments": {
                "capability": "mcp-test/integration",
                "outcome": "succeeded",
                "latency_ms": 42,
                "input_size": 1000,
                "context": "integration testing MCP protocol",
                "model": "test-model"
            }
        }
    }));

    assert!(
        resp["error"].is_null(),
        "trace_record should succeed: {resp}"
    );
    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(text).expect("response should be structured JSON");
    assert_eq!(parsed["recorded"], true);
    assert_eq!(parsed["capability"], "mcp-test/integration");
    assert!(!parsed["trace_id"].as_str().unwrap().is_empty());

    let resp = harness.call(json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {
            "name": "substrate_query",
            "arguments": {
                "context": "evaluating test capabilities",
                "intent": "evaluate",
                "capability": "mcp-test/integration"
            }
        }
    }));

    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(text).expect("evaluate response should be JSON");
    assert_eq!(parsed["capability"], "mcp-test/integration");
    assert_eq!(parsed["stats"]["total_traces"], 1);
    let sr = parsed["stats"]["success_rate"].as_f64().unwrap();
    assert!(
        sr > 0.5 && sr <= 1.0,
        "success_rate should be > 0.5, got {sr}"
    );

    let resp = harness.call(json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "tools/call",
        "params": {
            "name": "substrate_query",
            "arguments": {
                "context": "discovering what tools are available",
                "intent": "explore"
            }
        }
    }));

    let text = resp["result"]["content"][0]["text"].as_str().unwrap();
    let parsed: Value = serde_json::from_str(text).expect("explore response should be JSON");
    let caps = parsed["capabilities"].as_array().unwrap();
    assert!(!caps.is_empty(), "should find at least one capability");
    assert_eq!(caps[0]["capability"], "tool:integration");

    let resp = harness.call(json!({
        "jsonrpc": "2.0",
        "id": 99,
        "method": "nonexistent"
    }));

    assert_eq!(resp["error"]["code"], -32601);

    harness.shutdown();
}
