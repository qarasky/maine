import Foundation
@testable import Maine
import Testing

/// End-to-end against the real Rust server. Skipped unless
/// `MAINE_SYNC_SERVER_BIN` points at a built `maine-sync-server` binary, which
/// is how CI and local integration runs enable it.
@Suite(.enabled(if: ProcessInfo.processInfo.environment["MAINE_SYNC_SERVER_BIN"] != nil))
struct SyncServerIntegrationTests {
    @Test
    func createJoinPushPullConflictRotateDelete() async throws {
        let binary = try #require(ProcessInfo.processInfo.environment["MAINE_SYNC_SERVER_BIN"])
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }

        let port = Int.random(in: 20_000...40_000)
        let server = try #require(URL(string: "http://127.0.0.1:\(port)"))
        let process = Process()
        process.executableURL = URL(fileURLWithPath: binary)
        process.arguments = ["--bind", "127.0.0.1:\(port)",
                             "--data-dir", directory.appendingPathComponent("server").path]
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        try process.run()
        defer { process.terminate() }
        try await waitForHealth(server)

        // Device A creates the slot from its vault; the token comes back once.
        let a = try makeVault(directory: directory.appendingPathComponent("a"), password: "integration master password")
        defer { try? a.lock() }
        let transport = URLSessionSyncTransport()
        let engineA = SyncEngine(vaultURL: a.fileURL, passwords: a, transport: transport)
        let created = try await engineA.create(at: server, createSecret: nil)
        #expect(created.state.version == 1)
        let token = created.token

        // Device B joins with the same master password; the bytes match.
        let b = try makeVault(directory: directory.appendingPathComponent("b"), password: "bootstrap different password")
        defer { try? b.lock() }
        let engineB = SyncEngine(vaultURL: b.fileURL, passwords: b, transport: transport)
        switch try await engineB.prepareJoin(server: server, token: token, masterPassword: "integration master password") {
        case .emptySlot:
            Issue.record("the slot should not be empty")
        case .ready(let bytes, let state):
            try engineB.adoptPreparedJoin(bytes, state: state)
        }
        #expect(try Data(contentsOf: b.fileURL) == (try Data(contentsOf: a.fileURL)))
        // B unlocks the adopted vault, exactly like the join flow does.
        try b.unlock(masterPassword: "integration master password")

        // A edits and pushes; B pulls the exact bytes.
        try a.saveHost(Host(name: "Sync", hostname: "sync.example.com"), password: "a saved password")
        let pushed = try await engineA.push(server: server, token: token,
                                            lastVersion: 1, lastSHA256: created.state.sha256)
        guard case .stored(let pushedState) = pushed else {
            Issue.record("expected the push to be stored")
            return
        }
        #expect(pushedState.version == 2)

        let pulled = try await engineB.pull(server: server, token: token, lastVersion: 1, isDirty: false)
        guard case .adopted(let pulledState, let conflictCopy) = pulled else {
            Issue.record("expected the pull to adopt version 2")
            return
        }
        #expect(pulledState.version == 2)
        #expect(conflictCopy == nil)
        #expect(try Data(contentsOf: b.fileURL) == (try Data(contentsOf: a.fileURL)))

        // A stale write is a conflict the client can see and reconcile.
        do {
            _ = try await engineA.push(server: server, token: token, lastVersion: 1, lastSHA256: "stale")
            Issue.record("expected a conflict")
        } catch let error as SyncTransportError {
            #expect(error == .conflict(currentVersion: 2))
        }

        // Rotation kills the old token immediately.
        let rotated = try await engineA.rotate(server: server, token: token)
        #expect(rotated != token)
        do {
            _ = try await engineA.head(server: server, token: token, lastVersion: 2)
            Issue.record("the old token should be rejected")
        } catch let error as SyncTransportError {
            #expect(error == .unauthorized)
        }

        // Deletion wipes the slot; the new token stops working too.
        try await engineA.delete(server: server, token: rotated)
        do {
            _ = try await engineA.head(server: server, token: rotated, lastVersion: 2)
            Issue.record("the deleted slot should not answer")
        } catch let error as SyncTransportError {
            #expect(error == .unauthorized)
        }
    }

    private func makeVault(directory: URL, password: String) throws -> PasswordStore {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let store = PasswordStore(fileURL: directory.appendingPathComponent("passwords.maine"),
                                  keychainService: "app.maine.tests.sync-integration.\(UUID())")
        try store.unlock(masterPassword: password)
        return store
    }

    private func waitForHealth(_ server: URL) async throws {
        let deadline = ContinuousClock.now + .seconds(15)
        while ContinuousClock.now < deadline {
            var request = URLRequest(url: server.appendingPathComponent("v1/health"))
            request.timeoutInterval = 1
            if let (_, response) = try? await URLSession.shared.data(for: request),
               (response as? HTTPURLResponse)?.statusCode == 200 {
                return
            }
            try? await Task.sleep(for: .milliseconds(150))
        }
        throw SyncTransportError.offline("the sync server did not start in time")
    }
}
