//! The agent switchboard (the macOS `AgentCoordinator`, without the UI state):
//! the master switch, the socket server, paired clients, device-local access,
//! hand-off, and what locking does to agents. The app calls into this from its
//! settings and menu code and implements [`AgentHost`] for the rest.

use crate::access::AgentAccess;
use crate::audit::{AgentAuditEntry, AuditKind};
use crate::clients::{AgentClient, AgentClientStore, Pairing};
use crate::clock::Clock;
use crate::driver::AgentDriver;
use crate::error::AgentError;
use crate::host::{AgentHost, AgentSessionRef, HandOffError};
use crate::server::AgentServer;
use crate::tools::AgentToolHandler;
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use uuid::Uuid;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

pub struct AgentCoordinator {
    host: Arc<dyn AgentHost>,
    clients: Arc<AgentClientStore>,
    clock: Arc<dyn Clock>,
    server: AgentServer,
    enabled: AtomicBool,
    paused_by_lock: Mutex<HashSet<Uuid>>,
    last_error: Mutex<Option<String>>,
}

impl AgentCoordinator {
    /// `socket_path`: see [`default_socket_path`](crate::default_socket_path).
    /// The master switch starts off: persist it in the app's preferences and
    /// call [`set_enabled`](Self::set_enabled) at launch.
    pub fn new(
        host: Arc<dyn AgentHost>,
        clients: Arc<AgentClientStore>,
        socket_path: PathBuf,
        clock: Arc<dyn Clock>,
    ) -> Self {
        let handler = Arc::new(AgentToolHandler::new(host.clone()));
        Self {
            server: AgentServer::new(socket_path, clients.clone(), handler),
            host,
            clients,
            clock,
            enabled: AtomicBool::new(false),
            paused_by_lock: Mutex::new(HashSet::new()),
            last_error: Mutex::new(None),
        }
    }

    pub fn server(&self) -> &AgentServer {
        &self.server
    }

    /// The last problem worth showing in Settings (the socket could not start,
    /// a token or the rules could not be stored).
    pub fn last_error(&self) -> Option<String> {
        lock(&self.last_error).clone()
    }

    fn fail(&self, error: AgentError) {
        *lock(&self.last_error) = Some(error.to_string());
    }

