import Foundation

/// Opt-in, local JSON-lines diagnostics. Call sites supply metadata only, never
/// passwords, terminal output, commands, environment variables or field values.
@MainActor
public final class DebugLog: ObservableObject {
    public static let shared = DebugLog(
        fileURL: defaultFileURL,
        enabled: UserDefaults.standard.bool(forKey: "MaineDebugLoggingEnabled")
            || ProcessInfo.processInfo.environment["MAINE_DEBUG"] == "1"
            || CommandLine.arguments.contains("--debug"),
        preferences: .standard
    )

    public static var defaultFileURL: URL {
        FileManager.default.homeDirectoryForCurrentUser
            .appendingPathComponent("Library/Logs/Maine", isDirectory: true)
            .appendingPathComponent("debug.jsonl")
    }

    public let fileURL: URL
    public let runID = UUID()
    @Published public private(set) var isEnabled: Bool
    @Published public private(set) var lastError: String?
    private let preferences: UserDefaults?
    private let maximumBytes: Int
    private var sequence = 0

    public init(fileURL: URL, enabled: Bool = false, preferences: UserDefaults? = nil,
                maximumBytes: Int = 5 * 1024 * 1024) {
        self.fileURL = fileURL
        self.isEnabled = enabled
        self.preferences = preferences
        self.maximumBytes = maximumBytes
    }

    public func setEnabled(_ enabled: Bool) {
        lastError = nil
        if !enabled { record("debug.disabled", flush: true) }
        isEnabled = enabled
        preferences?.set(enabled, forKey: "MaineDebugLoggingEnabled")
        if enabled { record("debug.enabled", fields: ["format": "disconnect-diagnostics-v1"], flush: true) }
    }

    public func record(_ event: String, fields: @autoclosure () -> [String: String] = [:], flush: Bool = false) {
        guard isEnabled else { return }
        do {
            try prepareFile()
            sequence += 1
            let record = Record(timestamp: Date().ISO8601Format(), runID: runID,
                                sequence: sequence, uptime: ProcessInfo.processInfo.systemUptime,
                                appPID: ProcessInfo.processInfo.processIdentifier,
                                event: event, fields: fields())
            let encoder = JSONEncoder()
            encoder.outputFormatting = [.sortedKeys]
            var data = try encoder.encode(record)
            data.append(0x0a)
            let handle = try FileHandle(forWritingTo: fileURL)
            defer { try? handle.close() }
            try handle.seekToEnd()
            try handle.write(contentsOf: data)
            if flush { try handle.synchronize() }
        } catch {
            let message = "Debug log could not be written: \(error.localizedDescription)"
            if lastError != message { lastError = message }
        }
    }

    public var previousFileURL: URL {
        fileURL.deletingLastPathComponent().appendingPathComponent("debug.previous.jsonl")
    }

    private struct Record: Encodable {
        let timestamp: String
        let runID: UUID
        let sequence: Int
        let uptime: TimeInterval
        let appPID: Int32
        let event: String
        let fields: [String: String]
    }

    private func prepareFile() throws {
        let files = FileManager.default
        try files.createDirectory(at: fileURL.deletingLastPathComponent(), withIntermediateDirectories: true)
        if files.fileExists(atPath: fileURL.path) {
            let attributes = try files.attributesOfItem(atPath: fileURL.path)
            if ((attributes[.size] as? NSNumber)?.intValue ?? 0) >= maximumBytes {
                if files.fileExists(atPath: previousFileURL.path) { try files.removeItem(at: previousFileURL) }
                try files.moveItem(at: fileURL, to: previousFileURL)
            }
        }
        if !files.fileExists(atPath: fileURL.path) {
            guard files.createFile(atPath: fileURL.path, contents: nil,
                                   attributes: [.posixPermissions: 0o600]) else {
                throw CocoaError(.fileWriteUnknown)
            }
        }
        try files.setAttributes([.posixPermissions: 0o600], ofItemAtPath: fileURL.path)
    }
}
