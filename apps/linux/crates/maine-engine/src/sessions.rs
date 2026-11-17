use crate::askpass::{Broker, ENV_HOST_ID, ENV_SOCKET};
use crate::diag;
use maine_core::model::Host;
use maine_core::ssh::{terminal_environment, SshTarget};
use maine_core::vault::{SavedSession, Unlocked, VaultFile};
use maine_term::{SessionConfig, TermSession};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use uuid::Uuid;

const HISTORY_LIMIT: usize = 1_048_576;
/// Seconds between 1970-01-01 and 2001-01-01 (Swift's reference date).
const APPLE_EPOCH: f64 = 978_307_200.0;

/// How a session's terminal gets started. Tests substitute a shell.
pub trait Launcher: Send + Sync {
    fn launch(
        &self,
        host: &Host,
        cols: u16,
        rows: u16,
        history: Option<&str>,
    ) -> Result<TermSession, String>;
}

/// Runs the system `ssh`, wiring askpass in when the host has a saved password.
pub struct SshLauncher {
    pub broker: Arc<Broker>,
    /// The `maine` executable ssh runs as `SSH_ASKPASS`.
    pub askpass_exe: PathBuf,
    pub askpass_socket: PathBuf,
}

impl Launcher for SshLauncher {
    fn launch(
        &self,
        host: &Host,
        cols: u16,
        rows: u16,
        history: Option<&str>,
    ) -> Result<TermSession, String> {
        if !SshTarget::is_available() {
            return Err(format!(
                "Maine could not run the SSH client at {}. Install OpenSSH (for example the openssh-client package).",
                maine_core::ssh::SSH_PATH
            ));
        }
        if !host.is_valid() {
            return Err(host.issues().join(" "));
        }
        let target = SshTarget::make(host);
        let mut env = terminal_environment();
        if self.broker.password(host.id).is_some() {
            let overrides = [
                (
                    "SSH_ASKPASS",
                    self.askpass_exe.to_string_lossy().to_string(),
                ),
                ("SSH_ASKPASS_REQUIRE", "force".to_string()),
                ("MAINE_SSH_ASKPASS", "1".to_string()),
                (ENV_HOST_ID, host.id.hyphenated().to_string().to_uppercase()),
                (
                    ENV_SOCKET,
                    self.askpass_socket.to_string_lossy().to_string(),
                ),
            ];
            env.retain(|(k, _)| !overrides.iter().any(|(o, _)| o == k));
            env.extend(overrides.into_iter().map(|(k, v)| (k.to_string(), v)));
        }
        // No saved password: plain ssh, so keys, ssh-agent and ~/.ssh/config
        // work and any prompt appears in the terminal.
        let mut config = SessionConfig::new(target.executable, target.arguments);
        config.env = env;
        config.cols = cols;
        config.rows = rows;
        config.history = history.map(str::to_string);
        TermSession::spawn(config)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionState {
    /// ssh was launched; nothing seen yet.
    Connecting,
    Connected,
    /// ssh exited. `None` = died from a signal.
    Closed(Option<i32>),
    /// The user asked to disconnect.
    Disconnected(Option<i32>),
    Failed(String),
}

impl SessionState {
    pub fn is_live(&self) -> bool {
        matches!(self, Self::Connecting | Self::Connected)
    }

    pub fn label(&self) -> String {
        match self {
            Self::Connecting => "Connecting…".into(),
            Self::Connected => "Connected".into(),
            Self::Closed(code) => format!(
                "Closed ({})",
                code.map(|c| c.to_string())
                    .unwrap_or_else(|| "interrupted".into())
            ),
            Self::Disconnected(_) => "Disconnected".into(),
            Self::Failed(_) => "Could not connect".into(),
        }
    }

    /// The problem worth showing the user, if any.
    pub fn notice(&self) -> Option<String> {
        match self {
            Self::Closed(Some(code)) if *code != 0 => Some(format!(
                "The connection closed with exit code {code}. The full session output, including what ssh reported, is in the terminal below."
            )),
            Self::Failed(message) => Some(message.clone()),
            _ => None,
        }
    }
}

/// An MCP client driving a session.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentLink {
    pub client_id: Uuid,
    pub client_name: String,
    /// The user opened the session and handed it over: it stays an ordinary
    /// saved session and returns to the user when control ends.
    pub handed_off: bool,
}

#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub id: Uuid,
    pub host_id: Uuid,
    pub label: String,
    pub state: SessionState,
    pub saved: bool,
    /// The agent driving it now, if any.
    pub agent: Option<AgentLink>,
    /// Control ended but the session stays open, still ephemeral, until the user keeps or closes it.
    pub former_agent: Option<String>,
    /// Never saved or restored (agent sessions until the user keeps them).
    pub ephemeral: bool,
}

