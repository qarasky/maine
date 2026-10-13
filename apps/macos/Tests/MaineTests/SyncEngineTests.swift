import Foundation
@testable import Maine
import Testing

/// A programmable stand-in for the sync server. Every call consumes the next
/// queued result, so tests describe exactly what the server would answer.
actor FakeSyncTransport: SyncTransport {
    private var createQueue: [Result<(token: String, state: SyncSlotState), Error>] = []
    private var fetchQueue: [Result<SyncFetchResult, Error>] = []
    private var putQueue: [Result<SyncSlotState, Error>] = []
    private var rotateQueue: [Result<String, Error>] = []
    private var deleteQueue: [Result<Void, Error>] = []
    private(set) var calls: [String] = []

    func enqueueCreate(_ result: Result<(token: String, state: SyncSlotState), Error>) { createQueue.append(result) }
    func enqueueFetch(_ result: Result<SyncFetchResult, Error>) { fetchQueue.append(result) }
    func enqueuePut(_ result: Result<SyncSlotState, Error>) { putQueue.append(result) }
    func enqueueRotate(_ result: Result<String, Error>) { rotateQueue.append(result) }
    func enqueueDelete(_ result: Result<Void, Error>) { deleteQueue.append(result) }

    func createVault(at server: URL, createSecret: String?, body: Data) async throws -> (token: String, state: SyncSlotState) {
        calls.append("create")
        guard !createQueue.isEmpty else { throw SyncTransportError.malformed("no create queued") }
        return try createQueue.removeFirst().get()
    }

    func fetchVault(at server: URL, token: String, ifNoneMatch: Int?) async throws -> SyncFetchResult {
        calls.append("fetch")
        guard !fetchQueue.isEmpty else { throw SyncTransportError.malformed("no fetch queued") }
        return try fetchQueue.removeFirst().get()
    }

    func headVault(at server: URL, token: String, ifNoneMatch: Int?) async throws -> SyncFetchResult {
        calls.append("head")
        guard !fetchQueue.isEmpty else { throw SyncTransportError.malformed("no head queued") }
        return try fetchQueue.removeFirst().get()
    }

    func putVault(at server: URL, token: String, body: Data, expectedVersion: Int?) async throws -> SyncSlotState {
        calls.append("put")
        guard !putQueue.isEmpty else { throw SyncTransportError.malformed("no put queued") }
        return try putQueue.removeFirst().get()
    }

    func rotateToken(at server: URL, token: String) async throws -> String {
        calls.append("rotate")
        guard !rotateQueue.isEmpty else { throw SyncTransportError.malformed("no rotate queued") }
        return try rotateQueue.removeFirst().get()
    }

    func deleteVault(at server: URL, token: String) async throws {
        calls.append("delete")
        guard !deleteQueue.isEmpty else { throw SyncTransportError.malformed("no delete queued") }
        return try deleteQueue.removeFirst().get()
    }
}

@Suite
struct SyncEngineTests {
    private let server = URL(string: "http://127.0.0.1:8484")!

    private func temporaryDirectory() -> URL {
        FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    }

    private func makeVault(directory: URL, password: String) throws -> PasswordStore {
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        let store = PasswordStore(fileURL: directory.appendingPathComponent("passwords.maine"),
                                  keychainService: "app.maine.tests.sync.\(UUID())")
        try store.unlock(masterPassword: password)
        return store
    }

    private func names(in directory: URL) throws -> [String] {
        try FileManager.default.contentsOfDirectory(atPath: directory.path)
    }

    @Test
    func createUploadsTheCurrentVaultAndReturnsTheToken() async throws {
        let directory = temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let passwords = try makeVault(directory: directory, password: "sync create master password")
        defer { try? passwords.lock() }

        let transport = FakeSyncTransport()
        let state = SyncSlotState(version: 1, size: 400, sha256: "abc123")
        await transport.enqueueCreate(.success((token: "maine-TEST-TEST", state: state)))
        let engine = SyncEngine(vaultURL: passwords.fileURL, passwords: passwords, transport: transport)

        let created = try await engine.create(at: server, createSecret: nil)

        #expect(created.token == "maine-TEST-TEST")
        #expect(created.state == state)
        let calls = await transport.calls
        #expect(calls == ["create"])
    }

