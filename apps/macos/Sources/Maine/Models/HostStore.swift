import AppKit
import Foundation

public struct HostStoreError: Error, LocalizedError, Equatable {
    public let message: String

    public init(_ message: String) {
        self.message = message
    }

    public var errorDescription: String? { message }
}

/// Owns the host list backed by the encrypted vault.
///
/// There is no backend and no syncing: the file below is the only copy, and it
/// is written atomically after every change.
@MainActor
public final class HostStore: ObservableObject {
    /// A load problem that Maine refuses to guess about. The file is left
    /// untouched so the user can look at it and decide.
    public struct LoadFailure: Identifiable, Equatable {
        public let id = UUID()
        public let message: String
        public let fileURL: URL

        public init(message: String, fileURL: URL) {
            self.message = message
            self.fileURL = fileURL
        }
    }

    @Published public private(set) var hosts: [Host] = []
    @Published public private(set) var loadFailure: LoadFailure?
    @Published public private(set) var saveFailure: String?

    public let fileURL: URL
    private let vault: PasswordStore?

    public init(fileURL: URL? = nil) {
        self.fileURL = fileURL ?? Self.defaultFileURL
        self.vault = fileURL == nil ? .shared : nil
    }

    public init(vault: PasswordStore) {
        self.fileURL = vault.fileURL
        self.vault = vault
    }

    /// The unified encrypted vault. Explicit JSON URLs support legacy tooling/tests.
    public static var defaultFileURL: URL {
        PasswordStore.defaultFileURL
    }

    // MARK: - Reading

    public func load() {
        if let vault {
            do {
                hosts = try vault.hosts()
                loadFailure = nil
            } catch PasswordStore.VaultError.locked {
                hosts = []
                loadFailure = nil
            } catch {
                hosts = []
                loadFailure = LoadFailure(message: error.localizedDescription, fileURL: fileURL)
            }
            return
        }
        guard FileManager.default.fileExists(atPath: fileURL.path) else {
            // First run: no file is a perfectly good empty list.
            hosts = []
            loadFailure = nil
            return
        }

        do {
            let data = try Data(contentsOf: fileURL)
            let configuration = try HostConfiguration(data: data)
            hosts = configuration.hosts
            loadFailure = nil
        } catch {
            hosts = []
            loadFailure = LoadFailure(message: error.localizedDescription, fileURL: fileURL)
        }
    }

    // MARK: - Writing

    @discardableResult
    public func add(_ host: Host) -> Result<Void, HostStoreError> {
        guard host.isValid else { return .failure(HostStoreError(host.issues.joined(separator: " "))) }
        hosts.append(host)
        return save()
    }

    @discardableResult
    public func add(_ newHosts: [Host]) -> Result<Void, HostStoreError> {
        let invalid = newHosts.filter { !$0.isValid }
        guard invalid.isEmpty else {
            return .failure(HostStoreError(
                invalid.map(\.displayName).joined(separator: ", ") + " cannot be added yet."
            ))
        }
        hosts.append(contentsOf: newHosts)
        return save()
    }

    @discardableResult
    public func update(_ host: Host) -> Result<Void, HostStoreError> {
        guard host.isValid else { return .failure(HostStoreError(host.issues.joined(separator: " "))) }
        guard let index = hosts.firstIndex(where: { $0.id == host.id }) else {
            return .failure(HostStoreError("That host no longer exists."))
        }
        hosts[index] = host
        return save()
    }

    public func delete(_ host: Host) {
        if let vault {
            do {
                try vault.deleteHost(host.id)
                hosts.removeAll { $0.id == host.id }
                saveFailure = nil
            } catch { saveFailure = error.localizedDescription }
            return
        }
        hosts.removeAll { $0.id == host.id }
        _ = save()
    }

    public func delete(at offsets: IndexSet) {
        hosts.remove(atOffsets: offsets)
        _ = save()
    }

    public func containsHost(named hostname: String) -> Bool {
        hosts.contains { $0.hostname.compare(hostname, options: .caseInsensitive) == .orderedSame }
    }

    public func host(id: Host.ID?) -> Host? {
        guard let id else { return nil }
        return hosts.first { $0.id == id }
    }

    public func saveHost(_ host: Host, password: String) throws {
        if let vault {
            try vault.saveHost(host, password: password)
            load()
        } else {
            try PasswordStore.shared.save(password, for: host.id)
            if self.host(id: host.id) == nil { try add(host).get() }
            else { try update(host).get() }
        }
    }

    public func savedSessions() throws -> [SavedSession] {
        guard let vault else { return [] }
        return try vault.savedSessions()
    }

    public var persistsSessions: Bool { vault != nil }

    public func saveSessions(_ sessions: [SavedSession]) throws {
        guard let vault else { return }
        try vault.saveSessions(sessions)
    }

    public func saveSession(_ session: SavedSession) throws {
        guard let vault else { throw PasswordStore.VaultError.locked }
        try vault.saveSession(session)
    }

    public func deleteSavedSession(_ id: UUID) throws {
        guard let vault else { throw PasswordStore.VaultError.locked }
        try vault.deleteSavedSession(id)
    }

    /// Writes the whole list, refusing to save a host that would not load back.
    @discardableResult
    public func save() -> Result<Void, HostStoreError> {
        let invalid = hosts.filter { !$0.isValid }
        guard invalid.isEmpty else {
            let names = invalid.map(\.displayName).joined(separator: ", ")
            saveFailure = "Maine did not save: \(names) need attention."
            return .failure(HostStoreError(saveFailure ?? ""))
        }
        guard HostConfiguration(hosts: hosts).isSupportedByThisVersion else {
            return .failure(HostStoreError("Internal error: unsupported schema version."))
        }

        do {
            if let vault {
                try vault.saveHosts(hosts)
                saveFailure = nil
                return .success(())
            }
            let data = try HostConfiguration(hosts: hosts).encoded()
            let folder = fileURL.deletingLastPathComponent()
            try FileManager.default.createDirectory(
                at: folder,
                withIntermediateDirectories: true
            )
            // Atomic replacement keeps a crash from leaving a half written file.
            try data.write(to: fileURL, options: .atomic)
            try FileManager.default.setAttributes(
                [.posixPermissions: 0o600],
                ofItemAtPath: fileURL.path
            )
            saveFailure = nil
            return .success(())
        } catch {
            saveFailure = "Maine could not write \(fileURL.path): \(error.localizedDescription)"
            return .failure(HostStoreError(saveFailure ?? ""))
        }
    }

    public func revealInFinder() {
        NSWorkspace.shared.activateFileViewerSelecting([fileURL])
    }

    public func reportSaveFailure(_ message: String) {
        saveFailure = message
    }

    public func clearSaveFailure() { saveFailure = nil }
}
