//! The tools behind the MCP surface (`docs/mcp.md`, Tools). Runs in the app,
//! next to the sessions it drives. Every id an agent passes is checked against
//! device-local access and the calling client; a host or session it may not use
//! answers exactly like one that doesn't exist.

use crate::audit::{AgentAuditEntry, AuditKind};
use crate::catalog;
use crate::clients::AgentClient;
use crate::driver::{AgentDriver, AgentKey, DriverError};
use crate::host::{AgentHost, AgentSessionRef, CONNECTING_LABEL};
use crate::json::{json_text, uuid_text, ValueExt};
use crate::protocol::{AgentCall, AgentReply};
use serde_json::{json, Value};
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::Arc;
use std::time::{Duration, Instant};
use uuid::Uuid;

/// Answers tool calls for an authenticated client. [`AgentToolHandler`] in the
/// app; tests use small fakes. Blocking: the server calls it on a thread per call.
pub trait AgentCallHandler: Send + Sync {
    fn handle(&self, call: AgentCall, client: &AgentClient) -> AgentReply;
}

pub const LOCKED: &str = "Maine is locked. Ask the user to unlock it.";
pub const NOT_FOUND: &str = "not found";

#[derive(Debug)]
struct ToolError(String);

impl ToolError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    fn not_found() -> Self {
        Self::new(NOT_FOUND)
    }
}

impl From<DriverError> for ToolError {
    fn from(error: DriverError) -> Self {
        Self(error.to_string())
    }
}

pub struct AgentToolHandler {
    host: Arc<dyn AgentHost>,
}

/// Swift's `UUID(uuidString:)`: the hyphenated spelling, either case.
fn parse_uuid(text: &str) -> Option<Uuid> {
    if text.len() != 36 {
        return None;
    }
    Uuid::parse_str(text).ok()
}

impl AgentToolHandler {
    pub fn new(host: Arc<dyn AgentHost>) -> Self {
        Self { host }
    }

    // MARK: - Limits

    /// `run`'s timeout: 1 to 600 s, default 60.
    pub fn run_timeout(seconds: Option<i64>) -> Duration {
        Duration::from_secs(seconds.unwrap_or(60).clamp(1, 600) as u64)
    }

    /// A read's wait: 0 to 60 s; `None` means do not wait.
    pub fn read_wait(seconds: Option<i64>) -> Option<Duration> {
        seconds.map(|s| Duration::from_secs(s.clamp(0, 60) as u64))
    }

    /// A read's size: 1 to 65536 bytes, default 65536.
    pub fn max_bytes(value: Option<i64>) -> usize {
        value.unwrap_or(65_536).clamp(1, 65_536) as usize
    }

    // MARK: - Dispatch

    fn handle_call(&self, call: AgentCall, client: &AgentClient) -> AgentReply {
        let started = Instant::now();
        let reply = match self.perform(&call, client) {
            Ok(result) => AgentReply::success(call.id, result),
            Err(error) => AgentReply::failure(call.id, error.0),
        };
        // Metadata only: never arguments, output, host names or tokens.
        let session = call
            .arguments
            .string("session_id")
            .and_then(parse_uuid)
            .map(|id| uuid_text(&id))
            .unwrap_or_default();
        self.host.diagnostics(
            "agent.tool",
            &[
                (
                    "tool",
                    if catalog::contains(&call.tool) {
                        call.tool.clone()
                    } else {
                        "unknown".into()
                    },
                ),
                ("client", uuid_text(&client.id)),
                ("session", session),
                (
                    "outcome",
                    if reply.error.is_none() { "ok" } else { "error" }.into(),
                ),
                ("ms", started.elapsed().as_millis().to_string()),
                (
                    "bytes",
                    reply
                        .result
                        .as_ref()
                        .map_or(0, |result| json_text(result).len())
                        .to_string(),
                ),
            ],
        );
        reply
    }

