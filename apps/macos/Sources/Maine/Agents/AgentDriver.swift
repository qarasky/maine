import Foundation

/// Why a session could not be handed to an agent.
public enum AgentHandOffError: Error, Equatable, LocalizedError {
    case notFound, notAllowed, alreadyDriven, notAtPrompt

    public var errorDescription: String? {
        switch self {
        case .notFound: "That session or agent no longer exists."
        case .notAllowed: "Agent access is off, or this host is not allowed. Turn it on in Settings → Agents."
        case .alreadyDriven: "An agent is already driving this session."
        case .notAtPrompt: "Wait until the shell is at a prompt, with no command or full-screen program running."
        }
    }
}

public enum AgentDriverError: Error, Equatable, LocalizedError {
    case busy, paused, notConnected
    /// ssh is up but no remote shell is ready yet (still connecting, or
    /// waiting at a password, passphrase or host-key prompt).
    case notReady
    /// `type_password` found no password prompt on the last screen line.
    case noPasswordPrompt
    /// The caller's `shouldStop` fired; the handler decides what to report.
    case stopped

    public var errorDescription: String? {
        switch self {
        case .busy: "a command is still running; use read_output, or send_keys ctrl-c"
        case .paused: "paused by the user"
        case .notConnected: "session is not connected"
        case .notReady:
            "ssh is still connecting and the remote shell is not ready, so the command was not typed. "
                + "Use read_screen to see where it is (it may wait at a password or host-key prompt: "
                + "type_password or send_keys answers those), or wait and retry"
        case .stopped: "not found"
        case .noPasswordPrompt: "no password prompt is showing; the password was not typed"
        }
    }
}

/// A key or literal text an agent sends to its session.
enum AgentKey: Equatable, Sendable {
    case named(String)
    case text(String)

    private static let named: [String: [UInt8]] = {
        var keys: [String: [UInt8]] = [
            "enter": [0x0D], "tab": [0x09], "esc": [0x1B], "backspace": [0x7F],
            "up": Array("\u{1B}[A".utf8), "down": Array("\u{1B}[B".utf8),
            "right": Array("\u{1B}[C".utf8), "left": Array("\u{1B}[D".utf8),
        ]
        for (offset, letter) in "abcdefghijklmnopqrstuvwxyz".enumerated() {
            keys["ctrl-\(letter)"] = [UInt8(offset + 1)]
        }
        return keys
    }()

    init?(_ raw: String) {
        if raw.hasPrefix("text:") { self = .text(String(raw.dropFirst(5))); return }
        guard Self.named[raw] != nil else { return nil }
        self = .named(raw)
    }

    var bytes: [UInt8] {
        switch self {
        case .named(let name): Self.named[name] ?? []
        case .text(let text): Array(text.utf8)
        }
    }

    /// Ctrl-C / Ctrl-D end a running command without a completion marker.
    var interrupts: Bool { self == .named("ctrl-c") || self == .named("ctrl-d") }
}

/// Drives one agent session: types wrapped commands, keeps the cleaned
/// output, detects completion, and answers reads. Owned by its
/// `TerminalSession`; everything runs on the main actor, so a session's calls
/// are naturally ordered. Waiting is done by short polls, which keeps the
/// state machine small.
@MainActor
public final class AgentDriver: ObservableObject {
    struct RunResult: Equatable {
        var completed: Bool
        var exitCode: Int32?
        var output: String
        var cursor: Int
        var truncated: Bool
        var mode: String
    }

    struct ReadResult: Equatable {
        var output: String
        var cursor: Int
        var busy: Bool
        var truncated: Bool
    }

    struct ScreenResult: Equatable {
        var rows: [String]
        var cursorRow: Int
        var cursorCol: Int
        var alternateScreen: Bool
    }

    private enum Mode: String { case unknown, marker, quiet, osc133 }

    private struct PendingRun {
        var startCursor: Int
        var nonce: String?
        var detector: CompletionDetector?
        var endCursor: Int?
        var exitCode: Int32?
        var interrupted = false
        var finished = false
        /// The marker probe, which must not release a `run`'s busy reservation.
        var isProbe = false
        /// Where the command's own output starts, when the shell said so (`OSC 133;C`).
        var outputStart: Int?
    }

    public let clientID: UUID
    public let clientName: String
    @Published public var isPaused = false
    @Published public private(set) var isBusy = false
    @Published public private(set) var isActive = false

