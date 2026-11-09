import Foundation

/// One MCP tool as advertised by `tools/list`.
struct AgentToolDefinition: Sendable {
    let name: String
    let description: String
    let inputSchema: JSONValue
}

/// The tools (`docs/mcp.md` § Tools). The bridge advertises these;
/// `AgentToolHandler` implements them in the app.
enum AgentToolCatalog {
    private static let sessionID: JSONValue = ["type": "string", "description": "A session_id from open_session or list_sessions."]

    private static func schema(_ properties: [String: JSONValue] = [:], required: [String] = []) -> JSONValue {
        ["type": "object", "properties": .object(properties), "required": .array(required.map(JSONValue.string)),
         "additionalProperties": false]
    }

    static let tools: [AgentToolDefinition] = [
        AgentToolDefinition(
            name: "list_hosts",
            description: "Lists the SSH hosts the user allowed agents to use: id, name, group, hostname, port, username and whether a password is saved. Returns {hosts: [...]}.",
            inputSchema: schema()),
        AgentToolDefinition(
            name: "list_sessions",
            description: "Lists the sessions this client opened: session_id, host_id, name, state and busy. Returns {sessions: [...]}.",
            inputSchema: schema()),
        AgentToolDefinition(
            name: "open_session",
            description: "Opens a new SSH session to an allowed host in Maine, visible to the user, and waits up to 30 s for login. Returns {session_id, state, output}.",
            inputSchema: schema(["host_id": ["type": "string", "description": "An id from list_hosts."],
                                 "name": ["type": "string", "description": "Optional session name shown to the user."]],
                                required: ["host_id"])),
        AgentToolDefinition(
            name: "run",
            description: "Types a shell command into the session and waits for it to finish. Returns {completed, exit_code, output, cursor, truncated, mode}; if not completed within the timeout the command keeps running — use read_output or send_keys.",
            inputSchema: schema(["session_id": sessionID,
                                 "command": ["type": "string", "description": "A shell command line; may span lines."],
                                 "timeout_seconds": ["type": "integer", "minimum": 1, "maximum": 600, "default": 60]],
                                required: ["session_id", "command"])),
        AgentToolDefinition(
            name: "read_output",
            description: "Returns session output after a cursor (default: everything retained), optionally waiting for new output. Returns {output, cursor, busy, truncated}.",
            inputSchema: schema(["session_id": sessionID,
                                 "since": ["type": "integer", "minimum": 0, "description": "A cursor from an earlier result."],
                                 "wait_seconds": ["type": "integer", "minimum": 0, "maximum": 60, "default": 0],
                                 "max_bytes": ["type": "integer", "minimum": 1, "maximum": 65536, "default": 65536]],
                                required: ["session_id"])),
        AgentToolDefinition(
            name: "read_screen",
            description: "Returns the visible terminal screen as text rows, with the cursor position and whether a full-screen program is active. Use it for full-screen programs, prompts and multi-line progress that redraws in place (docker, apt). With wait_seconds, waits until the screen differs from what the previous read_screen returned, or the command ends. Returns {rows, cursor_row, cursor_col, alternate_screen, changed, busy}.",
            inputSchema: schema(["session_id": sessionID,
                                 "wait_seconds": ["type": "integer", "minimum": 0, "maximum": 60, "default": 0]],
                                required: ["session_id"])),
        AgentToolDefinition(
            name: "send_keys",
            description: "Sends keys to the session: named keys (enter, tab, esc, backspace, up, down, left, right, ctrl-a … ctrl-z) or literal text as \"text:<literal>\". Returns {sent}.",
            inputSchema: schema(["session_id": sessionID,
                                 "keys": ["type": "array", "items": ["type": "string"], "minItems": 1]],
                                required: ["session_id", "keys"])),
        AgentToolDefinition(
            name: "type_password",
            description: "Types the host's saved password plus Enter into the session, only when the last screen line is a password prompt (sudo, ssh, su). The password is never returned and is masked in all later output. Returns {typed: true}.",
            inputSchema: schema(["session_id": sessionID], required: ["session_id"])),
        AgentToolDefinition(
            name: "close_session",
            description: "Closes a session this client opened. Returns {closed: true}.",
            inputSchema: schema(["session_id": sessionID], required: ["session_id"])),
    ]

    static func contains(_ name: String) -> Bool { tools.contains { $0.name == name } }
}
