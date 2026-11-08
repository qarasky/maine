import Foundation

/// One thing an agent did, as the user can review it later. Kept in the
/// encrypted device-local state file (never in the vault, never synced, never
/// in diagnostics). It holds commands the agent ran and the fact that a saved
/// password was typed; never the password, never output, and never literal
/// text sent with `send_keys` (only its length).
public struct AgentAuditEntry: Codable, Equatable, Identifiable, Sendable {
    public enum Kind: String, Codable, Sendable { case open, run, keys, password, close
        /// The user handed one of their own sessions to this client.
        case handoff
    }

    /// Newest entries kept per vault.
    public static let limit = 1000
    /// Longest command text stored, in bytes.
    public static let detailLimit = 2048

    public var id: UUID
    public var at: Date
    public var client: String
    public var hostName: String
    public var sessionID: UUID
    public var kind: Kind
    public var detail: String?
    public var exitCode: Int32?

    public init(id: UUID = UUID(), at: Date = Date(), client: String, hostName: String, sessionID: UUID,
                kind: Kind, detail: String? = nil, exitCode: Int32? = nil) {
        self.id = id
        self.at = at
        self.client = client
        self.hostName = hostName
        self.sessionID = sessionID
        self.kind = kind
        self.detail = detail.map(Self.trimmed)
        self.exitCode = exitCode
    }

    private static func trimmed(_ text: String) -> String {
        guard text.utf8.count > detailLimit else { return text }
        var kept = "", bytes = 0
        for character in text {
            let size = String(character).utf8.count
            if bytes + size > detailLimit { break }
            kept.append(character)
            bytes += size
        }
        return kept + "…"
    }
}
