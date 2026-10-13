import CommonCrypto
import CryptoKit
import Foundation
import Security

/// Portable AES-256-GCM hosts and credentials protected by a master password.
/// The master password never goes on disk. Keychain remembers the derived key
/// on this Mac only, so OpenSSH's askpass helper can reuse saved credentials.
public struct PasswordStore: Sendable {
    public static let shared = PasswordStore()
    public static var defaultFileURL: URL {
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Maine", isDirectory: true)
            .appendingPathComponent("passwords.maine")
    }
    public let fileURL: URL
    private let keys: LocalVaultKeyStore
    private let legacyHostsFileURL: URL?
    private static let iterations = 600_000
    private static let marker = Data("Maine password vault v1".utf8)

    public enum VaultError: Error, LocalizedError {
        case locked, invalidPassword, invalidFile, weakPassword

        public var errorDescription: String? {
            switch self {
            case .locked: "The password vault is locked. Choose Host → Unlock Password Vault to enter its master password."
            case .invalidPassword: "The master password is incorrect, or the vault has been damaged."
            case .invalidFile: "This password vault is damaged or uses an unsupported format. It has not been changed."
            case .weakPassword: "Use a master password of at least 12 characters."
            }
        }
    }

    private struct Vault: Codable {
        var version = 1
        var id: UUID
        var salt: Data
        var iterations: Int
        var verification: Data
        var passwords: [String: Data]
        var payload: Data?

        var context: Data {
            Data("maine-vault:\(version):\(id.uuidString):\(salt.base64EncodedString()):\(iterations)".utf8)
        }
    }

    private struct Contents: Codable {
        var hosts: [Host] = []
        var passwords: [String: String] = [:]
        var sessions: [SavedSession]?
    }

    public static var legacyHostsURL: URL {
        defaultFileURL.deletingLastPathComponent().appendingPathComponent("hosts.json")
    }

    public init(fileURL: URL = PasswordStore.defaultFileURL, keychainService: String = "app.maine.vault-key",
                legacyHostsFileURL: URL? = nil) {
        self.fileURL = fileURL
        keys = LocalVaultKeyStore(service: keychainService)
        self.legacyHostsFileURL = legacyHostsFileURL ?? (fileURL == Self.defaultFileURL ? Self.legacyHostsURL : nil)
    }

    public var exists: Bool { FileManager.default.fileExists(atPath: fileURL.path) }

    /// Creates a vault on first use, or verifies and unlocks an existing copy.
    public func unlock(masterPassword: String) throws {
        if var vault = try read() {
            let key = try derive(masterPassword, salt: vault.salt, iterations: vault.iterations)
            var contents = Contents()
            do {
                guard try open(vault.verification, key: key, context: vault.context) == Self.marker else {
                    throw VaultError.invalidPassword
                }
                // Verify every entry before remembering a key for this vault.
                for (hostID, data) in vault.passwords {
                    let plain = try open(data, key: key, context: vault.context + Data(hostID.utf8))
                    guard let password = String(data: plain, encoding: .utf8) else { throw VaultError.invalidFile }
                    contents.passwords[hostID] = password
                }
                if vault.version == 2 { contents = try decodeContents(vault, key: key) }
            } catch {
                throw VaultError.invalidPassword
            }
            if vault.version == 1 {
                // Read legacy metadata only after authenticating the master key.
                // Keep the originals as backups; never overwrite malformed input.
                if let legacyHostsFileURL,
                   FileManager.default.fileExists(atPath: legacyHostsFileURL.path) {
                    contents.hosts = try HostConfiguration(data: Data(contentsOf: legacyHostsFileURL)).hosts
                }
                let backup = fileURL.appendingPathExtension("v1-backup")
                if !FileManager.default.fileExists(atPath: backup.path) {
                    try FileManager.default.copyItem(at: fileURL, to: backup)
                }
                vault.version = 2
                vault.passwords = [:]
                vault.verification = try seal(Self.marker, key: key, context: vault.context)
                vault.payload = try seal(JSONEncoder().encode(contents), key: key, context: vault.context)
                try write(vault)
            }
            try keys.save(key.withUnsafeBytes { Data($0) }, for: vault.id)
        } else {
            guard masterPassword.count >= 12 else { throw VaultError.weakPassword }
            var salt = Data(count: 32)
            let status = salt.withUnsafeMutableBytes { SecRandomCopyBytes(kSecRandomDefault, $0.count, $0.baseAddress!) }
            guard status == errSecSuccess else { throw VaultError.invalidFile }
            var vault = Vault(id: UUID(), salt: salt, iterations: Self.iterations,
                              verification: Data(), passwords: [:])
            vault.version = 2
            let key = try derive(masterPassword, salt: salt, iterations: vault.iterations)
            vault.verification = try seal(Self.marker, key: key, context: vault.context)
            var contents = Contents()
            if let legacyHostsFileURL,
               FileManager.default.fileExists(atPath: legacyHostsFileURL.path) {
                contents.hosts = try HostConfiguration(data: Data(contentsOf: legacyHostsFileURL)).hosts
            }
            vault.payload = try seal(JSONEncoder().encode(contents), key: key, context: vault.context)
            try write(vault)
            try keys.save(key.withUnsafeBytes { Data($0) }, for: vault.id)
        }
    }

