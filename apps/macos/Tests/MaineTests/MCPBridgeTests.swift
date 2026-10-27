import Foundation
@testable import Maine
import Testing

private struct ToolEcho: AgentCallHandling {
    func handle(_ call: AgentCall, client: AgentClient) async -> AgentReply {
        if call.tool == "close_session" { return AgentReply(id: call.id, result: nil, error: "not found") }
        return AgentReply(id: call.id, result: ["tool": .string(call.tool), "arguments": call.arguments], error: nil)
    }
}

/// Counts calls and cuts the connection before answering, like an app that
/// died mid-call.
private final class DroppingHandler: AgentCallHandling, @unchecked Sendable {
    private let lock = NSLock()
    private var calls = 0
    var server: AgentServer?
    var count: Int { lock.withLock { calls } }

    func handle(_ call: AgentCall, client: AgentClient) async -> AgentReply {
        lock.withLock { calls += 1 }
        server?.disconnect(clientID: client.id)
        try? await Task.sleep(for: .milliseconds(100))
        return AgentReply(id: call.id, result: [:], error: nil)
    }
}

/// `Maine --mcp`: the MCP (JSON-RPC over stdio) side, driven line by line.
@Suite(.serialized)
struct MCPBridgeTests {
    private func object(_ line: String?) throws -> JSONValue {
        let text = try #require(line)
        return try JSONDecoder().decode(JSONValue.self, from: Data(text.utf8))
    }

    private func socketURL() -> URL {
        URL(fileURLWithPath: "/tmp/m-\(UUID().uuidString.prefix(8))/agent.sock")
    }