struct Managed {
    id: Uuid,
    host: Host,
    term: Option<TermSession>,
    custom_name: Option<String>,
    number: i64,
    saved_id: Option<Uuid>,
    disconnect_requested: bool,
    failed: Option<String>,
    seen_output: Arc<AtomicBool>,
    agent: Option<AgentLink>,
    former_agent: Option<String>,
}

impl Managed {
    /// Agent sessions are never checkpointed until the user keeps them.
    fn is_ephemeral(&self) -> bool {
        self.agent.as_ref().is_some_and(|a| !a.handed_off) || self.former_agent.is_some()
    }

    fn state(&self) -> SessionState {
        if let Some(message) = &self.failed {
            return SessionState::Failed(message.clone());
        }
        let Some(term) = &self.term else {
            return SessionState::Failed("No terminal.".into());
        };
        match term.exit_status() {
            Some(code) if self.disconnect_requested => SessionState::Disconnected(code),
            Some(code) => SessionState::Closed(code),
            None if self.seen_output.load(Ordering::Acquire) || term.bytes_received() > 0 => {
                SessionState::Connected
            }
            None => SessionState::Connecting,
        }
    }

    fn label(&self) -> String {
        self.custom_name
            .clone()
            .unwrap_or_else(|| format!("Session {}", self.number))
    }
}

/// Tracks the open terminal sessions and which one is shown. Sessions have
/// independent identities (session UUIDs), even for the same host.
pub struct SessionManager {
    launcher: Arc<dyn Launcher>,
    sessions: Vec<Managed>,
    selected: Option<Uuid>,
    vault: Option<(VaultFile, Unlocked)>,
    save_failure: Option<String>,
    size: (u16, u16),
}

impl SessionManager {
    pub fn new(launcher: Arc<dyn Launcher>) -> Self {
        Self {
            launcher,
            sessions: Vec::new(),
            selected: None,
            vault: None,
            save_failure: None,
            size: (80, 24),
        }
    }

    pub fn set_vault(&mut self, vault: Option<(VaultFile, Unlocked)>) {
        self.vault = vault;
    }

    /// The size new terminals start with, until the UI lays them out.
    pub fn set_initial_size(&mut self, cols: u16, rows: u16) {
        self.size = (cols.max(1), rows.max(1));
    }

    pub fn save_failure(&self) -> Option<&str> {
        self.save_failure.as_deref()
    }

    // MARK: queries

    pub fn list(&self) -> Vec<SessionInfo> {
        self.sessions
            .iter()
            .map(|m| SessionInfo {
                id: m.id,
                host_id: m.host.id,
                label: m.label(),
                state: m.state(),
                saved: m.saved_id.is_some(),
                agent: m.agent.clone(),
                former_agent: m.former_agent.clone(),
                ephemeral: m.is_ephemeral(),
            })
            .collect()
    }

    pub fn selected(&self) -> Option<Uuid> {
        self.selected
    }

    pub fn terminal(&self, id: Uuid) -> Option<TermSession> {
        self.sessions
            .iter()
            .find(|m| m.id == id)
            .and_then(|m| m.term.clone())
    }

    pub fn host_of(&self, id: Uuid) -> Option<Host> {
        self.sessions
            .iter()
            .find(|m| m.id == id)
            .map(|m| m.host.clone())
    }

    pub fn state(&self, id: Uuid) -> Option<SessionState> {
        self.sessions
            .iter()
            .find(|m| m.id == id)
            .map(Managed::state)
    }

    pub fn is_open(&self, id: Uuid) -> bool {
        self.sessions.iter().any(|m| m.id == id)
    }

    /// The user's session for a host: the selected one if it is that host's,
    /// else a live one, else the first.
    pub fn session_for_host(&self, host_id: Uuid) -> Option<Uuid> {
        if let Some(selected) = self.selected {
            if self
                .sessions
                .iter()
                .any(|m| m.id == selected && m.host.id == host_id && !m.is_ephemeral())
            {
                return Some(selected);
            }
        }
        // Agent sessions are reached only through their own rows.
        let mine: Vec<&Managed> = self
            .sessions
            .iter()
            .filter(|m| m.host.id == host_id && !m.is_ephemeral())
            .collect();
        mine.iter()
            .find(|m| m.state().is_live())
            .or(mine.first())
            .map(|m| m.id)
    }

