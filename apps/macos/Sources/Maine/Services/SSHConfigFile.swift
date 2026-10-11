import Foundation

#if canImport(Darwin)
import Darwin
#endif

/// One `Host` block found in the user's `~/.ssh/config`.
///
/// Maine only looks at the directives it needs to build a host record. Every
/// other directive — `IdentityFile`, `ProxyJump`, `ForwardAgent`, `Match`, and
/// so on — is left to OpenSSH, which applies them when the session starts.
public struct SSHConfigEntry: Identifiable, Hashable, Sendable {
    public var pattern: String
    public var hostname: String?
    public var user: String?
    public var port: Int?

    public var id: String { pattern }

    public init(pattern: String, hostname: String? = nil, user: String? = nil, port: Int? = nil) {
        self.pattern = pattern
        self.hostname = hostname
        self.user = user
        self.port = port
    }

    /// True when the pattern names exactly one host (no wildcards), so it can be
    /// offered as a Maine host.
    public static func isConnectable(_ pattern: String) -> Bool {
        guard !pattern.isEmpty, !pattern.contains(where: \.isWhitespace) else { return false }
        return !pattern.contains(where: { $0 == "*" || $0 == "?" })
    }
}

/// A best effort reader for the OpenSSH client configuration.
///
/// It exists so Maine can offer the user's existing hosts when adding a host,
/// and so the UI can show which `~/.ssh/config` stanza a connection comes from.
/// Maine never writes to this file.
public struct SSHConfigFile: Sendable {
    public var entries: [SSHConfigEntry]

    public init(entries: [SSHConfigEntry] = []) {
        self.entries = entries
    }

    public static let defaultURL: URL = URL(fileURLWithPath: NSHomeDirectory())
        .appendingPathComponent(".ssh", isDirectory: true)
        .appendingPathComponent("config")

    public static var exists: Bool {
        FileManager.default.isReadableFile(atPath: defaultURL.path)
    }

    /// Reads the user's configuration. A missing or unreadable file is not an
    /// error — it just means there is nothing to import.
    public static func load(at url: URL = defaultURL) -> SSHConfigFile {
        var entries: [SSHConfigEntry] = []
        parse(file: url, into: &entries, visited: [])
        return SSHConfigFile(entries: entries)
    }

    public func entry(for pattern: String) -> SSHConfigEntry? {
        entries.first { $0.pattern.caseInsensitiveCompare(pattern) == .orderedSame }
    }

    // MARK: - Parsing

    private static func parse(
        file url: URL,
        into entries: inout [SSHConfigEntry],
        visited: Set<String>,
        depth: Int = 0
    ) {
        let path = url.standardizedFileURL.path
        // Guard against `Include` cycles and runaway chains.
        guard depth < 8, !visited.contains(path) else { return }
        var visited = visited
        visited.insert(path)

        guard let contents = try? String(contentsOf: url, encoding: .utf8) else { return }

        // Patterns of the `Host` block currently being read.
        var active: [String] = []

        for rawLine in contents.split(separator: "\n", omittingEmptySubsequences: false) {
            let tokens = tokenize(String(rawLine))
            guard let keyword = tokens.first?.lowercased() else { continue }
            let values = Array(tokens.dropFirst())

            switch keyword {
            case "host":
                active = []
                for pattern in values {
                    guard !pattern.isEmpty else { continue }
                    // The same pattern can turn up in several blocks or files.
                    // Record it once, but keep reading it: OpenSSH keeps the
                    // first value it obtains for a keyword, so a later block can
                    // still supply a value an earlier one left out.
                    if entries.contains(where: { $0.pattern == pattern }) == false {
                        entries.append(SSHConfigEntry(pattern: pattern))
                    }
                    active.append(pattern)
                }

            case "hostname":
                for index in activeIndices(in: entries, for: active) where entries[index].hostname == nil {
                    entries[index].hostname = values.first
                }
            case "user":
                for index in activeIndices(in: entries, for: active) where entries[index].user == nil {
                    entries[index].user = values.first
                }
            case "port":
                guard let port = values.first.flatMap(Int.init) else { continue }
                for index in activeIndices(in: entries, for: active) where entries[index].port == nil {
                    entries[index].port = port
                }
            case "include":
                for argument in values {
                    for included in expandedInclude(argument, relativeTo: url.deletingLastPathComponent()) {
                        parse(
                            file: URL(fileURLWithPath: included),
                            into: &entries,
                            visited: visited,
                            depth: depth + 1
                        )
                    }
                }

            default:
                // Not needed to build a host record; OpenSSH applies everything else.
                continue
            }
        }
    }

    /// Indices of the entries the current `Host` block talks about.
    private static func activeIndices(in entries: [SSHConfigEntry], for patterns: [String]) -> [Int] {
        entries.indices.filter { patterns.contains(entries[$0].pattern) }
    }

    /// Splits one line into tokens, dropping comments and handling `Keyword=value`.
    private static func tokenize(_ line: String) -> [String] {
        var tokens: [String] = []
        var current = ""

        for character in stripComment(line) {
            if character == " " || character == "\t" || character == "=" {
                if current.isEmpty == false {
                    tokens.append(current)
                    current = ""
                }
            } else {
                current.append(character)
            }
        }
        if current.isEmpty == false {
            tokens.append(current)
        }
        return tokens.map { unquote($0) }
    }

    /// Drops a trailing `#` comment. The "#" only opens a comment at the start of
    /// a line or after whitespace, so a value that contains one survives.
    private static func stripComment(_ line: String) -> String {
        guard let hash = line.firstIndex(of: "#") else { return line }

        if hash != line.startIndex,
            line[line.index(before: hash)].isWhitespace == false {
            return line
        }
        return String(line[line.startIndex..<hash])
    }

    private static func unquote(_ token: String) -> String {
        guard token.count >= 2 else { return token }
        if (token.hasPrefix("\"") && token.hasSuffix("\""))
            || (token.hasPrefix("'") && token.hasSuffix("'")) {
            return String(token.dropFirst().dropLast())
        }
        return token
    }

    /// Expands `~`, braces and wildcards in an `Include` argument. A relative
    /// argument is resolved against the directory of the file that contained the
    /// directive, which is where OpenSSH looks for it too.
    private static func expandedInclude(_ argument: String, relativeTo folder: URL) -> [String] {
        let expanded = (argument as NSString).expandingTildeInPath
        let pattern = expanded.hasPrefix("/")
            ? expanded
            : folder.appendingPathComponent(expanded).path
        var matches: [String] = []

        var globState = glob_t()
        defer { globfree(&globState) }

        #if canImport(Darwin)
        let result = glob(pattern, GLOB_BRACE, nil, &globState)
        #else
        let result = GLOB_NOMATCH
        #endif

        if result == 0, let paths = globState.gl_pathv {
            for index in 0..<Int(globState.gl_matchc) {
                if let path = paths[index] {
                    matches.append(String(cString: path))
                }
            }
        }
        return matches
    }
}