    private weak var session: TerminalSession?
    private var output = AgentOutput()
    private var mode = Mode.unknown
    private var pending: PendingRun?
    private var lastOutputAt = Date.distantPast
    private var shellMarks = OSC133Detector()
    /// The remote shell emitted `OSC 133;A` itself, so it has shell integration.
    private var hasShellIntegration = false
    /// Set when agent control ends; every wait loop gives up.
    private var isDetached = false
    /// Latched once a remote shell is up; see `isReady`.
    private var reachedShell = false
    private static let poll = Duration.milliseconds(20)

    init(session: TerminalSession, client: AgentClient) {
        self.session = session
        clientID = client.id
        clientName = client.name
    }

    /// Bytes from the PTY, before the terminal renders them.
    func ingest(_ bytes: ArraySlice<UInt8>) {
        lastOutputAt = Date()
        let marks = shellMarks.scan(bytes)
        if marks.contains(where: { $0.mark == .promptStart }) { hasShellIntegration = true }
        if mode == .osc133, !marks.isEmpty {
            ingestIntegrated(bytes, marks: marks)
            return
        }
        guard var run = pending, !run.finished, var detector = run.detector else {
            output.append(bytes)
            return
        }
        let hit = detector.scanForEnd(bytes)
        run.detector = detector
        guard let (code, end) = hit else {
            pending = run
            output.append(bytes)
            return
        }
        output.append(bytes[bytes.startIndex..<end])
        run.endCursor = output.cursor
        run.exitCode = code
        run.finished = true
        pending = run
        if !run.isProbe {
            isBusy = false
            isActive = false
        }
        output.append(bytes[end...])
    }

    /// A shell with its own integration marks: `C` starts the command's
    /// output, the next `D` ends the command. Bytes still flow unchanged.
    private func ingestIntegrated(_ bytes: ArraySlice<UInt8>,
                                  marks: [(mark: OSC133Detector.Mark, end: ArraySlice<UInt8>.Index)]) {
        var position = bytes.startIndex
        for (mark, end) in marks {
            output.append(bytes[position..<end])
            position = end
            guard var run = pending, !run.finished else { continue }
            switch mark {
            case .outputStart:
                if run.outputStart == nil { run.outputStart = output.cursor }
            case .commandDone(let code):
                run.endCursor = output.cursor
                run.exitCode = code
                run.finished = true
                isBusy = false
                isActive = false
            default:
                break
            }
            pending = run
        }
        output.append(bytes[position...])
    }

    // MARK: - Tools

    /// A remote shell is up and can take commands. False while ssh is still
    /// connecting or authenticating, including at a password, passphrase or
    /// host-key prompt. Latches, so a long-running command never un-readies it.
    var isReady: Bool {
        if reachedShell { return true }
        guard let session, session.state == .connected else { return false }
        if hasShellIntegration || session.isAtShellPrompt {
            reachedShell = true
        } else if output.cursor > 0, Date().timeIntervalSince(lastOutputAt) >= 1.5, !showsInputPrompt() {
            // An unrecognised prompt style: output has gone quiet at no known question.
            reachedShell = true
        }
        return reachedShell
    }

    /// ssh waits for the user: a password, passphrase, MFA code or host-key answer.
    private func showsInputPrompt() -> Bool {
        guard let session else { return false }
        let last = session.screenText().rows.last { !$0.trimmingCharacters(in: .whitespaces).isEmpty } ?? ""
        let line = last.trimmingCharacters(in: .whitespaces).lowercased()
        return SSHAskPass.isPasswordPrompt(last) || line.contains("(yes/no") || line.contains("passphrase")
            || line.hasSuffix(":") || line.hasSuffix("?")
    }

    /// After `open_session`: waits until the remote shell is ready, or until
    /// ssh stops at a prompt only the agent can answer, or the session dies.
    /// Then checks once whether this shell supports the completion marker.
    func settleAfterLogin(timeout: Duration) async -> ReadResult {
        let deadline = ContinuousClock.now + timeout
        while ContinuousClock.now < deadline, let session, session.state.isLive {
            if isReady { break }
            // At a prompt: hand control back so the agent can answer it.
            if session.state == .connected, output.cursor > 0,
               Date().timeIntervalSince(lastOutputAt) >= 1.5, showsInputPrompt() { break }
            try? await Task.sleep(for: Self.poll)
        }
        let login = output.text(since: 0)
        if isReady, mode == .unknown { await chooseMode() }
        return ReadResult(output: login.text, cursor: login.cursor, busy: isBusy, truncated: login.truncated)
    }

