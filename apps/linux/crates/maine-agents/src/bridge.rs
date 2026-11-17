//! `maine --mcp`: an MCP server over stdio (one JSON-RPC 2.0 message per line)
//! that forwards tool calls to the running app over the agent socket. It never
//! creates a window and never launches Maine; it holds no state beyond the
//! connection.

use crate::catalog;
use crate::client::{AgentSocketClient, ClientError};
use crate::json::{json_text, ValueExt};
use crate::protocol::AgentHello;
use crate::server::{default_socket_path, NOT_PAIRED};
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

pub const NOT_RUNNING: &str = "Maine is not running. Open Maine and unlock the vault.";
pub const LOST: &str = "Maine closed the connection before answering; the call may have run.";
/// MCP protocol versions the bridge speaks; a requested one is echoed back,
/// anything else gets the first.
pub const SUPPORTED_VERSIONS: [&str; 3] = ["2025-06-18", "2025-03-26", "2024-11-05"];

const INSTRUCTIONS: &str = "Maine runs real SSH sessions, on hosts the user allowed, in terminals the user can watch. Use run for commands (it returns output and the exit code), read_output for long-running commands, read_screen (optionally waiting for a change) for full-screen programs, prompts and multi-line progress, and send_keys to answer prompts or interrupt with ctrl-c.";

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

enum Outcome {
    Success(Value),
    Failure(String),
}

/// The bridge's per-process state: the lazily opened, re-openable app connection.
pub struct McpBridgeSession {
    socket_path: PathBuf,
    token: Option<String>,
    server_version: String,
    client: Mutex<Option<Arc<AgentSocketClient>>>,
    /// One connection attempt at a time, shared by concurrent first calls.
    connecting: Mutex<()>,
}

impl McpBridgeSession {
    pub fn new(socket_path: PathBuf, token: Option<String>) -> Self {
        Self {
            socket_path,
            token,
            server_version: env!("CARGO_PKG_VERSION").to_string(),
            client: Mutex::new(None),
            connecting: Mutex::new(()),
        }
    }

    /// The version reported as `serverInfo.version` (the app's version).
    pub fn with_server_version(mut self, version: impl Into<String>) -> Self {
        self.server_version = version.into();
        self
    }

    /// The reply line for one incoming line, or `None` for notifications.
    pub fn handle_line(&self, line: &str) -> Option<String> {
        let message = match serde_json::from_str::<Value>(line) {
            Ok(message @ Value::Object(_)) => message,
            _ => return Some(error_line(Value::Null, -32700, "Parse error")),
        };
        let method = message.string("method")?;
        let id = message.get("id").filter(|id| !id.is_null())?.clone(); // no id: a notification
        let empty = json!({});
        let params = message.get("params").unwrap_or(&empty);
        Some(match method {
            "initialize" => {
                let requested = params.string("protocolVersion").unwrap_or("");
                let version = if SUPPORTED_VERSIONS.contains(&requested) {
                    requested
                } else {
                    SUPPORTED_VERSIONS[0]
                };
                result_line(
                    id,
                    json!({
                        "protocolVersion": version,
                        "capabilities": {"tools": {"listChanged": false}},
                        "serverInfo": {"name": "maine", "version": self.server_version},
                        "instructions": INSTRUCTIONS,
                    }),
                )
            }
            "ping" => result_line(id, json!({})),
            "tools/list" => {
                let tools: Vec<Value> = catalog::tools()
                    .iter()
                    .map(|tool| {
                        json!({"name": tool.name, "description": tool.description, "inputSchema": tool.input_schema})
                    })
                    .collect();
                result_line(id, json!({"tools": tools}))
            }
            "tools/call" => {
                let Some(name) = params.string("name").filter(|name| catalog::contains(name))
                else {
                    return Some(error_line(id, -32602, "Unknown tool"));
                };
                let arguments = params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                match self.call(name, arguments) {
                    Outcome::Success(result) => result_line(
                        id,
                        json!({
                            "content": [{"type": "text", "text": json_text(&result)}],
                            "structuredContent": result,
                            "isError": false,
                        }),
                    ),
                    Outcome::Failure(message) => result_line(
                        id,
                        json!({"content": [{"type": "text", "text": message}], "isError": true}),
                    ),
                }
            }
            _ => error_line(id, -32601, "Method not found"),
        })
    }

