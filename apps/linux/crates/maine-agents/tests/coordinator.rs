//! The agent switchboard: server lifecycle, pairing, access, hand-off, lock.
//! Ports of the Swift `AgentCoordinatorTests`; the rows that read the sealed
//! local state file (audit persistence) belong to the app, which implements
//! `AgentHost::record_audit` and `store_access`.

mod common;

use common::*;
use maine_agents::{
    AgentAccess, AgentClient, AgentClientStore, AgentCoordinator, AgentHost, AgentSession,
    AuditKind, HandOffError, MemorySecretStore,
};
use maine_core::model::Host;
use std::sync::Arc;
use std::time::Duration;
use uuid::Uuid;

struct Fixture {
    host: Arc<TestHost>,
    secrets: Arc<MemorySecretStore>,
    coordinator: AgentCoordinator,
    dir: tempfile::TempDir,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        self.coordinator.set_enabled(false);
        self.host.tear_down();
    }
}

impl Fixture {
    fn new() -> Self {
        let host = TestHost::new();
        let secrets = Arc::new(MemorySecretStore::new());
        let dir = short_socket_dir();
        let coordinator = AgentCoordinator::new(
            host.clone(),
            Arc::new(AgentClientStore::new(secrets.clone())),
            dir.path().join("agent.sock"),
            system_clock(),
        );
        Self {
            host,
            secrets,
            coordinator,
            dir,
        }
    }

    fn socket(&self) -> std::path::PathBuf {
        self.dir.path().join("agent.sock")
    }

    fn pair(&self, name: &str) -> AgentClient {
        self.coordinator
            .add_client(name, "/usr/bin/maine")
            .unwrap()
            .client
    }

    /// An agent-opened session on `web`.
    fn agent_session(&self, client: &AgentClient) -> Uuid {
        let opened = self
            .host
            .open_agent_session(&self.host.web, client, None)
            .unwrap();
        opened.id
    }

    fn live_session_ref(&self, id: Uuid) -> maine_agents::AgentSessionRef {
        self.host.session(id).unwrap()
    }
}

#[test]
fn enabling_starts_and_disabling_stops_the_server() {
    let f = Fixture::new();
    assert!(!f.coordinator.is_enabled());
    assert!(!f.socket().exists());
    f.coordinator.set_enabled(true);
    assert!(f.coordinator.is_enabled());
    assert!(f.socket().exists());
    assert_eq!(f.coordinator.last_error(), None);

    let client = f.pair("Agent");
    let session = f.agent_session(&client);
    f.coordinator.set_enabled(false);
    assert!(!f.socket().exists());
    assert!(f.host.driver(session).is_none());
    assert_eq!(
        f.host
            .with_record(session, |r| r.former_agent.clone())
            .as_deref(),
        Some("Agent")
    );
    assert!(
        f.host.ids().contains(&session),
        "the session stays open for the user"
    );
}

#[test]
fn a_socket_that_cannot_start_is_reported_not_fatal() {
    let f = Fixture::new();
    let other = AgentCoordinator::new(
        f.host.clone(),
        Arc::new(AgentClientStore::new(Arc::new(MemorySecretStore::new()))),
        f.socket(),
        system_clock(),
    );
    f.coordinator.set_enabled(true);
    other.set_enabled(true);
    assert_eq!(
        other.last_error().as_deref(),
        Some("Another copy of Maine is already serving agents.")
    );
    assert!(other.is_enabled());
}

#[test]
fn lock_pauses_and_unlock_resumes_only_what_lock_paused() {
    let f = Fixture::new();
    let client = f.pair("Agent");
    let (first, second) = (f.agent_session(&client), f.agent_session(&client));
    f.host.driver(second).unwrap().set_paused(true);
    f.coordinator.vault_did_lock();
    assert!(f.host.driver(first).unwrap().is_paused());
    assert!(f.host.driver(second).unwrap().is_paused());
    f.coordinator.vault_did_unlock();
    assert!(!f.host.driver(first).unwrap().is_paused());
    assert!(
        f.host.driver(second).unwrap().is_paused(),
        "the user's own pause stays"
    );
}

