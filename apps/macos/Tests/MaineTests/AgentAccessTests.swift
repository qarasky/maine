import Foundation
@testable import Maine
import Testing

/// Which groups and hosts agents may use: device-local, per vault.
@Suite
struct AgentAccessTests {
    private let prod = HostGroup(name: "Prod")
    private let lab = HostGroup(name: "Lab")

    private func layout() -> (HostLayout, web: Maine.Host, db: Maine.Host, loose: Maine.Host) {
        let loose = Maine.Host(name: "Loose", hostname: "loose.example")
        let web = Maine.Host(name: "Web", hostname: "web.example", groupID: prod.id)
        let db = Maine.Host(name: "DB", hostname: "db.example", groupID: lab.id)
        return (HostLayout(groups: [prod, lab], hosts: [loose, web, db]), web, db, loose)
    }

    @Test
    func aHostIsAllowedByItsGroupOrByItself() {
        let (layout, web, db, loose) = layout()
        let byGroup = AgentAccess(groups: [prod.id])
        #expect(byGroup.allows(web))
        #expect(!byGroup.allows(db))
        #expect(!byGroup.allows(loose))
        #expect(AgentAccess(hosts: [loose.id]).allows(loose))
        #expect(AgentAccess(groups: [prod.id], hosts: [loose.id]).allowedHosts(in: layout).map(\.name) == ["Loose", "Web"])
        #expect(AgentAccess().allowedHosts(in: layout).isEmpty)
    }

    @Test
    func pruningDropsRulesForGoneGroupsAndHosts() {
        let (layout, web, _, _) = layout()
        let stale = AgentAccess(groups: [prod.id, UUID()], hosts: [web.id, UUID()])
        #expect(stale.pruned(to: layout) == AgentAccess(groups: [prod.id], hosts: [web.id]))
    }

    @Test
    func rulesLiveInTheLocalFileNotTheVault() throws {
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
        let master = "agent access master password"
        try store.unlock(masterPassword: master)
        let web = Maine.Host(name: "Web", hostname: "web.example")
        try store.saveHost(web, password: "secret")
        try store.saveLayout(groups: [prod], hosts: [web])
        let saved = SavedSession(hostID: web.id, name: "Session", history: "")
        try store.saveSession(saved)
        try store.setCollapsedGroupIDs([prod.id])

        let vaultBefore = try Data(contentsOf: store.fileURL)
        #expect(try store.agentAccess() == AgentAccess())
        let access = AgentAccess(hosts: [web.id])
        try store.setAgentAccess(access)
        #expect(try Data(contentsOf: store.fileURL) == vaultBefore)
        #expect(try store.agentAccess() == access)
        #expect(try store.savedSessions() == [saved])
        #expect(try store.collapsedGroupIDs() == [prod.id])
        let text = try String(contentsOf: try #require(store.sessionsFileURL), encoding: .utf8)
        #expect(!text.contains(web.id.uuidString) && !text.contains("agentAccess"))

        try FileManager.default.createDirectory(at: other, withIntermediateDirectories: true)
        try FileManager.default.copyItem(at: store.fileURL, to: copy.fileURL)
        try copy.unlock(masterPassword: master)
        #expect(try copy.agentAccess() == AgentAccess())

        try store.lock()
        #expect(throws: PasswordStore.VaultError.locked) { try store.agentAccess() }
    }
}
