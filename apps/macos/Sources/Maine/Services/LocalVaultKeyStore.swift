import Foundation
import Security

/// Only the local vault unlock key lives here. SSH passwords live in the file.
struct LocalVaultKeyStore: Sendable {
    private let service: String

    init(service: String = "app.maine.vault-key") {
        self.service = service
    }

    private func query(_ vaultID: UUID) -> [String: Any] {
        [kSecClass as String: kSecClassGenericPassword,
         kSecAttrService as String: service,
         kSecAttrAccount as String: vaultID.uuidString]
    }

    func key(for vaultID: UUID) throws -> Data? {
        var query = query(vaultID)
        query[kSecReturnData as String] = true
        query[kSecMatchLimit as String] = kSecMatchLimitOne
        var result: CFTypeRef?
        let status = SecItemCopyMatching(query as CFDictionary, &result)
        if status == errSecItemNotFound { return nil }
        try check(status)
        guard let data = result as? Data else {
            throw HostStoreError("The remembered vault key could not be read.")
        }
        return data
    }

    func save(_ key: Data, for vaultID: UUID) throws {
        let query = query(vaultID)
        let value = [kSecValueData as String: key]
        let status = SecItemUpdate(query as CFDictionary, value as CFDictionary)
        if status == errSecItemNotFound {
            var item = query.merging(value) { _, new in new }
            item[kSecAttrAccessible as String] = kSecAttrAccessibleWhenUnlockedThisDeviceOnly
            try check(SecItemAdd(item as CFDictionary, nil))
        } else {
            try check(status)
        }
    }

    func delete(for vaultID: UUID) throws {
        let status = SecItemDelete(query(vaultID) as CFDictionary)
        if status != errSecItemNotFound { try check(status) }
    }

    private func check(_ status: OSStatus) throws {
        guard status == errSecSuccess else {
            let message = SecCopyErrorMessageString(status, nil) as String? ?? "Error \(status)"
            throw HostStoreError("Keychain: \(message)")
        }
    }
}
