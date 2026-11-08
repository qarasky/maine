import AppKit
@testable import Maine
import SwiftTerm
import Testing

/// Covers the part of Maine that makes SSH work at all: a process running inside
/// a pseudo terminal, output reaching the terminal view, and the session
/// closing when the process exits.
@MainActor
@Suite
struct TerminalSessionTests {
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
    func layoutCallbackDoesNotPublishConnectionStateSynchronously() async throws {
        let session = session(executable: "/bin/echo")
        session.connect()
        session.capturedSizeChange()
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
    func processOutputReachesTheTerminal() async throws {
        let session = TerminalSession(host: Host(name: "Test", hostname: "localhost"), executablePath: "/bin/echo",
                                      processArguments: ["hello-from-the-pty"])
        // Give the PTY a size the way SwiftUI does when it lays the view out.
        session.terminalView.frame = NSRect(x: 0, y: 0, width: 400, height: 240)
        session.connect()

        await poll { session.screenText().rows.contains { $0.contains("hello-from-the-pty") } }

        #expect(session.screenText().rows.contains { $0.contains("hello-from-the-pty") })
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