    /// A connection found closed before a call is sent is replaced (the app
    /// restarted); a call that was already sent is never re-sent, because it
    /// may have run.
    fn call(&self, tool: &str, arguments: Value) -> Outcome {
        for _ in 0..2 {
            let connected = match self.connect() {
                Ok(connected) => connected,
                Err(message) => return Outcome::Failure(message),
            };
            match connected.call(tool, arguments.clone()) {
                Ok(reply) => {
                    return match reply.error {
                        Some(error) => Outcome::Failure(error),
                        None => Outcome::Success(reply.result.unwrap_or_else(|| json!({}))),
                    };
                }
                Err(ClientError::NotSent) => {
                    self.forget(&connected);
                }
                Err(_) => {
                    self.forget(&connected);
                    return Outcome::Failure(LOST.to_string());
                }
            }
        }
        Outcome::Failure(NOT_RUNNING.to_string())
    }

    fn forget(&self, connected: &Arc<AgentSocketClient>) {
        let mut slot = lock(&self.client);
        if slot
            .as_ref()
            .is_some_and(|current| Arc::ptr_eq(current, connected))
        {
            *slot = None;
        }
    }

    fn connect(&self) -> Result<Arc<AgentSocketClient>, String> {
        let _attempt = lock(&self.connecting);
        if let Some(existing) = lock(&self.client).clone() {
            if !existing.is_closed() {
                return Ok(existing);
            }
        }
        *lock(&self.client) = None;
        let fresh =
            AgentSocketClient::connect(&self.socket_path).map_err(|_| NOT_RUNNING.to_string())?;
        let Some(token) = &self.token else {
            fresh.close();
            return Err(NOT_PAIRED.to_string());
        };
        let reply = fresh
            .hello(&AgentHello {
                token: token.clone(),
                client_name: None,
            })
            .map_err(|_| NOT_RUNNING.to_string())?;
        if !reply.ok {
            fresh.close();
            return Err(reply.error.unwrap_or_else(|| NOT_PAIRED.to_string()));
        }
        let fresh = Arc::new(fresh);
        *lock(&self.client) = Some(fresh.clone());
        Ok(fresh)
    }
}

fn result_line(id: Value, result: Value) -> String {
    json_text(&json!({"jsonrpc": "2.0", "id": id, "result": result}))
}

fn error_line(id: Value, code: i64, message: &str) -> String {
    json_text(&json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}))
}

/// Reads `input` until EOF, answering each line on `output`. Every line is
/// handled on its own thread (calls may take a long time and may overlap);
/// replies are written one at a time. Returns once all have been answered.
pub fn serve_lines<R, W>(session: Arc<McpBridgeSession>, input: R, output: W)
where
    R: BufRead,
    W: Write + Send + 'static,
{
    let output = Arc::new(Mutex::new(output));
    let mut workers = Vec::new();
    for line in input.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let (session, output) = (session.clone(), output.clone());
        workers.push(std::thread::spawn(move || {
            if let Some(reply) = session.handle_line(&line) {
                let mut output = lock(&output);
                let _ = writeln!(output, "{reply}");
                let _ = output.flush();
            }
        }));
    }
    for worker in workers {
        let _ = worker.join();
    }
}

/// `maine --mcp`: serves MCP over this process's stdin and stdout until EOF.
/// Returns the process exit status.
pub fn run_bridge(socket_path: PathBuf, token: Option<String>) -> i32 {
    let session = Arc::new(McpBridgeSession::new(socket_path, token));
    serve_lines(session, std::io::stdin().lock(), std::io::stdout());
    0
}

/// [`run_bridge`] with the socket from `MAINE_AGENT_SOCKET` (or the default
/// path) and the token from `MAINE_AGENT_TOKEN`.
pub fn run_bridge_from_env() -> i32 {
    run_bridge(
        default_socket_path(),
        std::env::var("MAINE_AGENT_TOKEN").ok(),
    )
}