    // MARK: - Master switch

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Acquire)
    }

    /// Turning it on starts the socket server; off closes it and ends agent
    /// control of every session (the sessions stay open).
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::Release);
        if enabled {
            match self.server.start() {
                Ok(()) => *lock(&self.last_error) = None,
                Err(error) => self.fail(error),
            }
        } else {
            self.server.stop();
            self.stop_all();
        }
    }

    // MARK: - Clients

    pub fn clients(&self) -> Vec<AgentClient> {
        self.clients.clients().unwrap_or_default()
    }

    /// Pairs a client; the token in the result is shown once. `executable` is
    /// the path of the running `maine` binary, for the setup snippets.
    pub fn add_client(&self, name: &str, executable: &str) -> Result<Pairing, AgentError> {
        self.clients.pair(name, executable)
    }

    /// Revokes a client: its token stops working, its connections close and
    /// agent control of its sessions ends (they stay open for the user).
    pub fn revoke(&self, id: Uuid) {
        if let Err(error) = self.clients.revoke(id) {
            self.fail(error);
        }
        self.server.disconnect(id);
        for session in self.agent_sessions() {
            if session
                .driver
                .as_ref()
                .is_some_and(|driver| driver.client_id() == id)
            {
                self.end_control(&session);
            }
        }
    }

    // MARK: - Access

    /// The current rules, pruned to the hosts and groups that still exist.
    pub fn access(&self) -> AgentAccess {
        self.host.access().pruned(&self.host.layout())
    }

    /// Stores the rules (pruned) and enforces them at once: sessions on hosts
    /// no longer allowed lose their agent and stay open for the user.
    pub fn set_access(&self, access: &AgentAccess) {
        let pruned = access.pruned(&self.host.layout());
        if let Err(error) = self.host.store_access(&pruned) {
            self.fail(error);
            return;
        }
        for session in self.agent_sessions() {
            let allowed = self
                .host
                .host(session.host_id)
                .is_some_and(|host| pruned.allows(&host));
            if !allowed {
                self.end_control(&session);
            }
        }
    }

    // MARK: - Hand to agent

    /// Lets `client_id` drive a session the user opened. Needs the master
    /// switch on, an unlocked vault and a host the rules allow, exactly like
    /// every other way an agent reaches a host.
    pub fn hand_to_agent(&self, session_id: Uuid, client_id: Uuid) -> Result<(), HandOffError> {
        let client = self
            .clients()
            .into_iter()
            .find(|client| client.id == client_id)
            .ok_or(HandOffError::NotFound)?;
        let session = self
            .host
            .session(session_id)
            .ok_or(HandOffError::NotFound)?;
        if !self.may_reach(&session) {
            return Err(HandOffError::NotAllowed);
        }
        if session.driver.is_some() {
            return Err(HandOffError::AlreadyDriven);
        }
        if !session.is_live || !session.terminal.is_at_shell_prompt() {
            return Err(HandOffError::NotAtPrompt);
        }
        let driver = AgentDriver::new(
            session.terminal.clone(),
            client.id,
            client.name.clone(),
            self.clock.clone(),
        );
        self.host.attach_agent(session.id, driver, true);
        let host_name = self
            .host
            .host(session.host_id)
            .map(|host| host.display_name())
            .unwrap_or_default();
        self.host.record_audit(AgentAuditEntry::new(
            client.name,
            host_name,
            session.id,
            AuditKind::Handoff,
            None,
            None,
        ));
        Ok(())
    }

    /// Whether the menu may offer hand-off for this session.
    pub fn can_hand_off(&self, session: &AgentSessionRef) -> bool {
        session.driver.is_none() && !self.clients().is_empty() && self.may_reach(session)
    }

    fn may_reach(&self, session: &AgentSessionRef) -> bool {
        self.is_enabled()
            && self.host.is_unlocked()
            && self
                .host
                .host(session.host_id)
                .is_some_and(|host| self.host.access().allows(&host))
    }

    // MARK: - Lock and stop

    fn agent_sessions(&self) -> Vec<AgentSessionRef> {
        self.host
            .sessions()
            .into_iter()
            .filter(|session| session.driver.is_some())
            .collect()
    }

    /// Sessions an agent drives or has paused.
    pub fn active_count(&self) -> usize {
        self.agent_sessions().len()
    }

    /// The vault locked: every agent session pauses (the SSH processes keep running).
    pub fn vault_did_lock(&self) {
        let mut paused = lock(&self.paused_by_lock);
        for session in self.agent_sessions() {
            if let Some(driver) = session.driver.filter(|driver| !driver.is_paused()) {
                driver.set_paused(true);
                paused.insert(session.id);
            }
        }
    }

    /// The vault unlocked: resumes only what locking paused.
    pub fn vault_did_unlock(&self) {
        let mut paused = lock(&self.paused_by_lock);
        for session in self.agent_sessions() {
            if paused.contains(&session.id) {
                if let Some(driver) = &session.driver {
                    driver.set_paused(false);
                }
            }
        }
        paused.clear();
    }

    /// Ends agent control everywhere; every session stays open.
    pub fn stop_all(&self) {
        for session in self.agent_sessions() {
            self.end_control(&session);
        }
        lock(&self.paused_by_lock).clear();
    }

    /// Ends agent control of one session (the user pressed Stop).
    pub fn stop(&self, session_id: Uuid) {
        if let Some(session) = self
            .host
            .session(session_id)
            .filter(|session| session.driver.is_some())
        {
            self.end_control(&session);
        }
    }

    fn end_control(&self, session: &AgentSessionRef) {
        if let Some(driver) = &session.driver {
            driver.detach();
        }
        self.host.end_agent_control(session.id);
    }
}

impl Drop for AgentCoordinator {
    fn drop(&mut self) {
        self.server.stop();
    }
}
