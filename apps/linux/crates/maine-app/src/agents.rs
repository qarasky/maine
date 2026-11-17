//! MCP agent access: the app's implementation of `maine_agents::AgentHost` over
//! the engine, the coordinator singleton and the Settings state. Opt-in, scoped
//! to allowed hosts, local only (AGENTS.md invariant 12).

use crate::secrets::AgentSecrets;
use crate::state::with_engine;
use maine_agents::{
    AgentAccess, AgentAuditEntry, AgentClientStore, AgentCoordinator, AgentDriver, AgentHost,
    AgentSession, AgentSessionRef, SystemClock, TermAgentSession,
};
use maine_agents::{AgentClient, AgentError};
use maine_core::model::{Host, HostLayout};
use maine_engine::{AgentLink, Paths};
use maine_term::TermSession;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use uuid::Uuid;
use zeroize::Zeroizing;

static COORDINATOR: OnceLock<AgentCoordinator> = OnceLock::new();
static CONFIG_DIR: OnceLock<PathBuf> = OnceLock::new();
static DRIVERS: Mutex<Option<HashMap<Uuid, Arc<AgentDriver>>>> = Mutex::new(None);
static ADAPTERS: Mutex<Option<HashMap<Uuid, Arc<TermAgentSession>>>> = Mutex::new(None);

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The one adapter per session: the driver, the key handler and the UI share it.
pub fn adapter_for(session_id: Uuid, term: &TermSession) -> Arc<TermAgentSession> {
    lock(&ADAPTERS)
        .get_or_insert_with(HashMap::new)
        .entry(session_id)
        .or_insert_with(|| Arc::new(TermAgentSession::new(term.clone())))
        .clone()
}

fn driver_of(session_id: Uuid) -> Option<Arc<AgentDriver>> {
    lock(&DRIVERS)
        .as_ref()
        .and_then(|m| m.get(&session_id).cloned())
}

fn set_driver(session_id: Uuid, driver: Option<Arc<AgentDriver>>) {
    let mut guard = lock(&DRIVERS);
    let map = guard.get_or_insert_with(HashMap::new);
    match driver {
        Some(d) => {
            map.insert(session_id, d);
        }
        None => {
            map.remove(&session_id);
        }
    }
}

struct AppAgentHost;

impl AppAgentHost {
    fn session_ref(info: maine_engine::SessionInfo, term: &TermSession) -> AgentSessionRef {
        let terminal: Arc<dyn AgentSession> = adapter_for(info.id, term);
        AgentSessionRef {
            id: info.id,
            host_id: info.host_id,
            name: info.label,
            state_label: info.state.label(),
            is_live: info.state.is_live(),
            notice: info.state.notice(),
            terminal,
            driver: driver_of(info.id),
        }
    }
}

impl AgentHost for AppAgentHost {
    fn is_unlocked(&self) -> bool {
        with_engine(|e| e.unlocked().is_some())
    }

    fn layout(&self) -> HostLayout {
        with_engine(|e| e.layout().clone())
    }

    fn host(&self, id: Uuid) -> Option<Host> {
        with_engine(|e| e.host(id).cloned())
    }

    fn access(&self) -> AgentAccess {
        with_engine(|e| {
            let key = e.unlocked()?;
            let state = e.vault_file().local_state(key).ok()?;
            AgentAccess::from_value(state.agent_access.as_ref()?).ok()
        })
        .unwrap_or_default()
    }

    fn store_access(&self, access: &AgentAccess) -> Result<(), AgentError> {
        with_engine(|e| {
            let key = e
                .unlocked()
                .ok_or_else(|| AgentError::new("The vault is locked."))?
                .clone();
            let value = access.to_value();
            e.vault_file()
                .update_local_state(&key, |l| l.agent_access = Some(value))
                .map_err(|err| AgentError::new(err.to_string()))
        })
    }

    fn has_saved_password(&self, host_id: Uuid) -> bool {
        with_engine(|e| e.has_saved_password(host_id))
    }

