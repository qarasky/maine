import Foundation
import CommonCrypto
import CryptoKit
@testable import Maine
import Testing

/// Session snapshots (names + scrollback) are device-local: they live in an
/// encrypted file next to the vault, never inside the synced `passwords.maine`.
@Suite
struct LocalSessionStoreTests {
    private func makeStore(_ directory: URL, file: String = "passwords.maine") -> PasswordStore {
        PasswordStore(fileURL: directory.appendingPathComponent(file),
                      keychainService: "app.maine.tests.\(UUID())")
    }

    @Test
    func sessionsStayOnThisDeviceAndNeverTouchTheSyncedVault() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let other = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let store = makeStore(directory)
        let copy = makeStore(other)
        defer {
            try? store.lock(); try? copy.lock()
            try? FileManager.default.removeItem(at: directory)
            try? FileManager.default.removeItem(at: other)
        }
        let master = "local sessions master password"
        let host = Maine.Host(name: "Server", hostname: "server.example")
        try store.unlock(masterPassword: master)
        try store.saveHost(host, password: "host secret")

        let vaultBefore = try Data(contentsOf: store.fileURL)
        let saved = SavedSession(hostID: host.id, name: "Private session", history: "Sensitive terminal output")
        try store.saveSession(saved)
        try store.saveSessions([saved])
        #expect(try Data(contentsOf: store.fileURL) == vaultBefore)
        #expect(try store.savedSessions() == [saved])

        let sessionsURL = try #require(store.sessionsFileURL)
        #expect(sessionsURL != store.fileURL)
        let text = try String(contentsOf: sessionsURL, encoding: .utf8)
        #expect(!text.contains(saved.name) && !text.contains(saved.history) && !text.contains(host.id.uuidString))
        let permissions = try FileManager.default.attributesOfItem(atPath: sessionsURL.path)[.posixPermissions] as? Int
        #expect(permissions == 0o600)

        try store.lock()
        #expect(throws: PasswordStore.VaultError.self) { try store.savedSessions() }
        try store.unlock(masterPassword: master)
        #expect(try store.savedSessions() == [saved])

        // The synced object is the vault alone: another device gets the hosts
        // but none of this device's sessions.
        try FileManager.default.createDirectory(at: other, withIntermediateDirectories: true)
        try FileManager.default.copyItem(at: store.fileURL, to: copy.fileURL)
        try copy.unlock(masterPassword: master)
        #expect(try copy.hosts() == [host])
        #expect(try copy.savedSessions().isEmpty)

