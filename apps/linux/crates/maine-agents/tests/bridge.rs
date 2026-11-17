//! `maine --mcp`: the MCP (JSON-RPC over stdio) side, driven line by line.
//! Ports of the Swift `MCPBridgeTests`.

mod common;

use common::*;
use maine_agents::json::ValueExt;
use maine_agents::protocol::{AgentCall, AgentReply};
use maine_agents::{
    serve_lines, AgentCallHandler, AgentClient, AgentClientStore, AgentServer, McpBridgeSession,
    MemorySecretStore,
};
use serde_json::{json, Value};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

struct ToolEcho;

impl AgentCallHandler for ToolEcho {
    fn handle(&self, call: AgentCall, _client: &AgentClient) -> AgentReply {
        if call.tool == "close_session" {
            return AgentReply::failure(call.id, "not found");
        }
        AgentReply::success(
            call.id,
            json!({"tool": call.tool, "arguments": call.arguments}),
        )
    }
}

/// Counts calls and cuts the connection before answering, like an app that
/// died mid-call.
struct DroppingHandler {
    calls: AtomicUsize,
    server: OnceLock<Arc<AgentServer>>,
}

impl AgentCallHandler for DroppingHandler {
    fn handle(&self, call: AgentCall, client: &AgentClient) -> AgentReply {
        self.calls.fetch_add(1, Ordering::SeqCst);
        if let Some(server) = self.server.get() {
            server.disconnect(client.id);
        }
        std::thread::sleep(Duration::from_millis(100));
        AgentReply::success(call.id, json!({}))
    }
}

fn object(line: Option<String>) -> Value {
    serde_json::from_str(&line.expect("a reply")).expect("json")
}

fn socket_path() -> (tempfile::TempDir, PathBuf) {
    let dir = short_socket_dir();
    let path = dir.path().join("agent.sock");
    (dir, path)
}

struct Served {
    _dir: tempfile::TempDir,
    socket: PathBuf,
    clients: Arc<AgentClientStore>,
    server: Arc<AgentServer>,
}

fn serve(handler: Arc<dyn AgentCallHandler>) -> Served {
    let (dir, socket) = socket_path();
    let clients = Arc::new(AgentClientStore::new(Arc::new(MemorySecretStore::new())));
    let server = Arc::new(AgentServer::new(socket.clone(), clients.clone(), handler));
    server.start().unwrap();
    Served {
        _dir: dir,
        socket,
        clients,
        server,
    }
}