    pub fn sessions_for_host(&self, host_id: Uuid) -> Vec<Uuid> {
        self.sessions
            .iter()
            .filter(|m| m.host.id == host_id)
            .map(|m| m.id)
            .collect()
    }

    /// The UI laid the terminal out: ssh now has a real terminal, which counts
    /// as "connected" even if the server prints nothing yet.
    pub fn mark_sized(&self, id: Uuid) {
        if let Some(m) = self.sessions.iter().find(|m| m.id == id) {
            m.seen_output.store(true, Ordering::Release);
        }
    }

    // MARK: opening

    fn make(&mut self, host: &Host, history: Option<&str>) -> Managed {
        let seen = Arc::new(AtomicBool::new(false));
        let (term, failed) = match self
            .launcher
            .launch(host, self.size.0, self.size.1, history)
        {
            Ok(term) => (Some(term), None),
            Err(message) => (None, Some(message)),
        };
        Managed {
            id: Uuid::new_v4(),
            host: host.clone(),
            term,
            custom_name: None,
            number: 1,
            saved_id: None,
            disconnect_requested: false,
            failed,
            seen_output: seen,
            agent: None,
            former_agent: None,
        }
    }

    fn next_number(&self, host_id: Uuid) -> i64 {
        self.sessions
            .iter()
            .filter(|m| m.host.id == host_id)
            .map(|m| m.number)
            .max()
            .map_or(1, |n| n + 1)
    }

    /// Connects to `host`, or reveals the existing session. Returns the session shown.
    pub fn open(&mut self, host: &Host) -> Uuid {
        if let Some(existing) = self.session_for_host(host.id) {
            if !self.state(existing).is_some_and(|s| s.is_live()) {
                return self
                    .reconnect_session(existing, Some(host))
                    .unwrap_or(existing);
            }
            self.selected = Some(existing);
            return existing;
        }
        self.new_session(host)
    }

    /// Opens another independent connection without replacing any.
    pub fn new_session(&mut self, host: &Host) -> Uuid {
        let mut managed = self.make(host, None);
        managed.number = self.next_number(host.id);
        let id = managed.id;
        diag::event(
            "session.new",
            &[("session", id.to_string()), ("host", host.id.to_string())],
        );
        self.sessions.push(managed);
        self.selected = Some(id);
        self.checkpoint_quietly();
        id
    }

    /// A brand new connection in the same place: the old process is stopped and
    /// the terminal starts from the previous scrollback plus a marker.
    pub fn reconnect_session(&mut self, id: Uuid, current_host: Option<&Host>) -> Option<Uuid> {
        let index = self.sessions.iter().position(|m| m.id == id)?;
        if self.sessions[index].agent.is_some() {
            return None; // an agent holds a driver on this terminal
        }
        let host = current_host
            .cloned()
            .unwrap_or_else(|| self.sessions[index].host.clone());
        let previous = &self.sessions[index];
        if let Some(term) = &previous.term {
            term.terminate();
        }
        let history = (previous.saved_id.is_some()).then(|| {
            let old = previous
                .term
                .as_ref()
                .map(|t| t.history_text(HISTORY_LIMIT))
                .unwrap_or_default();
            format!("{old}\n— New connection —")
        });
        let (custom_name, saved_id, number) = (
            previous.custom_name.clone(),
            previous.saved_id,
            previous.number,
        );
        let mut replacement = self.make(&host, history.as_deref());
        replacement.custom_name = custom_name;
        replacement.saved_id = saved_id;
        replacement.number = number;
        let new_id = replacement.id;
        diag::event(
            "session.reconnect",
            &[
                ("session", id.to_string()),
                ("replacement", new_id.to_string()),
            ],
        );
        let was_selected = self.selected == Some(id);
        self.sessions[index] = replacement;
        if was_selected || self.selected.is_none() {
            self.selected = Some(new_id);
        }
        self.checkpoint_quietly();
        Some(new_id)
    }

    pub fn select(&mut self, id: Uuid) -> bool {
        let known = self.is_open(id);
        if known {
            self.selected = Some(id);
        }
        known
    }

    pub fn disconnect(&mut self, id: Uuid) {
        if let Some(m) = self.sessions.iter_mut().find(|m| m.id == id) {
            if m.state().is_live() {
                diag::event("session.disconnect", &[("session", id.to_string())]);
                m.disconnect_requested = true;
                if let Some(term) = &m.term {
                    term.terminate();
                }
            }
        }
    }

    // MARK: agent sessions