        try store.deleteSavedSession(saved.id)
        #expect(try store.savedSessions().isEmpty)
        #expect(try Data(contentsOf: store.fileURL) == vaultBefore)
    }

    @Test
    func sessionsInsideAnOlderVaultMoveToTheLocalFileOnUnlock() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let other = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: other, withIntermediateDirectories: true)
        let master = "legacy sessions master password"
        let host = Maine.Host(name: "Server", hostname: "server.example")
        let saved = SavedSession(hostID: host.id, name: "Old session", history: "old scrollback")
        let original = try Self.vaultWithEmbeddedSessions(master: master, host: host, sessions: [saved])
        let store = makeStore(directory)
        let copy = makeStore(other)
        defer {
            try? store.lock(); try? copy.lock()
            try? FileManager.default.removeItem(at: directory)
            try? FileManager.default.removeItem(at: other)
        }
        try original.write(to: store.fileURL)

        try store.unlock(masterPassword: master)
        #expect(try store.hosts() == [host])
        #expect(try store.password(for: host.id) == "host secret")
        #expect(try store.savedSessions() == [saved])
        #expect(try Data(contentsOf: store.fileURL) != original)

        // The rewritten vault no longer carries the sessions.
        try FileManager.default.copyItem(at: store.fileURL, to: copy.fileURL)
        try copy.unlock(masterPassword: master)
        #expect(try copy.hosts() == [host])
        #expect(try copy.savedSessions().isEmpty)

        // Later unlocks keep the migrated records without duplicating them.
        try store.lock()
        try store.unlock(masterPassword: master)
        #expect(try store.savedSessions() == [saved])
    }

    @Test
    func sessionsForHostsThatLeftTheVaultAreDropped() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let store = makeStore(directory)
        defer { try? store.lock(); try? FileManager.default.removeItem(at: directory) }
        try store.unlock(masterPassword: "pruning test master password")
        let kept = Maine.Host(name: "Kept", hostname: "kept.example")
        let deleted = Maine.Host(name: "Deleted", hostname: "deleted.example")
        let synced = Maine.Host(name: "Removed elsewhere", hostname: "elsewhere.example")
        for host in [kept, deleted, synced] { try store.saveHost(host, password: "secret") }
        let sessions = [kept, deleted, synced].map { SavedSession(hostID: $0.id, name: $0.name, history: "") }
        try store.saveSessions(sessions)

        try store.deleteHost(deleted.id)
        #expect(try store.savedSessions().map(\.hostID) == [kept.id, synced.id])

        // A pulled vault without `synced`: pruning follows the vault's own host list.
        try store.saveHosts([kept])
        #expect(try store.savedSessions().count == 2)
        try store.deleteSavedSessionsForMissingHosts()
        #expect(try store.savedSessions() == [sessions[0]])
    }

    @Test
    func groupsTravelWithTheVaultAndSurviveHostEdits() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let other = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let store = makeStore(directory)
        let copy = makeStore(other)
        defer {
            try? store.lock(); try? copy.lock()
            try? FileManager.default.removeItem(at: directory)
            try? FileManager.default.removeItem(at: other)
        }
        let master = "groups test master password"
        try store.unlock(masterPassword: master)
        let prod = HostGroup(name: "Production"), empty = HostGroup(name: "Empty")
        var web = Maine.Host(name: "Web", hostname: "web.example")
        let loose = Maine.Host(name: "Loose", hostname: "loose.example")
        try store.saveHost(web, password: "secret")
        try store.saveHost(loose, password: "secret")
        web.groupID = prod.id
        try store.saveLayout(groups: [prod, empty], hosts: [loose, web])
        #expect(try store.groups() == [prod, empty])
        #expect(try store.hosts() == [loose, web])

        // Editing, adding and deleting hosts keeps the groups.
        web.name = "Web 2"
        try store.saveHost(web, password: "secret")
        try store.saveHosts([loose, web])
        try store.deleteHost(loose.id)
        #expect(try store.groups() == [prod, empty])
        #expect(try store.hosts() == [web])
        #expect(try store.password(for: web.id) == "secret")

        try FileManager.default.createDirectory(at: other, withIntermediateDirectories: true)
        try FileManager.default.copyItem(at: store.fileURL, to: copy.fileURL)
        try copy.unlock(masterPassword: master)
        #expect(try copy.groups() == [prod, empty])
        #expect(try copy.hosts() == [web])
    }

    @Test
    func collapsedGroupsAreRememberedOnThisDeviceOnly() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let other = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let store = makeStore(directory)
        let copy = makeStore(other)
        defer {
            try? store.lock(); try? copy.lock()
            try? FileManager.default.removeItem(at: directory)
            try? FileManager.default.removeItem(at: other)
        }
        let master = "collapse test master password"
        try store.unlock(masterPassword: master)
        let group = HostGroup(name: "Private group name")
        let host = Maine.Host(name: "Web", hostname: "web.example")
        try store.saveHost(host, password: "secret")
        try store.saveLayout(groups: [group], hosts: [host])
        let saved = SavedSession(hostID: host.id, name: "Session", history: "")
        try store.saveSession(saved)

        let vaultBefore = try Data(contentsOf: store.fileURL)
        #expect(try store.collapsedGroupIDs().isEmpty)
        try store.setCollapsedGroupIDs([group.id])
        #expect(try Data(contentsOf: store.fileURL) == vaultBefore)
        #expect(try store.collapsedGroupIDs() == [group.id])
        #expect(try store.savedSessions() == [saved])
        let text = try String(contentsOf: try #require(store.sessionsFileURL), encoding: .utf8)
        #expect(!text.contains(group.id.uuidString) && !text.contains(group.name))

        try store.lock()
        #expect(throws: PasswordStore.VaultError.self) { try store.collapsedGroupIDs() }
        try store.unlock(masterPassword: master)
        #expect(try store.collapsedGroupIDs() == [group.id])

        try FileManager.default.createDirectory(at: other, withIntermediateDirectories: true)
        try FileManager.default.copyItem(at: store.fileURL, to: copy.fileURL)
        try copy.unlock(masterPassword: master)
        #expect(try copy.collapsedGroupIDs().isEmpty)
    }

    @Test
    func aFirstVersionLocalFileStillLoadsItsSessions() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let store = makeStore(directory)
        defer { try? store.lock(); try? FileManager.default.removeItem(at: directory) }
        let master = "local file v1 master password"
        let host = Maine.Host(name: "Server", hostname: "server.example")
        let saved = SavedSession(hostID: host.id, name: "Kept", history: "scrollback")
        let vault = try Self.craftVault(master: master, host: host, sessions: nil)
        try vault.data.write(to: store.fileURL)
        // The first local format: the payload is a bare session array.
        let payload = try #require(AES.GCM.seal(JSONEncoder().encode([saved]), using: vault.key,
                                                authenticating: vault.context + Data(":sessions".utf8)).combined)
        let file = try JSONSerialization.data(withJSONObject: [
            "version": 1, "vault": vault.id.uuidString, "payload": payload.base64EncodedString()
        ])
        try file.write(to: directory.appendingPathComponent("sessions-\(vault.id.uuidString).maine"))

        try store.unlock(masterPassword: master)
        #expect(try store.savedSessions() == [saved])
        #expect(try store.collapsedGroupIDs().isEmpty)
        try store.setCollapsedGroupIDs([UUID()])
        #expect(try store.savedSessions() == [saved])
    }

    struct CraftedVault {
        var data: Data
        var key: SymmetricKey
        var context: Data
        var id: UUID
    }

    /// A v2 vault as written by builds that kept sessions in the payload.
    static func vaultWithEmbeddedSessions(master: String, host: Maine.Host, sessions: [SavedSession]) throws -> Data {
        try craftVault(master: master, host: host, sessions: sessions).data
    }

    static func craftVault(master: String, host: Maine.Host, sessions: [SavedSession]?) throws -> CraftedVault {
        let id = UUID(), salt = Data(repeating: 9, count: 32)
        var bytes = Data(count: 32)
        let status = bytes.withUnsafeMutableBytes { output in
            salt.withUnsafeBytes { saltBytes in
                master.withCString { password in
                    CCKeyDerivationPBKDF(CCPBKDFAlgorithm(kCCPBKDF2), password, master.utf8.count,
                        saltBytes.baseAddress!.assumingMemoryBound(to: UInt8.self), salt.count,
                        CCPseudoRandomAlgorithm(kCCPRFHmacAlgSHA256), 600_000,
                        output.baseAddress!.assumingMemoryBound(to: UInt8.self), output.count)
                }
            }
        }
        #expect(status == kCCSuccess)
        struct Contents: Encodable {
            var hosts: [Maine.Host]
            var passwords: [String: String]
            var sessions: [SavedSession]?
        }
        let key = SymmetricKey(data: bytes)
        let context = Data("maine-vault:2:\(id.uuidString):\(salt.base64EncodedString()):600000".utf8)
        let verification = try #require(AES.GCM.seal(Data("Maine password vault v1".utf8), using: key,
                                                    authenticating: context).combined)
        let plain = try JSONEncoder().encode(Contents(hosts: [host], passwords: [host.id.uuidString: "host secret"],
                                                      sessions: sessions))
        let payload = try #require(AES.GCM.seal(plain, using: key, authenticating: context).combined)
        let data = try JSONSerialization.data(withJSONObject: [
            "version": 2, "id": id.uuidString, "salt": salt.base64EncodedString(), "iterations": 600_000,
            "verification": verification.base64EncodedString(), "passwords": [String: String](),
            "payload": payload.base64EncodedString()
        ])
        return CraftedVault(data: data, key: key, context: context, id: id)
    }
}
