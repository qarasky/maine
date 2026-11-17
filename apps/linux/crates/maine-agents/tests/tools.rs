//! The MCP tools end to end, minus the socket: a host list with rules and real
//! `/bin/sh` sessions standing in for ssh. Ports of the Swift `AgentToolsTests`.

mod common;

use common::*;
use maine_agents::json::{json_text, ValueExt};
use maine_agents::protocol::{AgentCall, AgentReply};
use maine_agents::{
    AgentAccess, AgentCallHandler, AgentClient, AgentToolHandler, AuditKind, MASK_REPLACEMENT,
};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

struct Fixture {
    host: Arc<TestHost>,
    handler: AgentToolHandler,
    alice: AgentClient,
    bob: AgentClient,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.host.tear_down();
    }
}

impl Fixture {
    fn new() -> Self {
        let host = TestHost::new();
        Self {
            handler: AgentToolHandler::new(host.clone()),
            host,
            alice: client("Alice agent"),
            bob: client("Bob agent"),
        }
    }

    fn call(&self, tool: &str, arguments: Value) -> AgentReply {
        self.call_as(tool, arguments, &self.alice)
    }

    fn call_as(&self, tool: &str, arguments: Value, client: &AgentClient) -> AgentReply {
        self.handler.handle(
            AgentCall {
                id: 1,
                tool: tool.into(),
                arguments,
            },
            client,
        )
    }

    fn open(&self, host: &maine_core::model::Host) -> String {
        self.open_as(host, &self.alice)
    }

    fn open_as(&self, host: &maine_core::model::Host, client: &AgentClient) -> String {
        let reply = self.call_as(
            "open_session",
            json!({"host_id": uuid_text(&host.id)}),
            client,
        );
        match reply.result.as_ref().and_then(|r| r.string("session_id")) {
            Some(id) => id.to_string(),
            None => panic!("open_session failed: {:?}", reply.error),
        }
    }
}

fn uuid_text(id: &Uuid) -> String {
    id.hyphenated().to_string().to_uppercase()
}

fn result(reply: &AgentReply) -> &Value {
    reply
        .result
        .as_ref()
        .unwrap_or_else(|| panic!("error: {:?}", reply.error))
}

#[test]
fn list_hosts_shows_only_allowed_hosts() {
    let f = Fixture::new();
    let reply = f.call("list_hosts", json!({}));
    let hosts = result(&reply).array("hosts").unwrap();
    let names: Vec<&str> = hosts.iter().filter_map(|h| h.string("name")).collect();
    assert_eq!(names, ["Web"]);
    let web = &hosts[0];
    assert_eq!(web.string("group"), Some("Prod"));
    assert_eq!(web.string("hostname"), Some("web.example"));
    assert_eq!(web.string("username"), Some("deploy"));
    assert_eq!(web.int("port"), Some(22));
    assert_eq!(web["has_saved_password"], true);
    let mut keys: Vec<&String> = web.as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "group",
            "has_saved_password",
            "hostname",
            "id",
            "name",
            "port",
            "username"
        ]
    );
    let text = json_text(result(&reply));
    assert!(!text.contains("secret"), "{text}");

    f.host.set_access(AgentAccess::new([], [f.host.loose.id]));
    let loose = f.call("list_hosts", json!({}));
    assert_eq!(
        result(&loose).array("hosts").unwrap()[0]["group"],
        Value::Null
    );
}

#[test]
fn list_hosts_follows_sidebar_order_and_hides_everything_else() {
    let f = Fixture::new();
    f.host
        .set_access(AgentAccess::new([f.host.prod.id], [f.host.loose.id]));
    let reply = f.call("list_hosts", json!({}));
    let names: Vec<&str> = result(&reply)
        .array("hosts")
        .unwrap()
        .iter()
        .filter_map(|h| h.string("name"))
        .collect();
    assert_eq!(names, ["Loose", "Web"]);
    assert!(!json_text(result(&reply)).contains("db.example"));
}

