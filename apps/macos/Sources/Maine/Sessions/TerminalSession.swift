import AppKit
import Foundation
import SwiftTerm

/// One live connection: a SwiftTerm view backed by the system `ssh` client
/// running inside a PTY.
///
/// Each session owns its own terminal view, so reconnecting starts from a clean
/// screen. Nothing here knows about SwiftUI beyond being `@MainActor`, and
/// nothing here knows where the host list is stored.
@MainActor
public final class TerminalSession: ObservableObject, Identifiable {
    public enum State: Equatable, Sendable {
        /// The view exists but nothing has been started.
        case idle
        /// `ssh` has been launched and the connection is being established.
        case connecting
        /// The session has a terminal and `ssh` is running.
        case connected
        /// `ssh` exited. A nil exit code means it died from a signal.
        case closed(exitCode: Int32?)
        /// The user requested disconnect. Preserve the process's raw exit code,
        /// but do not present OpenSSH's SIGTERM/255 exit as a connection error.
        case disconnected(exitCode: Int32?)
        /// The session could not be started at all.
        case failed(String)

        public var isLive: Bool {
            switch self {
            case .connecting, .connected: return true
            case .idle, .closed, .disconnected, .failed: return false
            }
        }

        /// A short label for the status indicator.
        public var label: String {
            switch self {
            case .idle: return "Not connected"
            case .connecting: return "Connecting…"
            case .connected: return "Connected"
            case .closed(let exitCode): return "Closed (\(exitCode.map(String.init) ?? "interrupted"))"
            case .disconnected: return "Disconnected"
            case .failed: return "Could not connect"
            }
        }

        /// The problem worth showing the user, if any.
        public var notice: String? {
            switch self {
            case .closed(let exitCode):
                guard let exitCode, exitCode != 0 else { return nil }
                return "The connection closed with exit code \(exitCode). "
                    + "The full session output, including what ssh reported, is in the terminal below."
            case .failed(let message):
                return message
            case .idle, .connecting, .connected, .disconnected:
                return nil
            }
        }
    }

    public let id = UUID()
    public let host: Host
    /// The executable this session runs: the system OpenSSH client in the app.
    public let executablePath: String
    public let terminalView: LocalProcessTerminalView
    @Published public internal(set) var customName: String?
    @Published public internal(set) var savedSessionID: UUID?

    @Published public private(set) var state: State = .idle {
        didSet {
            DebugLog.shared.record("session.state", fields: debugFields.merging(["previousState": oldValue.debugName]) { _, new in new })
        }
    }
    @Published public private(set) var needsVaultUnlock = false
    @Published public private(set) var needsPassword = false
    private var disconnectRequested = false

    public init(host: Host, executablePath: String = SSHTarget.executablePath) {
        self.host = host
        self.executablePath = executablePath
        // A sensible starting size: the view is usually laid out before ssh
        // produces output, and the PTY is resized as soon as the frame is real.
        terminalView = LocalProcessTerminalView(
            frame: NSRect(x: 0, y: 0, width: 800, height: 480),
            options: TerminalOptions(cols: 80, rows: 24)
        )
        terminalView.processDelegate = self
        // Stable terminal contrast in both system appearances. ANSI colors and
        // the remote application's escape sequences remain terminal-owned.
        terminalView.font = NSFont.monospacedSystemFont(ofSize: 13, weight: .regular)
        terminalView.nativeBackgroundColor = NSColor(calibratedWhite: 0.055, alpha: 1)
        terminalView.nativeForegroundColor = NSColor(calibratedWhite: 0.88, alpha: 1)
        DebugLog.shared.record("session.created", fields: debugFields)
    }

    // MARK: - Lifecycle

    /// Snapshot text only. Terminal output can contain secrets, so it belongs
    /// exclusively in the encrypted vault, never diagnostic logs.
    public func historySnapshot() -> String {
        let data = terminalView.getBufferAsData()
        return String(decoding: data.suffix(1_048_576), as: UTF8.self)
            .trimmingCharacters(in: .newlines)
    }

    public func restoreHistory(_ history: String) {
        // Remove control characters before display: no OSC/escape commands,
        // clipboard actions, or input sent to a child process.
        let text = String(history.unicodeScalars.filter {
            $0 == "\n" || $0 == "\t" || !CharacterSet.controlCharacters.contains($0)
        })
        terminalView.feed(text: text.replacingOccurrences(of: "\n", with: "\r\n") + "\r\n")
    }