    @Test
    func initializeNegotiatesTheProtocolVersion() async throws {
        let bridge = MCPBridgeSession(socketURL: socketURL(), token: nil)
        let reply = try object(await bridge.handle(line: #"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"t","version":"1"}}}"#))
        #expect(reply["id"] == 1)
        #expect(reply["result"]?.string("protocolVersion") == "2025-03-26")
        #expect(reply["result"]?["serverInfo"]?.string("name") == "maine")
        #expect(reply["result"]?["capabilities"]?["tools"] != nil)

        let unknown = try object(await bridge.handle(line: #"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#))
        #expect(unknown["result"]?.string("protocolVersion") == "2025-06-18")
    }

    @Test
    func toolsListMatchesTheCatalog() async throws {
        let bridge = MCPBridgeSession(socketURL: socketURL(), token: nil)
        let reply = try object(await bridge.handle(line: #"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#))
        let names = reply["result"]?.array("tools")?.compactMap { $0.string("name") }
        #expect(names == AgentToolCatalog.tools.map(\.name))
        #expect(names == ["list_hosts", "list_sessions", "open_session", "run", "read_output", "read_screen",
                          "send_keys", "close_session"])
        for tool in reply["result"]?.array("tools") ?? [] {
            #expect(tool["inputSchema"]?.string("type") == "object")
            #expect(!(tool.string("description") ?? "").isEmpty)
        }
    }

    @Test
    func callsWithoutAServerReturnTheNotRunningError() async throws {
        let bridge = MCPBridgeSession(socketURL: socketURL(), token: "maine-agent-X")
        let reply = try object(await bridge.handle(line: #"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"list_hosts","arguments":{}}}"#))
        #expect(reply["result"]?["isError"] == true)
        #expect(reply["result"]?.array("content")?.first?.string("text")
                == "Maine is not running. Open Maine and unlock the vault.")
    }

    @Test
    func callsAreForwardedToTheApp() async throws {
        let socket = socketURL()
        let clients = AgentClientStore(service: "app.maine.tests.\(UUID())")
        let server = AgentServer(socketURL: socket, clients: clients, handler: ToolEcho())
        defer {
            server.stop()
            for client in (try? clients.clients()) ?? [] { try? clients.revoke(client.id) }
            try? FileManager.default.removeItem(at: socket.deletingLastPathComponent())
        }
        try server.start()
        let (_, token) = try clients.add(name: "Bridge test")

        let unpaired = MCPBridgeSession(socketURL: socket, token: "maine-agent-NOPE")
        let refused = try object(await unpaired.handle(line: #"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"list_hosts"}}"#))
        #expect(refused["result"]?.array("content")?.first?.string("text") == "This MCP client is not paired with Maine.")

        let bridge = MCPBridgeSession(socketURL: socket, token: token)
        let reply = try object(await bridge.handle(line: #"{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"run","arguments":{"session_id":"x","command":"ls"}}}"#))
        #expect(reply["result"]?["isError"] == false)
        #expect(reply["result"]?["structuredContent"]?.string("tool") == "run")
        #expect(reply["result"]?["structuredContent"]?["arguments"]?.string("command") == "ls")
        let text = try #require(reply["result"]?.array("content")?.first?.string("text"))
        #expect(try JSONDecoder().decode(JSONValue.self, from: Data(text.utf8))["tool"] == "run")

        let failed = try object(await bridge.handle(line: #"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"close_session","arguments":{}}}"#))
        #expect(failed["result"]?["isError"] == true)
        #expect(failed["result"]?.array("content")?.first?.string("text") == "not found")
    }

    @Test
    func unknownMethodsAndToolsAreJSONRPCErrors() async throws {
        let bridge = MCPBridgeSession(socketURL: socketURL(), token: nil)
        let method = try object(await bridge.handle(line: #"{"jsonrpc":"2.0","id":8,"method":"resources/list"}"#))
        #expect(method["error"]?.int("code") == -32601)
        let tool = try object(await bridge.handle(line: #"{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"rm_rf"}}"#))
        #expect(tool["error"]?.int("code") == -32602)
        let garbage = try object(await bridge.handle(line: "not json"))
        #expect(garbage["error"]?.int("code") == -32700)
        let ping = try object(await bridge.handle(line: #"{"jsonrpc":"2.0","id":10,"method":"ping"}"#))
        #expect(ping["result"] == [:])
    }

    @Test
    func notificationsGetNoReply() async {
        let bridge = MCPBridgeSession(socketURL: socketURL(), token: nil)
        #expect(await bridge.handle(line: #"{"jsonrpc":"2.0","method":"notifications/initialized"}"#) == nil)
        #expect(await bridge.handle(line: #"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":1}}"#) == nil)
    }

    // MARK: - Final review fixes

    @Test
    func aCallLostAfterSendingIsNeverRetried() async throws {
        let socket = socketURL()
        let clients = AgentClientStore(service: "app.maine.tests.\(UUID())")
        let handler = DroppingHandler()
        let server = AgentServer(socketURL: socket, clients: clients, handler: handler)
        handler.server = server
        defer {
            server.stop()
            for client in (try? clients.clients()) ?? [] { try? clients.revoke(client.id) }
            try? FileManager.default.removeItem(at: socket.deletingLastPathComponent())
        }
        try server.start()
        let (_, token) = try clients.add(name: "Retry test")
        let bridge = MCPBridgeSession(socketURL: socket, token: token)
        let reply = try object(await bridge.handle(line: #"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"run","arguments":{"session_id":"x","command":"rm important"}}}"#))
        #expect(reply["result"]?["isError"] == true)
        #expect(handler.count == 1, "a run that may have executed must not be sent twice")
    }

    @Test
    func theBridgeReconnectsAfterTheAppRestarts() async throws {
        let socket = socketURL()
        let clients = AgentClientStore(service: "app.maine.tests.\(UUID())")
        var server = AgentServer(socketURL: socket, clients: clients, handler: ToolEcho())
        defer {
            server.stop()
            for client in (try? clients.clients()) ?? [] { try? clients.revoke(client.id) }
            try? FileManager.default.removeItem(at: socket.deletingLastPathComponent())
        }
        try server.start()
        let (_, token) = try clients.add(name: "Restart test")
        let bridge = MCPBridgeSession(socketURL: socket, token: token)
        let line = #"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_hosts","arguments":{}}}"#
        #expect(try object(await bridge.handle(line: line))["result"]?["isError"] == false)
        server.stop()
        try? await Task.sleep(for: .milliseconds(100))
        server = AgentServer(socketURL: socket, clients: clients, handler: ToolEcho())
        try server.start()
        #expect(try object(await bridge.handle(line: line))["result"]?["isError"] == false)
    }
}
