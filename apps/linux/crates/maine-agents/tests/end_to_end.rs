//! The whole path with no ssh and no UI: an MCP client's JSON-RPC lines go
//! through the real bridge code, over a real Unix socket, to the real server
//! and tool handler, which drive a real `maine_term` PTY running `/bin/sh` (or
//! bash for OSC 133). The only fake is the app's `AgentHost`.
//!
//! Not testable here: the `SO_PEERCRED` rejection of another user's process
//! (a test cannot become another uid). The check is a same-uid comparison made
//! on every accepted connection before any byte is read.

mod common;

use common::*;
use maine_agents::json::{json_text, ValueExt};
use maine_agents::{
    AgentAccess, AgentClientStore, AgentCoordinator, AgentServer, AgentToolHandler,
    McpBridgeSession, MemorySecretStore, MASK_REPLACEMENT,
};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

struct Stack {
    host: Arc<TestHost>,
    clients: Arc<AgentClientStore>,
    server: AgentServer,
    socket: PathBuf,
    _dir: tempfile::TempDir,
}

impl Drop for Stack {
    fn drop(&mut self) {
        self.server.stop();
        self.host.tear_down();
    }
}

impl Stack {
    fn new() -> Self {
        let host = TestHost::new();
        let dir = short_socket_dir();
        let socket = dir.path().join("agent.sock");
        let clients = Arc::new(AgentClientStore::new(Arc::new(MemorySecretStore::new())));
        let server = AgentServer::new(
            socket.clone(),
            clients.clone(),
            Arc::new(AgentToolHandler::new(host.clone())),
        );
        server.start().unwrap();
        Self {
            host,
            clients,
            server,
            socket,
            _dir: dir,
        }
    }

    /// A bridge for a freshly paired client, as `maine --mcp` would start.
    fn bridge(&self, name: &str) -> (Bridge, uuid::Uuid, String) {
        let (client, token) = self.clients.add(name).unwrap();
        (
            Bridge {
                session: McpBridgeSession::new(self.socket.clone(), Some(token.clone())),
                next: 0.into(),
            },
            client.id,
            token,
        )
    }
}

struct Bridge {
    session: McpBridgeSession,
    next: std::sync::atomic::AtomicI64,
}

impl Bridge {
    fn rpc(&self, method: &str, params: Value) -> Value {
        let id = self.next.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let line =
            json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}).to_string();
        let reply = self.session.handle_line(&line).expect("a reply");
        let reply: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(reply["id"], id);
        reply
    }

    /// `tools/call`: the structured result, or the error text.
    fn tool(&self, name: &str, arguments: Value) -> Result<Value, String> {
        let reply = self.rpc("tools/call", json!({"name": name, "arguments": arguments}));
        let result = &reply["result"];
        let text = result.array("content").unwrap()[0]
            .string("text")
            .unwrap()
            .to_string();
        if result["isError"] == true {
            Err(text)
        } else {
            let parsed: Value = serde_json::from_str(&text).unwrap();
            assert_eq!(
                parsed, result["structuredContent"],
                "text and structuredContent agree"
            );
            Ok(parsed)
        }
    }

    fn open(&self, host: &maine_core::model::Host) -> String {
        let opened = self
            .tool(
                "open_session",
                json!({"host_id": host.id.hyphenated().to_string().to_uppercase()}),
            )
            .unwrap();
        assert_eq!(opened["ready"], true, "{opened}");
        opened.string("session_id").unwrap().to_string()
    }
}

