import AppKit
import Foundation
@testable import Maine
import Testing

/// An agent driving a real PTY. `/bin/sh` stands in for ssh; no window.
@MainActor
@Suite(.serialized)
struct AgentDriverTests {
    private let client = AgentClient(id: UUID(), name: "Test agent", createdAt: Date())

    private func shellSession(for host: Maine.Host = Maine.Host(name: "Shell", hostname: "localhost")) -> TerminalSession {
        TerminalSession(host: host, executablePath: "/bin/sh", processArguments: [], capturesOutput: true)
    }

    /// An interactive sh ignores SIGTERM, so tests end it the way a remote
    /// shell ends: `exit`, then a disconnect for good measure.
    private func endShell(_ session: TerminalSession) {
        session.sendInput(Array("exit\n".utf8))
        session.disconnect()
    }

    /// A connected shell with a driver attached, past its login prompt.
    private func drivenShell() async -> (TerminalSession, AgentDriver) {
        let session = shellSession()
        let driver = AgentDriver(session: session, client: client)
        session.agent = driver
        session.connect()
        _ = await driver.settleAfterLogin(timeout: .seconds(10))
        return (session, driver)
    }

    @Test
    func runReturnsOutputAndExitCode() async throws {
        let (session, driver) = await drivenShell()
        defer { endShell(session) }
        let result = try await driver.run("echo hi; false", timeout: .seconds(10))
        #expect(result.completed)
        #expect(result.exitCode == 1)
        #expect(result.mode == "marker")
        #expect(result.output.trimmingCharacters(in: .whitespacesAndNewlines) == "hi")
        #expect(!driver.isBusy)

        let multiline = try await driver.run("echo one # a comment\necho two", timeout: .seconds(10))
        #expect(multiline.exitCode == 0)
        #expect(multiline.output.trimmingCharacters(in: .whitespacesAndNewlines) == "one\ntwo")
    }

    @Test
    func timeoutsReturnPartialOutputAndLeaveTheCommandRunning() async throws {
        let (session, driver) = await drivenShell()
        defer { endShell(session) }
        let result = try await driver.run("echo start; sleep 3; echo end", timeout: .seconds(1))
        #expect(!result.completed)
        #expect(result.exitCode == nil)
        #expect(result.output.contains("start"))
        #expect(driver.isBusy)
        var read = await driver.readOutput(since: result.cursor, wait: .seconds(6), maxBytes: 65_536)
        let deadline = ContinuousClock.now + .seconds(6)
        while read.busy, ContinuousClock.now < deadline {
            read = await driver.readOutput(since: result.cursor, wait: .seconds(1), maxBytes: 65_536)
        }
        #expect(await driver.readOutput(since: result.cursor, wait: nil, maxBytes: 65_536).output.contains("end"))
        #expect(!driver.isBusy)
    }

    @Test
    func aBusySessionRefusesAnotherRun() async throws {
        let (session, driver) = await drivenShell()
        defer { endShell(session) }
        _ = try await driver.run("sleep 2", timeout: .milliseconds(200))
        await #expect(throws: AgentDriverError.busy) { _ = try await driver.run("echo no", timeout: .seconds(1)) }
    }

    @Test
    func pausedDriversRefuseInput() async throws {
        let (session, driver) = await drivenShell()
        defer { endShell(session) }
        driver.isPaused = true
        await #expect(throws: AgentDriverError.paused) { _ = try await driver.run("echo no", timeout: .seconds(1)) }
        await #expect(throws: AgentDriverError.paused) { try await driver.send(keys: [.named("enter")]) }
    }

    @Test
    func aDeadSessionFailsFast() async throws {
        let (session, driver) = await drivenShell()
        // The remote side ends the session (an interactive sh ignores SIGTERM).
        try await driver.send(keys: [.text("exit"), .named("enter")])
        let deadline = ContinuousClock.now + .seconds(5)
        while session.state.isLive, ContinuousClock.now < deadline { try? await Task.sleep(for: .milliseconds(20)) }
        let started = ContinuousClock.now
        await #expect(throws: AgentDriverError.notConnected) { _ = try await driver.run("echo no", timeout: .seconds(10)) }
        #expect(ContinuousClock.now - started < .seconds(1))
    }