#[test]
fn revoking_a_client_ends_its_control() {
    let f = Fixture::new();
    let pairing = f.coordinator.add_client("Agent", "/usr/bin/maine").unwrap();
    let (client, token) = (pairing.client, pairing.token);
    let other = f.pair("Other");
    let (mine, theirs) = (f.agent_session(&client), f.agent_session(&other));
    f.coordinator.revoke(client.id);
    assert!(f.host.driver(mine).is_none());
    assert!(f.host.ids().contains(&mine));
    assert_eq!(f.host.driver(theirs).unwrap().client_id(), other.id);
    assert_eq!(
        AgentClientStore::new(f.secrets.clone())
            .client_for_token(&token)
            .unwrap(),
        None
    );
    let names: Vec<String> = f
        .coordinator
        .clients()
        .into_iter()
        .map(|c| c.name)
        .collect();
    assert_eq!(names, ["Other"]);
}

#[test]
fn setup_snippets_use_the_given_executable() {
    let f = Fixture::new();
    let pairing = f
        .coordinator
        .add_client("Claude Code", "/opt/maine/bin/maine")
        .unwrap();
    assert!(pairing
        .claude_code_command
        .starts_with("claude mcp add maine --env MAINE_AGENT_TOKEN=maine-agent-"));
    assert!(pairing
        .claude_code_command
        .ends_with("\"/opt/maine/bin/maine\" --mcp"));
    assert!(pairing.claude_code_command.contains(&pairing.token));
    let desktop: serde_json::Value = serde_json::from_str(&pairing.desktop_json).unwrap();
    assert_eq!(
        desktop["mcpServers"]["maine"]["args"],
        serde_json::json!(["--mcp"])
    );
    assert_eq!(
        desktop["mcpServers"]["maine"]["env"]["MAINE_AGENT_TOKEN"],
        pairing.token.as_str()
    );
}

#[test]
fn access_is_pruned_and_enforced_immediately() {
    let f = Fixture::new();
    let prod = f.host.prod.id;
    f.coordinator
        .set_access(&AgentAccess::new([prod, Uuid::new_v4()], []));
    assert_eq!(f.coordinator.access(), AgentAccess::new([prod], []));
    assert_eq!(
        lock(&f.host.stored_access).clone(),
        Some(AgentAccess::new([prod], []))
    );

    let client = f.pair("Agent");
    let session = f.agent_session(&client);
    f.coordinator.set_access(&AgentAccess::default());
    assert!(f.host.driver(session).is_none());
    assert!(f.host.ids().contains(&session));
}

#[test]
fn changing_rules_keeps_sessions_on_hosts_that_stay_allowed() {
    let f = Fixture::new();
    let client = f.pair("Agent");
    let session = f.agent_session(&client);
    f.coordinator
        .set_access(&AgentAccess::new([], [f.host.web.id, f.host.loose.id]));
    assert!(
        f.host.driver(session).is_some(),
        "web is allowed by itself now"
    );
}

#[test]
fn stop_all_ends_every_agent() {
    let f = Fixture::new();
    let client = f.pair("Agent");
    f.agent_session(&client);
    let second = f.agent_session(&client);
    assert_eq!(f.coordinator.active_count(), 2);
    f.coordinator.stop(second);
    assert_eq!(f.coordinator.active_count(), 1);
    f.coordinator.stop_all();
    assert_eq!(f.coordinator.active_count(), 0);
    assert_eq!(f.host.ids().len(), 2);
}

// MARK: - Hand to agent

/// A session the user opened, waited until its shell shows a prompt.
fn human_session_at_prompt(f: &Fixture) -> Uuid {
    let id = f.host.open_user_session(&f.host.web);
    let terminal = f.host.terminal(id);
    assert!(
        wait_until(Duration::from_secs(10), || terminal.is_at_shell_prompt()),
        "no prompt"
    );
    id
}

#[test]
fn a_handed_session_is_driven_by_the_agent_but_stays_the_users() {
    let f = Fixture::new();
    f.coordinator.set_enabled(true);
    f.coordinator
        .set_access(&AgentAccess::new([f.host.prod.id], []));
    let client = f.pair("Agent");
    let session = human_session_at_prompt(&f);
    assert!(f.coordinator.can_hand_off(&f.live_session_ref(session)));

    f.coordinator.hand_to_agent(session, client.id).unwrap();
    let driver = f.host.driver(session).unwrap();
    assert_eq!(driver.client_id(), client.id);
    assert!(
        f.host.with_record(session, |r| r.handed_off),
        "the user opened it; it is still checkpointed and restored"
    );
    assert!(f.host.terminal(session).is_agent_driven());
    let result = driver
        .run("echo handed", Duration::from_secs(10), None)
        .unwrap();
    assert!(result.completed && result.output.contains("handed"));
    let kinds: Vec<AuditKind> = f.host.audit_entries().iter().map(|e| e.kind).collect();
    assert_eq!(kinds, [AuditKind::Handoff]);
    assert_eq!(f.host.audit_entries()[0].host_name, "Web");
    assert!(
        !f.coordinator.can_hand_off(&f.live_session_ref(session)),
        "already driven"
    );

    f.coordinator.stop(session);
    assert!(f.host.driver(session).is_none());
    assert_eq!(
        f.host.with_record(session, |r| r.former_agent.clone()),
        None,
        "no 'agent disconnected' leftover: it was never an agent session"
    );
    assert!(!f.host.terminal(session).is_agent_driven());
}

