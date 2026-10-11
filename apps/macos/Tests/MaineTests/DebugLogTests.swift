import AppKit
@testable import Maine
import Testing

@MainActor
@Suite(.serialized)
struct DebugLogTests {
    private func directory() -> URL {
        FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    }

    @Test
    func disabledLoggingDoesNotCreateFilesOrEvaluateFields() {
        let directory = directory()
        let log = DebugLog(fileURL: directory.appendingPathComponent("debug.jsonl"))
        var evaluated = false
        func fields() -> [String: String] { evaluated = true; return [:] }
        log.record("test", fields: fields())
        #expect(!evaluated)
        #expect(!FileManager.default.fileExists(atPath: directory.path))
    }

    @Test
    func logsArePrivateJSONLinesAndToggleStopsAppending() throws {
        let directory = directory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let log = DebugLog(fileURL: directory.appendingPathComponent("debug.jsonl"))
        log.setEnabled(true)
        log.record("disconnect.request", fields: ["session": "test-session"], flush: true)
        log.setEnabled(false)
        let data = try Data(contentsOf: log.fileURL)
        log.record("must-not-be-written")
        #expect(try Data(contentsOf: log.fileURL) == data)
        let text = try #require(String(data: data, encoding: .utf8))
        let records = try text.split(separator: "\n").map {
            try #require(JSONSerialization.jsonObject(with: Data($0.utf8)) as? [String: Any])
        }
        #expect(records.compactMap { $0["event"] as? String } == ["debug.enabled", "disconnect.request", "debug.disabled"])
        #expect(records.compactMap { $0["sequence"] as? Int } == [1, 2, 3])
        #expect(records.allSatisfy { ($0["runID"] as? String) == log.runID.uuidString })
        let attributes = try FileManager.default.attributesOfItem(atPath: log.fileURL.path)
        #expect((attributes[.posixPermissions] as? NSNumber)?.intValue == 0o600)
        #expect(log.lastError == nil)
    }

    @Test
    func rotationKeepsOnlyOnePreviousLog() throws {
        let directory = directory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let log = DebugLog(fileURL: directory.appendingPathComponent("debug.jsonl"), enabled: true, maximumBytes: 1)
        log.record("first")
        log.record("second")
        log.record("third")
        let current = try String(contentsOf: log.fileURL, encoding: .utf8)
        let previous = try String(contentsOf: log.previousFileURL, encoding: .utf8)
        #expect(current.contains("third"))
        #expect(previous.contains("second"))
        #expect(!previous.contains("first"))
        #expect(try FileManager.default.contentsOfDirectory(atPath: directory.path).count == 2)
    }

    @Test
    func snapshotsExcludeFieldValuesTitlesAndPrivateHostData() throws {
        _ = NSApplication.shared
        let directory = directory()
        defer { try? FileManager.default.removeItem(at: directory) }
        let log = DebugLog(fileURL: directory.appendingPathComponent("debug.jsonl"), enabled: true)
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 400, height: 200),
                              styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.title = "PRIVATE-WINDOW-TITLE"
        let password = NSSecureTextField(frame: NSRect(x: 0, y: 0, width: 200, height: 24))
        password.stringValue = "PRIVATE-MASTER-PASSWORD"
        window.contentView?.addSubview(password)
        defer { window.close() }
        let host = Host(name: "PRIVATE-HOST-NAME", hostname: "private-host.example", username: "PRIVATE-USERNAME")
        let session = TerminalSession(host: host, executablePath: "/nonexistent")
        log.record("session", fields: session.debugFields)
        DebugDiagnostics.capture("test", log: log)
        let text = try String(contentsOf: log.fileURL, encoding: .utf8)
        for secret in [window.title, password.stringValue, host.name, host.hostname, host.username] {
            #expect(!text.contains(secret))
        }
        #expect(text.contains("snapshot.window"))
        #expect(text.contains("snapshot.view"))
        #expect(text.contains("snapshot.end"))
        #expect(TerminalSession.State.failed("PRIVATE-ERROR").debugName == "failed")
    }
}