    fn perform(&self, call: &AgentCall, client: &AgentClient) -> Result<Value, ToolError> {
        if !self.host.is_unlocked() {
            return Err(ToolError::new(LOCKED));
        }
        let arguments = &call.arguments;
        match call.tool.as_str() {
            "list_hosts" => Ok(self.list_hosts()),
            "list_sessions" => Ok(self.list_sessions(client)),
            "open_session" => self.open_session(arguments, client),
            "run" => self.run(arguments, client),
            "read_output" => {
                let (_, driver) = self.resolve(arguments, client)?;
                let read = driver.read_output(
                    arguments.int("since"),
                    Self::read_wait(arguments.int("wait_seconds")),
                    Self::max_bytes(arguments.int("max_bytes")),
                );
                Ok(
                    json!({"output": read.output, "cursor": read.cursor, "busy": read.busy, "truncated": read.truncated}),
                )
            }
            "read_screen" => {
                let (_, driver) = self.resolve(arguments, client)?;
                let screen = driver.read_screen(Self::read_wait(arguments.int("wait_seconds")));
                Ok(json!({
                    "rows": screen.rows, "cursor_row": screen.cursor_row, "cursor_col": screen.cursor_col,
                    "alternate_screen": screen.alternate_screen, "changed": screen.changed, "busy": screen.busy
                }))
            }
            "send_keys" => self.send_keys(arguments, client),
            "type_password" => {
                let (session, driver) = self.resolve(arguments, client)?;
                let password = match self.host.saved_password(session.host_id) {
                    Some(password) if !password.is_empty() => password,
                    _ => return Err(ToolError::new("no saved password for this host")),
                };
                driver.type_password(&password)?;
                self.record(AuditKind::Password, &session, client, None, None);
                Ok(json!({"typed": true}))
            }
            "close_session" => {
                let (session, _) = self.resolve(arguments, client)?;
                self.record(AuditKind::Close, &session, client, None, None);
                self.host.close_session(session.id);
                Ok(json!({"closed": true}))
            }
            other => Err(ToolError::new(format!("unknown tool: {other}"))),
        }
    }

    fn list_hosts(&self) -> Value {
        let layout = self.host.layout();
        let hosts: Vec<Value> = self
            .host
            .access()
            .allowed_hosts(&layout)
            .into_iter()
            .map(|host| {
                json!({
                    "id": uuid_text(&host.id),
                    "name": host.display_name(),
                    "group": layout.group(host.group_id).map(|group| group.name.clone()),
                    "hostname": host.hostname,
                    "port": host.port,
                    "username": host.username,
                    "has_saved_password": self.host.has_saved_password(host.id),
                })
            })
            .collect();
        json!({"hosts": hosts})
    }

    fn list_sessions(&self, client: &AgentClient) -> Value {
        let sessions: Vec<Value> = self
            .host
            .sessions()
            .into_iter()
            .filter(|session| session.driver.as_ref().is_some_and(|d| d.client_id() == client.id) && self.allowed(session))
            .map(|session| {
                let connecting = session.is_live && session.driver.as_ref().is_some_and(|d| !d.is_ready());
                json!({
                    "session_id": uuid_text(&session.id),
                    "host_id": uuid_text(&session.host_id),
                    "name": session.name,
                    "state": if connecting { CONNECTING_LABEL.to_string() } else { session.state_label.clone() },
                    "busy": session.driver.as_ref().is_some_and(|d| d.is_busy()),
                })
            })
            .collect();
        json!({"sessions": sessions})
    }

    fn open_session(&self, arguments: &Value, client: &AgentClient) -> Result<Value, ToolError> {
        let host = arguments
            .string("host_id")
            .and_then(parse_uuid)
            .and_then(|id| self.host.host(id))
            .filter(|host| self.host.access().allows(host))
            .ok_or_else(ToolError::not_found)?;
        let opened = self
            .host
            .open_agent_session(&host, client, arguments.string("name"))
            .map_err(ToolError::new)?;
        let driver = opened.driver.clone().ok_or_else(ToolError::not_found)?;
        let login = driver.settle_after_login(Duration::from_secs(30));
        let current = self.host.session(opened.id).unwrap_or(opened);
        if let Some(notice) = current.notice.clone().filter(|_| !current.is_live) {
            // Nothing for the agent to drive; the user sees the failure.
            self.end_control(&current, &driver);
            return Err(ToolError::new(notice));
        }
        self.record(AuditKind::Open, &current, client, None, None);
        let ready = driver.is_ready();
        let mut result = json!({
            "session_id": uuid_text(&current.id),
            "state": if ready { current.state_label.clone() } else { CONNECTING_LABEL.to_string() },
            "ready": ready,
            "output": login.output,
        });
        if !ready {
            result["hint"] = json!(
                "The remote shell is not ready, so run is refused. Use read_screen; if ssh waits at a password or \
                 host-key prompt, answer it with type_password or send_keys."
            );
        }
        Ok(result)
    }