    @Test
    func ctrlCInterruptsARunningCommand() async throws {
        let (session, driver) = await drivenShell()
        defer { endShell(session) }
        _ = try await driver.run("sleep 30", timeout: .seconds(1))
        #expect(driver.isBusy)
        try await driver.send(keys: [.named("ctrl-c")])
        let deadline = ContinuousClock.now + .seconds(5)
        while driver.isBusy, ContinuousClock.now < deadline { _ = await driver.readOutput(since: nil, wait: .milliseconds(200), maxBytes: 1) }
        #expect(!driver.isBusy)
        let after = try await driver.run("echo alive", timeout: .seconds(10))
        #expect(after.output.contains("alive"))
    }

    @Test
    func keysMapToBytes() {
        #expect(AgentKey("ctrl-c") == .named("ctrl-c"))
        #expect(AgentKey("ctrl-c")?.bytes == [0x03])
        #expect(AgentKey("enter")?.bytes == [0x0D])
        #expect(AgentKey("up")?.bytes == Array("\u{1B}[A".utf8))
        #expect(AgentKey("text:ls -la")?.bytes == Array("ls -la".utf8))
        #expect(AgentKey("bogus") == nil)
        #expect(AgentKey("ctrl-1") == nil)
    }

    @Test
    func screenReflectsTheVisibleTerminal() async throws {
        let (session, driver) = await drivenShell()
        defer { endShell(session) }
        _ = try await driver.run("printf 'row-a\\n'", timeout: .seconds(10))
        let screen = driver.screen()
        #expect(screen.rows.contains { $0.contains("row-a") })
        #expect(!screen.alternateScreen)
        #expect(screen.cursorRow >= 0 && screen.cursorCol >= 0)
    }

    @Test
    func agentSessionsAreNeverSavedOrRestored() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let vault = PasswordStore(fileURL: directory.appendingPathComponent("vault.maine"),
                                  keychainService: "app.maine.tests.\(UUID())")
        defer { try? vault.lock(); try? FileManager.default.removeItem(at: directory) }
        try vault.unlock(masterPassword: "agent sessions master password")
        let store = HostStore(vault: vault)
        let host = Maine.Host(name: "Shell", hostname: "localhost")
        try store.saveHost(host, password: "secret")
        let manager = SessionManager(store: store, makeSession: { self.shellSession(for: $0) },
                                     makeAgentSession: { self.shellSession(for: $0) })
        let session = manager.openAgentSession(host, client: client, name: "Agent work")
        defer { endShell(session); manager.closeAll() }
        #expect(session.agent?.clientID == client.id)
        #expect(manager.agentSessions.map(\.id) == [session.id])
        #expect(manager.selectedSessionID != session.id)
        try manager.checkpointSessions()
        #expect(try store.savedSessions().isEmpty)

        manager.endAgentControl(session.id)
        #expect(session.agent == nil)
        #expect(session.formerAgentName == "Test agent")
        try manager.checkpointSessions()
        #expect(try store.savedSessions().isEmpty)