#[test]
fn disallowed_and_missing_ids_look_the_same() {
    let f = Fixture::new();
    let denied = f.call("open_session", json!({"host_id": uuid_text(&f.host.db.id)}));
    let missing = f.call(
        "open_session",
        json!({"host_id": uuid_text(&Uuid::new_v4())}),
    );
    assert_eq!(denied.error.as_deref(), Some("not found"));
    assert_eq!(denied, missing);
    assert_eq!(
        f.call(
            "run",
            json!({"session_id": uuid_text(&Uuid::new_v4()), "command": "ls"})
        )
        .error
        .as_deref(),
        Some("not found")
    );
    // Not a UUID at all: the same answer again.
    assert_eq!(
        f.call("open_session", json!({"host_id": "web"}))
            .error
            .as_deref(),
        Some("not found")
    );
    assert_eq!(
        f.call("open_session", json!({})).error.as_deref(),
        Some("not found")
    );
    assert_eq!(
        f.call("run", json!({"session_id": 5, "command": "ls"}))
            .error
            .as_deref(),
        Some("not found")
    );
}

#[test]
fn locked_vault_refuses_every_tool() {
    let f = Fixture::new();
    f.host.set_unlocked(false);
    for tool in maine_agents::catalog::tools()
        .iter()
        .map(|t| t.name)
        .chain(["nope"])
    {
        assert_eq!(
            f.call(tool, json!({})).error.as_deref(),
            Some("Maine is locked. Ask the user to unlock it."),
            "{tool}"
        );
    }
}

#[test]
fn sessions_are_scoped_to_their_client() {
    let f = Fixture::new();
    let id = f.open(&f.host.web);
    let mine = f.call("list_sessions", json!({}));
    let ids: Vec<&str> = result(&mine)
        .array("sessions")
        .unwrap()
        .iter()
        .filter_map(|s| s.string("session_id"))
        .collect();
    assert_eq!(ids, [id.as_str()]);
    let theirs = f.call_as("list_sessions", json!({}), &f.bob);
    assert_eq!(result(&theirs).array("sessions").unwrap().len(), 0);
    let stolen = f.call_as("run", json!({"session_id": id, "command": "true"}), &f.bob);
    assert_eq!(stolen.error.as_deref(), Some("not found"));
}

#[test]
fn list_sessions_describes_each_session() {
    let f = Fixture::new();
    let id = f.open(&f.host.web);
    let reply = f.call("list_sessions", json!({}));
    let session = &result(&reply).array("sessions").unwrap()[0];
    assert_eq!(session.string("session_id"), Some(id.as_str()));
    assert_eq!(
        session.string("host_id"),
        Some(uuid_text(&f.host.web.id).as_str())
    );
    assert_eq!(session.string("state"), Some("Connected"));
    assert_eq!(session["busy"], false);
    assert!(session.string("name").unwrap().starts_with("Session "));
}

#[test]
fn run_over_the_tool_surface() {
    let f = Fixture::new();
    let id = f.open(&f.host.web);
    let reply = f.call(
        "run",
        json!({"session_id": id, "command": "echo hi; false"}),
    );
    let run = result(&reply);
    assert_eq!(run["completed"], true);
    assert_eq!(run.int("exit_code"), Some(1));
    assert_eq!(run.string("output").unwrap().trim(), "hi");
    assert_eq!(run.string("mode"), Some("marker"));
    let mut keys: Vec<&String> = run.as_object().unwrap().keys().collect();
    keys.sort();
    assert_eq!(
        keys,
        [
            "completed",
            "cursor",
            "exit_code",
            "mode",
            "output",
            "truncated"
        ]
    );

    // Let the prompt after the marker land before looking at the screen.
    std::thread::sleep(Duration::from_millis(400));
    let screen = f.call("read_screen", json!({"session_id": id}));
    assert!(result(&screen)
        .array("rows")
        .unwrap()
        .iter()
        .any(|row| row.as_str().is_some_and(|r| r.contains("hi"))));
    assert_eq!(result(&screen)["changed"], true);
    assert_eq!(result(&screen)["busy"], false);
    let unchanged = f.call("read_screen", json!({"session_id": id, "wait_seconds": 1}));
    assert_eq!(result(&unchanged)["changed"], false);
    let read = f.call("read_output", json!({"session_id": id}));
    assert!(result(&read).string("output").unwrap().contains("hi"));
    assert_eq!(
        result(&f.call(
            "send_keys",
            json!({"session_id": id, "keys": ["text:echo typed", "enter"]})
        ))
        .int("sent"),
        Some(2)
    );
    assert_eq!(
        f.call("send_keys", json!({"session_id": id, "keys": ["bogus"]}))
            .error
            .as_deref(),
        Some("unknown key: bogus")
    );
}