    @Test
    func pushSkipsWhenTheFileMatchesTheLastSyncedHash() async throws {
        let directory = temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let passwords = try makeVault(directory: directory, password: "sync push master password")
        defer { try? passwords.lock() }
        let sha = SyncEngine.sha256Hex(try Data(contentsOf: passwords.fileURL))
        let transport = FakeSyncTransport()
        let engine = SyncEngine(vaultURL: passwords.fileURL, passwords: passwords, transport: transport)

        let result = try await engine.push(server: server, token: "maine-TEST", lastVersion: 3, lastSHA256: sha)

        guard case .skipped(let state) = result else {
            Issue.record("expected the push to be skipped")
            return
        }
        #expect(state.version == 3)
        let calls = await transport.calls
        #expect(calls.isEmpty)
    }

    @Test
    func pushStoresNewBytesAndSurfacesConflicts() async throws {
        let directory = temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let passwords = try makeVault(directory: directory, password: "sync push master password")
        defer { try? passwords.lock() }
        let transport = FakeSyncTransport()
        let engine = SyncEngine(vaultURL: passwords.fileURL, passwords: passwords, transport: transport)

        await transport.enqueuePut(.success(SyncSlotState(version: 4, size: 10, sha256: "def456")))
        let stored = try await engine.push(server: server, token: "maine-TEST", lastVersion: 3, lastSHA256: "old")
        guard case .stored(let state) = stored else {
            Issue.record("expected a stored push")
            return
        }
        #expect(state.version == 4)

        await transport.enqueuePut(.failure(SyncTransportError.conflict(currentVersion: 9)))
        do {
            _ = try await engine.push(server: server, token: "maine-TEST", lastVersion: 4, lastSHA256: "stale")
            Issue.record("expected a conflict")
        } catch let error as SyncTransportError {
            #expect(error == .conflict(currentVersion: 9))
        }
    }

    @Test
    func joinWithTheWrongMasterPasswordChangesNothing() async throws {
        let localDirectory = temporaryDirectory()
        let remoteDirectory = temporaryDirectory()
        defer {
            try? FileManager.default.removeItem(at: localDirectory)
            try? FileManager.default.removeItem(at: remoteDirectory)
        }
        let local = try makeVault(directory: localDirectory, password: "local sync master password")
        let remote = try makeVault(directory: remoteDirectory, password: "remote sync master password")
        defer {
            try? local.lock()
            try? remote.lock()
        }
        let localBytes = try Data(contentsOf: local.fileURL)
        let remoteBytes = try Data(contentsOf: remote.fileURL)
        let state = SyncSlotState(version: 2, size: remoteBytes.count, sha256: SyncEngine.sha256Hex(remoteBytes))

        let transport = FakeSyncTransport()
        await transport.enqueueFetch(.success(.blob(remoteBytes, state)))
        let engine = SyncEngine(vaultURL: local.fileURL, passwords: local, transport: transport)

        do {
            _ = try await engine.prepareJoin(server: server, token: "maine-TEST", masterPassword: "definitely wrong")
            Issue.record("expected the wrong password to be rejected")
        } catch let error as SyncEngineError {
            guard case .wrongMasterPassword = error else {
                Issue.record("unexpected error \(error)")
                return
            }
        }

        let after = try Data(contentsOf: local.fileURL)
        #expect(after == localBytes)
        let files = try names(in: localDirectory)
        #expect(files.contains { $0.hasPrefix("passwords.remote-") })
    }

    @Test
    func joinIntoAnEmptySlotOffersToPush() async throws {
        let directory = temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let passwords = try makeVault(directory: directory, password: "sync empty slot master")
        defer { try? passwords.lock() }
        let transport = FakeSyncTransport()
        await transport.enqueueFetch(.success(.empty(SyncSlotState(version: 0))))
        let engine = SyncEngine(vaultURL: passwords.fileURL, passwords: passwords, transport: transport)

        let preparation = try await engine.prepareJoin(server: server, token: "maine-TEST", masterPassword: "whatever")

        #expect(preparation == .emptySlot)
    }

