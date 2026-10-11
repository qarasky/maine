import Foundation
import CommonCrypto
import CryptoKit
@testable import Maine
import Testing

@Suite
struct PasswordStoreTests {
    @Test
    func passwordsAreStoredUpdatedAndRemovedPerHost() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let store = PasswordStore(fileURL: directory.appendingPathComponent("passwords.maine"),
                                  keychainService: "app.maine.tests.\(UUID())")
        defer {
            try? store.lock()
            try? FileManager.default.removeItem(at: directory)
        }
        let first = UUID()
        let second = UUID()
        defer {
            try? store.delete(for: first)
            try? store.delete(for: second)
        }
        #expect(try store.password(for: first) == nil)
        try store.unlock(masterPassword: "a long test master password")
        try store.save("test secret with spaces 🔐", for: first)
        try store.save("another test secret", for: second)
        #expect(try store.password(for: first) == "test secret with spaces 🔐")
        let bytes = try Data(contentsOf: store.fileURL)
        let text = try #require(String(data: bytes, encoding: .utf8))
        #expect(!text.contains("test secret"))
        #expect(!text.contains("a long test master password"))
        let permissions = try FileManager.default.attributesOfItem(atPath: store.fileURL.path)[.posixPermissions] as? Int
        #expect(permissions == 0o600)
        try store.save("changed secret", for: first)
        #expect(try store.password(for: first) == "changed secret")
        #expect(try store.password(for: second) == "another test secret")
        try store.save("", for: first)
        #expect(try store.password(for: first) == nil)
        try store.delete(for: first)
    }

    @Test
    func copiedVaultUnlocksWithMasterPasswordOnAnotherInstallation() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let original = PasswordStore(fileURL: directory.appendingPathComponent("original.maine"),
                                     keychainService: "app.maine.tests.\(UUID())")
        let copy = PasswordStore(fileURL: directory.appendingPathComponent("copy.maine"),
                                 keychainService: "app.maine.tests.\(UUID())")
        defer {
            try? original.lock()
            try? copy.lock()
            try? FileManager.default.removeItem(at: directory)
        }
        let hostID = UUID()
        let host = Maine.Host(id: hostID, name: "Private server", hostname: "private.example", username: "private-user")
        try original.unlock(masterPassword: "portable master password")
        try original.saveHost(host, password: "shared test password")
        let saved = SavedSession(hostID: hostID, name: "Private session", history: "Sensitive terminal output")
        try original.saveSession(saved)
        let text = try String(contentsOf: original.fileURL, encoding: .utf8)
        for value in [host.name, host.hostname, host.username, hostID.uuidString, "shared test password", saved.name, saved.history] {
            #expect(!text.contains(value))
        }
        try FileManager.default.copyItem(at: original.fileURL, to: copy.fileURL)
        #expect(throws: PasswordStore.VaultError.self) { try copy.password(for: hostID) }
        let before = try Data(contentsOf: copy.fileURL)
        #expect(throws: PasswordStore.VaultError.self) {
            try copy.unlock(masterPassword: "incorrect master password")
        }
        #expect(try Data(contentsOf: copy.fileURL) == before)
        try copy.unlock(masterPassword: "portable master password")
        #expect(try copy.hosts() == [host])
        #expect(try copy.savedSessions() == [saved])
        #expect(try copy.password(for: hostID) == "shared test password")
        try copy.lock()
        #expect(throws: PasswordStore.VaultError.self) { try copy.hosts() }
        #expect(throws: PasswordStore.VaultError.self) { try copy.savedSessions() }
        #expect(throws: PasswordStore.VaultError.self) { try copy.password(for: hostID) }
        #expect(throws: PasswordStore.VaultError.self) { try copy.save("replacement", for: hostID) }
        #expect(try Data(contentsOf: copy.fileURL) == before)
    }

    @Test @MainActor
    func unifiedHostStoreCommitsAndDeletesMetadataAndPasswordTogether() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let vault = PasswordStore(fileURL: directory.appendingPathComponent("vault.maine"),
                                  keychainService: "app.maine.tests.\(UUID())")
        defer { try? vault.lock(); try? FileManager.default.removeItem(at: directory) }
        try vault.unlock(masterPassword: "unified test master password")
        let store = HostStore(vault: vault)
        let host = Maine.Host(name: "Server", hostname: "server.example")
        try store.saveHost(host, password: "host secret")
        #expect(store.hosts == [host])
        #expect(try vault.password(for: host.id) == "host secret")
        try vault.lock()
        store.load()
        #expect(store.hosts.isEmpty)
        #expect(throws: PasswordStore.VaultError.self) { try store.saveHost(host, password: "replacement") }
        try vault.unlock(masterPassword: "unified test master password")
        store.load()
        #expect(store.hosts == [host])
        store.delete(host)
        #expect(try vault.hosts().isEmpty)
        #expect(try vault.password(for: host.id) == nil)
    }

    @Test
    func legacyVaultMigratesOnlyAfterAuthenticationAndPreservesOriginals() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let hostsURL = directory.appendingPathComponent("hosts.json")
        let vaultURL = directory.appendingPathComponent("passwords.maine")
        let master = "legacy test master password"
        let host = Maine.Host(name: "Legacy host", hostname: "legacy.example")
        let hostData = try HostConfiguration(hosts: [host]).encoded()
        try hostData.write(to: hostsURL)
        let id = UUID(), salt = Data(repeating: 7, count: 32)
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
        let key = SymmetricKey(data: bytes)
        let context = Data("maine-vault:1:\(id.uuidString):\(salt.base64EncodedString()):600000".utf8)
        let verification = try #require(AES.GCM.seal(Data("Maine password vault v1".utf8), using: key,
                                                    authenticating: context).combined)
        let secret = try #require(AES.GCM.seal(Data("legacy secret".utf8), using: key,
            authenticating: context + Data(host.id.uuidString.utf8)).combined)
        let original = try JSONSerialization.data(withJSONObject: [
            "version": 1, "id": id.uuidString, "salt": salt.base64EncodedString(), "iterations": 600_000,
            "verification": verification.base64EncodedString(), "passwords": [host.id.uuidString: secret.base64EncodedString()]
        ])
        try original.write(to: vaultURL)
        let vault = PasswordStore(fileURL: vaultURL, keychainService: "app.maine.tests.\(UUID())",
                                  legacyHostsFileURL: hostsURL)
        defer { try? vault.lock(); try? FileManager.default.removeItem(at: directory) }
        #expect(throws: PasswordStore.VaultError.self) { try vault.unlock(masterPassword: "wrong master password") }
        #expect(try Data(contentsOf: vaultURL) == original)
        try Data("not JSON".utf8).write(to: hostsURL)
        #expect(throws: (any Error).self) { try vault.unlock(masterPassword: master) }
        #expect(try Data(contentsOf: vaultURL) == original)
        try hostData.write(to: hostsURL)
        try vault.unlock(masterPassword: master)
        #expect(try vault.hosts() == [host])
        #expect(try vault.password(for: host.id) == "legacy secret")
        #expect(try Data(contentsOf: vaultURL.appendingPathExtension("v1-backup")) == original)
        #expect(try Data(contentsOf: hostsURL) == hostData)
        // Migration must not re-import stale plaintext on subsequent unlocks.
        try vault.deleteHost(host.id)
        try vault.lock()
        try vault.unlock(masterPassword: master)
        #expect(try vault.hosts().isEmpty)
    }

    @Test
    func tamperedPayloadIsRejected() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let store = PasswordStore(fileURL: directory.appendingPathComponent("passwords.maine"),
                                  keychainService: "app.maine.tests.\(UUID())")
        defer {
            try? store.lock()
            try? FileManager.default.removeItem(at: directory)
        }
        let first = UUID(), second = UUID()
        try store.unlock(masterPassword: "test integrity master password")
        try store.save("first secret", for: first)
        try store.save("second secret", for: second)
        var json = try #require(JSONSerialization.jsonObject(with: Data(contentsOf: store.fileURL)) as? [String: Any])
        let encoded = try #require(json["payload"] as? String)
        var ciphertext = try #require(Data(base64Encoded: encoded))
        ciphertext[ciphertext.count - 1] ^= 1
        json["payload"] = ciphertext.base64EncodedString()
        try JSONSerialization.data(withJSONObject: json).write(to: store.fileURL)
        #expect(throws: (any Error).self) { try store.password(for: second) }
        #expect(throws: PasswordStore.VaultError.self) {
            try store.unlock(masterPassword: "test integrity master password")
        }
    }

    @Test
    func askPassEnvironmentContainsOnlyAReferenceNotTheSecret() {
        let hostID = UUID()
        let environment = SSHAskPass.environment(
            for: hostID,
            base: ["TERM=xterm", "SSH_AUTH_SOCK=/agent", "SSH_ASKPASS=/old", "SSH_ASKPASS_REQUIRE=never"],
            executable: "/Applications/Maine.app/Contents/MacOS/Maine"
        )
        #expect(environment.contains("TERM=xterm"))
        #expect(environment.contains("SSH_AUTH_SOCK=/agent"))
        #expect(environment.contains("MAINE_SSH_HOST_ID=\(hostID.uuidString)"))
        #expect(environment.contains("SSH_ASKPASS_REQUIRE=force"))
        #expect(environment.filter { $0.hasPrefix("SSH_ASKPASS=") }.count == 1)
        #expect(!environment.contains("SSH_ASKPASS_REQUIRE=never"))
    }

    @Test
    func savedPasswordIsNotUsedForTrustPassphrasesOrMFA() {
        #expect(SSHAskPass.isPasswordPrompt("pi@raspberrypi's password:"))
        #expect(SSHAskPass.isPasswordPrompt("Password:"))
        #expect(!SSHAskPass.isPasswordPrompt("Enter passphrase for key '/Users/me/.ssh/id_ed25519':"))
        #expect(!SSHAskPass.isPasswordPrompt("Verification password:"))
        #expect(!SSHAskPass.isPasswordPrompt("One-time password:"))
        #expect(!SSHAskPass.isPasswordPrompt("Are you sure you want to continue connecting (yes/no/[fingerprint])?"))
    }
}
