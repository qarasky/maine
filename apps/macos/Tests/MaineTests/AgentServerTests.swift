import Foundation
@testable import Maine
import Testing

/// A handler that echoes the tool it was asked for, and who asked.
private struct EchoHandler: AgentCallHandling {
    func handle(_ call: AgentCall, client: AgentClient) async -> AgentReply {
        AgentReply(id: call.id, result: ["tool": .string(call.tool), "client": .string(client.name)], error: nil)
    }
}

/// Replies with a large result, so writes take longer than the socket buffer.
private struct BigHandler: AgentCallHandling {
    func handle(_ call: AgentCall, client: AgentClient) async -> AgentReply {
        AgentReply(id: call.id, result: ["blob": .string(String(repeating: "x", count: 1_048_576))], error: nil)
    }
}

/// The app side of the agent socket.
@Suite(.serialized)
struct AgentServerTests {
    /// Short paths: `sun_path` holds at most 104 bytes.
    private func makeDirectory() -> URL {
        URL(fileURLWithPath: "/tmp/m-\(UUID().uuidString.prefix(8))", isDirectory: true)
    }

    private func withServer(_ body: (AgentServer, AgentClientStore, URL) async throws -> Void) async throws {
        let directory = makeDirectory()
        let socket = directory.appendingPathComponent("agent.sock")
        let clients = AgentClientStore(service: "app.maine.tests.\(UUID())")
        let server = AgentServer(socketURL: socket, clients: clients, handler: EchoHandler())
        defer {
            server.stop()
            for client in (try? clients.clients()) ?? [] { try? clients.revoke(client.id) }
            try? FileManager.default.removeItem(at: directory)
        }
        try server.start()
        try await body(server, clients, socket)
    }

    @Test
    func pairedClientsAreServedAndOthersRefused() async throws {
        try await withServer { _, clients, socket in
            let (_, token) = try clients.add(name: "Claude Code")
            let client = try AgentSocketClient(socketURL: socket)
            #expect(try await client.hello(AgentHello(token: token, clientName: "claude-code")) == AgentHelloReply(ok: true, error: nil))
            let reply = try await client.call(tool: "list_hosts", arguments: [:])
            #expect(reply.result == ["tool": "list_hosts", "client": "Claude Code"])
            async let first = client.call(tool: "a", arguments: [:])
            async let second = client.call(tool: "b", arguments: [:])
            #expect(try await [first.result?.string("tool"), second.result?.string("tool")] == ["a", "b"])
            client.close()

            let stranger = try AgentSocketClient(socketURL: socket)
            let refused = try await stranger.hello(AgentHello(token: "maine-agent-WRONG", clientName: nil))
            #expect(refused == AgentHelloReply(ok: false, error: "This MCP client is not paired with Maine."))
            stranger.close()
        }
    }

    @Test
    func socketFileIsUserOnly() async throws {
        try await withServer { _, _, socket in
            let socketMode = try FileManager.default.attributesOfItem(atPath: socket.path)[.posixPermissions] as? Int
            let directoryMode = try FileManager.default.attributesOfItem(
                atPath: socket.deletingLastPathComponent().path)[.posixPermissions] as? Int
            #expect(socketMode == 0o600)
            #expect(directoryMode == 0o700)
        }
    }

    @Test
    func aStaleSocketFileDoesNotBlockStart() async throws {
        let directory = makeDirectory()
        let socket = directory.appendingPathComponent("agent.sock")
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try Data("stale".utf8).write(to: socket)
        let clients = AgentClientStore(service: "app.maine.tests.\(UUID())")
        let server = AgentServer(socketURL: socket, clients: clients, handler: EchoHandler())
        defer { server.stop(); try? FileManager.default.removeItem(at: directory) }
        try server.start()
        let (client, token) = try clients.add(name: "After crash")
        defer { try? clients.revoke(client.id) }
        let connection = try AgentSocketClient(socketURL: socket)
        #expect(try await connection.hello(AgentHello(token: token, clientName: nil)).ok)
        connection.close()
    }

    @Test
    func revokingDisconnectsLiveConnections() async throws {
        try await withServer { server, clients, socket in
            let (client, token) = try clients.add(name: "Revoked")
            let connection = try AgentSocketClient(socketURL: socket)
            #expect(try await connection.hello(AgentHello(token: token, clientName: nil)).ok)
            server.disconnect(clientID: client.id)
            await #expect(throws: AgentSocketClient.ClientError.self) {
                _ = try await connection.call(tool: "list_hosts", arguments: [:])
            }
        }
    }

    @Test
    func stoppingRemovesTheSocket() async throws {
        try await withServer { server, _, socket in
            server.stop()
            #expect(!FileManager.default.fileExists(atPath: socket.path))
            #expect(throws: AgentSocketClient.ClientError.self) { _ = try AgentSocketClient(socketURL: socket) }
        }
    }

    // MARK: - Final review fixes

    @Test
    func largeConcurrentTrafficDoesNotDeadlock() async throws {
        let directory = makeDirectory()
        let socket = directory.appendingPathComponent("agent.sock")
        let clients = AgentClientStore(service: "app.maine.tests.\(UUID())")
        let server = AgentServer(socketURL: socket, clients: clients, handler: BigHandler())
        defer {
            server.stop()
            for client in (try? clients.clients()) ?? [] { try? clients.revoke(client.id) }
            try? FileManager.default.removeItem(at: directory)
        }
        try server.start()
        let (_, token) = try clients.add(name: "Load")
        let connection = try AgentSocketClient(socketURL: socket)
        #expect(try await connection.hello(AgentHello(token: token, clientName: nil)).ok)
        let argument: JSONValue = ["pad": .string(String(repeating: "y", count: 512 * 1024))]
        // Unstructured tasks: on a deadlock, closing the connection fails the
        // stuck calls so nothing is left waiting after the test.
        final class Counter: @unchecked Sendable {
            private let lock = NSLock()
            private var done = 0, failures = 0
            func finish(_ ok: Bool) { lock.withLock { done += 1; if !ok { failures += 1 } } }
            var state: (done: Int, failures: Int) { lock.withLock { (done, failures) } }
        }
        let counter = Counter()
        for _ in 0..<12 {
            Task.detached { counter.finish((try? await connection.call(tool: "x", arguments: argument))?.result != nil) }
        }
        let deadline = ContinuousClock.now + .seconds(20)
        while counter.state.done < 12, ContinuousClock.now < deadline { try? await Task.sleep(for: .milliseconds(50)) }
        let finished = counter.state.done == 12 && counter.state.failures == 0
        if !finished { connection.close() }
        #expect(finished, "12 parallel 512 KiB requests with 1 MiB replies must all complete")
        connection.close()
    }
}
