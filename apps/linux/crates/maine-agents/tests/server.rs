//! The app side of the agent socket. Ports of the Swift `AgentServerTests`.
//! The `SO_PEERCRED` check rejects connections from another uid; a test cannot
//! become another user, so that branch is covered by reading the code (and by
//! the accepted same-uid connections here).

mod common;

use common::*;
use maine_agents::json::ValueExt;
use maine_agents::protocol::{
    encode_frame, AgentCall, AgentHello, AgentHelloReply, AgentReply, FrameReader,
};
use maine_agents::{
    AgentCallHandler, AgentClient, AgentClientStore, AgentServer, AgentSocketClient, ClientError,
    MemorySecretStore, NOT_PAIRED,
};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// A handler that echoes the tool it was asked for, and who asked.
struct EchoHandler;

impl AgentCallHandler for EchoHandler {
    fn handle(&self, call: AgentCall, client: &AgentClient) -> AgentReply {
        AgentReply::success(call.id, json!({"tool": call.tool, "client": client.name}))
    }
}

/// Replies with a large result, so writes take longer than the socket buffer.
struct BigHandler;

impl AgentCallHandler for BigHandler {
    fn handle(&self, call: AgentCall, _client: &AgentClient) -> AgentReply {
        AgentReply::success(call.id, json!({"blob": "x".repeat(1_048_576)}))
    }
}

struct Fixture {
    dir: tempfile::TempDir,
    socket: PathBuf,
    clients: Arc<AgentClientStore>,
    server: AgentServer,
}

impl Fixture {
    fn new(handler: Arc<dyn AgentCallHandler>) -> Self {
        let dir = short_socket_dir();
        let socket = dir.path().join("sub").join("agent.sock");
        let clients = Arc::new(AgentClientStore::new(Arc::new(MemorySecretStore::new())));
        let server = AgentServer::new(socket.clone(), clients.clone(), handler);
        server.start().expect("start");
        Self {
            dir,
            socket,
            clients,
            server,
        }
    }

    fn connect(&self, token: &str) -> AgentSocketClient {
        let client = AgentSocketClient::connect(&self.socket).expect("connect");
        let reply = client
            .hello(&AgentHello {
                token: token.into(),
                client_name: Some("test".into()),
            })
            .expect("hello");
        assert!(reply.ok, "{reply:?}");
        client
    }
}

#[test]
fn paired_clients_are_served_and_others_refused() {
    let f = Fixture::new(Arc::new(EchoHandler));
    let (_, token) = f.clients.add("Claude Code").unwrap();
    let client = AgentSocketClient::connect(&f.socket).unwrap();
    let hello = client
        .hello(&AgentHello {
            token,
            client_name: Some("claude-code".into()),
        })
        .unwrap();
    assert_eq!(
        hello,
        AgentHelloReply {
            ok: true,
            error: None
        }
    );
    let reply = client.call("list_hosts", json!({})).unwrap();
    assert_eq!(
        reply.result,
        Some(json!({"tool": "list_hosts", "client": "Claude Code"}))
    );
    let (first, second) = std::thread::scope(|scope| {
        let a = scope.spawn(|| client.call("a", json!({})));
        let b = scope.spawn(|| client.call("b", json!({})));
        (a.join().unwrap().unwrap(), b.join().unwrap().unwrap())
    });
    assert_eq!(first.result.unwrap().string("tool"), Some("a"));
    assert_eq!(second.result.unwrap().string("tool"), Some("b"));
    client.close();

    let stranger = AgentSocketClient::connect(&f.socket).unwrap();
    let refused = stranger
        .hello(&AgentHello {
            token: "maine-agent-WRONG".into(),
            client_name: None,
        })
        .unwrap();
    assert_eq!(
        refused,
        AgentHelloReply {
            ok: false,
            error: Some("This MCP client is not paired with Maine.".into())
        }
    );
    assert_eq!(NOT_PAIRED, "This MCP client is not paired with Maine.");
    // The server hangs up on an unpaired client.
    assert!(wait_until(Duration::from_secs(2), || stranger.is_closed()));
    assert_eq!(
        stranger.call("list_hosts", json!({})),
        Err(ClientError::NotSent)
    );
}

#[test]
fn socket_file_is_user_only() {
    let f = Fixture::new(Arc::new(EchoHandler));
    let mode =
        |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode(&f.socket), 0o600);
    assert_eq!(mode(f.socket.parent().unwrap()), 0o700);
}

