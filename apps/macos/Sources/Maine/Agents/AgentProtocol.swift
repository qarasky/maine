import Foundation

/// Messages between `Maine --mcp` (the stdio bridge) and the running app,
/// over the agent socket. Each frame is a 4-byte big-endian length followed
/// by that many bytes of UTF-8 JSON.
enum AgentProtocolError: Error, Equatable {
    case frameTooLarge
}

enum AgentFrame {
    static let maxLength = 8 * 1024 * 1024

    static func encode<T: Encodable>(_ value: T) throws -> Data {
        let body = try JSONEncoder().encode(value)
        guard body.count <= maxLength else { throw AgentProtocolError.frameTooLarge }
        var length = UInt32(body.count).bigEndian
        return Data(bytes: &length, count: 4) + body
    }

    /// Accumulates bytes from the socket and yields whole frames.
    struct Reader: Sendable {
        private var buffer = Data()

        mutating func feed(_ data: Data) throws -> [Data] {
            buffer.append(data)
            var frames: [Data] = []
            while buffer.count >= 4 {
                let header = buffer.prefix(4)
                let length = header.reduce(0) { Int($0) << 8 | Int($1) }
                guard length <= AgentFrame.maxLength else { throw AgentProtocolError.frameTooLarge }
                guard buffer.count >= 4 + length else { break }
                frames.append(Data(buffer.dropFirst(4).prefix(length)))
                buffer = Data(buffer.dropFirst(4 + length))
            }
            return frames
        }
    }
}

/// First frame from the bridge. `clientName` is informational only.
struct AgentHello: Codable, Equatable, Sendable {
    var token: String
    var clientName: String?
}

struct AgentHelloReply: Codable, Equatable, Sendable {
    var ok: Bool
    var error: String?
}

struct AgentCall: Codable, Equatable, Sendable {
    var id: Int
    var tool: String
    var arguments: JSONValue
}

struct AgentReply: Codable, Equatable, Sendable {
    var id: Int
    var result: JSONValue?
    var error: String?
}

/// Untyped JSON for tool arguments and results.
enum JSONValue: Codable, Equatable, Sendable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])

    init(from decoder: any Decoder) throws {
        let container = try decoder.singleValueContainer()
        if container.decodeNil() { self = .null }
        else if let value = try? container.decode(Bool.self) { self = .bool(value) }
        else if let value = try? container.decode(Double.self) { self = .number(value) }
        else if let value = try? container.decode(String.self) { self = .string(value) }
        else if let value = try? container.decode([JSONValue].self) { self = .array(value) }
        else { self = .object(try container.decode([String: JSONValue].self)) }
    }

    func encode(to encoder: any Encoder) throws {
        var container = encoder.singleValueContainer()
        switch self {
        case .null: try container.encodeNil()
        case .bool(let value): try container.encode(value)
        case .number(let value):
            if value.rounded() == value, abs(value) < 1e15 { try container.encode(Int64(value)) }
            else { try container.encode(value) }
        case .string(let value): try container.encode(value)
        case .array(let value): try container.encode(value)
        case .object(let value): try container.encode(value)
        }
    }

    subscript(key: String) -> JSONValue? {
        if case .object(let object) = self { return object[key] }
        return nil
    }

    func string(_ key: String) -> String? {
        if case .string(let value)? = self[key] { return value }
        return nil
    }

    func int(_ key: String) -> Int? {
        if case .number(let value)? = self[key], value.rounded() == value, abs(value) < 1e15 { return Int(value) }
        return nil
    }

    func array(_ key: String) -> [JSONValue]? {
        if case .array(let value)? = self[key] { return value }
        return nil
    }

    var stringValue: String? {
        if case .string(let value) = self { return value }
        return nil
    }

    /// Compact JSON text, for MCP `text` content.
    var jsonText: String {
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.sortedKeys, .withoutEscapingSlashes]
        return (try? String(decoding: encoder.encode(self), as: UTF8.self)) ?? "null"
    }
}

extension JSONValue: ExpressibleByStringLiteral, ExpressibleByIntegerLiteral, ExpressibleByBooleanLiteral,
                     ExpressibleByArrayLiteral, ExpressibleByDictionaryLiteral, ExpressibleByNilLiteral,
                     ExpressibleByFloatLiteral {
    init(stringLiteral value: String) { self = .string(value) }
    init(integerLiteral value: Int) { self = .number(Double(value)) }
    init(floatLiteral value: Double) { self = .number(value) }
    init(booleanLiteral value: Bool) { self = .bool(value) }
    init(arrayLiteral elements: JSONValue...) { self = .array(elements) }
    init(dictionaryLiteral elements: (String, JSONValue)...) {
        self = .object(Dictionary(elements, uniquingKeysWith: { _, new in new }))
    }
    init(nilLiteral: ()) { self = .null }
}