    public func lock() throws {
        if let vault = try read() { try keys.delete(for: vault.id) }
    }

    public func password(for hostID: Host.ID) throws -> String? {
        guard let vault = try read() else { return nil }
        return try decodeContents(vault, key: unlockedKey(vault)).passwords[hostID.uuidString]
    }

    public func save(_ password: String, for hostID: Host.ID) throws {
        if password.isEmpty { try delete(for: hostID); return }
        try updateContents { $0.passwords[hostID.uuidString] = password }
    }

    public func delete(for hostID: Host.ID) throws {
        guard exists else { return }
        try updateContents { $0.passwords.removeValue(forKey: hostID.uuidString) }
    }

    public func hosts() throws -> [Host] {
        guard let vault = try read() else { return [] }
        return try decodeContents(vault, key: unlockedKey(vault)).hosts
    }

    public func saveHosts(_ hosts: [Host]) throws {
        try updateContents { $0.hosts = hosts }
    }

    /// Commit metadata and its credential together in one atomic replacement.
    public func saveHost(_ host: Host, password: String) throws {
        guard host.isValid, !password.isEmpty else { throw VaultError.invalidFile }
        try updateContents { contents in
            if let index = contents.hosts.firstIndex(where: { $0.id == host.id }) {
                contents.hosts[index] = host
            } else {
                contents.hosts.append(host)
            }
            contents.passwords[host.id.uuidString] = password
        }
    }

    public func deleteHost(_ hostID: Host.ID) throws {
        try updateContents {
            $0.hosts.removeAll { $0.id == hostID }
            $0.passwords.removeValue(forKey: hostID.uuidString)
            $0.sessions?.removeAll { $0.hostID == hostID }
        }
    }

    public func savedSessions() throws -> [SavedSession] {
        guard let vault = try read() else { return [] }
        return try decodeContents(vault, key: unlockedKey(vault)).sessions ?? []
    }

    public func saveSession(_ session: SavedSession) throws {
        guard !session.name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else {
            throw VaultError.invalidFile
        }
        try updateContents { contents in
            var sessions = contents.sessions ?? []
            if let index = sessions.firstIndex(where: { $0.id == session.id }) { sessions[index] = session }
            else { sessions.append(session) }
            contents.sessions = sessions
        }
    }

    public func saveSessions(_ sessions: [SavedSession]) throws {
        try updateContents { contents in
            var saved = contents.sessions ?? []
            for session in sessions {
                if let index = saved.firstIndex(where: { $0.id == session.id }) { saved[index] = session }
                else { saved.append(session) }
            }
            contents.sessions = saved
        }
    }

    public func deleteSavedSession(_ id: UUID) throws {
        try updateContents { $0.sessions?.removeAll { $0.id == id } }
    }

    private func updateContents(_ update: (inout Contents) -> Void) throws {
        guard var vault = try read() else { throw VaultError.locked }
        let key = try unlockedKey(vault)
        var contents = try decodeContents(vault, key: key)
        update(&contents)
        vault.payload = try seal(JSONEncoder().encode(contents), key: key, context: vault.context)
        try write(vault)
    }

    private func decodeContents(_ vault: Vault, key: SymmetricKey) throws -> Contents {
        guard vault.version == 2, let payload = vault.payload else { throw VaultError.invalidFile }
        let contents = try JSONDecoder().decode(Contents.self, from: open(payload, key: key, context: vault.context))
        _ = try HostConfiguration(data: HostConfiguration(hosts: contents.hosts).encoded())
        guard contents.passwords.keys.allSatisfy({ UUID(uuidString: $0) != nil }) else { throw VaultError.invalidFile }
        return contents
    }

    private func unlockedKey(_ vault: Vault) throws -> SymmetricKey {
        guard let data = try keys.key(for: vault.id), data.count == 32 else { throw VaultError.locked }
        let key = SymmetricKey(data: data)
        guard try open(vault.verification, key: key, context: vault.context) == Self.marker else {
            throw VaultError.invalidFile
        }
        return key
    }