#[test]
fn a_stale_socket_file_does_not_block_start() {
    let dir = short_socket_dir();
    let socket = dir.path().join("agent.sock");
    std::fs::write(&socket, b"stale").unwrap();
    let clients = Arc::new(AgentClientStore::new(Arc::new(MemorySecretStore::new())));
    let server = AgentServer::new(socket.clone(), clients.clone(), Arc::new(EchoHandler));
    server.start().unwrap();
    let (_, token) = clients.add("After crash").unwrap();
    let connection = AgentSocketClient::connect(&socket).unwrap();
    assert!(
        connection
            .hello(&AgentHello {
                token,
                client_name: None
            })
            .unwrap()
            .ok
    );
    connection.close();
}

#[test]
fn a_live_server_is_not_replaced() {
    let f = Fixture::new(Arc::new(EchoHandler));
    let clients = Arc::new(AgentClientStore::new(Arc::new(MemorySecretStore::new())));
    let second = AgentServer::new(f.socket.clone(), clients, Arc::new(EchoHandler));
    let error = second.start().unwrap_err();
    assert_eq!(
        error.to_string(),
        "Another copy of Maine is already serving agents."
    );
    assert!(!second.is_running());
    // The first one still answers.
    let (_, token) = f.clients.add("Still here").unwrap();
    f.connect(&token);
}

#[test]
fn revoking_disconnects_live_connections() {
    let f = Fixture::new(Arc::new(EchoHandler));
    let (client, token) = f.clients.add("Revoked").unwrap();
    let (_, other_token) = f.clients.add("Bystander").unwrap();
    let connection = f.connect(&token);
    let bystander = f.connect(&other_token);
    f.server.disconnect(client.id);
    assert!(connection.call("list_hosts", json!({})).is_err());
    assert!(
        bystander.call("list_hosts", json!({})).is_ok(),
        "other clients keep working"
    );
}

#[test]
fn stopping_removes_the_socket() {
    let f = Fixture::new(Arc::new(EchoHandler));
    let (_, token) = f.clients.add("Live").unwrap();
    let live = f.connect(&token);
    f.server.stop();
    assert!(!f.socket.exists());
    assert!(!f.server.is_running());
    assert_eq!(
        AgentSocketClient::connect(&f.socket).err(),
        Some(ClientError::NotRunning)
    );
    assert!(
        live.call("x", json!({})).is_err(),
        "live connections end with the server"
    );
    f.server.stop(); // idempotent
    f.server.start().unwrap(); // and it can start again
    f.connect(&f.clients.add("Again").unwrap().1);
}

// MARK: - Final review fixes

#[test]
fn large_concurrent_traffic_does_not_deadlock() {
    let f = Fixture::new(Arc::new(BigHandler));
    let (_, token) = f.clients.add("Load").unwrap();
    let connection = Arc::new(f.connect(&token));
    let pad = "y".repeat(512 * 1024);
    let done = Arc::new(AtomicUsize::new(0));
    let failures = Arc::new(AtomicUsize::new(0));
    for _ in 0..12 {
        let (connection, done, failures, pad) = (
            connection.clone(),
            done.clone(),
            failures.clone(),
            pad.clone(),
        );
        std::thread::spawn(move || {
            let ok = connection
                .call("x", json!({"pad": pad}))
                .is_ok_and(|reply| reply.result.is_some());
            if !ok {
                failures.fetch_add(1, Ordering::SeqCst);
            }
            done.fetch_add(1, Ordering::SeqCst);
        });
    }
    let finished = wait_until(Duration::from_secs(20), || {
        done.load(Ordering::SeqCst) == 12
    });
    if !finished {
        connection.close(); // fail the stuck calls so nothing is left waiting
    }
    assert!(
        finished && failures.load(Ordering::SeqCst) == 0,
        "12 parallel 512 KiB requests with 1 MiB replies must all complete"
    );
}

#[test]
fn garbage_and_oversized_input_just_closes_the_connection() {
    let f = Fixture::new(Arc::new(EchoHandler));
    // A first frame that is not a hello.
    let mut raw = UnixStream::connect(&f.socket).unwrap();
    raw.write_all(&encode_frame(&json!({"not": "a hello"})).unwrap())
        .unwrap();
    let mut reader = FrameReader::new();
    let mut buffer = [0u8; 1024];
    let count = raw.read(&mut buffer).unwrap();
    let frames = reader.feed(&buffer[..count]).unwrap();
    let reply: AgentHelloReply = serde_json::from_slice(&frames[0]).unwrap();
    assert!(!reply.ok);
    assert_eq!(
        raw.read(&mut buffer).unwrap(),
        0,
        "closed after the refusal"
    );

    // An oversized length prefix.
    let (_, token) = f.clients.add("Bad").unwrap();
    let mut raw = UnixStream::connect(&f.socket).unwrap();
    raw.write_all(
        &encode_frame(&AgentHello {
            token,
            client_name: None,
        })
        .unwrap(),
    )
    .unwrap();
    assert!(raw.read(&mut buffer).unwrap() > 0, "the hello reply");
    raw.write_all(&[0x7F, 0xFF, 0xFF, 0xFF]).unwrap();
    assert_eq!(raw.read(&mut buffer).unwrap_or(0), 0);

    // A call that is not a call.
    let (_, token) = f.clients.add("Bad2").unwrap();
    let mut raw = UnixStream::connect(&f.socket).unwrap();
    raw.write_all(
        &encode_frame(&AgentHello {
            token,
            client_name: None,
        })
        .unwrap(),
    )
    .unwrap();
    assert!(raw.read(&mut buffer).unwrap() > 0, "the hello reply");
    raw.write_all(&encode_frame(&json!({"id": "x"})).unwrap())
        .unwrap();
    assert_eq!(raw.read(&mut buffer).unwrap_or(0), 0);

    // The server is fine.
    let (_, token) = f.clients.add("Good").unwrap();
    f.connect(&token);
}