#[test]
fn initialize_negotiates_the_protocol_version() {
    let (_dir, socket) = socket_path();
    let bridge = McpBridgeSession::new(socket, None).with_server_version("9.9.9");
    let reply = object(bridge.handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#));
    assert_eq!(reply["id"], 1);
    assert_eq!(
        reply["result"].string("protocolVersion"),
        Some("2025-03-26")
    );
    assert_eq!(reply["result"]["serverInfo"].string("name"), Some("maine"));
    assert_eq!(
        reply["result"]["serverInfo"].string("version"),
        Some("9.9.9")
    );
    assert!(!reply["result"]["capabilities"]["tools"].is_null());
    assert!(reply["result"]
        .string("instructions")
        .unwrap()
        .contains("read_screen"));

    let unknown = object(bridge.handle_line(r#"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#));
    assert_eq!(
        unknown["result"].string("protocolVersion"),
        Some("2025-06-18")
    );
    for version in ["2025-06-18", "2024-11-05"] {
        let line = format!(
            r#"{{"jsonrpc":"2.0","id":3,"method":"initialize","params":{{"protocolVersion":"{version}"}}}}"#
        );
        assert_eq!(
            object(bridge.handle_line(&line))["result"].string("protocolVersion"),
            Some(version)
        );
    }
}

#[test]
fn tools_list_matches_the_catalog() {
    let (_dir, socket) = socket_path();
    let bridge = McpBridgeSession::new(socket, None);
    let reply = object(bridge.handle_line(r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#));
    let tools = reply["result"].array("tools").unwrap();
    let names: Vec<&str> = tools.iter().filter_map(|t| t.string("name")).collect();
    let catalog: Vec<&str> = maine_agents::catalog::tools()
        .iter()
        .map(|t| t.name)
        .collect();
    assert_eq!(names, catalog);
    assert_eq!(
        names,
        [
            "list_hosts",
            "list_sessions",
            "open_session",
            "run",
            "read_output",
            "read_screen",
            "send_keys",
            "type_password",
            "close_session"
        ]
    );
    for tool in tools {
        assert_eq!(tool["inputSchema"].string("type"), Some("object"));
        assert!(!tool.string("description").unwrap_or_default().is_empty());
    }
}

#[test]
fn calls_without_a_server_return_the_not_running_error() {
    let (_dir, socket) = socket_path();
    let bridge = McpBridgeSession::new(socket, Some("maine-agent-X".into()));
    let reply = object(bridge.handle_line(r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"list_hosts","arguments":{}}}"#));
    assert_eq!(reply["result"]["isError"], true);
    assert_eq!(
        reply["result"].array("content").unwrap()[0].string("text"),
        Some("Maine is not running. Open Maine and unlock the vault.")
    );
}

#[test]
fn calls_are_forwarded_to_the_app() {
    let served = serve(Arc::new(ToolEcho));
    let (_, token) = served.clients.add("Bridge test").unwrap();

    let unpaired = McpBridgeSession::new(served.socket.clone(), Some("maine-agent-NOPE".into()));
    let refused = object(unpaired.handle_line(
        r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"list_hosts"}}"#,
    ));
    assert_eq!(
        refused["result"].array("content").unwrap()[0].string("text"),
        Some("This MCP client is not paired with Maine.")
    );
    let tokenless = McpBridgeSession::new(served.socket.clone(), None);
    let missing = object(tokenless.handle_line(
        r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"list_hosts"}}"#,
    ));
    assert_eq!(
        missing["result"].array("content").unwrap()[0].string("text"),
        Some("This MCP client is not paired with Maine.")
    );

    let bridge = McpBridgeSession::new(served.socket.clone(), Some(token));
    let reply = object(bridge.handle_line(r#"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"run","arguments":{"session_id":"x","command":"ls"}}}"#));
    assert_eq!(reply["result"]["isError"], false);
    assert_eq!(
        reply["result"]["structuredContent"].string("tool"),
        Some("run")
    );
    assert_eq!(
        reply["result"]["structuredContent"]["arguments"].string("command"),
        Some("ls")
    );
    let text = reply["result"].array("content").unwrap()[0]
        .string("text")
        .unwrap();
    let parsed: Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed["tool"], "run");
    assert_eq!(
        reply["result"].array("content").unwrap()[0].string("type"),
        Some("text")
    );

    let failed = object(bridge.handle_line(r#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"close_session","arguments":{}}}"#));
    assert_eq!(failed["result"]["isError"], true);
    assert_eq!(
        failed["result"].array("content").unwrap()[0].string("text"),
        Some("not found")
    );

    // Missing arguments are an empty object.
    let bare = object(bridge.handle_line(
        r#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"list_hosts"}}"#,
    ));
    assert_eq!(bare["result"]["structuredContent"]["arguments"], json!({}));
}

#[test]
fn unknown_methods_and_tools_are_json_rpc_errors() {
    let (_dir, socket) = socket_path();
    let bridge = McpBridgeSession::new(socket, None);
    let method =
        object(bridge.handle_line(r#"{"jsonrpc":"2.0","id":8,"method":"resources/list"}"#));
    assert_eq!(method["error"].int("code"), Some(-32601));
    let tool = object(bridge.handle_line(
        r#"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"rm_rf"}}"#,
    ));
    assert_eq!(tool["error"].int("code"), Some(-32602));
    assert_eq!(tool["id"], 9);
    let garbage = object(bridge.handle_line("not json"));
    assert_eq!(garbage["error"].int("code"), Some(-32700));
    assert_eq!(garbage["id"], Value::Null);
    let not_an_object = object(bridge.handle_line("[1,2]"));
    assert_eq!(not_an_object["error"].int("code"), Some(-32700));
    let ping = object(bridge.handle_line(r#"{"jsonrpc":"2.0","id":10,"method":"ping"}"#));
    assert_eq!(ping["result"], json!({}));
    let string_id = object(bridge.handle_line(r#"{"jsonrpc":"2.0","id":"abc","method":"ping"}"#));
    assert_eq!(string_id["id"], "abc");
}

#[test]
fn notifications_get_no_reply() {
    let (_dir, socket) = socket_path();
    let bridge = McpBridgeSession::new(socket, None);
    assert_eq!(
        bridge.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#),
        None
    );
    assert_eq!(
        bridge.handle_line(
            r#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}"#
        ),
        None
    );
    assert_eq!(
        bridge.handle_line(r#"{"jsonrpc":"2.0","id":null,"method":"ping"}"#),
        None
    );
    assert_eq!(bridge.handle_line(r#"{"jsonrpc":"2.0","id":1}"#), None);
}

// MARK: - Final review fixes

#[test]
fn a_call_lost_after_sending_is_never_retried() {
    let handler = Arc::new(DroppingHandler {
        calls: AtomicUsize::new(0),
        server: OnceLock::new(),
    });
    let served = serve(handler.clone());
    handler.server.set(served.server.clone()).ok();
    let (_, token) = served.clients.add("Retry test").unwrap();
    let bridge = McpBridgeSession::new(served.socket.clone(), Some(token));
    let reply = object(bridge.handle_line(r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"run","arguments":{"session_id":"x","command":"rm important"}}}"#));
    assert_eq!(reply["result"]["isError"], true);
    assert_eq!(
        reply["result"].array("content").unwrap()[0].string("text"),
        Some("Maine closed the connection before answering; the call may have run.")
    );
    assert_eq!(
        handler.calls.load(Ordering::SeqCst),
        1,
        "a run that may have executed must not be sent twice"
    );
}

#[test]
fn the_bridge_reconnects_after_the_app_restarts() {
    let (dir, socket) = socket_path();
    let clients = Arc::new(AgentClientStore::new(Arc::new(MemorySecretStore::new())));
    let first = AgentServer::new(socket.clone(), clients.clone(), Arc::new(ToolEcho));
    first.start().unwrap();
    let (_, token) = clients.add("Restart test").unwrap();
    let bridge = McpBridgeSession::new(socket.clone(), Some(token));
    let line = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_hosts","arguments":{}}}"#;
    assert_eq!(object(bridge.handle_line(line))["result"]["isError"], false);
    first.stop();
    std::thread::sleep(Duration::from_millis(100));
    let second = AgentServer::new(socket, clients, Arc::new(ToolEcho));
    second.start().unwrap();
    assert_eq!(object(bridge.handle_line(line))["result"]["isError"], false);
    drop(dir);
}

#[test]
fn a_call_while_the_app_is_down_is_not_running_then_works_once_it_is_up() {
    let (_dir, socket) = socket_path();
    let clients = Arc::new(AgentClientStore::new(Arc::new(MemorySecretStore::new())));
    let (_, token) = clients.add("Late").unwrap();
    let bridge = McpBridgeSession::new(socket.clone(), Some(token));
    let line = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_hosts","arguments":{}}}"#;
    assert_eq!(object(bridge.handle_line(line))["result"]["isError"], true);
    let server = AgentServer::new(socket, clients, Arc::new(ToolEcho));
    server.start().unwrap();
    assert_eq!(object(bridge.handle_line(line))["result"]["isError"], false);
}

/// A writer the test can read back after the bridge's threads are done.
#[derive(Clone, Default)]
struct Sink(Arc<Mutex<Vec<u8>>>);

impl Write for Sink {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[test]
fn serve_lines_answers_each_line_and_skips_notifications_and_blanks() {
    let served = serve(Arc::new(ToolEcho));
    let (_, token) = served.clients.add("Stdio").unwrap();
    let session = Arc::new(McpBridgeSession::new(served.socket.clone(), Some(token)));
    let input = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        "",
        "   ",
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"list_hosts"}}"#,
        "garbage",
    ]
    .join("\n");
    let sink = Sink::default();
    serve_lines(session, std::io::Cursor::new(input), sink.clone());
    let text = String::from_utf8(sink.0.lock().unwrap().clone()).unwrap();
    let mut replies: Vec<Value> = text
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(replies.len(), 4, "{text}");
    replies.sort_by_key(|r| r["id"].as_i64().unwrap_or(0));
    assert_eq!(replies[0]["error"].int("code"), Some(-32700)); // the garbage line, id null
    assert_eq!(replies[1]["id"], 1);
    assert_eq!(replies[2]["id"], 2);
    assert_eq!(replies[3]["result"]["isError"], false);
}
