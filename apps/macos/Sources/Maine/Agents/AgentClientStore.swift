import CryptoKit
import Foundation
import Security

/// A paired MCP client. Its name is what the user typed in Settings.
struct AgentClient: Identifiable, Equatable, Sendable {
    let id: UUID
    var name: String
    var createdAt: Date
}

/// Paired clients in the login Keychain (service `Maine Agents`, account =
/// client ID). Only SHA-256 of a token is stored: the token is shown once,
/// at pairing, and lives in the client's MCP configuration afterwards.
struct AgentClientStore: Sendable {
    static let tokenPrefix = "maine-agent-"
    private static let alphabet = Array("0123456789ABCDEFGHJKMNPQRSTVWXYZ")
    private let service: String

    init(service: String = "Maine Agents") {
        self.service = service
    }

    func clients() throws -> [AgentClient] {
        var query = baseQuery()
        query[kSecReturnAttributes as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitAll
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound { return [] }
        try check(status)
        let items = result as? [[String: Any]] ?? []
        return items.compactMap { item in
            guard let account = item[kSecAttrAccount as String] as? String, let id = UUID(uuidString: account) else {
                return nil
            }
            return AgentClient(id: id, name: item[kSecAttrLabel as String] as? String ?? "MCP client",
                               createdAt: item[kSecAttrCreationDate as String] as? Date ?? .distantPast)
        }
        .sorted { $0.createdAt < $1.createdAt }
    }

    func add(name: String) throws -> (client: AgentClient, token: String) {
        let token = Self.makeToken()
        let client = AgentClient(id: UUID(), name: name, createdAt: Date())
        var item = baseQuery()
        item[kSecAttrAccount as String] = client.id.uuidString
        item[kSecAttrLabel as String] = name
        item[kSecValueData as String] = Data(Self.hash(token).utf8)
        item[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
        try check(SecItemAdd(item as CFDictionary, nil))
        return (client, token)
    }

    func revoke(_ id: UUID) throws {
        var query = baseQuery()
        query[kSecAttrAccount as String] = id.uuidString
        let status = SecItemDelete(query as CFDictionary)
        if status != errSecItemNotFound { try check(status) }
    }

    /// The client a presented token belongs to, comparing hashes in constant time.
    func client(forToken token: String) throws -> AgentClient? {
        guard token.hasPrefix(Self.tokenPrefix) else { return nil }
        let presented = Array(Self.hash(token).utf8)
        var match: AgentClient?
        for client in try clients() {
            guard let stored = try storedHash(for: client.id) else { continue }
            if Self.constantTimeEqual(Array(stored), presented) { match = client }
        }
        return match
    }

    // MARK: - Private

    private func baseQuery() -> [String: Any] {
        [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service]
    }

    private func storedHash(for id: UUID) throws -> Data? {
        var query = baseQuery()
        query[kSecAttrAccount as String] = id.uuidString
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        try check(status)
        return result as? Data
    }

    static func makeToken() -> String {
        var bytes = [UInt8](repeating: 0, count: 32)
        precondition(SecRandomCopyBytes(kSecRandomDefault, bytes.count, &bytes) == errSecSuccess)
        var bits = 0, value = 0
        var output = ""
        for byte in bytes {
            value = value << 8 | Int(byte)
            bits += 8
            while bits >= 5 {
                output.append(alphabet[(value >> (bits - 5)) & 31])
                bits -= 5
            }
        }
        if bits > 0 { output.append(alphabet[(value << (5 - bits)) & 31]) }
        return tokenPrefix + output
    }

    static func hash(_ token: String) -> String {
        SHA256.hash(data: Data(token.utf8)).map { String(format: "%02x", $0) }.joined()
    }

    private static func constantTimeEqual(_ left: [UInt8], _ right: [UInt8]) -> Bool {
        guard left.count == right.count else { return false }
        var difference: UInt8 = 0
        for index in left.indices { difference |= left[index] ^ right[index] }
        return difference == 0
    }

    private func check(_ status: OSStatus) throws {
        guard status == errSecSuccess else {
            let message = SecCopyErrorMessageString(status, nil) as String? ?? "Error \(status)"
            throw HostStoreError("Keychain: \(message)")
        }
    }
}