    public func connect() {
        DebugLog.shared.record("session.connect-request", fields: debugFields)
        switch state {
        case .closed, .disconnected, .failed, .idle:
            break
        case .connecting, .connected:
            return
        }
        disconnectRequested = false

        if FileManager.default.isExecutableFile(atPath: executablePath) == false {
            state = .failed("Maine could not run the SSH client at \(executablePath). "
                + "macOS ships OpenSSH in /usr/bin; check that the file is there.")
            return
        }
        if !host.isValid {
            state = .failed(host.issues.joined(separator: " "))
            return
        }

        let target = SSHTarget.make(for: host)
        needsVaultUnlock = false
        needsPassword = false
        var environment = TerminalEnvironment.variables
        if executablePath == SSHTarget.executablePath {
            do {
                if try PasswordStore.shared.password(for: host.id) != nil {
                    environment = SSHAskPass.environment(
                        for: host.id, base: environment,
                        executable: Bundle.main.executableURL?.path ?? CommandLine.arguments[0]
                    )
                } else {
                    needsPassword = true
                    state = .failed("This host needs a saved password. Edit the host to add one.")
                    return
                }
            } catch {
                if case PasswordStore.VaultError.locked = error { needsVaultUnlock = true }
                state = .failed(error.localizedDescription)
                return
            }
        }
        state = .connecting
        terminalView.startProcess(
            executable: executablePath,
            args: target.arguments,
            environment: environment
        )
        DebugLog.shared.record("session.process-start-returned", fields: debugFields)
    }

    /// Ends the session. Any output ssh still had to say is left on screen.
    public func disconnect() {
        DebugLog.shared.record("disconnect.request", fields: debugFields, flush: true)
        guard state.isLive else {
            DebugLog.shared.record("disconnect.ignored-not-live", fields: debugFields)
            return
        }
        let correlation = UUID().uuidString
        disconnectRequested = true
        DebugDiagnostics.capture("disconnect.\(correlation).before")
        DebugLog.shared.record("disconnect.terminate-begin", fields: debugFields.merging(["correlation": correlation]) { _, new in new }, flush: true)
        let start = ProcessInfo.processInfo.systemUptime
        terminalView.terminate()
        DebugLog.shared.record("disconnect.terminate-returned", fields: debugFields.merging([
            "correlation": correlation,
            "elapsedMS": String((ProcessInfo.processInfo.systemUptime - start) * 1000)
        ]) { _, new in new }, flush: true)
        DebugDiagnostics.scheduleDisconnectSnapshots(session: self, correlation: correlation)
    }

    /// Gives the terminal keyboard focus.
    public func focus() {
        terminalView.window?.makeFirstResponder(terminalView)
    }
}

extension TerminalSession: LocalProcessTerminalViewDelegate {
    /// The terminal has been sized for the first time, which means `ssh` now has
    /// a real terminal to write to. Used as the "connection is up" signal,
    /// because ssh says nothing when it connects quietly.
    public func sizeChanged(source: LocalProcessTerminalView, newCols: Int, newRows: Int) {
        DebugLog.shared.record("terminal.size-changed", fields: [
            "session": id.uuidString, "cols": String(newCols), "rows": String(newRows), "state": state.debugName
        ])
        guard case .connecting = state else { return }
        // AppKit calls this during SwiftUI layout. Publishing synchronously here
        // invalidates the view tree while it is being laid out (undefined UI).
        Task { @MainActor [weak self] in
            guard let self, case .connecting = self.state else { return }
            self.state = .connected
        }
    }

    public func setTerminalTitle(source: LocalProcessTerminalView, title: String) {
        // Maine labels sessions with the host name, so the title ssh reports
        // (which is usually the host it connected to) is left in the terminal.
    }

    public func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {
        // Available for future use; nothing needs it yet.
    }

    public func processTerminated(source: TerminalView, exitCode: Int32?) {
        DebugLog.shared.record("process.terminated-callback", fields: debugFields.merging([
            "exitCode": exitCode.map(String.init) ?? "signal"
        ]) { _, new in new }, flush: true)
        state = disconnectRequested ? .disconnected(exitCode: exitCode) : .closed(exitCode: exitCode)
    }

    public func processFailedToStart(source: TerminalView, error: LocalProcessError) {
        DebugLog.shared.record("process.failed-callback", fields: debugFields)
        state = .failed(error.message)
    }
}

extension LocalProcessError {
    /// A short, human readable sentence for each failure.
    var message: String {
        switch self {
        case .alreadyRunning:
            return "A terminal session is already running here."
        case .notRunning:
            return "There is no terminal session running here."
        case .forkFailed(let code):
            return "The terminal could not be started (fork failed, error \(code))."
        case .writeChannelFailed(let code):
            return "The terminal could not be started (error \(code))."
        case .writeFailed(let code, let bytesWritten):
            return "Keystrokes could not be sent to the terminal "
                + "(\(bytesWritten) of the bytes were written, error \(code))."
        }
    }
}