        manager.keepSession(session.id)
        #expect(session.formerAgentName == nil)
        try manager.checkpointSessions()
        #expect(try store.savedSessions().count == 1)
    }

    // MARK: - Final review fixes

    private func capture<T>(_ body: () async throws -> T) async -> Result<T, Error> {
        do { return .success(try await body()) } catch { return .failure(error) }
    }

    @Test
    func agentSessionsCannotReadOrWriteTheUsersClipboard() throws {
        let session = shellSession()
        let view = try #require(session.terminalView as? CapturingTerminalView)
        let pasteboard = NSPasteboard(name: NSPasteboard.Name("app.maine.tests.\(UUID())"))
        defer { pasteboard.releaseGlobally() }
        pasteboard.clearContents()
        pasteboard.setString("IP: 10.0.0.4\npassword: hunter2", forType: .string)
        view.pasteboard = pasteboard
        #expect(view.clipboardRead(source: view) == nil)
        let before = pasteboard.changeCount
        view.clipboardCopy(source: view, content: Data("planted".utf8))
        #expect(pasteboard.changeCount == before)
        #expect(pasteboard.string(forType: .string) == "IP: 10.0.0.4\npassword: hunter2")
    }

    @Test
    func parallelRunsCannotBothPassTheBusyCheck() async throws {
        let (session, driver) = await drivenShell()
        defer { endShell(session) }
        session.capturedUserInput() // the user just typed: both runs wait first
        async let first = capture { try await driver.run("echo A", timeout: .seconds(10)) }
        async let second = capture { try await driver.run("echo B", timeout: .seconds(10)) }
        let results = await [first, second]
        let busy = results.filter { if case .failure(let error) = $0 { return (error as? AgentDriverError) == .busy }; return false }
        #expect(busy.count == 1)
    }

    @Test
    func stoppingAnAgentEndsItsRunningCall() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let vault = PasswordStore(fileURL: directory.appendingPathComponent("vault.maine"),
                                  keychainService: "app.maine.tests.\(UUID())")
        defer { try? vault.lock(); try? FileManager.default.removeItem(at: directory) }
        try vault.unlock(masterPassword: "stop test master password")
        let store = HostStore(vault: vault)
        let host = Maine.Host(name: "Shell", hostname: "localhost")
        try store.saveHost(host, password: "secret")
        let manager = SessionManager(store: store, makeSession: { self.shellSession(for: $0) },
                                     makeAgentSession: { self.shellSession(for: $0) })
        let session = manager.openAgentSession(host, client: client, name: nil)
        defer { endShell(session); manager.closeAll() }
        let driver = try #require(session.agent)
        _ = await driver.settleAfterLogin(timeout: .seconds(10))
        async let run = capture { try await driver.run("sleep 5", timeout: .seconds(30)) }
        try? await Task.sleep(for: .milliseconds(300))
        let stopped = ContinuousClock.now
        manager.endAgentControl(session.id)
        let result = await run
        #expect(ContinuousClock.now - stopped < .seconds(1))
        if case .failure(let error) = result { #expect((error as? AgentDriverError) == .stopped) }
        else { Issue.record("run should have been stopped") }
    }

    @Test
    func agentSessionsReportConnectedOnceOutputArrives() async throws {
        let (session, _) = await drivenShell()
        defer { endShell(session) }
        #expect(session.state == .connected)
    }

    @Test
    func hostLevelActionsIgnoreAgentSessions() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let vault = PasswordStore(fileURL: directory.appendingPathComponent("vault.maine"),
                                  keychainService: "app.maine.tests.\(UUID())")
        defer { try? vault.lock(); try? FileManager.default.removeItem(at: directory) }
        try vault.unlock(masterPassword: "host actions master password")
        let store = HostStore(vault: vault)
        let host = Maine.Host(name: "Shell", hostname: "localhost")
        try store.saveHost(host, password: "secret")
        let manager = SessionManager(store: store, makeSession: { self.shellSession(for: $0) },
                                     makeAgentSession: { self.shellSession(for: $0) })
        let agentSession = manager.openAgentSession(host, client: client, name: nil)
        defer { for open in manager.openSessions { endShell(open) }; manager.closeAll() }
        #expect(manager.session(for: host.id) == nil)
        manager.open(host)
        let human = try #require(manager.session(for: host.id))
        #expect(human !== agentSession)
        #expect(!human.isEphemeral)
        manager.close(host.id)
        #expect(manager.openSessions.contains { $0 === agentSession })
    }
}