    private func read() throws -> Vault? {
        guard exists else { return nil }
        return try decodeVault(from: Data(contentsOf: fileURL))
    }

    /// Structural validation shared by `read()` and the sync helpers. It never
    /// touches the file or the Keychain.
    private func decodeVault(from data: Data) throws -> Vault {
        do {
            let vault = try JSONDecoder().decode(Vault.self, from: data)
            guard [1, 2].contains(vault.version), vault.salt.count == 32,
                  vault.iterations == Self.iterations,
                  (vault.version == 1 || (vault.payload != nil && vault.passwords.isEmpty)),
                  vault.passwords.keys.allSatisfy({ UUID(uuidString: $0) != nil }) else {
                throw VaultError.invalidFile
            }
            return vault
        } catch { throw VaultError.invalidFile }
    }

    /// True when the current session key opens `data` as one of this vault's
    /// versions. Sync calls this before adopting a remote blob; it never
    /// writes, migrates or remembers anything.
    public func sessionKeyOpens(_ data: Data) -> Bool {
        guard let vault = try? decodeVault(from: data),
              let keyData = try? keys.key(for: vault.id), keyData.count == 32 else { return false }
        let key = SymmetricKey(data: keyData)
        guard let marker = try? open(vault.verification, key: key, context: vault.context) else { return false }
        return marker == Self.marker
    }

    /// Validates `data` as a full vault encrypted with `masterPassword`,
    /// without touching this instance's file or Keychain state. Throws
    /// `.invalidPassword` when the password does not open it and
    /// `.invalidFile` when it is not a vault this client understands.
    public func validate(_ data: Data, masterPassword: String) throws {
        let vault = try decodeVault(from: data)
        let key = try derive(masterPassword, salt: vault.salt, iterations: vault.iterations)
        let marker: Data
        do {
            marker = try open(vault.verification, key: key, context: vault.context)
        } catch {
            throw VaultError.invalidPassword
        }
        guard marker == Self.marker else { throw VaultError.invalidPassword }
        do {
            for (hostID, payload) in vault.passwords {
                let plain = try open(payload, key: key, context: vault.context + Data(hostID.utf8))
                guard String(data: plain, encoding: .utf8) != nil else { throw VaultError.invalidFile }
            }
            if vault.version == 2 { _ = try decodeContents(vault, key: key) }
        } catch let error as VaultError {
            throw error
        } catch {
            throw VaultError.invalidFile
        }
    }

    private func write(_ vault: Vault) throws {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
        try FileManager.default.createDirectory(at: fileURL.deletingLastPathComponent(), withIntermediateDirectories: true)
        try encoder.encode(vault).write(to: fileURL, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: fileURL.path)
        // Sync listens for local vault writes to schedule a push. Adopting a
        // remote blob writes the file directly, so this cannot loop.
        NotificationCenter.default.post(name: .maineVaultDidWrite, object: nil)
    }

    private func derive(_ password: String, salt: Data, iterations: Int) throws -> SymmetricKey {
        let bytes = Array(password.utf8)
        var result = Data(count: 32)
        let status = result.withUnsafeMutableBytes { output in
            salt.withUnsafeBytes { saltBytes in
                bytes.withUnsafeBytes { passwordBytes in
                    CCKeyDerivationPBKDF(CCPBKDFAlgorithm(kCCPBKDF2),
                        passwordBytes.baseAddress?.assumingMemoryBound(to: Int8.self), bytes.count,
                        saltBytes.baseAddress?.assumingMemoryBound(to: UInt8.self), salt.count,
                        CCPseudoRandomAlgorithm(kCCPRFHmacAlgSHA256), UInt32(iterations),
                        output.baseAddress?.assumingMemoryBound(to: UInt8.self), output.count)
                }
            }
        }
        guard status == kCCSuccess else { throw VaultError.invalidFile }
        return SymmetricKey(data: result)
    }

    private func seal(_ data: Data, key: SymmetricKey, context: Data) throws -> Data {
        guard let combined = try AES.GCM.seal(data, using: key, authenticating: context).combined else {
            throw VaultError.invalidFile
        }
        return combined
    }

    private func open(_ data: Data, key: SymmetricKey, context: Data) throws -> Data {
        try AES.GCM.open(AES.GCM.SealedBox(combined: data), using: key, authenticating: context)
    }
}

public extension Notification.Name {
    /// Posted after the local vault file is written. Sync schedules a push
    /// from this; adopting a remote blob writes the file directly, so the
    /// notification cannot loop. It carries no user data.
    static let maineVaultDidWrite = Notification.Name("app.maine.vault-did-write")
}