#[test]
fn read_output_follows_cursors_and_limits() {
    let f = Fixture::new();
    let id = f.open(&f.host.web);
    let first = f.call("run", json!({"session_id": id, "command": "echo one"}));
    let cursor = result(&first).int("cursor").unwrap();
    f.call("run", json!({"session_id": id, "command": "echo two"}));
    let after = f.call("read_output", json!({"session_id": id, "since": cursor}));
    let text = result(&after).string("output").unwrap().to_string();
    assert!(text.contains("two") && !text.contains("one\n"), "{text:?}");
    let tiny = f.call("read_output", json!({"session_id": id, "max_bytes": 1}));
    assert_eq!(result(&tiny)["truncated"], true);
    let waited = Instant::now();
    let nothing = f.call(
        "read_output",
        json!({"session_id": id, "since": result(&after).int("cursor"), "wait_seconds": 1}),
    );
    assert!(
        waited.elapsed() >= Duration::from_millis(900),
        "a wait with nothing new takes its time"
    );
    assert_eq!(result(&nothing).string("output"), Some(""));
}

#[test]
fn argument_problems_are_reported_plainly() {
    let f = Fixture::new();
    let id = f.open(&f.host.web);
    assert_eq!(
        f.call("run", json!({"session_id": id})).error.as_deref(),
        Some("command is required")
    );
    assert_eq!(
        f.call("send_keys", json!({"session_id": id, "keys": []}))
            .error
            .as_deref(),
        Some("keys is required")
    );
    assert_eq!(
        f.call("send_keys", json!({"session_id": id}))
            .error
            .as_deref(),
        Some("keys is required")
    );
    assert_eq!(
        f.call("frobnicate", json!({})).error.as_deref(),
        Some("unknown tool: frobnicate")
    );
}

#[test]
fn run_on_a_dead_session_fails_fast() {
    let f = Fixture::new();
    let id = f.open(&f.host.web);
    f.call(
        "send_keys",
        json!({"session_id": id, "keys": ["text:exit", "enter"]}),
    );
    let session = f.host.ids()[0];
    assert!(wait_until(Duration::from_secs(5), || !f
        .host
        .terminal(session)
        .term()
        .is_running()));
    let started = Instant::now();
    let reply = f.call("run", json!({"session_id": id, "command": "true"}));
    assert_eq!(reply.error.as_deref(), Some("session is not connected"));
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn huge_output_is_truncated() {
    let f = Fixture::new();
    let id = f.open(&f.host.web);
    let reply = f.call(
        "run",
        json!({"session_id": id, "command": "yes | head -n 200000", "timeout_seconds": 60}),
    );
    let run = result(&reply);
    assert_eq!(run["truncated"], true);
    assert!(run.string("output").unwrap().len() <= 65_536);
    assert_eq!(run.int("exit_code"), Some(0));
}

#[test]
fn concurrent_runs_follow_the_busy_rule() {
    let f = Fixture::new();
    let first = f.open(&f.host.web);
    let second = f.open(&f.host.web);
    let started = Instant::now();
    std::thread::scope(|scope| {
        let a = scope.spawn(|| {
            f.call(
                "run",
                json!({"session_id": first, "command": "sleep 1; echo A"}),
            )
        });
        std::thread::sleep(Duration::from_millis(100));
        let busy = f.call("run", json!({"session_id": first, "command": "true"}));
        assert_eq!(
            busy.error.as_deref(),
            Some("a command is still running; use read_output, or send_keys ctrl-c")
        );
        let b = scope.spawn(|| {
            f.call(
                "run",
                json!({"session_id": second, "command": "sleep 1; echo B"}),
            )
        });
        let (reply_a, reply_b) = (a.join().unwrap(), b.join().unwrap());
        assert!(result(&reply_a).string("output").unwrap().contains('A'));
        assert!(result(&reply_b).string("output").unwrap().contains('B'));
    });
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "{:?}",
        started.elapsed()
    );
}