    @Test
    func adoptingAJoinMovesTheLocalVaultToAPreSyncCopy() async throws {
        let localDirectory = temporaryDirectory()
        let remoteDirectory = temporaryDirectory()
        defer {
            try? FileManager.default.removeItem(at: localDirectory)
            try? FileManager.default.removeItem(at: remoteDirectory)
        }
        let local = try makeVault(directory: localDirectory, password: "local sync master password")
        let remote = try makeVault(directory: remoteDirectory, password: "remote sync master password")
        defer {
            try? local.lock()
            try? remote.lock()
        }
        let remoteBytes = try Data(contentsOf: remote.fileURL)
        let state = SyncSlotState(version: 2, size: remoteBytes.count, sha256: SyncEngine.sha256Hex(remoteBytes))
        let engine = SyncEngine(vaultURL: local.fileURL, passwords: local, transport: FakeSyncTransport())

        try engine.adoptPreparedJoin(remoteBytes, state: state)

        let installed = try Data(contentsOf: local.fileURL)
        #expect(installed == remoteBytes)
        let files = try names(in: localDirectory)
        #expect(files.contains { $0.hasPrefix("passwords.pre-sync-") })
        let attributes = try FileManager.default.attributesOfItem(atPath: local.fileURL.path)
        #expect((attributes[.posixPermissions] as? NSNumber)?.intValue == 0o600)
    }

    @Test
    func pullKeepsADirtyLocalVaultAsAConflictCopy() async throws {
        let directory = temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let passwords = try makeVault(directory: directory, password: "sync pull master password")
        defer { try? passwords.lock() }
        let original = try Data(contentsOf: passwords.fileURL)
        let edited = Data("a local edit that must survive".utf8)
        try edited.write(to: passwords.fileURL)
        let state = SyncSlotState(version: 7, size: original.count, sha256: SyncEngine.sha256Hex(original))

        let transport = FakeSyncTransport()
        await transport.enqueueFetch(.success(.blob(original, state)))
        let engine = SyncEngine(vaultURL: passwords.fileURL, passwords: passwords, transport: transport)

        let result = try await engine.pull(server: server, token: "maine-TEST", lastVersion: 3, isDirty: true)

        guard case .adopted(let adopted, let conflictCopy) = result else {
            Issue.record("expected the remote vault to be adopted")
            return
        }
        #expect(adopted.version == 7)
        let name = try #require(conflictCopy)
        #expect(name.hasPrefix("passwords.conflict-"))
        let copy = try Data(contentsOf: directory.appendingPathComponent(name))
        #expect(copy == edited)
        let live = try Data(contentsOf: passwords.fileURL)
        #expect(live == original)
    }

    @Test
    func pullRefusesAVaultThisSessionKeyCannotOpen() async throws {
        let localDirectory = temporaryDirectory()
        let remoteDirectory = temporaryDirectory()
        defer {
            try? FileManager.default.removeItem(at: localDirectory)
            try? FileManager.default.removeItem(at: remoteDirectory)
        }
        let local = try makeVault(directory: localDirectory, password: "local sync master password")
        let remote = try makeVault(directory: remoteDirectory, password: "remote sync master password")
        defer {
            try? local.lock()
            try? remote.lock()
        }
        let localBytes = try Data(contentsOf: local.fileURL)
        let remoteBytes = try Data(contentsOf: remote.fileURL)
        let state = SyncSlotState(version: 5, size: remoteBytes.count, sha256: SyncEngine.sha256Hex(remoteBytes))

        let transport = FakeSyncTransport()
        await transport.enqueueFetch(.success(.blob(remoteBytes, state)))
        let engine = SyncEngine(vaultURL: local.fileURL, passwords: local, transport: transport)

        do {
            _ = try await engine.pull(server: server, token: "maine-TEST", lastVersion: 2, isDirty: false)
            Issue.record("expected the foreign vault to be refused")
        } catch let error as SyncEngineError {
            guard case .foreignVault = error else {
                Issue.record("unexpected error \(error)")
                return
            }
        }

        let live = try Data(contentsOf: local.fileURL)
        #expect(live == localBytes)
        let files = try names(in: localDirectory)
        #expect(files.contains { $0.hasPrefix("passwords.remote-") })
    }

    @Test
    func rotateAndDeletePassThroughToTheServer() async throws {
        let directory = temporaryDirectory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let passwords = try makeVault(directory: directory, password: "sync manage master password")
        defer { try? passwords.lock() }
        let transport = FakeSyncTransport()
        let engine = SyncEngine(vaultURL: passwords.fileURL, passwords: passwords, transport: transport)

        await transport.enqueueRotate(.success("maine-NEW1-NEW2"))
        let rotated = try await engine.rotate(server: server, token: "maine-TEST")
        #expect(rotated == "maine-NEW1-NEW2")

        await transport.enqueueDelete(.success(()))
        try await engine.delete(server: server, token: rotated)

        let calls = await transport.calls
        #expect(calls == ["rotate", "delete"])
    }
}
