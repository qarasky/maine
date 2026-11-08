import Foundation
@testable import Maine
import Testing

/// The swap helper runs after Maine quits, so it must be a plain, explainable
/// bash script: wait for the old process, move the bundles, restore on failure.
@Suite
struct UpdateInstallerTests {
    @Test
    func sha256MatchesKnownVector() throws {
        let file = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: file) }
        try Data("abc".utf8).write(to: file)

        #expect(try UpdateInstaller.sha256(ofFileAt: file)
                == "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad")
    }

    @Test
    func swapReplacesTheBundleAfterTheParentExits() throws {
        let root = try makeRoot()
        defer { try? FileManager.default.removeItem(at: root) }
        let target = root.appendingPathComponent("Maine.app", isDirectory: true)
        let staged = root.appendingPathComponent(".Maine-update-\(UUID().uuidString)/Maine.app", isDirectory: true)
        try makeApp(target, marker: "old")
        try makeApp(staged, marker: "new")
        let pid = try exitedPID()

        let script = UpdateInstaller.swapScript(parentPID: pid, staged: staged, target: target)
        let status = try runSwap(script, parentPID: pid, staged: staged, target: target)

        #expect(status == 0)
        #expect(try marker(at: target) == "new")
        #expect(!FileManager.default.fileExists(atPath: staged.path))
        #expect(!FileManager.default.fileExists(atPath: target.appendingPathExtension("old").path))
        #expect(!FileManager.default.fileExists(atPath: staged.deletingLastPathComponent().path))
    }

    @Test
    func swapRestoresTheOldBundleWhenStagingIsMissing() throws {
        let root = try makeRoot()
        defer { try? FileManager.default.removeItem(at: root) }
        let target = root.appendingPathComponent("Maine.app", isDirectory: true)
        let missing = root.appendingPathComponent(".Maine-update-\(UUID().uuidString)/Maine.app", isDirectory: true)
        try makeApp(target, marker: "old")
        let pid = try exitedPID()

        let script = UpdateInstaller.swapScript(parentPID: pid, staged: missing, target: target)
        let status = try runSwap(script, parentPID: pid, staged: missing, target: target)

        #expect(status != 0)
        #expect(try marker(at: target) == "old")
        #expect(!FileManager.default.fileExists(atPath: target.appendingPathExtension("old").path))
    }

    @Test
    func swapHandlesPathsWithSpacesAndQuotes() throws {
        let root = try makeRoot(named: "Maine update's dir")
        defer { try? FileManager.default.removeItem(at: root) }
        let target = root.appendingPathComponent("Old Maine's App.app", isDirectory: true)
        let staged = root.appendingPathComponent(".Maine-update-\(UUID().uuidString)/Maine's App.app", isDirectory: true)
        try makeApp(target, marker: "old")
        try makeApp(staged, marker: "new")
        let pid = try exitedPID()

        let script = UpdateInstaller.swapScript(parentPID: pid, staged: staged, target: target)
        // Paths never appear in the script text; they arrive as arguments only.
        #expect(!script.contains(target.path))
        #expect(!script.contains(staged.path))
        let status = try runSwap(script, parentPID: pid, staged: staged, target: target, in: root)

        #expect(status == 0)
        #expect(try marker(at: target) == "new")
        #expect(!FileManager.default.fileExists(atPath: staged.deletingLastPathComponent().path))
    }

    @Test
    func canInstallRefusesVolumesAndReadOnlyParents() throws {
        #expect(UpdateInstaller.canInstall(over: URL(fileURLWithPath: "/Volumes/Maine/Maine.app")) == false)
        #expect(UpdateInstaller.canInstall(over: URL(fileURLWithPath: "/private/var/folders/x/AppTranslocation/abc/d/Maine.app")) == false)
        #expect(UpdateInstaller.canInstall(over: URL(fileURLWithPath: "/tmp/not-an-app")) == false)

        let root = try makeRoot()
        let readOnly = root.appendingPathComponent("read-only", isDirectory: true)
        try FileManager.default.createDirectory(at: readOnly, withIntermediateDirectories: true)
        try FileManager.default.setAttributes([.posixPermissions: 0o555], ofItemAtPath: readOnly.path)
        defer {
            try? FileManager.default.setAttributes([.posixPermissions: 0o755], ofItemAtPath: readOnly.path)
            try? FileManager.default.removeItem(at: root)
        }

        #expect(UpdateInstaller.canInstall(over: readOnly.appendingPathComponent("Maine.app")) == false)
        #expect(UpdateInstaller.canInstall(over: root.appendingPathComponent("Maine.app")) == true)
    }

    // MARK: - Helpers

    private func makeRoot(named name: String = "Maine update test") throws -> URL {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent("\(name) \(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        return root
    }

    private func makeApp(_ url: URL, marker: String) throws {
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        try Data(marker.utf8).write(to: url.appendingPathComponent("marker.txt"))
    }

    private func marker(at app: URL) throws -> String {
        try String(contentsOf: app.appendingPathComponent("marker.txt"), encoding: .utf8)
    }

    /// A real PID that has already exited, so `kill -0` fails at once.
    private func exitedPID() throws -> Int32 {
        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/usr/bin/true")
        try process.run()
        process.waitUntilExit()
        return process.processIdentifier
    }

    /// Runs the helper the way `launchSwap` does: paths in as arguments only.
    @discardableResult
    private func runSwap(_ script: String,
                         parentPID: Int32,
                         staged: URL,
                         target: URL,
                         relaunch: String = "/usr/bin/true",
                         in directory: URL? = nil) throws -> Int32 {
        let scriptURL = (directory ?? FileManager.default.temporaryDirectory)
            .appendingPathComponent("swap-\(UUID().uuidString).sh")
        try script.write(to: scriptURL, atomically: true, encoding: .utf8)
        defer { try? FileManager.default.removeItem(at: scriptURL) }

        let process = Process()
        process.executableURL = URL(fileURLWithPath: "/bin/bash")
        process.arguments = [scriptURL.path, String(parentPID), staged.path, target.path, relaunch]
        process.standardOutput = FileHandle.nullDevice
        process.standardError = FileHandle.nullDevice
        try process.run()
        process.waitUntilExit()
        return process.terminationStatus
    }
}