#[test]
fn a_connection_that_never_says_hello_is_not_served_but_does_not_block_others() {
    let f = Fixture::new(Arc::new(EchoHandler));
    let _silent = UnixStream::connect(&f.socket).unwrap();
    let (_, token) = f.clients.add("Fast").unwrap();
    let started = Instant::now();
    let client = f.connect(&token);
    assert!(client.call("x", json!({})).is_ok());
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn a_call_before_hello_is_refused() {
    let f = Fixture::new(Arc::new(EchoHandler));
    let mut raw = UnixStream::connect(&f.socket).unwrap();
    let call = AgentCall {
        id: 1,
        tool: "list_hosts".into(),
        arguments: Value::Null,
    };
    raw.write_all(&encode_frame(&call).unwrap()).unwrap();
    let mut buffer = [0u8; 1024];
    let count = raw.read(&mut buffer).unwrap();
    let mut reader = FrameReader::new();
    let reply: AgentHelloReply =
        serde_json::from_slice(&reader.feed(&buffer[..count]).unwrap()[0]).unwrap();
    assert!(!reply.ok);
}

#[test]
fn a_path_that_is_too_long_is_reported() {
    let dir = short_socket_dir();
    let long = dir.path().join("x".repeat(120)).join("agent.sock");
    let clients = Arc::new(AgentClientStore::new(Arc::new(MemorySecretStore::new())));
    let server = AgentServer::new(long, clients, Arc::new(EchoHandler));
    let error = server.start().unwrap_err();
    assert_eq!(error.to_string(), "The agent socket path is too long.");
}

#[test]
fn a_panicking_handler_answers_with_an_error_and_the_connection_survives() {
    struct Panics;
    impl AgentCallHandler for Panics {
        fn handle(&self, call: AgentCall, client: &AgentClient) -> AgentReply {
            maine_agents::AgentToolHandler::new(std::sync::Arc::new(PanicHost)).handle(call, client)
        }
    }
    // The tool handler turns a panic in the host into an error reply.
    struct PanicHost;
    impl maine_agents::AgentHost for PanicHost {
        fn is_unlocked(&self) -> bool {
            panic!("boom")
        }
        fn layout(&self) -> maine_core::model::HostLayout {
            unreachable!()
        }
        fn access(&self) -> maine_agents::AgentAccess {
            unreachable!()
        }
        fn store_access(
            &self,
            _: &maine_agents::AgentAccess,
        ) -> Result<(), maine_agents::AgentError> {
            unreachable!()
        }
        fn has_saved_password(&self, _: uuid::Uuid) -> bool {
            unreachable!()
        }
        fn saved_password(&self, _: uuid::Uuid) -> Option<zeroize::Zeroizing<String>> {
            unreachable!()
        }
        fn open_agent_session(
            &self,
            _: &maine_core::model::Host,
            _: &AgentClient,
            _: Option<&str>,
        ) -> Result<maine_agents::AgentSessionRef, String> {
            unreachable!()
        }
        fn sessions(&self) -> Vec<maine_agents::AgentSessionRef> {
            unreachable!()
        }
        fn attach_agent(&self, _: uuid::Uuid, _: Arc<maine_agents::AgentDriver>, _: bool) {}
        fn end_agent_control(&self, _: uuid::Uuid) {}
        fn close_session(&self, _: uuid::Uuid) {}
        fn record_audit(&self, _: maine_agents::AgentAuditEntry) {}
    }
    let f = Fixture::new(Arc::new(Panics));
    let (_, token) = f.clients.add("P").unwrap();
    let connection = f.connect(&token);
    let reply = connection.call("list_hosts", json!({})).unwrap();
    assert_eq!(reply.error.as_deref(), Some("internal error"));
    assert_eq!(
        connection
            .call("list_hosts", json!({}))
            .unwrap()
            .error
            .as_deref(),
        Some("internal error")
    );
    let _ = &f.dir;
}
