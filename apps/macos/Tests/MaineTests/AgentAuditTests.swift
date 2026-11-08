import Foundation
@testable import Maine
import Testing

/// The encrypted, device-local record of what agents did (`docs/mcp.md` phase 3).
@Suite
struct AgentAuditTests {
    private func entry(_ kind: AgentAuditEntry.Kind = .run, detail: String? = nil, exit: Int32? = nil,
                       at: Date = Date()) -> AgentAuditEntry {
        AgentAuditEntry(at: at, client: "Claude Code", hostName: "Web", sessionID: UUID(), kind: kind,
                        detail: detail, exitCode: exit)
    }

    @Test
    func entriesAreSealedInTheLocalFileAndNeverTouchTheVault() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let other = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let store = PasswordStore(fileURL: directory.appendingPathComponent("passwords.maine"),
                                  keychainService: "app.maine.tests.\(UUID())")
        let copy = PasswordStore(fileURL: other.appendingPathComponent("passwords.maine"),
                                 keychainService: "app.maine.tests.\(UUID())")
        defer {
            try? store.lock(); try? copy.lock()
            try? FileManager.default.removeItem(at: directory)
            try? FileManager.default.removeItem(at: other)
        }
        let master = "agent audit master password"
        try store.unlock(masterPassword: master)
        try store.saveHost(Maine.Host(name: "Web", hostname: "web.example"), password: "secret")
        let vaultBefore = try Data(contentsOf: store.fileURL)

        #expect(try store.agentAudit().isEmpty)
        let first = entry(detail: "systemctl restart nginx", exit: 0)
        let second = entry(.password)
        try store.appendAgentAudit([first, second])
        #expect(try store.agentAudit() == [first, second])
        #expect(try Data(contentsOf: store.fileURL) == vaultBefore)
        let text = try String(contentsOf: try #require(store.sessionsFileURL), encoding: .utf8)
        #expect(!text.contains("nginx") && !text.contains("Claude Code"))

        try FileManager.default.createDirectory(at: other, withIntermediateDirectories: true)
        try FileManager.default.copyItem(at: store.fileURL, to: copy.fileURL)
        try copy.unlock(masterPassword: master)
        #expect(try copy.agentAudit().isEmpty)

        try store.clearAgentAudit()
        #expect(try store.agentAudit().isEmpty)
        try store.lock()
        #expect(throws: PasswordStore.VaultError.locked) { try store.agentAudit() }
    }

    @Test
    func theLogKeepsTheNewestEntriesAndTrimsLongCommands() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let store = PasswordStore(fileURL: directory.appendingPathComponent("passwords.maine"),
                                  keychainService: "app.maine.tests.\(UUID())")
        defer { try? store.lock(); try? FileManager.default.removeItem(at: directory) }
        try store.unlock(masterPassword: "agent audit cap master password")
        let batch = (0..<(AgentAuditEntry.limit + 25)).map { entry(detail: "cmd \($0)") }
        try store.appendAgentAudit(batch)
        let kept = try store.agentAudit()
        #expect(kept.count == AgentAuditEntry.limit)
        #expect(kept.first?.detail == "cmd 25")
        #expect(kept.last?.detail == "cmd \(AgentAuditEntry.limit + 24)")

        let long = String(repeating: "x", count: 10_000)
        #expect(entry(detail: long).detail?.utf8.count ?? 0 <= AgentAuditEntry.detailLimit + 16)
    }
}
