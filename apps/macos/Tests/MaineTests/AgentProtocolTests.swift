import Foundation
import Security
@testable import Maine
import Testing

/// The app ↔ bridge socket format, and the per-client tokens.
@Suite
struct AgentProtocolTests {
    @Test
    func framesRoundTripAndSplitAnywhere() throws {
        let call = AgentCall(id: 7, tool: "run", arguments: ["command": "ls", "timeout_seconds": 5, "nested": [true, nil]])
        let frame = try AgentFrame.encode(call)
        var reader = AgentFrame.Reader()
        var frames: [Data] = []
        for byte in frame { frames += try reader.feed(Data([byte])) }
        #expect(frames.count == 1)
        let decoded = try JSONDecoder().decode(AgentCall.self, from: frames[0])
        #expect(decoded == call)
        #expect(decoded.arguments.string("command") == "ls")
        #expect(decoded.arguments.int("timeout_seconds") == 5)
        #expect(decoded.arguments.int("missing") == nil)

        // Two frames in one read.
        frames = try reader.feed(frame + frame)
        #expect(frames.count == 2)
    }

    @Test
    func oversizedFramesAreRefused() {
        var reader = AgentFrame.Reader()
        #expect(throws: AgentProtocolError.frameTooLarge) {
            _ = try reader.feed(Data([0x00, 0x80, 0x00, 0x01]))
        }
    }

    @Test
    func tokensAreShownOnceAndStoredHashed() throws {
        let service = "app.maine.tests.\(UUID())"
        let store = AgentClientStore(service: service)
        defer { for client in (try? store.clients()) ?? [] { try? store.revoke(client.id) } }

        let (client, token) = try store.add(name: "Claude Code")
        #expect(token.hasPrefix("maine-agent-"))
        #expect(token.count == 64)
        #expect(try store.client(forToken: token)?.id == client.id)
        #expect(try store.client(forToken: token + "x") == nil)
        #expect(try store.client(forToken: "") == nil)

        let query: [String: Any] = [kSecClass as String: kSecClassGenericPassword,
                                    kSecAttrService as String: service,
                                    kSecAttrAccount as String: client.id.uuidString,
                                    kSecReturnData as String: true,
                                    kSecMatchLimit as String: kSecMatchLimitOne]
        var result: CFTypeRef?
        #expect(SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess)
        let stored = try #require(result as? Data)
        #expect(stored != Data(token.utf8))
        #expect(!String(decoding: stored, as: UTF8.self).contains(token))

        #expect(try store.clients().map(\.name) == ["Claude Code"])
        let (second, secondToken) = try store.add(name: "Desktop")
        #expect(token != secondToken)
        try store.revoke(client.id)
        #expect(try store.client(forToken: token) == nil)
        #expect(try store.client(forToken: secondToken)?.id == second.id)
    }
}
