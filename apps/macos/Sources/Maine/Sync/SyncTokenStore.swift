import Foundation
import Security

/// The sync token for one profile. It lives in the login Keychain on purpose:
/// it must survive a remote vault replacing the local file, and it is not part
/// of that vault. Only the token's profile ID is used as the Keychain account;
/// the token itself is never logged or shown again after pairing.
struct SyncTokenStore: Sendable {
    private let service: String

    init(service: String = "Maine Sync") {
        self.service = service
    }

    private func query(_ profileID: UUID) -> [String: Any] {
        [kSecClass as String: kSecClassGenericPassword,
         kSecAttrService as String: service,
         kSecAttrAccount as String: profileID.uuidString]
    }

    func token(for profileID: UUID) throws -> String? {
        var query = query(profileID)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        try check(status)
        guard let data = result as? Data, let token = String(data: data, encoding: .utf8) else {
            throw HostStoreError("The saved sync token could not be read.")
        }
        return token
    }

    func save(_ token: String, for profileID: UUID) throws {
        let data = Data(token.utf8)
        let query = query(profileID)
        let update = [kSecValueData as String: data]
        let status = SecItemUpdate(query as CFDictionary, update as CFDictionary)
        if status == errSecItemNotFound {
            var item = query.merging(update) { _, new in new }
            item[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
            try check(SecItemAdd(item as CFDictionary, nil))
        } else {
            try check(status)
        }
    }

    func delete(for profileID: UUID) throws {
        let status = SecItemDelete(query(profileID) as CFDictionary)
        if status != errSecItemNotFound { try check(status) }
    }

    private func check(_ status: OSStatus) throws {
        guard status == errSecSuccess else {
            let message = SecCopyErrorMessageString(status, nil) as String? ?? "Error \(status)"
            throw HostStoreError("Keychain: \(message)")
        }
    }
}
