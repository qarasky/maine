import Foundation
@testable import Maine
import Testing

@MainActor
@Suite
struct SyncProfileStoreTests {
    @Test
    func profilesAndTokensPersistAndRemoveCleanly() throws {
        let suiteName = "app.maine.tests.sync-profiles.\(UUID().uuidString)"
        let defaults = try #require(UserDefaults(suiteName: suiteName))
        defer { defaults.removePersistentDomain(forName: suiteName) }
        let service = "app.maine.tests.sync-tokens.\(UUID().uuidString)"

        let store = SyncProfileStore(defaults: defaults, keychainService: service)
        let profile = SyncProfile(name: "Homelab", serverURL: URL(string: "http://127.0.0.1:8484")!)
        try store.add(profile, token: "maine-TEST-TOKEN")

        #expect(store.profiles == [profile])
        #expect(store.activeProfileID == profile.id)
        #expect(try store.token(for: profile.id) == "maine-TEST-TOKEN")

        // A second store instance sees the same record and token.
        let reopened = SyncProfileStore(defaults: defaults, keychainService: service)
        #expect(reopened.profiles == [profile])
        #expect(reopened.activeProfileID == profile.id)
        #expect(try reopened.token(for: profile.id) == "maine-TEST-TOKEN")

        // Rotation replaces the token; removal forgets both halves.
        try reopened.replaceToken("maine-NEW-TOKEN", for: profile.id)
        #expect(try reopened.token(for: profile.id) == "maine-NEW-TOKEN")

        try reopened.remove(profile.id)
        #expect(reopened.profiles.isEmpty)
        #expect(reopened.activeProfileID == nil)
        #expect(try reopened.token(for: profile.id) == nil)
    }
}

@Suite
struct PasswordStoreSyncTests {
    private func temporaryStore() -> (PasswordStore, URL) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let store = PasswordStore(fileURL: directory.appendingPathComponent("passwords.maine"),
                                  keychainService: "app.maine.tests.\(UUID())")
        return (store, directory)
    }

    @Test
    func validateAcceptsTheVaultAndRejectsBadInput() throws {
        let (store, directory) = temporaryStore()
        defer {
            try? store.lock()
            try? FileManager.default.removeItem(at: directory)
        }
        try store.unlock(masterPassword: "validation test password")
        let bytes = try Data(contentsOf: store.fileURL)

        try store.validate(bytes, masterPassword: "validation test password")

        #expect(throws: PasswordStore.VaultError.invalidPassword) {
            try store.validate(bytes, masterPassword: "definitely not the password")
        }
        #expect(throws: PasswordStore.VaultError.invalidFile) {
            try store.validate(Data("not a vault at all".utf8), masterPassword: "validation test password")
        }
    }

    @Test
    func sessionKeyOpensOnlyThisVaultsData() throws {
        let (first, firstDirectory) = temporaryStore()
        let (second, secondDirectory) = temporaryStore()
        defer {
            try? first.lock()
            try? second.lock()
            try? FileManager.default.removeItem(at: firstDirectory)
            try? FileManager.default.removeItem(at: secondDirectory)
        }
        try first.unlock(masterPassword: "first vault master password")
        try second.unlock(masterPassword: "second vault master password")
        let firstBytes = try Data(contentsOf: first.fileURL)
        let secondBytes = try Data(contentsOf: second.fileURL)

        #expect(first.sessionKeyOpens(firstBytes))
        #expect(!first.sessionKeyOpens(secondBytes))
        #expect(!first.sessionKeyOpens(Data("not a vault".utf8)))
    }
}