    fn run(&self, arguments: &Value, client: &AgentClient) -> Result<Value, ToolError> {
        let (session, driver) = self.resolve(arguments, client)?;
        let command = arguments
            .string("command")
            .ok_or_else(|| ToolError::new("command is required"))?;
        let stop = || !self.allowed(&session);
        let result = match driver.run(
            command,
            Self::run_timeout(arguments.int("timeout_seconds")),
            Some(&stop),
        ) {
            Ok(result) => result,
            Err(DriverError::Stopped) => {
                self.end_control(&session, &driver);
                return Err(ToolError::not_found());
            }
            Err(error) => return Err(error.into()),
        };
        if !self.allowed(&session) {
            self.end_control(&session, &driver);
            return Err(ToolError::not_found());
        }
        self.record(
            AuditKind::Run,
            &session,
            client,
            Some(command),
            result.exit_code,
        );
        Ok(json!({
            "completed": result.completed, "exit_code": result.exit_code, "output": result.output,
            "cursor": result.cursor, "truncated": result.truncated, "mode": result.mode,
        }))
    }

    fn send_keys(&self, arguments: &Value, client: &AgentClient) -> Result<Value, ToolError> {
        let (session, driver) = self.resolve(arguments, client)?;
        let raw: Vec<&str> = arguments
            .array("keys")
            .map(|keys| keys.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        if raw.is_empty() {
            return Err(ToolError::new("keys is required"));
        }
        let mut keys = Vec::with_capacity(raw.len());
        for name in &raw {
            keys.push(
                AgentKey::parse(name)
                    .ok_or_else(|| ToolError::new(format!("unknown key: {name}")))?,
            );
        }
        driver.send_keys(&keys)?;
        // Names only: literal text may itself be a secret the agent typed.
        let detail: Vec<String> = raw
            .iter()
            .map(|key| match key.strip_prefix("text:") {
                Some(text) => format!("text({} chars)", text.chars().count()),
                None => (*key).to_string(),
            })
            .collect();
        self.record(
            AuditKind::Keys,
            &session,
            client,
            Some(&detail.join(", ")),
            None,
        );
        Ok(json!({"sent": keys.len()}))
    }

    fn record(
        &self,
        kind: AuditKind,
        session: &AgentSessionRef,
        client: &AgentClient,
        detail: Option<&str>,
        exit: Option<i32>,
    ) {
        let host_name = self
            .host
            .host(session.host_id)
            .map(|host| host.display_name())
            .unwrap_or_default();
        self.host.record_audit(AgentAuditEntry::new(
            client.name.clone(),
            host_name,
            session.id,
            kind,
            detail,
            exit,
        ));
    }

    /// The caller's own session on a host it may still use, or `not found`.
    fn resolve(
        &self,
        arguments: &Value,
        client: &AgentClient,
    ) -> Result<(AgentSessionRef, Arc<AgentDriver>), ToolError> {
        let session = arguments
            .string("session_id")
            .and_then(parse_uuid)
            .and_then(|id| self.host.session(id))
            .ok_or_else(ToolError::not_found)?;
        let driver = session
            .driver
            .clone()
            .filter(|driver| driver.client_id() == client.id)
            .ok_or_else(ToolError::not_found)?;
        if !self.allowed(&session) {
            self.end_control(&session, &driver);
            return Err(ToolError::not_found());
        }
        if driver.is_paused() {
            return Err(DriverError::Paused.into());
        }
        Ok((session, driver))
    }

    fn end_control(&self, session: &AgentSessionRef, driver: &AgentDriver) {
        driver.detach();
        self.host.end_agent_control(session.id);
    }

    /// Access is judged on the host as it is now (its group may have changed).
    fn allowed(&self, session: &AgentSessionRef) -> bool {
        self.host
            .host(session.host_id)
            .is_some_and(|host| self.host.access().allows(&host))
    }
}

impl AgentCallHandler for AgentToolHandler {
    fn handle(&self, call: AgentCall, client: &AgentClient) -> AgentReply {
        let id = call.id;
        match catch_unwind(AssertUnwindSafe(|| self.handle_call(call, client))) {
            Ok(reply) => reply,
            Err(_) => AgentReply::failure(id, "internal error"),
        }
    }
}
