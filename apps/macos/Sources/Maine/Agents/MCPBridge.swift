import Foundation

/// `Maine --mcp`: an MCP server over stdio (one JSON-RPC message per line)
/// that forwards tool calls to the running app over the agent socket. It
/// never creates an `NSApplication` and never launches Maine.
enum MCPBridge {
    static let notRunning = "Maine is not running. Open Maine and unlock the vault."
    static let supportedVersions = ["2025-06-18", "2025-03-26", "2024-11-05"]

    /// Reads stdin until EOF; returns the process exit status.
    static func runStdio() -> Int32 {
        let environment = ProcessInfo.processInfo.environment
        let socketURL = environment["MAINE_AGENT_SOCKET"].map { URL(fileURLWithPath: $0) } ?? AgentServer.defaultSocketURL
        let session = MCPBridgeSession(socketURL: socketURL, token: environment["MAINE_AGENT_TOKEN"])
        let output = StdoutWriter()
        let group = DispatchGroup()
        while let line = readLine(strippingNewline: true) {
            guard !line.trimmingCharacters(in: .whitespaces).isEmpty else { continue }
            group.enter()
            Task.detached {
                if let reply = await session.handle(line: line) { output.write(reply) }
                group.leave()
            }
        }
        group.wait()
        return 0
    }

    private final class StdoutWriter: @unchecked Sendable {
        private let lock = NSLock()
        func write(_ line: String) {
            lock.lock()
            FileHandle.standardOutput.write(Data((line + "\n").utf8))
            lock.unlock()
        }
    }
}

/// The bridge's per-process state: the lazily opened, re-openable app connection.
final class MCPBridgeSession: @unchecked Sendable {
    private let socketURL: URL
    private let token: String?
    private let connection: BridgeConnection

    init(socketURL: URL, token: String?) {
        self.socketURL = socketURL
        self.token = token
        connection = BridgeConnection(socketURL: socketURL, token: token)
    }

    /// The reply line for one incoming line, or nil for notifications.
    func handle(line: String) async -> String? {
        guard let message = try? JSONDecoder().decode(JSONValue.self, from: Data(line.utf8)),
              case .object = message else {
            return Self.error(id: .null, code: -32700, message: "Parse error")
        }
        guard let method = message.string("method") else { return nil }
        guard let id = message["id"], id != .null else { return nil } // notification
        let params = message["params"] ?? [:]
        switch method {
        case "initialize":
            let requested = params.string("protocolVersion") ?? ""
            let version = MCPBridge.supportedVersions.contains(requested) ? requested : MCPBridge.supportedVersions[0]
            let appVersion = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "development"
            return Self.result(id: id, [
                "protocolVersion": .string(version),
                "capabilities": ["tools": ["listChanged": false]],
                "serverInfo": ["name": "maine", "version": .string(appVersion)],
                "instructions": "Maine runs real SSH sessions, on hosts the user allowed, in terminals the user can watch. Use run for commands (it returns output and the exit code), read_output for long-running commands, read_screen for full-screen programs and prompts, and send_keys to answer prompts or interrupt with ctrl-c.",
            ])
        case "ping":
            return Self.result(id: id, [:])
        case "tools/list":
            let tools = AgentToolCatalog.tools.map { tool -> JSONValue in
                ["name": .string(tool.name), "description": .string(tool.description), "inputSchema": tool.inputSchema]
            }
            return Self.result(id: id, ["tools": .array(tools)])
        case "tools/call":
            guard let name = params.string("name"), AgentToolCatalog.contains(name) else {
                return Self.error(id: id, code: -32602, message: "Unknown tool")
            }
            let reply = await connection.call(tool: name, arguments: params["arguments"] ?? [:])
            switch reply {
            case .success(let result):
                return Self.result(id: id, ["content": [["type": "text", "text": .string(result.jsonText)]],
                                            "structuredContent": result, "isError": false])
            case .failure(let message):
                return Self.result(id: id, ["content": [["type": "text", "text": .string(message)]], "isError": true])
            }
        default:
            return Self.error(id: id, code: -32601, message: "Method not found")
        }
    }

    private static func result(id: JSONValue, _ result: JSONValue) -> String {
        JSONValue.object(["jsonrpc": "2.0", "id": id, "result": result]).jsonText
    }

    private static func error(id: JSONValue, code: Int, message: String) -> String {
        JSONValue.object(["jsonrpc": "2.0", "id": id,
                          "error": ["code": .number(Double(code)), "message": .string(message)]]).jsonText
    }
}

/// Connects on first use and says hello with the token. A connection found
/// closed before a call is sent is replaced (the app restarted); a call that
/// was already sent is never re-sent, because it may have run.
private actor BridgeConnection {
    enum Outcome { case success(JSONValue), failure(String) }
    static let lost = "Maine closed the connection before answering; the call may have run."

    private let socketURL: URL
    private let token: String?
    private var client: AgentSocketClient?
    private var connecting: Task<Connected, Never>?

    init(socketURL: URL, token: String?) {
        self.socketURL = socketURL
        self.token = token
    }

    func call(tool: String, arguments: JSONValue) async -> Outcome {
        for _ in 0..<2 {
            let connected: AgentSocketClient
            switch await connect() {
            case .success(let value): connected = value
            case .failure(let message): return .failure(message)
            }
            do {
                let reply = try await connected.call(tool: tool, arguments: arguments)
                if let error = reply.error { return .failure(error) }
                return .success(reply.result ?? [:])
            } catch AgentSocketClient.ClientError.notSent {
                if client === connected { client = nil }
                continue
            } catch {
                if client === connected { client = nil }
                return .failure(Self.lost)
            }
        }
        return .failure(MCPBridge.notRunning)
    }

    fileprivate enum Connected { case success(AgentSocketClient), failure(String) }

    /// One connection attempt at a time, shared by concurrent first calls.
    private func connect() async -> Connected {
        if let client, !client.isClosed { return .success(client) }
        client = nil
        if let connecting { return await connecting.value }
        let socketURL = socketURL, token = token
        let attempt = Task { () -> Connected in
            guard let fresh = try? AgentSocketClient(socketURL: socketURL) else { return .failure(MCPBridge.notRunning) }
            guard let token else { fresh.close(); return .failure(AgentServer.notPaired) }
            guard let reply = try? await fresh.hello(AgentHello(token: token, clientName: nil)) else {
                return .failure(MCPBridge.notRunning)
            }
            guard reply.ok else { fresh.close(); return .failure(reply.error ?? AgentServer.notPaired) }
            return .success(fresh)
        }
        connecting = attempt
        let result = await attempt.value
        connecting = nil
        if case .success(let fresh) = result { client = fresh }
        return result
    }
}