    /// Opens a session for an MCP client. It shows in the sidebar like any
    /// session but is not selected (the user's focus stays where it is) and is
    /// never saved or restored.
    pub fn open_agent_session(&mut self, host: &Host, link: AgentLink, name: Option<&str>) -> Uuid {
        let mut managed = self.make(host, None);
        managed.number = self.next_number(host.id);
        managed.custom_name = name
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .map(str::to_string);
        managed.agent = Some(link);
        let id = managed.id;
        diag::event(
            "session.agent",
            &[("session", id.to_string()), ("host", host.id.to_string())],
        );
        self.sessions.push(managed);
        id
    }

    /// Hands a session the user opened to a client. A session an agent opened
    /// earlier stays ephemeral; the user's own does not.
    pub fn attach_agent(&mut self, id: Uuid, mut link: AgentLink) {
        if let Some(m) = self.sessions.iter_mut().find(|m| m.id == id) {
            link.handed_off = link.handed_off && m.former_agent.is_none();
            m.agent = Some(link);
        }
    }

    /// Agent control ended; the session stays open, marked, until the user
    /// keeps or closes it. A handed-off session simply goes back to the user.
    pub fn end_agent_control(&mut self, id: Uuid) {
        if let Some(m) = self.sessions.iter_mut().find(|m| m.id == id) {
            if let Some(link) = m.agent.take() {
                m.former_agent = (!link.handed_off).then_some(link.client_name);
            }
        }
    }

    /// Turns an agent session into an ordinary saved session.
    pub fn keep_session(&mut self, id: Uuid) {
        if let Some(m) = self.sessions.iter_mut().find(|m| m.id == id) {
            m.agent = None;
            m.former_agent = None;
        }
        self.checkpoint_quietly();
    }

    pub fn agent_link(&self, id: Uuid) -> Option<AgentLink> {
        self.sessions
            .iter()
            .find(|m| m.id == id)
            .and_then(|m| m.agent.clone())
    }

    // MARK: closing

    /// Explicit close deletes the encrypted record first (a failure keeps the
    /// session visible), then stops the process.
    pub fn close_session(&mut self, id: Uuid) -> Result<(), String> {
        let Some(index) = self.sessions.iter().position(|m| m.id == id) else {
            return Ok(());
        };
        if let (Some(saved), Some((file, unlocked))) = (self.sessions[index].saved_id, &self.vault)
        {
            if let Err(error) =
                file.update_local_state(unlocked, |l| l.sessions.retain(|s| s.id != saved))
            {
                let message = format!("Session not closed: {error}");
                self.save_failure = Some(message.clone());
                return Err(message);
            }
            self.save_failure = None;
        }
        let closed = self.sessions.remove(index);
        if let Some(term) = &closed.term {
            term.terminate();
        }
        if self.selected == Some(id) {
            self.selected = self
                .sessions
                .iter()
                .rev()
                .find(|m| m.host.id == closed.host.id)
                .or(self.sessions.last())
                .map(|m| m.id);
        }
        Ok(())
    }

    pub fn close_all(&mut self) {
        for id in self.sessions.iter().map(|m| m.id).collect::<Vec<_>>() {
            let _ = self.close_session(id);
        }
    }

    pub fn close_sessions_for_host(&mut self, host_id: Uuid) {
        for id in self.sessions_for_host(host_id) {
            let _ = self.close_session(id);
        }
    }

    /// Closes sessions whose host disappeared from an adopted vault and drops
    /// their records; every surviving host keeps its running terminal.
    pub fn close_missing_hosts(&mut self, hosts: &[Host]) {
        let surviving: std::collections::HashSet<Uuid> = hosts.iter().map(|h| h.id).collect();
        if let Some((file, unlocked)) = &self.vault {
            let _ = file.update_local_state(unlocked, |l| {
                l.sessions.retain(|s| surviving.contains(&s.host_id))
            });
        }
        let vanished: Vec<Uuid> = self
            .sessions
            .iter()
            .filter(|m| !surviving.contains(&m.host.id))
            .map(|m| m.id)
            .collect();
        for id in &vanished {
            if let Some(m) = self.sessions.iter().find(|m| m.id == *id) {
                if let Some(term) = &m.term {
                    term.terminate();
                }
            }
        }
        self.sessions.retain(|m| surviving.contains(&m.host.id));
        if self.selected.is_some_and(|s| vanished.contains(&s)) {
            self.selected = self.sessions.last().map(|m| m.id);
        }
    }

    /// Quit ends processes but keeps their records, unlike an explicit close.
    pub fn shutdown(&mut self) {
        diag::event(
            "session.shutdown",
            &[("open", self.sessions.len().to_string())],
        );
        let _ = self.checkpoint();
        self.stop_for_quit();
    }

