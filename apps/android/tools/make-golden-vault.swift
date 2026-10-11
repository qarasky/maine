// Generates the Android golden test vector with the SAME primitives as
// apps/macos Sources/Maine/Services/PasswordStore.swift (PBKDF2-SHA256 with
// 600,000 iterations, AES-256-GCM via CryptoKit, "maine-vault:..." AAD), so
// the Android test proves byte-for-byte vault interoperability.
//
// IDs are written lowercase (the schema recommendation) to exercise the
// Android client's uppercase canonicalization in the other direction.
//
// This fixture is a published throwaway test vector, never a real vault.
// The password is intentionally checked into the Android test sources.
//
// Usage (needs the macOS SDK; Command Line Tools are enough):
//   swift tools/make-golden-vault.swift <password> <out.json>

import CommonCrypto
import CryptoKit
import Foundation

struct GoldenHost: Codable {
    var id: UUID
    var name: String
    var hostname: String
    var port: Int
    var username: String
}

struct GoldenSession: Codable {
    var id: UUID
    var hostID: UUID
    var name: String
    var history: String
    var savedAt: Date
    var usesDefaultName: Bool?
    var number: Int?
}

struct GoldenContents: Codable {
    var hosts: [GoldenHost]
    var passwords: [String: String]
    var sessions: [GoldenSession]?
}

struct GoldenVault: Codable {
    var version = 2
    var id: UUID
    var salt: Data
    var iterations: Int
    var verification: Data
    var passwords: [String: Data] = [:]
    var payload: Data?

    var context: Data {
        Data("maine-vault:\(version):\(id.uuidString):\(salt.base64EncodedString()):\(iterations)".utf8)
    }
}

func derive(_ password: String, salt: Data, iterations: Int) throws -> SymmetricKey {
    let bytes = Array(password.utf8)
    var result = Data(count: 32)
    let status = result.withUnsafeMutableBytes { output in
        salt.withUnsafeBytes { saltBytes in
            bytes.withUnsafeBytes { passwordBytes in
                CCKeyDerivationPBKDF(CCPBKDFAlgorithm(kCCPBKDF2),
                    passwordBytes.baseAddress?.assumingMemoryBound(to: Int8.self), bytes.count,
                    saltBytes.baseAddress?.assumingMemoryBound(to: UInt8.self), salt.count,
                    CCPseudoRandomAlgorithm(kCCPRFHmacAlgSHA256), UInt32(iterations),
                    output.baseAddress?.assumingMemoryBound(to: UInt8.self), output.count)
            }
        }
    }
    guard status == kCCSuccess else { throw NSError(domain: "golden", code: 1) }
    return SymmetricKey(data: result)
}

func seal(_ data: Data, key: SymmetricKey, context: Data) throws -> Data {
    guard let combined = try AES.GCM.seal(data, using: key, authenticating: context).combined else {
        throw NSError(domain: "golden", code: 2)
    }
    return combined
}

guard CommandLine.arguments.count == 3 else {
    fputs("usage: make-golden-vault.swift <password> <out.json>\n", stderr)
    exit(2)
}
let masterPassword = CommandLine.arguments[1]
let outURL = URL(fileURLWithPath: CommandLine.arguments[2])

// Lowercase IDs on purpose: the Android client must canonicalize these.
let host1 = GoldenHost(
    id: UUID(uuidString: "0f8fad5b-d9cb-469f-a165-70867728950e")!,
    name: "Golden web", hostname: "203.0.113.10", port: 2222, username: "deploy")
let host2 = GoldenHost(
    id: UUID(uuidString: "6ba7b810-9dad-11d1-80b4-00c04fd430c8")!,
    name: "Bastion", hostname: "bastion.example", port: 22, username: "")
let session = GoldenSession(
    id: UUID(uuidString: "6ba7b811-9dad-11d1-80b4-00c04fd430c8")!,
    hostID: host1.id, name: "Session 1", history: "golden scrollback\n$ ",
    savedAt: Date(timeIntervalSince1970: 1_783_000_000.5),
    usesDefaultName: true, number: 1)
let contents = GoldenContents(
    hosts: [host1, host2],
    passwords: [host1.id.uuidString.lowercased(): "golden-password-1",
                host2.id.uuidString.lowercased(): "golden-password-2"],
    sessions: [session])

var salt = Data(count: 32)
guard SecRandomCopyBytes(kSecRandomDefault, salt.count, salt.withUnsafeMutableBytes({ $0.baseAddress! })) == errSecSuccess else {
    fputs("no entropy\n", stderr)
    exit(1)
}
var vault = GoldenVault(id: UUID(), salt: salt, iterations: 600_000,
                        verification: Data(), payload: nil)
let key = try derive(masterPassword, salt: salt, iterations: vault.iterations)
vault.verification = try seal(Data("Maine password vault v1".utf8), key: key, context: vault.context)
vault.payload = try seal(JSONEncoder().encode(contents), key: key, context: vault.context)

let encoder = JSONEncoder()
encoder.outputFormatting = [.prettyPrinted, .sortedKeys]
try encoder.encode(vault).write(to: outURL)
print("wrote \(outURL.path)")
