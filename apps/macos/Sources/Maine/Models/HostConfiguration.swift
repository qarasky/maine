import Foundation

/// The on-disk host list: the exact JSON Maine writes to
/// `~/Library/Application Support/Maine/hosts.json`.
///
/// The format is defined by `packages/schema/hosts.schema.json`.
public struct HostConfiguration: Codable, Equatable, Sendable {
    /// The format version this build of Maine writes and understands.
    public static let currentSchemaVersion = 1

    public var schemaVersion: Int
    public var hosts: [Host]

    public init(schemaVersion: Int = HostConfiguration.currentSchemaVersion, hosts: [Host]) {
        self.schemaVersion = schemaVersion
        self.hosts = hosts
    }

    /// The version this build of Maine understands, used to detect files
    /// written by a newer client.
    public var isSupportedByThisVersion: Bool { schemaVersion <= Self.currentSchemaVersion }

    public enum DecodeError: LocalizedError, Equatable {
        /// The file is valid JSON but was written by a client with a newer
        /// schema. Maine must not load it, and must not overwrite it.
        case unsupportedVersion(Int)
        /// The file is not readable as a version 1 host list.
        case malformed(String)

        public var errorDescription: String? {
            switch self {
            case .unsupportedVersion(let version):
                return "This host list was written with a newer version of the Maine "
                    + "configuration format (version \(version)). Update Maine to read it."
            case .malformed(let detail):
                return "Maine could not read this host list: \(detail)"
            }
        }
    }

    public init(data: Data) throws {
        let configuration: HostConfiguration
        do {
            configuration = try JSONDecoder().decode(HostConfiguration.self, from: data)
        } catch let error as DecodingError {
            throw DecodeError.malformed(error.prettyDescription)
        } catch {
            throw DecodeError.malformed(error.localizedDescription)
        }
        guard configuration.isSupportedByThisVersion else {
            throw DecodeError.unsupportedVersion(configuration.schemaVersion)
        }
        self = configuration
    }

    /// The JSON to write to disk: stable key order, 4 space indent, no escapes
    /// added to slashes, so the file stays easy to read and diff.
    public func encoded() throws -> Data {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .withoutEscapingSlashes]
        return try encoder.encode(self)
    }
}

private extension DecodingError {
    /// A short explanation of what went wrong, without dumping Swift internals.
    var prettyDescription: String {
        switch self {
        case .typeMismatch(let type, let context):
            return "\"\(context.path)\" should hold a \(type)."
        case .valueNotFound(let type, let context):
            return "\"\(context.path)\" is missing a \(type) value."
        case .keyNotFound(let missingKey, _):
            return "\"\(missingKey.stringValue)\" is missing."
        case .dataCorrupted:
            return "the file is not valid JSON."
        @unknown default:
            return localizedDescription
        }
    }
}

private extension DecodingError.Context {
    /// `hosts.0.port`, or a friendly name when the problem is at the root.
    var path: String {
        guard codingPath.isEmpty == false else { return "the host list" }
        return codingPath.map(\.stringValue).joined(separator: ".")
    }
}
