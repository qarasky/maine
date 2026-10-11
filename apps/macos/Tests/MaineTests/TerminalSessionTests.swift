import AppKit
import Maine
import SwiftTerm
import Testing

/// Covers the part of Maine that makes SSH work at all: a process running inside
/// a pseudo terminal, output reaching the terminal view, and the session
/// closing when the process exits.
@MainActor
@Suite
struct TerminalSessionTests {
    /// A flag the terminal's output callback can set from its own thread.
    private final class OutputFlag: @unchecked Sendable {
        private let lock = NSLock()
        private var marked = false

        var seen: Bool {
            lock.lock()
            defer { lock.unlock() }
            return marked
        }

        func mark() {
            lock.lock()
            defer { lock.unlock() }
            marked = true
        }
    }

    /// Waits for `condition`, up to five seconds, without blocking the actor.
    private func poll(_ condition: () -> Bool) async {
        let deadline = ContinuousClock.now + .seconds(5)
        while condition() == false, ContinuousClock.now < deadline {
            try? await Task.sleep(nanoseconds: 10_000_000)
        }
    }

    /// `echo` stands in for ssh: what is under test is the PTY plumbing and the
    /// session state machine, not the network connection.
    private func session(executable: String) -> TerminalSession {
        TerminalSession(
            host: Host(name: "Test", hostname: "localhost"),
            executablePath: executable
        )
    }

    @Test
    func startingASessionGoesThroughConnecting() {
        let session = session(executable: "/bin/echo")
        session.connect()

        #expect(session.state == .connecting || session.state == .connected)
    }

    @Test
    func missingExecutableIsReportedInsteadOfCrashing() {
        let session = session(executable: "/nonexistent/ssh")
        session.connect()

        guard case .failed(let message) = session.state else {
            Issue.record("Expected a failure message, got \(session.state)")
            return
        }
        #expect(message.contains("/nonexistent/ssh"))
    }

    @Test
    func layoutCallbackDoesNotPublishConnectionStateSynchronously() async {
        let session = session(executable: "/bin/echo")
        session.connect()
        session.sizeChanged(source: session.terminalView, newCols: 80, newRows: 24)
        #expect(session.state == .connecting)
        session.disconnect()
        await poll { !session.state.isLive }
        #expect(!session.state.isLive)
    }

    @Test
    func invalidHostIsRefused() {
        let session = TerminalSession(
            host: Host(name: "", hostname: ""),
            executablePath: "/bin/echo"
        )
        session.connect()

        #expect(session.state.isLive == false)
    }

    @Test(.timeLimit(.minutes(1)))
    func processOutputReachesTheTerminal() async {
        let session = session(executable: "/bin/echo")
        // Give the PTY a size the way SwiftUI does when it lays the view out.
        session.terminalView.frame = NSRect(x: 0, y: 0, width: 400, height: 240)

        let output = OutputFlag()
        session.terminalView.setProcessOutputHandler { output.mark() }
        session.connect()

        await poll { output.seen }

        #expect(output.seen)
        #expect(session.terminalView.terminalDimensions.cols > 0)
        #expect(session.terminalView.terminalDimensions.rows > 0)
    }

    @Test(.timeLimit(.minutes(1)))
    func sessionClosesWhenTheProcessExits() async {
        let session = session(executable: "/bin/echo")

        session.connect()
        await poll { session.state.isLive == false }

        #expect(session.state == .closed(exitCode: 0))
    }

    @Test(.timeLimit(.minutes(1)))
    func disconnectingEndsTheSession() async {
        let session = session(executable: "/bin/sleep")
        session.connect()

        #expect(session.state.isLive)
        session.disconnect()
        await poll { session.state.isLive == false }

        #expect(session.state.isLive == false)
    }
}
