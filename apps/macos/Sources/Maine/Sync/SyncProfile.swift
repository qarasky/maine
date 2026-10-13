import Foundation

/// One paired sync slot, as remembered on this device. The token is not part
/// of this record; it lives in the Keychain, keyed by `id`.
public struct SyncProfile: Codable, Identifiable, Equatable, Sendable {
    public var id: UUID
    public var name: String
    public var serverURL: URL
    /// The version the local vault came from. `0` means never synced.
    public var lastVersion: Int
    /// Hex SHA-256 of the file at `lastVersion`, used to skip no-op pushes.
    public var lastSHA256: String?
    public var lastSyncedAt: Date?

    public init(id: UUID = UUID(), name: String, serverURL: URL,
                lastVersion: Int = 0, lastSHA256: String? = nil, lastSyncedAt: Date? = nil) {
        self.id = id
        self.name = name
        self.serverURL = serverURL
        self.lastVersion = lastVersion
        self.lastSHA256 = lastSHA256
        self.lastSyncedAt = lastSyncedAt
    }
}

public extension URL {
    /// Normalizes what a user types into a base URL: adds `http://` when the
    /// scheme is missing and trims trailing slashes. A reverse-proxy path is
    /// kept (`https://host/maine` stays as typed).
    static func syncBase(from raw: String) -> URL? {
        var text = raw.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { return nil }
        if !text.contains("://") { text = "http://" + text }
        guard var components = URLComponents(string: text),
              let scheme = components.scheme?.lowercased(),
              scheme == "http" || scheme == "https",
              let host = components.host, !host.isEmpty else { return nil }
        let path = components.path.trimmingCharacters(in: CharacterSet(charactersIn: "/"))
        components.path = path.isEmpty ? "" : "/" + path
        return components.url
    }
}
