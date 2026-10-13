import CryptoKit
import Foundation

public enum SyncEngineError: Error, LocalizedError, Equatable, Sendable {
    case noLocalVault
    case wrongMasterPassword(remoteCopy: String)
    case unreadableRemoteVault(remoteCopy: String)
    case foreignVault(remoteCopy: String)
    case shaMismatch(remoteCopy: String)

    public var errorDescription: String? {
        switch self {
        case .noLocalVault:
            return "There is no local vault to sync yet."
        case .wrongMasterPassword(let copy):
            return "The master password did not open the vault on the server. Nothing was changed; the downloaded copy is \(copy)."
        case .unreadableRemoteVault(let copy):
            return "The server holds a vault this version of Maine cannot read. Update the app; the downloaded copy is \(copy)."
        case .foreignVault(let copy):
            return "The vault on the server belongs to a different master password. Nothing was changed; the downloaded copy is \(copy)."
        case .shaMismatch(let copy):
            return "The server's vault does not match its checksum. It was kept as \(copy); nothing was changed."
        }
    }
}

/// The client-side sync algorithms from `docs/sync-client.md`: whole-file,
/// byte for byte, with a conflict copy whenever a local edit would be lost.
/// Everything here is files and one `SyncTransport`; nothing knows about
/// SwiftUI or the run loop.
public struct SyncEngine: Sendable {
    public let vaultURL: URL
    public let passwords: PasswordStore
    public let transport: any SyncTransport
    private let timestamp: @Sendable () -> String

    public init(vaultURL: URL,
                passwords: PasswordStore,
                transport: any SyncTransport,
                timestamp: @escaping @Sendable () -> String = { SyncEngine.fileTimestamp() }) {
        self.vaultURL = vaultURL
        self.passwords = passwords
        self.transport = transport
        self.timestamp = timestamp
    }

    public static func fileTimestamp(_ date: Date = Date()) -> String {
        let formatter = DateFormatter()
        formatter.locale = Locale(identifier: "en_US_POSIX")
        formatter.dateFormat = "yyyyMMdd-HHmmss"
        return formatter.string(from: date)
    }

    public static func sha256Hex(_ data: Data) -> String {
        SHA256.hash(data: data).map { String(format: "%02x", $0) }.joined()
    }

    // MARK: - Local files

    public var localVaultExists: Bool {
        FileManager.default.fileExists(atPath: vaultURL.path)
    }

    public func localVaultData() throws -> Data {
        guard localVaultExists else { throw SyncEngineError.noLocalVault }
        return try Data(contentsOf: vaultURL)
    }

    /// Hex SHA-256 of the live vault, or nil when there is none.
    public func localVaultSHA256() -> String? {
        guard let data = try? Data(contentsOf: vaultURL) else { return nil }
        return Self.sha256Hex(data)
    }

    public func workspaceURL(for profileID: UUID) -> URL {
        vaultURL.deletingLastPathComponent()
            .appendingPathComponent("passwords.workspace-\(profileID.uuidString).maine")
    }

    /// Saves a copy next to the vault, `0600`, and returns its file name. Used
    /// for the pre-sync, remote and conflict cases; the copies are ordinary
    /// vault files and sync never deletes them.
    @discardableResult
    public func saveCopy(_ data: Data, kind: String) throws -> String {
        let name = "passwords.\(kind)-\(timestamp()).maine"
        let url = vaultURL.deletingLastPathComponent().appendingPathComponent(name)
        try FileManager.default.createDirectory(at: url.deletingLastPathComponent(), withIntermediateDirectories: true)
        try data.write(to: url, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: url.path)
        return name
    }

    /// Copies the live vault to `destination`, `0600`, leaving the live file
    /// in place.
    public func copyLocalVault(to destination: URL) throws {
        guard let data = try? localVaultData() else { return }
        if FileManager.default.fileExists(atPath: destination.path) {
            try FileManager.default.removeItem(at: destination)
        }
        try data.write(to: destination, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: destination.path)
    }

    /// Moves the live vault to `destination` when it exists.
    public func moveLocalVault(to destination: URL) throws {
        guard localVaultExists else { return }
        if FileManager.default.fileExists(atPath: destination.path) {
            try FileManager.default.removeItem(at: destination)
        }
        try FileManager.default.moveItem(at: vaultURL, to: destination)
    }

    /// Moves a live vault aside as a `passwords.<kind>-….maine` copy.
    @discardableResult
    public func moveLocalVaultAside(kind: String) throws -> String? {
        guard localVaultExists else { return nil }
        let name = "passwords.\(kind)-\(timestamp()).maine"
        try moveLocalVault(to: vaultURL.deletingLastPathComponent().appendingPathComponent(name))
        return name
    }

    /// Moves a workspace file into place as the live vault.
    public func adoptLocalVault(from source: URL) throws {
        guard FileManager.default.fileExists(atPath: source.path) else { throw SyncEngineError.noLocalVault }
        if localVaultExists {
            try FileManager.default.removeItem(at: vaultURL)
        }
        try FileManager.default.moveItem(at: source, to: vaultURL)
    }