    /// Types `command` and waits for it. `shouldStop` is polled while waiting
    /// (access revoked mid-run); when it fires, `stopped` is thrown.
    func run(_ command: String, timeout: Duration, shouldStop: (@MainActor () -> Bool)? = nil) async throws -> RunResult {
        try ensureUsable()
        // Nothing is typed before the remote shell is up: a command typed into
        // a half-open connection lands in a password prompt or is lost.
        guard isReady else { throw AgentDriverError.notReady }
        guard !isBusy else { throw AgentDriverError.busy }
        // Reserve before the first suspension, so a parallel run sees busy.
        isBusy = true
        isActive = true
        do {
            await waitForHumanPause()
            try ensureUsable()
            if mode == .unknown { await chooseMode() }
            if isDetached { throw AgentDriverError.stopped }
        } catch {
            isBusy = false
            isActive = false
            throw error
        }
        let start = output.cursor
        if mode == .marker {
            let wrapper = CommandWrapper(command: command)
            pending = PendingRun(startCursor: start, nonce: wrapper.nonce, detector: CompletionDetector(nonce: wrapper.nonce))
            session?.sendInput(Array(wrapper.typedText.utf8))
        } else {
            // Quiet-period and shell-integration modes type the command as is.
            pending = PendingRun(startCursor: start)
            session?.sendInput(Array((command + "\n").utf8))
        }
        let deadline = ContinuousClock.now + timeout
        while ContinuousClock.now < deadline {
            refreshCompletion()
            if pending?.finished == true || session?.state.isLive != true { break }
            if isDetached || shouldStop?() == true { throw AgentDriverError.stopped }
            try? await Task.sleep(for: Self.poll)
        }
        refreshCompletion()
        if session?.state.isLive != true, pending?.finished != true {
            isBusy = false
            isActive = false
        }
        return runResult()
    }

    func readOutput(since: Int?, wait: Duration?, maxBytes: Int) async -> ReadResult {
        let since = since ?? 0
        // Always let queued PTY output in first: a caller polling in a loop
        // must not starve the main queue that delivers it.
        await Task.yield()
        if let wait {
            let deadline = ContinuousClock.now + wait
            let wasBusy = isBusy
            while ContinuousClock.now < deadline {
                refreshCompletion()
                if output.cursor > since || (wasBusy && !isBusy) || session?.state.isLive != true || isDetached { break }
                try? await Task.sleep(for: Self.poll)
            }
        }
        refreshCompletion()
        let text = output.text(since: since, maxBytes: maxBytes)
        return ReadResult(output: text.text, cursor: text.cursor, busy: isBusy, truncated: text.truncated)
    }

    func screen() -> ScreenResult {
        guard let session else { return ScreenResult(rows: [], cursorRow: 0, cursorCol: 0, alternateScreen: false) }
        let screen = session.screenText()
        let rows = screen.rows.map { row in
            output.masks.reduce(row) { $0.replacingOccurrences(of: $1, with: AgentOutput.maskReplacement) }
        }
        return ScreenResult(rows: rows, cursorRow: screen.cursorRow, cursorCol: screen.cursorCol,
                            alternateScreen: output.isAlternateScreen)
    }

    func send(keys: [AgentKey]) async throws {
        try ensureUsable()
        await waitForHumanPause()
        try ensureUsable()
        isActive = true
        for key in keys {
            session?.sendInput(key.bytes)
            if key.interrupts, isBusy, var run = pending {
                run.interrupted = true
                pending = run
            }
        }
        if !isBusy { isActive = false }
    }

    /// Types `password` plus Enter, but only when the last non-empty screen
    /// line is a password prompt (`SSHAskPass.isPasswordPrompt`), so it can
    /// never land in a shell line or an editor. The password joins the mask
    /// list first, so no later output or screen read can reveal it.
    func typePassword(_ password: String) async throws {
        try ensureUsable()
        await waitForHumanPause()
        try ensureUsable()
        // The prompt may still be on its way from the PTY to the screen.
        let deadline = ContinuousClock.now + .milliseconds(500)
        while !showsPasswordPrompt(), ContinuousClock.now < deadline { try? await Task.sleep(for: Self.poll) }
        guard showsPasswordPrompt() else { throw AgentDriverError.noPasswordPrompt }
        if !password.isEmpty, !output.masks.contains(password) { output.masks.append(password) }
        isActive = true
        session?.sendInput(Array((password + "\r").utf8))
        if !isBusy { isActive = false }
    }

    private func showsPasswordPrompt() -> Bool {
        guard let session else { return false }
        let last = session.screenText().rows.last { !$0.trimmingCharacters(in: .whitespaces).isEmpty }
        return last.map(SSHAskPass.isPasswordPrompt) ?? false
    }

    /// Ends this driver's control; the session stays open.
    func detach() {
        isDetached = true
        isBusy = false
        isActive = false
        pending = nil
    }

    // MARK: - Internals