#[test]
fn revoking_access_mid_run_ends_the_call() {
    let f = Fixture::new();
    let id = f.open(&f.host.web);
    std::thread::scope(|scope| {
        let run = scope.spawn(|| {
            f.call(
                "run",
                json!({"session_id": id, "command": "sleep 5", "timeout_seconds": 30}),
            )
        });
        std::thread::sleep(Duration::from_millis(300));
        f.host.set_access(AgentAccess::default());
        let started = Instant::now();
        assert_eq!(run.join().unwrap().error.as_deref(), Some("not found"));
        assert!(started.elapsed() < Duration::from_secs(2));
    });
    let session = f.host.ids()[0];
    assert!(f.host.driver(session).is_none());
    assert_eq!(
        f.host
            .with_record(session, |r| r.former_agent.clone())
            .as_deref(),
        Some("Alice agent")
    );
}

#[test]
fn moving_a_host_out_of_an_allowed_group_takes_effect_on_the_next_call() {
    let f = Fixture::new();
    let id = f.open(&f.host.web);
    // The host now sits in a group the rules do not cover.
    let mut web = f.host.web.clone();
    web.group_id = Some(f.host.lab.id);
    *lock(&f.host.layout) = maine_core::model::HostLayout::new(
        vec![f.host.prod.clone(), f.host.lab.clone()],
        vec![f.host.loose.clone(), web, f.host.db.clone()],
    );
    assert_eq!(
        f.call("run", json!({"session_id": id, "command": "true"}))
            .error
            .as_deref(),
        Some("not found")
    );
    assert!(
        f.host.driver(f.host.ids()[0]).is_none(),
        "agent control ended, the session stays open"
    );
    assert_eq!(f.host.ids().len(), 1);
}

#[test]
fn close_session_closes_it() {
    let f = Fixture::new();
    let id = f.open(&f.host.web);
    f.call(
        "send_keys",
        json!({"session_id": id, "keys": ["text:exit", "enter"]}),
    );
    assert_eq!(
        result(&f.call("close_session", json!({"session_id": id})))["closed"],
        true
    );
    assert!(f.host.ids().is_empty());
}

#[test]
fn argument_limits_are_clamped() {
    assert_eq!(AgentToolHandler::run_timeout(None), Duration::from_secs(60));
    assert_eq!(
        AgentToolHandler::run_timeout(Some(900)),
        Duration::from_secs(600)
    );
    assert_eq!(
        AgentToolHandler::run_timeout(Some(0)),
        Duration::from_secs(1)
    );
    assert_eq!(
        AgentToolHandler::run_timeout(Some(-5)),
        Duration::from_secs(1)
    );
    assert_eq!(
        AgentToolHandler::read_wait(Some(120)),
        Some(Duration::from_secs(60))
    );
    assert_eq!(AgentToolHandler::read_wait(Some(-1)), Some(Duration::ZERO));
    assert_eq!(AgentToolHandler::read_wait(None), None);
    assert_eq!(AgentToolHandler::max_bytes(Some(1_000_000)), 65_536);
    assert_eq!(AgentToolHandler::max_bytes(Some(0)), 1);
    assert_eq!(AgentToolHandler::max_bytes(None), 65_536);
}

// MARK: - Final review fixes

#[test]
fn paused_sessions_refuse_every_call() {
    let f = Fixture::new();
    let id = f.open(&f.host.web);
    f.host.driver(f.host.ids()[0]).unwrap().set_paused(true);
    for tool in [
        "read_output",
        "read_screen",
        "send_keys",
        "run",
        "type_password",
        "close_session",
    ] {
        let reply = f.call(
            tool,
            json!({"session_id": id, "command": "true", "keys": ["enter"]}),
        );
        assert_eq!(reply.error.as_deref(), Some("paused by the user"), "{tool}");
    }
    assert_eq!(f.host.ids().len(), 1);
}