#[test]
fn a_client_lists_hosts_opens_a_session_and_runs_a_command() {
    let stack = Stack::new();
    let (bridge, _, _) = stack.bridge("Claude Code");
    assert_eq!(
        bridge.rpc("initialize", json!({"protocolVersion": "2025-06-18"}))["result"]["serverInfo"]
            ["name"],
        "maine"
    );

    let hosts = bridge.tool("list_hosts", json!({})).unwrap();
    let names: Vec<&str> = hosts
        .array("hosts")
        .unwrap()
        .iter()
        .filter_map(|h| h.string("name"))
        .collect();
    assert_eq!(names, ["Web"]);

    let session = bridge.open(&stack.host.web);
    let run = bridge
        .tool(
            "run",
            json!({"session_id": session, "command": "echo hi; false"}),
        )
        .unwrap();
    assert_eq!(run["completed"], true);
    assert_eq!(run.int("exit_code"), Some(1));
    assert_eq!(run.string("output").unwrap().trim(), "hi");
    assert_eq!(run.string("mode"), Some("marker"));

    let again = bridge
        .tool(
            "run",
            json!({"session_id": session, "command": "cd /tmp && pwd"}),
        )
        .unwrap();
    assert_eq!(again.string("output").unwrap().trim(), "/tmp");
    let state = bridge
        .tool("run", json!({"session_id": session, "command": "pwd"}))
        .unwrap();
    assert_eq!(
        state.string("output").unwrap().trim(),
        "/tmp",
        "the shell persists between calls: {state}"
    );

    let listed = bridge.tool("list_sessions", json!({})).unwrap();
    assert_eq!(listed.array("sessions").unwrap().len(), 1);
    assert_eq!(
        bridge
            .tool("close_session", json!({"session_id": session}))
            .unwrap()["closed"],
        true
    );
    assert_eq!(
        bridge
            .tool("list_sessions", json!({}))
            .unwrap()
            .array("sessions")
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn read_screen_and_incremental_read_output() {
    let stack = Stack::new();
    // bash, like macOS' /bin/sh: its line editor prints PS2 inside the echo.
    // dash prints `> ` in front of the output, which is then on the screen too.
    stack
        .host
        .set_program("/usr/bin/bash", &["--norc", "--noprofile", "-i"]);
    let (bridge, _, _) = stack.bridge("Agent");
    let session = bridge.open(&stack.host.web);
    let run = bridge
        .tool(
            "run",
            json!({"session_id": session, "command": "printf 'row-a\\nrow-b\\n'"}),
        )
        .unwrap();
    std::thread::sleep(Duration::from_millis(400));
    let screen = bridge
        .tool("read_screen", json!({"session_id": session}))
        .unwrap();
    let rows: Vec<&str> = screen
        .array("rows")
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        rows.iter().any(|r| r.trim() == "row-a") && rows.iter().any(|r| r.trim() == "row-b"),
        "{rows:?}"
    );
    assert_eq!(screen["alternate_screen"], false);
    assert_eq!(screen["changed"], true);
    assert!(screen.int("cursor_row").is_some() && screen.int("cursor_col").is_some());

    // Slow output, read incrementally.
    let slow = bridge
        .tool("run", json!({"session_id": session, "command": "echo one; sleep 2; echo two; sleep 4", "timeout_seconds": 1}))
        .unwrap();
    assert_eq!(slow["completed"], false);
    assert!(slow.string("output").unwrap().contains("one"));
    let next = bridge
        .tool(
            "read_output",
            json!({"session_id": session, "since": slow["cursor"], "wait_seconds": 10}),
        )
        .unwrap();
    assert!(next.string("output").unwrap().contains("two"), "{next}");
    assert_eq!(next["busy"], true);
    assert!(next.int("cursor").unwrap() > slow.int("cursor").unwrap());
    let _ = run;
}

#[test]
fn type_password_keeps_the_secret_out_of_every_result() {
    let stack = Stack::new();
    let (bridge, _, _) = stack.bridge("Agent");
    let session = bridge.open(&stack.host.web);
    let mut transcript = vec![];
    let run = bridge
        .tool(
            "run",
            json!({"session_id": session, "command": "printf '[sudo] password for deploy: '; read -r p; echo \"got:$p\"", "timeout_seconds": 1}),
        )
        .unwrap();
    transcript.push(json_text(&run));
    assert_eq!(run["completed"], false);

    let typed = bridge
        .tool("type_password", json!({"session_id": session}))
        .unwrap();
    transcript.push(json_text(&typed));
    assert_eq!(typed, json!({"typed": true}));

    let wanted = format!("got:{MASK_REPLACEMENT}");
    let deadline = Instant::now() + Duration::from_secs(8);
    loop {
        let read = bridge
            .tool(
                "read_output",
                json!({"session_id": session, "wait_seconds": 1}),
            )
            .unwrap();
        transcript.push(json_text(&read));
        if read.string("output").unwrap().contains(&wanted) {
            break;
        }
        assert!(Instant::now() < deadline, "the mask never appeared: {read}");
    }
    transcript.push(json_text(
        &bridge
            .tool("read_screen", json!({"session_id": session}))
            .unwrap(),
    ));
    let all = transcript.join("\n");
    assert!(
        !all.contains("secret-Web"),
        "the saved password reached a tool result"
    );
    assert!(!stack.host.audit_entries().iter().any(|e| e
        .detail
        .as_deref()
        .is_some_and(|d| d.contains("secret-Web"))));
    // Not even the raw wire text of the whole session carries it.
    let lines = lock(&stack.host.diagnostics).join("\n");
    assert!(!lines.contains("secret-Web"));
}

#[test]
fn a_prompt_must_be_showing_before_a_password_is_typed() {
    let stack = Stack::new();
    let (bridge, _, _) = stack.bridge("Agent");
    let session = bridge.open(&stack.host.web);
    let error = bridge
        .tool("type_password", json!({"session_id": session}))
        .unwrap_err();
    assert_eq!(
        error,
        "no password prompt is showing; the password was not typed"
    );
}

#[test]
fn a_host_that_is_not_allowed_does_not_exist_for_the_client() {
    let stack = Stack::new();
    let (bridge, _, _) = stack.bridge("Agent");
    let db = stack.host.db.id.hyphenated().to_string().to_uppercase();
    let denied = bridge
        .tool("open_session", json!({"host_id": db}))
        .unwrap_err();
    let missing = bridge
        .tool(
            "open_session",
            json!({"host_id": uuid::Uuid::new_v4().to_string()}),
        )
        .unwrap_err();
    assert_eq!(
        (denied.as_str(), missing.as_str()),
        ("not found", "not found")
    );
    let hosts = json_text(&bridge.tool("list_hosts", json!({})).unwrap());
    // (Ids are hex, so a bare "DB" can occur inside one: look for names.)
    assert!(
        !hosts.contains("db.example") && !hosts.contains("\"DB\"") && !hosts.contains("Lab"),
        "{hosts}"
    );
    assert!(stack.host.ids().is_empty(), "nothing was opened");
}

#[test]
fn a_session_belongs_to_the_client_that_opened_it() {
    let stack = Stack::new();
    let (alice, _, _) = stack.bridge("Alice");
    let (bob, _, _) = stack.bridge("Bob");
    let session = alice.open(&stack.host.web);
    assert_eq!(
        bob.tool("run", json!({"session_id": session, "command": "true"}))
            .unwrap_err(),
        "not found"
    );
    assert_eq!(
        bob.tool("list_sessions", json!({}))
            .unwrap()
            .array("sessions")
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn a_revoked_token_is_rejected_and_its_connection_is_cut() {
    let stack = Stack::new();
    let (bridge, client_id, token) = stack.bridge("Soon revoked");
    let session = bridge.open(&stack.host.web);
    assert!(bridge.tool("list_hosts", json!({})).is_ok());

    stack.clients.revoke(client_id).unwrap();
    stack.server.disconnect(client_id);
    // The bridge notices the cut at its next call and tries to pair again;
    // the token no longer opens anything.
    let mut answer = String::new();
    assert!(
        wait_until(Duration::from_secs(5), || {
            answer = bridge.tool("list_hosts", json!({})).unwrap_err();
            answer == "This MCP client is not paired with Maine."
        }),
        "last answer: {answer}"
    );
    assert_eq!(stack.clients.client_for_token(&token).unwrap(), None);
    assert!(stack.host.ids().contains(&stack.host.ids()[0]));
    let _ = session;
}

#[test]
fn a_token_that_was_never_paired_is_rejected() {
    let stack = Stack::new();
    let bridge = Bridge {
        session: McpBridgeSession::new(stack.socket.clone(), Some("maine-agent-NEVER".into())),
        next: 0.into(),
    };
    assert_eq!(
        bridge.tool("list_hosts", json!({})).unwrap_err(),
        "This MCP client is not paired with Maine."
    );
    // Tools/list needs no pairing; it is the bridge's own catalog.
    assert!(bridge.rpc("tools/list", json!({}))["result"]
        .array("tools")
        .is_some());
}

#[test]
fn paused_sessions_refuse_and_resume() {
    let stack = Stack::new();
    let (bridge, _, _) = stack.bridge("Agent");
    let session = bridge.open(&stack.host.web);
    let driver = stack.host.driver(stack.host.ids()[0]).unwrap();
    driver.set_paused(true);
    for tool in [
        "run",
        "read_output",
        "read_screen",
        "send_keys",
        "type_password",
        "close_session",
    ] {
        let error = bridge
            .tool(
                tool,
                json!({"session_id": session, "command": "true", "keys": ["enter"]}),
            )
            .unwrap_err();
        assert_eq!(error, "paused by the user", "{tool}");
    }
    driver.set_paused(false);
    assert!(bridge
        .tool("run", json!({"session_id": session, "command": "true"}))
        .is_ok());
}

#[test]
fn a_locked_vault_refuses_over_the_wire() {
    let stack = Stack::new();
    let (bridge, _, _) = stack.bridge("Agent");
    stack.host.set_unlocked(false);
    assert_eq!(
        bridge.tool("list_hosts", json!({})).unwrap_err(),
        "Maine is locked. Ask the user to unlock it."
    );
    stack.host.set_unlocked(true);
    assert!(bridge.tool("list_hosts", json!({})).is_ok());
}

#[test]
fn osc133_shells_run_through_the_wire_without_the_wrapper() {
    let stack = Stack::new();
    stack
        .host
        .set_program("/usr/bin/bash", &["--norc", "--noprofile", "-i"]);
    // A shell that already emits shell integration; its first prompt waits a
    // moment so the driver's output tap is surely installed.
    lock(&stack.host.extra_env).push((
        "PROMPT_COMMAND".into(),
        "s=$?; [ -n \"$started\" ] || { sleep 0.3; started=1; }; printf '\\033]133;D;%d\\007\\033]133;A\\007' $s".into(),
    ));
    let (bridge, _, _) = stack.bridge("Agent");
    let session = bridge.open(&stack.host.web);
    let run = bridge
        .tool(
            "run",
            json!({"session_id": session, "command": "echo osc; (exit 7)"}),
        )
        .unwrap();
    assert_eq!(run.string("mode"), Some("osc133"));
    assert_eq!(run["completed"], true);
    assert_eq!(run.int("exit_code"), Some(7));
    assert_eq!(run.string("output").unwrap().trim(), "osc");
    let everything = bridge
        .tool("read_output", json!({"session_id": session}))
        .unwrap();
    assert!(
        !everything.string("output").unwrap().contains("7770"),
        "no wrapper noise on a shell with integration"
    );
}

#[test]
fn osc133_marks_printed_by_a_command_do_not_disturb_marker_mode() {
    // A shell snippet that prints the sequences by hand around a command.
    let stack = Stack::new();
    stack.host.set_program("/bin/sh", &[]);
    let (bridge, _, _) = stack.bridge("Agent");
    let session = bridge.open(&stack.host.web);
    // `A` makes the driver treat the shell as integrated for later runs.
    bridge
        .tool(
            "send_keys",
            json!({"session_id": session, "keys": ["text:printf '\\033]133;A\\007'", "enter"]}),
        )
        .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    let run = bridge
        .tool(
            "run",
            json!({"session_id": session, "command": "printf '\\033]133;C\\007'; echo snippet; printf '\\033]133;D;5\\007'",
                   "timeout_seconds": 10}),
        )
        .unwrap();
    // The session already chose its mode (marker) at open; the marks inside the
    // command's own output must not end or corrupt a marker-mode run.
    assert_eq!(run.string("mode"), Some("marker"));
    assert_eq!(run["completed"], true);
    assert_eq!(run.int("exit_code"), Some(0));
    assert!(run.string("output").unwrap().contains("snippet"));
}

#[test]
fn the_coordinator_drives_the_same_stack() {
    let host = TestHost::new();
    let dir = short_socket_dir();
    let socket = dir.path().join("agent.sock");
    let secrets = Arc::new(MemorySecretStore::new());
    let coordinator = AgentCoordinator::new(
        host.clone(),
        Arc::new(AgentClientStore::new(secrets)),
        socket.clone(),
        system_clock(),
    );
    coordinator.set_enabled(true);
    let pairing = coordinator
        .add_client("Claude Code", "/usr/bin/maine")
        .unwrap();
    let bridge = Bridge {
        session: McpBridgeSession::new(socket, Some(pairing.token)),
        next: 0.into(),
    };
    assert_eq!(
        bridge
            .tool("list_hosts", json!({}))
            .unwrap()
            .array("hosts")
            .unwrap()
            .len(),
        1
    );
    let session = bridge.open(&host.web);
    assert!(bridge
        .tool("run", json!({"session_id": session, "command": "echo up"}))
        .is_ok());

    // Revoke at the coordinator: control ends, the session stays, the token dies.
    coordinator.revoke(pairing.client.id);
    assert!(host.driver(host.ids()[0]).is_none());
    assert_eq!(host.ids().len(), 1);
    let mut answer = String::new();
    assert!(
        wait_until(Duration::from_secs(5), || {
            answer = bridge.tool("list_hosts", json!({})).unwrap_err();
            answer == "This MCP client is not paired with Maine."
        }),
        "last answer: {answer}"
    );

    // Stopping the master switch ends everything.
    coordinator.set_enabled(false);
    host.tear_down();
    host.set_access(AgentAccess::default());
}