    fn saved_password(&self, host_id: Uuid) -> Option<Zeroizing<String>> {
        with_engine(|e| e.saved_password(host_id)).map(Zeroizing::new)
    }

    fn open_agent_session(
        &self,
        host: &Host,
        client: &AgentClient,
        name: Option<&str>,
    ) -> Result<AgentSessionRef, String> {
        let link = AgentLink {
            client_id: client.id,
            client_name: client.name.clone(),
            handed_off: false,
        };
        let (id, term, failure) = with_engine(|e| {
            let id = e.sessions_mut().open_agent_session(host, link, name);
            let failure = e.sessions().state(id).and_then(|s| s.notice());
            (id, e.sessions().terminal(id), failure)
        });
        let Some(term) = term else {
            with_engine(|e| e.sessions_mut().close_session(id).ok());
            return Err(failure.unwrap_or_else(|| "The session could not be started.".into()));
        };
        let adapter = adapter_for(id, &term);
        let session: Arc<dyn AgentSession> = adapter;
        let driver = AgentDriver::new(
            session,
            client.id,
            client.name.clone(),
            Arc::new(SystemClock),
        );
        set_driver(id, Some(driver));
        let info = with_engine(|e| e.sessions().list().into_iter().find(|s| s.id == id))
            .ok_or("The session closed.")?;
        Ok(Self::session_ref(info, &term))
    }

    fn sessions(&self) -> Vec<AgentSessionRef> {
        let items = with_engine(|e| {
            e.sessions()
                .list()
                .into_iter()
                .map(|info| {
                    let term = e.sessions().terminal(info.id);
                    (info, term)
                })
                .collect::<Vec<_>>()
        });
        let live: std::collections::HashSet<Uuid> = items.iter().map(|(i, _)| i.id).collect();
        // Forget adapters and drivers of sessions that no longer exist.
        lock(&ADAPTERS)
            .get_or_insert_with(HashMap::new)
            .retain(|id, _| live.contains(id));
        lock(&DRIVERS)
            .get_or_insert_with(HashMap::new)
            .retain(|id, _| live.contains(id));
        items
            .into_iter()
            .filter_map(|(info, term)| term.map(|t| Self::session_ref(info, &t)))
            .collect()
    }

    fn attach_agent(&self, session_id: Uuid, driver: Arc<AgentDriver>, handed_off: bool) {
        let link = AgentLink {
            client_id: driver.client_id(),
            client_name: driver.client_name().to_string(),
            handed_off,
        };
        set_driver(session_id, Some(driver));
        with_engine(|e| e.sessions_mut().attach_agent(session_id, link));
    }

    fn end_agent_control(&self, session_id: Uuid) {
        set_driver(session_id, None);
        with_engine(|e| e.sessions_mut().end_agent_control(session_id));
    }

    fn close_session(&self, session_id: Uuid) {
        set_driver(session_id, None);
        with_engine(|e| e.sessions_mut().close_session(session_id).ok());
    }

    fn record_audit(&self, entry: AgentAuditEntry) {
        with_engine(|e| {
            let Some(key) = e.unlocked().cloned() else {
                return;
            };
            let _ = e.vault_file().update_local_state(&key, |l| {
                let mut entries =
                    AgentAuditEntry::list_from_value(&Value::Array(l.agent_audit.clone()))
                        .unwrap_or_default();
                AgentAuditEntry::append(&mut entries, [entry]);
                if let Value::Array(list) = AgentAuditEntry::list_to_value(&entries) {
                    l.agent_audit = list;
                }
            });
        });
    }

    fn diagnostics(&self, event: &str, fields: &[(&str, String)]) {
        maine_engine::diag::event(event, fields);
    }
}

fn pref_path() -> Option<PathBuf> {
    CONFIG_DIR.get().map(|d| d.join("agents.json"))
}

fn saved_enabled() -> bool {
    pref_path()
        .and_then(|p| std::fs::read(p).ok())
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
        .and_then(|v| v["enabled"].as_bool())
        .unwrap_or(false)
}

fn save_enabled(enabled: bool) {
    if let Some(p) = pref_path() {
        if let Some(dir) = p.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(p, json!({"enabled": enabled}).to_string());
    }
}