    private func atomicReplace(with data: Data) throws {
        try FileManager.default.createDirectory(at: vaultURL.deletingLastPathComponent(), withIntermediateDirectories: true)
        try data.write(to: vaultURL, options: .atomic)
        try FileManager.default.setAttributes([.posixPermissions: 0o600], ofItemAtPath: vaultURL.path)
    }

    // MARK: - Create

    /// Uploads the current vault as the first version of a new slot. The
    /// server returns the token exactly once; persist it before showing
    /// success.
    public func create(at server: URL, createSecret: String?) async throws -> (token: String, state: SyncSlotState) {
        let body = (try? localVaultData()) ?? Data()
        return try await transport.createVault(at: server, createSecret: createSecret, body: body)
    }

    // MARK: - Join

    public enum JoinPreparation: Equatable, Sendable {
        case emptySlot
        /// Validated remote bytes, ready to adopt once the caller has parked
        /// whatever vault is live now.
        case ready(Data, SyncSlotState)
    }

    /// Downloads and validates the remote vault without changing any file.
    public func prepareJoin(server: URL, token: String, masterPassword: String) async throws -> JoinPreparation {
        switch try await transport.fetchVault(at: server, token: token, ifNoneMatch: nil) {
        case .empty:
            return .emptySlot
        case .notModified, .current:
            throw SyncTransportError.malformed("The server answered without the vault body.")
        case .blob(let bytes, let state):
            if let expected = state.sha256, expected != Self.sha256Hex(bytes) {
                let copy = try saveCopy(bytes, kind: "remote")
                throw SyncEngineError.shaMismatch(remoteCopy: copy)
            }
            do {
                try passwords.validate(bytes, masterPassword: masterPassword)
            } catch PasswordStore.VaultError.invalidPassword {
                let copy = try saveCopy(bytes, kind: "remote")
                throw SyncEngineError.wrongMasterPassword(remoteCopy: copy)
            } catch {
                let copy = try saveCopy(bytes, kind: "remote")
                throw SyncEngineError.unreadableRemoteVault(remoteCopy: copy)
            }
            return .ready(bytes, state)
        }
    }

    /// Replaces the live vault with previously validated bytes. A local vault
    /// that is still present is moved to a pre-sync copy first.
    public func adoptPreparedJoin(_ bytes: Data, state: SyncSlotState) throws {
        guard state.version > 0 else { throw SyncTransportError.malformed("The slot reported version 0.") }
        if localVaultExists {
            let copyName = "passwords.pre-sync-\(timestamp()).maine"
            try moveLocalVault(to: vaultURL.deletingLastPathComponent().appendingPathComponent(copyName))
        }
        try atomicReplace(with: bytes)
    }

    // MARK: - Pull

    public enum PullResult: Equatable, Sendable {
        case notModified(SyncSlotState)
        case empty(SyncSlotState)
        case adopted(SyncSlotState, conflictCopy: String?)
    }

    public func pull(server: URL, token: String, lastVersion: Int, isDirty: Bool) async throws -> PullResult {
        switch try await transport.fetchVault(at: server, token: token, ifNoneMatch: lastVersion) {
        case .notModified(let state):
            return .notModified(state)
        case .empty(let state):
            return .empty(state)
        case .current:
            throw SyncTransportError.malformed("The server answered without the vault body.")
        case .blob(let bytes, let state):
            if let expected = state.sha256, expected != Self.sha256Hex(bytes) {
                let copy = try saveCopy(bytes, kind: "remote")
                throw SyncEngineError.shaMismatch(remoteCopy: copy)
            }
            guard passwords.sessionKeyOpens(bytes) else {
                let copy = try saveCopy(bytes, kind: "remote")
                throw SyncEngineError.foreignVault(remoteCopy: copy)
            }
            var conflictCopy: String?
            if isDirty, let local = try? localVaultData() {
                conflictCopy = try saveCopy(local, kind: "conflict")
            }
            try atomicReplace(with: bytes)
            return .adopted(state, conflictCopy: conflictCopy)
        }
    }

    /// A cheap `HEAD` used while polling; `304` almost always.
    public func head(server: URL, token: String, lastVersion: Int) async throws -> SyncFetchResult {
        try await transport.headVault(at: server, token: token, ifNoneMatch: lastVersion)
    }

    // MARK: - Push

    public enum PushResult: Equatable, Sendable {
        /// The file already matches the last synced version.
        case skipped(SyncSlotState)
        case stored(SyncSlotState)
    }

    public func push(server: URL, token: String, lastVersion: Int, lastSHA256: String?) async throws -> PushResult {
        let bytes = try localVaultData()
        let sha = Self.sha256Hex(bytes)
        if sha == lastSHA256 {
            return .skipped(SyncSlotState(version: lastVersion, size: bytes.count, sha256: sha))
        }
        let state = try await transport.putVault(at: server, token: token, body: bytes,
                                                 expectedVersion: lastVersion > 0 ? lastVersion : nil)
        return .stored(state)
    }

    // MARK: - Token and slot management

    public func rotate(server: URL, token: String) async throws -> String {
        try await transport.rotateToken(at: server, token: token)
    }

    public func delete(server: URL, token: String) async throws {
        try await transport.deleteVault(at: server, token: token)
    }
}
