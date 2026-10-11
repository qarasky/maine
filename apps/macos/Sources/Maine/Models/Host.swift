import Foundation

/// A single saved SSH host.
///
/// This type maps 1:1 onto the `host` object defined by
/// `packages/schema/hosts.schema.json`, which is the portable format shared with
/// future Maine clients. The field order here matches the schema so that encoded
/// JSON stays easy to diff by hand.
public struct Host: Identifiable, Codable, Hashable, Sendable {
    /// The port OpenSSH uses unless told otherwise. Storing 22 means "default":
    /// Maine then leaves the `-p` flag off so a matching `~/.ssh/config` stanza
    /// can pick the real port.
    public static let defaultPort = 22

    /// A host with empty fields, used as the starting point for the "New host" editor.
    public static var blank: Host { Host(name: "", hostname: "") }

    public var id: UUID
    public var name: String
    /// An IP address, a DNS name, or a `Host` alias defined in the user's `~/.ssh/config`.
    public var hostname: String
    public var port: Int
    /// Empty means "not specified"; OpenSSH then takes the user from `~/.ssh/config`,
    /// the key that authenticates, or the local user name.
    public var username: String

    public init(id: UUID = UUID(), name: String, hostname: String, port: Int = Host.defaultPort, username: String = "") {
        self.id = id
        self.name = name
        self.hostname = hostname
        self.port = port
        self.username = username
    }

    // Lenient reading: a field that is absent or explicitly null falls back to
    // its default so one odd host cannot stop the whole list from loading. A
    // field with the wrong JSON type still fails, and the store surfaces that.
    public init(from decoder: any Decoder) throws {
        let container = try decoder.container(keyedBy: CodingKeys.self)
        id = try container.decodeIfPresent(UUID.self, forKey: .id) ?? UUID()
        name = try container.decodeIfPresent(String.self, forKey: .name) ?? ""
        hostname = try container.decodeIfPresent(String.self, forKey: .hostname) ?? ""
        port = try container.decodeIfPresent(Int.self, forKey: .port) ?? Host.defaultPort
        username = try container.decodeIfPresent(String.self, forKey: .username) ?? ""
    }
}

public extension Host {
    /// The trimmed name used for display.
    var displayName: String {
        name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
            ? hostname
            : name.trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// `user@hostname`, or just `hostname` when no user is set.
    var target: String {
        username.isEmpty ? hostname : "\(username)@\(hostname)"
    }

    /// The port, only shown when it is not the default.
    var portDescription: String? {
        port == Host.defaultPort ? nil : ":\(port)"
    }

    /// A one line summary used in the sidebar and the session tabs.
    var subtitle: String {
        var line = target
        if let portDescription { line += portDescription }
        return line
    }

    /// Problems that stop this host from being used. Mirrors the constraints in
    /// the shared schema; the canonical version lives in `packages/schema`.
    var issues: [String] {
        var issues: [String] = []

        if name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
            issues.append("A name is required.")
        }

        let address = hostname.trimmingCharacters(in: .whitespacesAndNewlines)
        if address.isEmpty {
            issues.append("A hostname or IP address is required.")
        } else if address.contains(where: { $0.isWhitespace }) {
            issues.append("The hostname cannot contain spaces.")
        } else if address.hasPrefix("-") {
            issues.append("The hostname cannot start with \"-\", it would be read as an option by ssh.")
        } else if address.count > 253 {
            issues.append("The hostname cannot be longer than 253 characters.")
        }

        if !(1...65535).contains(port) {
            issues.append("The port must be between 1 and 65535.")
        }

        if username.contains(where: { $0.isWhitespace }) {
            issues.append("The username cannot contain spaces.")
        } else if username.count > 128 {
            issues.append("The username cannot be longer than 128 characters.")
        }

        return issues
    }

    var isValid: Bool { issues.isEmpty }

    /// Builds a host from a `~/.ssh/config` entry, filling in what the entry
    /// leaves out. `nil` when the pattern is not a single literal host name.
    init?(sshConfigEntry entry: SSHConfigEntry) {
        guard SSHConfigEntry.isConnectable(entry.pattern) else { return nil }
        self.init(
            name: entry.pattern,
            hostname: entry.pattern,
            port: entry.port ?? Host.defaultPort,
            username: entry.user ?? ""
        )
    }
}