/// Builds the coordinator (the master switch comes from the saved preference; default off).
pub fn init(paths: &Paths) {
    let _ = CONFIG_DIR.set(paths.config_dir.clone());
    let clients = Arc::new(AgentClientStore::new(Arc::new(AgentSecrets)));
    let coordinator = AgentCoordinator::new(
        Arc::new(AppAgentHost),
        clients,
        paths.agent_socket(),
        Arc::new(SystemClock),
    );
    let _ = COORDINATOR.set(coordinator);
    if saved_enabled() {
        coordinator_ref().set_enabled(true);
    }
}

fn coordinator_ref() -> &'static AgentCoordinator {
    COORDINATOR.get().expect("agents initialised in main")
}

pub fn coordinator() -> Option<&'static AgentCoordinator> {
    COORDINATOR.get()
}

pub fn set_enabled(enabled: bool) {
    if let Some(c) = coordinator() {
        c.set_enabled(enabled);
        save_enabled(enabled);
    }
}

pub fn vault_did_lock() {
    if let Some(c) = coordinator() {
        c.vault_did_lock();
    }
}

pub fn vault_did_unlock() {
    if let Some(c) = coordinator() {
        c.vault_did_unlock();
    }
}

/// Adds the agent bits the sidebar needs to the engine's JSON: whether each
/// agent-driven session is paused.
pub fn decorate(snapshot: &mut Value) {
    let paused = |id: &str| {
        Uuid::parse_str(id)
            .ok()
            .and_then(driver_of)
            .is_some_and(|d| d.is_paused())
    };
    let mut mark = |session: &mut Value| {
        let id = session["id"].as_str().unwrap_or_default().to_string();
        session["agentPaused"] = json!(!session["agent"].is_null() && paused(&id));
    };
    if let Some(list) = snapshot["sessions"].as_array_mut() {
        list.iter_mut().for_each(&mut mark);
    }
    for host in snapshot["ungrouped"].as_array_mut().into_iter().flatten() {
        host["sessions"]
            .as_array_mut()
            .into_iter()
            .flatten()
            .for_each(&mut mark);
    }
    for group in snapshot["groups"].as_array_mut().into_iter().flatten() {
        for host in group["hosts"].as_array_mut().into_iter().flatten() {
            host["sessions"]
                .as_array_mut()
                .into_iter()
                .flatten()
                .for_each(&mut mark);
        }
    }
}

/// Settings state: switch, paired clients, access rules, activity.
pub fn json() -> Value {
    let Some(c) = coordinator() else {
        return json!({ "ready": false });
    };
    let access = c.access();
    json!({
        "ready": true,
        "enabled": c.is_enabled(),
        "active": c.active_count(),
        "error": c.last_error(),
        "clients": c.clients().iter().map(|cl| json!({"id": cl.id.to_string(), "name": cl.name})).collect::<Vec<_>>(),
        "groups": access.groups.iter().map(|g| g.to_string()).collect::<Vec<_>>(),
        "hosts": access.hosts.iter().map(|h| h.to_string()).collect::<Vec<_>>(),
    })
}

/// The activity log, newest first.
pub fn audit_json() -> Value {
    let entries = with_engine(|e| {
        let key = e.unlocked()?;
        e.vault_file().local_state(key).ok().map(|l| l.agent_audit)
    })
    .unwrap_or_default();
    let parsed = AgentAuditEntry::list_from_value(&Value::Array(entries)).unwrap_or_default();
    Value::Array(
        parsed
            .iter()
            .rev()
            .map(|a| serde_json::to_value(a).unwrap_or(Value::Null))
            .collect(),
    )
}

pub fn clear_audit() {
    with_engine(|e| {
        if let Some(key) = e.unlocked().cloned() {
            let _ = e
                .vault_file()
                .update_local_state(&key, |l| l.agent_audit.clear());
        }
    });
}

pub fn pause(session_id: Uuid, paused: bool) {
    if let Some(d) = driver_of(session_id) {
        d.set_paused(paused);
    }
}