    pub fn stop_for_quit(&mut self) {
        for m in &self.sessions {
            if let Some(term) = &m.term {
                term.terminate();
            }
        }
    }

    // MARK: naming and persistence

    pub fn rename(&mut self, id: Uuid, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("Enter a session name.".into());
        }
        let Some(m) = self.sessions.iter_mut().find(|m| m.id == id) else {
            return Ok(());
        };
        if let (Some(saved), Some((file, unlocked))) = (m.saved_id, &self.vault) {
            file.update_local_state(unlocked, |l| {
                if let Some(s) = l.sessions.iter_mut().find(|s| s.id == saved) {
                    s.name = name.to_string();
                    s.uses_default_name = Some(false);
                }
            })
            .map_err(|e| e.to_string())?;
        }
        m.custom_name = Some(name.to_string());
        Ok(())
    }

    /// Saves names and scrollback of every session into the device-local file.
    pub fn checkpoint(&mut self) -> Result<(), String> {
        let Some((file, unlocked)) = self.vault.clone() else {
            return Ok(());
        };
        if self.sessions.is_empty() {
            return Ok(());
        }
        let stored = file
            .local_state(&unlocked)
            .map_err(|e| e.to_string())?
            .sessions;
        let mut changes: Vec<SavedSession> = Vec::new();
        for m in self.sessions.iter().filter(|m| !m.is_ephemeral()) {
            let id = m.saved_id.unwrap_or(m.id);
            let history = m
                .term
                .as_ref()
                .map(|t| t.history_text(HISTORY_LIMIT))
                .unwrap_or_default();
            let name = m.label();
            if stored
                .iter()
                .any(|s| s.id == id && s.name == name && s.history == history)
            {
                continue;
            }
            changes.push(SavedSession {
                id,
                host_id: m.host.id,
                name,
                history,
                saved_at: now_apple(),
                uses_default_name: Some(m.custom_name.is_none()),
                number: Some(m.number),
            });
        }
        if !changes.is_empty() {
            file.update_local_state(&unlocked, |l| {
                for change in changes {
                    match l.sessions.iter_mut().find(|s| s.id == change.id) {
                        Some(existing) => *existing = change,
                        None => l.sessions.push(change),
                    }
                }
            })
            .map_err(|e| e.to_string())?;
            self.save_failure = None;
        }
        for m in self.sessions.iter_mut().filter(|m| !m.is_ephemeral()) {
            m.saved_id.get_or_insert(m.id);
        }
        Ok(())
    }

    fn checkpoint_quietly(&mut self) {
        if let Err(message) = self.checkpoint() {
            self.save_failure = Some(message);
        }
    }

    /// Re-opens saved sessions whose host still exists, with their scrollback,
    /// and reconnects them. Sessions already open are skipped.
    pub fn restore(&mut self, hosts: &[Host]) -> Result<(), String> {
        let Some((file, unlocked)) = self.vault.clone() else {
            return Ok(());
        };
        let saved = file
            .local_state(&unlocked)
            .map_err(|e| e.to_string())?
            .sessions;
        for record in saved {
            if self.sessions.iter().any(|m| m.saved_id == Some(record.id)) {
                continue;
            }
            let Some(host) = hosts.iter().find(|h| h.id == record.host_id) else {
                continue;
            };
            let generated = record
                .uses_default_name
                .unwrap_or_else(|| is_default_name(&record.name));
            let mut managed = self.make(host, Some(&record.history));
            managed.custom_name = (!generated).then(|| record.name.clone());
            managed.saved_id = Some(record.id);
            managed.number = record
                .number
                .or_else(|| {
                    generated
                        .then(|| record.name.rsplit(' ').next().and_then(|n| n.parse().ok()))
                        .flatten()
                })
                .unwrap_or_else(|| {
                    self.sessions
                        .iter()
                        .filter(|m| m.host.id == host.id)
                        .count() as i64
                        + 1
                });
            self.sessions.push(managed);
        }
        if self.selected.is_none() {
            self.selected = self.sessions.first().map(|m| m.id);
        }
        Ok(())
    }

    /// Stops and forgets every session without deleting its record. Used when
    /// the live vault file is swapped for another workspace.
    pub fn detach_all(&mut self) {
        let _ = self.checkpoint();
        self.stop_for_quit();
        self.sessions.clear();
        self.selected = None;
    }
}

fn is_default_name(name: &str) -> bool {
    name.strip_prefix("Session ")
        .is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit()))
}

fn now_apple() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
        - APPLE_EPOCH
}
