import Foundation
import Maine
import Testing

/// Everything here runs against a throwaway directory: the tests must never
/// touch the real `~/Library/Application Support/Maine/hosts.json`.
///
/// Swift Testing runs the tests in this suite in parallel, so every test builds
/// its own scratch folder.
@MainActor
@Suite
struct HostStoreTests {
    private func scratchFolder() -> URL {
        let folder = FileManager.default.temporaryDirectory
            .appendingPathComponent("maine-store-tests-\(UUID().uuidString)", isDirectory: true)
        try? FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        return folder
    }

    /// A store and the file it writes to, inside a folder of its own.
    private func makeStore() -> (store: HostStore, fileURL: URL, folder: URL) {
        let folder = scratchFolder()
        let fileURL = folder.appendingPathComponent("hosts.json", conformingTo: .json)
        return (HostStore(fileURL: fileURL), fileURL, folder)
    }

    // MARK: - Loading

    @Test
    func missingFileLoadsAsAnEmptyList() async {
        let (store, _, _) = makeStore()
        store.load()

        #expect(store.hosts.isEmpty)
        #expect(store.loadFailure == nil)
    }

    @Test
    func savedHostsSurviveARestart() async throws {
        let (store, fileURL, _) = makeStore()
        defer { try? FileManager.default.removeItem(at: fileURL.deletingLastPathComponent()) }

        let host = Host(name: "Web", hostname: "example.com", port: 2222, username: "deploy")
        store.add(host)

        // A fresh store is what a relaunch sees.
        let relaunched = HostStore(fileURL: fileURL)
        relaunched.load()

        #expect(relaunched.hosts == [host])
        #expect(relaunched.loadFailure == nil)
    }

    @Test
    func savedFileIsPrivate() async throws {
        let (store, fileURL, _) = makeStore()
        defer { try? FileManager.default.removeItem(at: fileURL.deletingLastPathComponent()) }

        store.add(Host(name: "Web", hostname: "example.com"))

        let permissions = try FileManager.default
            .attributesOfItem(atPath: fileURL.path)[.posixPermissions] as? Int
        #expect(permissions == 0o600)
    }

    @Test
    func savedFileWritesTheSchemaVersion() async throws {
        let (store, fileURL, _) = makeStore()
        defer { try? FileManager.default.removeItem(at: fileURL.deletingLastPathComponent()) }

        store.add(Host(name: "Web", hostname: "example.com"))

        let object = try #require(
            try JSONSerialization.jsonObject(with: Data(contentsOf: fileURL)) as? [String: Any]
        )
        #expect(object["schemaVersion"] as? Int == HostConfiguration.currentSchemaVersion)
    }

    @Test
    func unreadableFileReportsAFailureAndLeavesTheFileAlone() async throws {
        let (store, fileURL, _) = makeStore()
        defer { try? FileManager.default.removeItem(at: fileURL.deletingLastPathComponent()) }

        try Data("this is not json".utf8).write(to: fileURL)
        store.load()

        #expect(store.hosts.isEmpty)
        #expect(store.loadFailure?.fileURL == fileURL)
        #expect(store.loadFailure?.message != nil)

        // Loading must not have replaced the broken file.
        #expect(try Data(contentsOf: fileURL) == Data("this is not json".utf8))
    }

    @Test
    func fileFromANewerVersionReportsAFailure() async throws {
        let (store, fileURL, _) = makeStore()
        defer { try? FileManager.default.removeItem(at: fileURL.deletingLastPathComponent()) }

        try #"{"schemaVersion":99,"hosts":[]}"#.data(using: .utf8)?.write(to: fileURL)
        store.load()

        #expect(store.hosts.isEmpty)
        #expect(store.loadFailure != nil)
    }

    // MARK: - Editing

    @Test
    func addingAnInvalidHostIsRefused() async {
        let (store, fileURL, _) = makeStore()
        defer { try? FileManager.default.removeItem(at: fileURL.deletingLastPathComponent()) }

        let result = store.add(Host(name: "", hostname: "example.com"))

        guard case .failure = result else {
            Issue.record("Expected the invalid host to be refused, got \(result)")
            return
        }
        #expect(store.hosts.isEmpty)
        #expect(FileManager.default.fileExists(atPath: fileURL.path) == false)
    }

    @Test
    func updatingAHostPersistsTheChange() async {
        let (store, fileURL, _) = makeStore()
        defer { try? FileManager.default.removeItem(at: fileURL.deletingLastPathComponent()) }

        var host = Host(name: "Web", hostname: "example.com")
        store.add(host)
        host.port = 2222

        store.update(host)

        let relaunched = HostStore(fileURL: fileURL)
        relaunched.load()
        #expect(relaunched.hosts.first?.port == 2222)
    }

    @Test
    func deletingAHostPersistsTheChange() async {
        let (store, fileURL, _) = makeStore()
        defer { try? FileManager.default.removeItem(at: fileURL.deletingLastPathComponent()) }

        let host = Host(name: "Web", hostname: "example.com")
        store.add(host)
        store.delete(host)

        let relaunched = HostStore(fileURL: fileURL)
        relaunched.load()
        #expect(relaunched.hosts.isEmpty)
    }

    @Test
    func batchAddWritesEveryHost() async {
        let (store, fileURL, _) = makeStore()
        defer { try? FileManager.default.removeItem(at: fileURL.deletingLastPathComponent()) }

        let hosts = (1...3).map { Host(name: "Host \($0)", hostname: "host\($0).example.com") }
        store.add(hosts)

        let relaunched = HostStore(fileURL: fileURL)
        relaunched.load()
        #expect(relaunched.hosts.count == 3)
    }

    @Test
    func lookupsAreCaseInsensitive() async {
        let (store, fileURL, _) = makeStore()
        defer { try? FileManager.default.removeItem(at: fileURL.deletingLastPathComponent()) }

        store.add(Host(name: "Web", hostname: "Example.com"))

        #expect(store.containsHost(named: "example.COM"))
        #expect(store.containsHost(named: "other.example.com") == false)
    }

    @Test
    func hostLookupByID() async {
        let (store, fileURL, _) = makeStore()
        defer { try? FileManager.default.removeItem(at: fileURL.deletingLastPathComponent()) }

        let host = Host(name: "Web", hostname: "example.com")
        store.add(host)

        #expect(store.host(id: host.id) == host)
        #expect(store.host(id: nil) == nil)
    }
}