#[test]
fn hand_off_refuses_what_agents_may_not_or_cannot_drive() {
    let f = Fixture::new();
    let client = f.pair("Agent");
    let session = human_session_at_prompt(&f);
    let hand = |session: Uuid, client: Uuid| f.coordinator.hand_to_agent(session, client);

    // Master switch off, then enabled but the host not allowed, then vault locked.
    assert_eq!(hand(session, client.id), Err(HandOffError::NotAllowed));
    f.coordinator.set_enabled(true);
    f.host.set_access(AgentAccess::default());
    assert_eq!(hand(session, client.id), Err(HandOffError::NotAllowed));
    f.coordinator
        .set_access(&AgentAccess::new([f.host.prod.id], []));
    f.host.set_unlocked(false);
    assert_eq!(hand(session, client.id), Err(HandOffError::NotAllowed));
    f.host.set_unlocked(true);
    assert_eq!(hand(session, Uuid::new_v4()), Err(HandOffError::NotFound));
    assert_eq!(hand(Uuid::new_v4(), client.id), Err(HandOffError::NotFound));

    // Something is running: the shell is not at a prompt.
    let terminal = f.host.terminal(session);
    terminal.send_input(b"sleep 30\n");
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(hand(session, client.id), Err(HandOffError::NotAtPrompt));
    assert!(f.host.driver(session).is_none());
    terminal.send_input(&[0x03]);
    assert!(wait_until(Duration::from_secs(5), || terminal.is_at_shell_prompt()));

    hand(session, client.id).unwrap();
    assert_eq!(hand(session, client.id), Err(HandOffError::AlreadyDriven));
    assert_eq!(f.host.audit_entries().len(), 1);
    assert_eq!(
        HandOffError::NotAtPrompt.to_string(),
        "Wait until the shell is at a prompt, with no command or full-screen program running."
    );
}

#[test]
fn a_dead_session_cannot_be_handed_over() {
    let f = Fixture::new();
    f.coordinator.set_enabled(true);
    f.coordinator
        .set_access(&AgentAccess::new([f.host.prod.id], []));
    let client = f.pair("Agent");
    let session = human_session_at_prompt(&f);
    f.host.terminal(session).send_input(b"exit\n");
    assert!(wait_until(Duration::from_secs(5), || !f
        .host
        .terminal(session)
        .is_running()));
    assert_eq!(
        f.coordinator.hand_to_agent(session, client.id),
        Err(HandOffError::NotAtPrompt)
    );
}

#[test]
fn clipboard_and_links_are_off_for_agents_and_on_for_people() {
    // The Swift test pokes the terminal view's pasteboard; the Linux terminal
    // item asks the session whether an agent drives it.
    let f = Fixture::new();
    let client: AgentClient = f.pair("Agent");
    let human = f.host.open_user_session(&f.host.web);
    let agent = f.agent_session(&client);
    assert!(!f.host.terminal(human).is_agent_driven());
    assert!(f.host.terminal(agent).is_agent_driven());
    f.coordinator.stop(agent);
    assert!(
        !f.host.terminal(agent).is_agent_driven(),
        "back to the user"
    );
}

#[test]
fn can_hand_off_needs_a_paired_client() {
    let f = Fixture::new();
    f.coordinator.set_enabled(true);
    f.coordinator
        .set_access(&AgentAccess::new([f.host.prod.id], []));
    let session = f.host.open_user_session(&f.host.web);
    assert!(!f.coordinator.can_hand_off(&f.live_session_ref(session)));
    f.pair("Agent");
    assert!(f.coordinator.can_hand_off(&f.live_session_ref(session)));
    f.host.set_unlocked(false);
    assert!(!f.coordinator.can_hand_off(&f.live_session_ref(session)));
}

#[allow(dead_code)]
fn unused(_: &Host) {}