#[test]
fn a_failed_open_leaves_no_agent_attached() {
    let f = Fixture::new();
    f.host.set_program("/nonexistent/ssh", &[]);
    let reply = f.call(
        "open_session",
        json!({"host_id": uuid_text(&f.host.web.id)}),
    );
    assert!(
        reply
            .error
            .as_deref()
            .is_some_and(|e| e.contains("/nonexistent/ssh")),
        "{:?}",
        reply.error
    );
    assert!(f.host.ids().is_empty());
    assert!(f.host.audit_entries().is_empty());
}

#[test]
fn a_session_that_dies_while_opening_reports_why_and_keeps_no_agent() {
    let f = Fixture::new();
    f.host
        .set_program("/bin/sh", &["-c", "echo 'ssh: no route to host'; exit 3"]);
    let reply = f.call(
        "open_session",
        json!({"host_id": uuid_text(&f.host.web.id)}),
    );
    assert!(
        reply
            .error
            .as_deref()
            .is_some_and(|e| e.contains("exit code 3")),
        "{:?}",
        reply.error
    );
    let session = f.host.ids()[0];
    assert!(f.host.driver(session).is_none());
    assert_eq!(
        f.host
            .with_record(session, |r| r.former_agent.clone())
            .as_deref(),
        Some("Alice agent")
    );
    assert!(f.host.audit_entries().is_empty(), "nothing was opened");
}

#[test]
fn a_session_stuck_at_a_prompt_opens_not_ready_with_a_hint() {
    // The prompt comes a moment after start, like ssh's. `TermSession::spawn`
    // reads at once, so output in the first instants can precede the driver's tap.
    let f = Fixture::new();
    f.host.set_program(
        "/bin/sh",
        &["-c", "sleep 0.4; printf 'deploy@web password: '; sleep 30"],
    );
    let reply = f.call(
        "open_session",
        json!({"host_id": uuid_text(&f.host.web.id)}),
    );
    let opened = result(&reply);
    assert_eq!(opened["ready"], false);
    assert_eq!(opened.string("state"), Some("Connecting…"));
    assert!(opened.string("hint").unwrap().contains("type_password"));
    assert!(opened.string("output").unwrap().contains("password:"));
    let id = opened.string("session_id").unwrap().to_string();
    let listed = f.call("list_sessions", json!({}));
    assert_eq!(
        result(&listed).array("sessions").unwrap()[0].string("state"),
        Some("Connecting…")
    );
    let run = f.call("run", json!({"session_id": id, "command": "true"}));
    assert!(
        run.error
            .as_deref()
            .is_some_and(|e| e.starts_with("ssh is still connecting")),
        "{:?}",
        run.error
    );
    // The prompt can still be answered.
    let screen = f.call("read_screen", json!({"session_id": id}));
    assert!(result(&screen)
        .array("rows")
        .unwrap()
        .iter()
        .any(|r| r.as_str().is_some_and(|s| s.contains("password:"))));
    assert_eq!(
        result(&f.call("type_password", json!({"session_id": id})))["typed"],
        true
    );
}

#[test]
fn the_diagnostics_never_record_agent_supplied_text() {
    let f = Fixture::new();
    f.call(
        "run --secret-flag",
        json!({"session_id": "prod-db-password-hunter2"}),
    );
    f.call(
        "run",
        json!({"session_id": "rm -rf /home/alice", "command": "cat /etc/shadow"}),
    );
    let id = f.open(&f.host.web);
    f.call(
        "run",
        json!({"session_id": id, "command": "echo hunter2 > /root/shadow"}),
    );
    let text = lock(&f.host.diagnostics).join("\n");
    assert!(text.contains("agent.tool"));
    assert!(
        text.contains("tool=unknown"),
        "an unknown tool name is not echoed"
    );
    for secret in [
        "secret-flag",
        "hunter2",
        "rm -rf",
        "shadow",
        "alice",
        "web.example",
        "secret-Web",
    ] {
        assert!(!text.contains(secret), "{secret} in {text}");
    }
    // Only ids, outcomes, durations and sizes.
    for line in text.lines() {
        assert!(line.starts_with("agent.tool tool="), "{line}");
        assert!(
            line.contains(" client=")
                && line.contains(" outcome=")
                && line.contains(" ms=")
                && line.contains(" bytes="),
            "{line}"
        );
    }
}