    private func ensureUsable() throws {
        guard !isPaused else { throw AgentDriverError.paused }
        guard let session, session.state.isLive else { throw AgentDriverError.notConnected }
    }

    /// Agent input waits while the user typed in the last 2 s, at most 5 s.
    private func waitForHumanPause() async {
        let deadline = ContinuousClock.now + .seconds(5)
        while ContinuousClock.now < deadline,
              let typed = session?.lastUserInputAt, Date().timeIntervalSince(typed) < 2 {
            try? await Task.sleep(for: .milliseconds(100))
        }
    }

    /// Quiet-mode completion and interrupted runs: no marker will arrive, so
    /// finish once output is quiet and the last line looks like a prompt.
    private func refreshCompletion() {
        guard var run = pending, !run.finished else { return }
        let quietFor = Date().timeIntervalSince(lastOutputAt)
        let needed: TimeInterval? = run.interrupted ? 0.5 : (mode == .quiet ? 1.5 : nil)
        guard let needed, quietFor >= needed, output.cursor > run.startCursor,
              PromptHeuristics.looksLikeShellPrompt(lastLine()) else { return }
        run.finished = true
        run.endCursor = output.cursor
        pending = run
        isBusy = false
        isActive = false
    }

    private func lastLine() -> String {
        let text = output.text(since: max(0, output.cursor - 512)).text
        return text.split(separator: "\n", omittingEmptySubsequences: false).last.map(String.init) ?? ""
    }

    private func runResult() -> RunResult {
        guard let run = pending else {
            return RunResult(completed: false, exitCode: nil, output: "", cursor: output.cursor, truncated: false,
                             mode: mode.rawValue)
        }
        if mode == .osc133 {
            // The shell marked where the output starts (when it emits `C`);
            // otherwise skip the echoed command line. It ends at `D`, so no
            // trailing prompt to trim.
            var text = output.text(since: run.outputStart ?? run.startCursor, upTo: run.endCursor, maxBytes: Int.max).text
            if run.outputStart == nil, let firstNewline = text.firstIndex(of: "\n") {
                text = String(text[text.index(after: firstNewline)...])
            }
            let (cut, truncated) = AgentOutput.middleTruncated(text, maxBytes: 65_536)
            return RunResult(completed: run.finished && !run.interrupted, exitCode: run.exitCode, output: cut,
                             cursor: run.endCursor ?? output.cursor, truncated: truncated, mode: mode.rawValue)
        }
        var text = output.text(since: run.startCursor, upTo: run.endCursor, maxBytes: Int.max).text
        if let nonce = run.nonce {
            // Drop the echoed prompt and wrapper: everything through the line
            // that carries this command's nonce.
            let lines = text.split(separator: "\n", omittingEmptySubsequences: false)
            if let echo = lines.lastIndex(where: { $0.contains("7770;\(nonce);") }) {
                text = lines[(echo + 1)...].joined(separator: "\n")
            }
        } else if let firstNewline = text.firstIndex(of: "\n") {
            text = String(text[text.index(after: firstNewline)...]) // the echoed command line
        }
        if run.finished, run.nonce == nil, let lastNewline = text.lastIndex(of: "\n") {
            text = String(text[..<lastNewline]) // the prompt that followed
        }
        let (cut, truncated) = AgentOutput.middleTruncated(text, maxBytes: 65_536)
        return RunResult(completed: run.finished && !run.interrupted, exitCode: run.exitCode, output: cut,
                         cursor: run.endCursor ?? output.cursor, truncated: truncated, mode: mode.rawValue)
    }

    /// A remote shell that already emits `OSC 133` needs no wrapper; any other
    /// is probed once for Maine's own marker, else it falls back to quiet mode.
    private func chooseMode() async {
        if hasShellIntegration { mode = .osc133 } else { await probeMarker() }
    }

    /// Types a wrapped `true` and waits up to 2 s for its marker.
    private func probeMarker() async {
        let wrapper = CommandWrapper(command: "true")
        pending = PendingRun(startCursor: output.cursor, nonce: wrapper.nonce,
                             detector: CompletionDetector(nonce: wrapper.nonce), isProbe: true)
        session?.sendInput(Array(wrapper.typedText.utf8))
        let deadline = ContinuousClock.now + .seconds(2)
        while ContinuousClock.now < deadline, pending?.finished != true, session?.state.isLive == true {
            try? await Task.sleep(for: Self.poll)
        }
        mode = pending?.finished == true ? .marker : .quiet
        pending = nil
        // Let the prompt after the probe arrive before the next command.
        let settle = ContinuousClock.now + .milliseconds(300)
        while ContinuousClock.now < settle { try? await Task.sleep(for: Self.poll) }
    }
}
