//! The tools (`docs/mcp.md`, Tools). The bridge advertises these; the
//! [`AgentToolHandler`](crate::tools::AgentToolHandler) implements them in the
//! app. Names, schemas and result shapes are the macOS ones, so an MCP client
//! behaves identically against either.

use serde_json::{json, Map, Value};
use std::sync::OnceLock;

/// One MCP tool as advertised by `tools/list`.
#[derive(Debug, Clone, PartialEq)]
pub struct ToolDefinition {
    pub name: &'static str,
    pub description: &'static str,
    pub input_schema: Value,
}

fn schema(properties: Value, required: &[&str]) -> Value {
    let properties = match properties {
        Value::Object(map) => map,
        _ => Map::new(),
    };
    json!({"type": "object", "properties": properties, "required": required, "additionalProperties": false})
}

fn session_id() -> Value {
    json!({"type": "string", "description": "A session_id from open_session or list_sessions."})
}

/// The tools in the order the macOS bridge lists them.
pub fn tools() -> &'static [ToolDefinition] {
    static TOOLS: OnceLock<Vec<ToolDefinition>> = OnceLock::new();
    TOOLS.get_or_init(|| {
        vec![
            ToolDefinition {
                name: "list_hosts",
                description: "Lists the SSH hosts the user allowed agents to use: id, name, group, hostname, port, username and whether a password is saved. Returns {hosts: [...]}.",
                input_schema: schema(json!({}), &[]),
            },
            ToolDefinition {
                name: "list_sessions",
                description: "Lists the sessions this client opened: session_id, host_id, name, state and busy. Returns {sessions: [...]}.",
                input_schema: schema(json!({}), &[]),
            },
            ToolDefinition {
                name: "open_session",
                description: "Opens a new SSH session to an allowed host in Maine, visible to the user, and waits up to 30 s for login. Returns {session_id, state, output}.",
                input_schema: schema(
                    json!({
                        "host_id": {"type": "string", "description": "An id from list_hosts."},
                        "name": {"type": "string", "description": "Optional session name shown to the user."}
                    }),
                    &["host_id"],
                ),
            },
            ToolDefinition {
                name: "run",
                description: "Types a shell command into the session and waits for it to finish. Returns {completed, exit_code, output, cursor, truncated, mode}; if not completed within the timeout the command keeps running; use read_output or send_keys.",
                input_schema: schema(
                    json!({
                        "session_id": session_id(),
                        "command": {"type": "string", "description": "A shell command line; may span lines."},
                        "timeout_seconds": {"type": "integer", "minimum": 1, "maximum": 600, "default": 60}
                    }),
                    &["session_id", "command"],
                ),
            },
            ToolDefinition {
                name: "read_output",
                description: "Returns session output after a cursor (default: everything retained), optionally waiting for new output. Returns {output, cursor, busy, truncated}.",
                input_schema: schema(
                    json!({
                        "session_id": session_id(),
                        "since": {"type": "integer", "minimum": 0, "description": "A cursor from an earlier result."},
                        "wait_seconds": {"type": "integer", "minimum": 0, "maximum": 60, "default": 0},
                        "max_bytes": {"type": "integer", "minimum": 1, "maximum": 65536, "default": 65536}
                    }),
                    &["session_id"],
                ),
            },
            ToolDefinition {
                name: "read_screen",
                description: "Returns the visible terminal screen as text rows, with the cursor position and whether a full-screen program is active. Use it for full-screen programs, prompts and multi-line progress that redraws in place (docker, apt). With wait_seconds, waits until the screen differs from what the previous read_screen returned, or the command ends. Returns {rows, cursor_row, cursor_col, alternate_screen, changed, busy}.",
                input_schema: schema(
                    json!({
                        "session_id": session_id(),
                        "wait_seconds": {"type": "integer", "minimum": 0, "maximum": 60, "default": 0}
                    }),
                    &["session_id"],
                ),
            },
            ToolDefinition {
                name: "send_keys",
                description: "Sends keys to the session: named keys (enter, tab, esc, backspace, up, down, left, right, ctrl-a … ctrl-z) or literal text as \"text:<literal>\". Returns {sent}.",
                input_schema: schema(
                    json!({
                        "session_id": session_id(),
                        "keys": {"type": "array", "items": {"type": "string"}, "minItems": 1}
                    }),
                    &["session_id", "keys"],
                ),
            },
            ToolDefinition {
                name: "type_password",
                description: "Types the host's saved password plus Enter into the session, only when the last screen line is a password prompt (sudo, ssh, su). The password is never returned and is masked in all later output. Returns {typed: true}.",
                input_schema: schema(json!({"session_id": session_id()}), &["session_id"]),
            },
            ToolDefinition {
                name: "close_session",
                description: "Closes a session this client opened. Returns {closed: true}.",
                input_schema: schema(json!({"session_id": session_id()}), &["session_id"]),
            },
        ]
    })
}

pub fn contains(name: &str) -> bool {
    tools().iter().any(|tool| tool.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_catalog_matches_the_macos_tool_list() {
        let names: Vec<&str> = tools().iter().map(|t| t.name).collect();
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
        for tool in tools() {
            assert_eq!(tool.input_schema["type"], "object");
            assert_eq!(tool.input_schema["additionalProperties"], false);
            assert!(!tool.description.is_empty());
            assert!(!tool.description.contains('\u{2014}'));
        }
        assert!(contains("run") && !contains("rm_rf"));
    }

    #[test]
    fn run_schema_limits() {
        let run = tools().iter().find(|t| t.name == "run").unwrap();
        assert_eq!(
            run.input_schema["required"],
            json!(["session_id", "command"])
        );
        assert_eq!(
            run.input_schema["properties"]["timeout_seconds"]["maximum"],
            600
        );
    }
}