#[test]
fn type_password_answers_a_prompt_without_returning_the_secret() {
    let f = Fixture::new();
    let id = f.open(&f.host.web);
    f.call("run", json!({"session_id": id, "command": "printf 'Password: '; read -r p; echo \"got:$p\"", "timeout_seconds": 1}));
    let reply = f.call("type_password", json!({"session_id": id}));
    assert!(reply.error.is_none(), "{:?}", reply.error);
    assert_eq!(result(&reply)["typed"], true);
    assert!(!json_text(result(&reply)).contains("secret-Web"));
    let wanted = format!("got:{MASK_REPLACEMENT}");
    let mut text = String::new();
    let deadline = Instant::now() + Duration::from_secs(8);
    while !text.contains(&wanted) && Instant::now() < deadline {
        let read = f.call("read_output", json!({"session_id": id, "wait_seconds": 1}));
        text = result(&read)
            .string("output")
            .unwrap_or_default()
            .to_string();
    }
    assert!(text.contains(&wanted), "{text:?}");
    assert!(!text.contains("secret-Web"));
    let screen = f.call("read_screen", json!({"session_id": id}));
    assert!(!json_text(result(&screen)).contains("secret-Web"));
}

#[test]
fn type_password_needs_a_prompt_a_saved_password_and_an_owned_session() {
    let f = Fixture::new();
    let id = f.open(&f.host.web);
    let none = f.call("type_password", json!({"session_id": id}));
    assert!(
        none.error
            .as_deref()
            .is_some_and(|e| e.contains("no password prompt")),
        "{:?}",
        none.error
    );
    assert!(!none.error.as_deref().unwrap_or("").contains("secret-Web"));
    assert_eq!(
        f.call_as("type_password", json!({"session_id": id}), &f.bob)
            .error
            .as_deref(),
        Some("not found")
    );
    assert_eq!(
        f.call(
            "type_password",
            json!({"session_id": uuid_text(&Uuid::new_v4())})
        )
        .error
        .as_deref(),
        Some("not found")
    );
    lock(&f.host.passwords).remove(&f.host.web.id);
    assert_eq!(
        f.call("type_password", json!({"session_id": id}))
            .error
            .as_deref(),
        Some("no saved password for this host")
    );
    let hosts = f.call("list_hosts", json!({}));
    assert_eq!(
        result(&hosts).array("hosts").unwrap()[0]["has_saved_password"],
        false
    );
}

#[test]
fn the_audit_records_what_agents_did_without_secrets() {
    let f = Fixture::new();
    let id = f.open(&f.host.web);
    f.call(
        "run",
        json!({"session_id": id, "command": "echo hi; false"}),
    );
    f.call("run", json!({"session_id": id, "command": "printf 'Password: '; read -r p", "timeout_seconds": 1}));
    f.call("type_password", json!({"session_id": id}));
    f.call(
        "send_keys",
        json!({"session_id": id, "keys": ["text:topsecret-literal", "enter", "ctrl-c"]}),
    );
    f.call("close_session", json!({"session_id": id}));
    f.call(
        "run",
        json!({"session_id": uuid_text(&Uuid::new_v4()), "command": "ignored"}),
    );

    let entries = f.host.audit_entries();
    let kinds: Vec<AuditKind> = entries.iter().map(|e| e.kind).collect();
    assert_eq!(
        kinds,
        [
            AuditKind::Open,
            AuditKind::Run,
            AuditKind::Run,
            AuditKind::Password,
            AuditKind::Keys,
            AuditKind::Close
        ]
    );
    assert!(entries
        .iter()
        .all(|e| e.client == "Alice agent" && e.host_name == "Web"));
    assert_eq!(
        (entries[1].detail.as_deref(), entries[1].exit_code),
        (Some("echo hi; false"), Some(1))
    );
    assert_eq!(entries[3].detail, None);
    assert_eq!(
        entries[4].detail.as_deref(),
        Some("text(17 chars), enter, ctrl-c")
    );
    let everything: String = entries
        .iter()
        .map(|e| e.detail.clone().unwrap_or_default())
        .collect();
    assert!(!everything.contains("secret-Web") && !everything.contains("topsecret-literal"));
}
