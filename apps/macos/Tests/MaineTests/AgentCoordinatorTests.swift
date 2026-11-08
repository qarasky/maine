import AppKit
import Foundation
@testable import Maine
import Testing

/// The app-level agent switchboard: server lifecycle, pairing, access, lock.
@MainActor
@Suite(.serialized)
struct AgentCoordinatorTests {
    @MainActor
    private final class Fixture {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let socketDirectory = URL(fileURLWithPath: "/tmp/m-\(UUID().uuidString.prefix(8))", isDirectory: true)
        let defaultsSuite = "app.maine.tests.\(UUID())"
        let clientService = "app.maine.tests.\(UUID())"
        let vault: PasswordStore
        let store: HostStore
        let sessions: SessionManager
        let appState: AppState
        let coordinator: AgentCoordinator
        let prod = HostGroup(name: "Prod")
        let web: Maine.Host

        var socket: URL { socketDirectory.appendingPathComponent("agent.sock") }

        init() throws {
            vault = PasswordStore(fileURL: directory.appendingPathComponent("vault.maine"),
                                  keychainService: "app.maine.tests.\(UUID())")
            try vault.unlock(masterPassword: "coordinator master password")
            store = HostStore(vault: vault)
            web = Maine.Host(name: "Web", hostname: "web.example", groupID: prod.id)
            try store.saveHost(web, password: "secret")
            try vault.saveLayout(groups: [prod], hosts: [web])
            store.load()
            let shell: (Maine.Host) -> TerminalSession = { host in
                TerminalSession(host: host, executablePath: "/bin/sh", processArguments: [])
            }
            sessions = SessionManager(store: store, makeSession: shell, makeAgentSession: shell)
            appState = AppState(passwords: vault)
            appState.vaultDidUnlock()
            coordinator = AgentCoordinator(defaults: UserDefaults(suiteName: defaultsSuite)!,
                                           clientStore: AgentClientStore(service: clientService),
                                           socketURL: socketDirectory.appendingPathComponent("agent.sock"),
                                           passwords: vault)
            coordinator.configure(store: store, sessions: sessions, appState: appState)
        }

        func agentSession(_ client: AgentClient) -> TerminalSession {
            sessions.openAgentSession(web, client: client, name: nil)
        }

        func tearDown() {
            coordinator.setEnabled(false)
            for session in sessions.openSessions { session.sendInput(Array("exit\n".utf8)) }
            sessions.closeAll()
            for client in coordinator.clients { coordinator.revoke(client.id) }
            UserDefaults().removePersistentDomain(forName: defaultsSuite)
            try? vault.lock()
            try? FileManager.default.removeItem(at: directory)
            try? FileManager.default.removeItem(at: socketDirectory)
        }
    }

    @Test
    func enablingStartsAndDisablingStopsTheServer() throws {
        let fixture = try Fixture()
        defer { fixture.tearDown() }
        #expect(!fixture.coordinator.isEnabled)
        #expect(!FileManager.default.fileExists(atPath: fixture.socket.path))
        fixture.coordinator.setEnabled(true)
        #expect(fixture.coordinator.isEnabled)
        #expect(FileManager.default.fileExists(atPath: fixture.socket.path))
        #expect(UserDefaults(suiteName: fixture.defaultsSuite)?.bool(forKey: "agentAccessEnabled") == true)

        let client = try fixture.coordinator.addClient(name: "Agent").client
        let session = fixture.agentSession(client)
        fixture.coordinator.setEnabled(false)
        #expect(!FileManager.default.fileExists(atPath: fixture.socket.path))
        #expect(session.agent == nil)
        #expect(session.formerAgentName == "Agent")
    }

    @Test
    func lockPausesAndUnlockResumesOnlyWhatLockPaused() throws {
        let fixture = try Fixture()
        defer { fixture.tearDown() }
        let client = try fixture.coordinator.addClient(name: "Agent").client
        let first = fixture.agentSession(client), second = fixture.agentSession(client)
        second.agent?.isPaused = true
        fixture.coordinator.vaultDidLock()
        #expect(first.agent?.isPaused == true)
        #expect(second.agent?.isPaused == true)
        fixture.coordinator.vaultDidUnlock()
        #expect(first.agent?.isPaused == false)
        #expect(second.agent?.isPaused == true)
    }

    @Test
    func revokingAClientEndsItsControl() throws {
        let fixture = try Fixture()
        defer { fixture.tearDown() }
        let pairing = try fixture.coordinator.addClient(name: "Agent")
        let client = pairing.client, token = pairing.token
        let other = try fixture.coordinator.addClient(name: "Other").client
        let mine = fixture.agentSession(client), theirs = fixture.agentSession(other)
        fixture.coordinator.revoke(client.id)
        #expect(mine.agent == nil)
        #expect(fixture.sessions.openSessions.contains { $0 === mine })
        #expect(theirs.agent?.clientID == other.id)
        #expect(try AgentClientStore(service: fixture.clientService).client(forToken: token) == nil)
        #expect(fixture.coordinator.clients.map(\.name) == ["Other"])
    }

    @Test
    func setupSnippetsUseTheRunningExecutable() throws {
        let fixture = try Fixture()
        defer { fixture.tearDown() }
        let pairing = try fixture.coordinator.addClient(name: "Claude Code")
        #expect(pairing.claudeCodeCommand.hasPrefix("claude mcp add maine --env MAINE_AGENT_TOKEN=maine-agent-"))
        #expect(pairing.claudeCodeCommand.hasSuffix(" --mcp"))
        #expect(pairing.claudeCodeCommand.contains(pairing.token))
        let desktop = try JSONDecoder().decode(JSONValue.self, from: Data(pairing.desktopJSON.utf8))
        #expect(desktop["mcpServers"]?["maine"]?.array("args") == ["--mcp"])
        #expect(desktop["mcpServers"]?["maine"]?["env"]?.string("MAINE_AGENT_TOKEN") == pairing.token)
    }

    @Test
    func accessIsPrunedAndEnforcedImmediately() throws {
        let fixture = try Fixture()
        defer { fixture.tearDown() }
        fixture.coordinator.setAccess(AgentAccess(groups: [fixture.prod.id, UUID()]))
        #expect(fixture.coordinator.access() == AgentAccess(groups: [fixture.prod.id]))
        #expect(try fixture.vault.agentAccess() == AgentAccess(groups: [fixture.prod.id]))

        let client = try fixture.coordinator.addClient(name: "Agent").client
        let session = fixture.agentSession(client)
        fixture.coordinator.setAccess(AgentAccess())
        #expect(session.agent == nil)
    }

    @Test
    func stopAllEndsEveryAgent() throws {
        let fixture = try Fixture()
        defer { fixture.tearDown() }
        let client = try fixture.coordinator.addClient(name: "Agent").client
        _ = fixture.agentSession(client)
        _ = fixture.agentSession(client)
        #expect(fixture.coordinator.activeCount == 2)
        fixture.coordinator.stopAll()
        #expect(fixture.coordinator.activeCount == 0)
        #expect(fixture.sessions.openSessions.count == 2)
    }

    @Test
    func auditEntriesPersistShowWhileUnlockedAndClear() throws {
        let fixture = try Fixture()
        defer { fixture.tearDown() }
        let entry = AgentAuditEntry(client: "Claude Code", hostName: "Web", sessionID: UUID(), kind: .run,
                                    detail: "uptime", exitCode: 0)
        fixture.coordinator.recordAudit(entry)
        #expect(fixture.coordinator.auditEntries == [entry])
        #expect(try fixture.vault.agentAudit() == [entry])

        fixture.coordinator.vaultDidLock()
        #expect(fixture.coordinator.auditEntries.isEmpty, "a locked Maine shows nothing")
        fixture.coordinator.vaultDidUnlock()
        #expect(fixture.coordinator.auditEntries == [entry])

        fixture.coordinator.clearAudit()
        #expect(fixture.coordinator.auditEntries.isEmpty)
        #expect(try fixture.vault.agentAudit().isEmpty)
    }

    // MARK: - Hand to agent

    /// A session the user opened, waited until its shell shows a prompt.
    private func humanSessionAtPrompt(_ fixture: Fixture) async throws -> TerminalSession {
        fixture.sessions.open(fixture.web)
        let session = try #require(fixture.sessions.session(for: fixture.web.id))
        let deadline = ContinuousClock.now + .seconds(10)
        while ContinuousClock.now < deadline {
            let last = session.screenText().rows.last { !$0.trimmingCharacters(in: .whitespaces).isEmpty } ?? ""
            if PromptHeuristics.looksLikeShellPrompt(last) { break }
            try? await Task.sleep(for: .milliseconds(50))
        }
        return session
    }

    @Test
    func aHandedSessionIsDrivenByTheAgentButStaysTheUsers() async throws {
        let fixture = try Fixture()
        defer { fixture.tearDown() }
        fixture.coordinator.setEnabled(true)
        fixture.coordinator.setAccess(AgentAccess(groups: [fixture.prod.id]))
        let client = try fixture.coordinator.addClient(name: "Agent").client
        let session = try await humanSessionAtPrompt(fixture)

        try fixture.coordinator.handToAgent(sessionID: session.id, clientID: client.id)
        let driver = try #require(session.agent)
        #expect(driver.clientID == client.id)
        #expect(!session.isEphemeral, "the user opened it; it is still checkpointed and restored")
        let result = try await driver.run("echo handed", timeout: .seconds(10))
        #expect(result.completed && result.output.contains("handed"))
        #expect(fixture.coordinator.auditEntries.map(\.kind) == [.handoff])

        fixture.sessions.endAgentControl(session.id)
        #expect(session.agent == nil)
        #expect(session.formerAgentName == nil, "no 'agent disconnected' leftover: it was never an agent session")
        #expect(!session.isEphemeral)
    }

    @Test
    func handOffRefusesWhatAgentsMayNotOrCannotDrive() async throws {
        let fixture = try Fixture()
        defer { fixture.tearDown() }
        let client = try fixture.coordinator.addClient(name: "Agent").client
        let session = try await humanSessionAtPrompt(fixture)

        // Master switch off, then enabled but the host not allowed.
        #expect(throws: AgentHandOffError.notAllowed) { try fixture.coordinator.handToAgent(sessionID: session.id, clientID: client.id) }
        fixture.coordinator.setEnabled(true)
        #expect(throws: AgentHandOffError.notAllowed) { try fixture.coordinator.handToAgent(sessionID: session.id, clientID: client.id) }
        fixture.coordinator.setAccess(AgentAccess(groups: [fixture.prod.id]))
        #expect(throws: AgentHandOffError.notFound) { try fixture.coordinator.handToAgent(sessionID: session.id, clientID: UUID()) }
        #expect(throws: AgentHandOffError.notFound) { try fixture.coordinator.handToAgent(sessionID: UUID(), clientID: client.id) }

        // Something is running: the shell is not at a prompt.
        session.sendInput(Array("sleep 30\n".utf8))
        try? await Task.sleep(for: .milliseconds(500))
        #expect(throws: AgentHandOffError.notAtPrompt) { try fixture.coordinator.handToAgent(sessionID: session.id, clientID: client.id) }
        #expect(session.agent == nil)
        session.sendInput([0x03])
        try? await Task.sleep(for: .milliseconds(500))

        try fixture.coordinator.handToAgent(sessionID: session.id, clientID: client.id)
        #expect(throws: AgentHandOffError.alreadyDriven) { try fixture.coordinator.handToAgent(sessionID: session.id, clientID: client.id) }
        #expect(fixture.coordinator.auditEntries.count == 1)
    }

    @Test
    func clipboardWorksInHumanSessionsAndIsOffForAgents() throws {
        let fixture = try Fixture()
        defer { fixture.tearDown() }
        let client = try fixture.coordinator.addClient(name: "Agent").client
        fixture.sessions.open(fixture.web)
        let session = try #require(fixture.sessions.session(for: fixture.web.id))
        let view = try #require(session.terminalView as? CapturingTerminalView)
        let board = NSPasteboard(name: NSPasteboard.Name("app.maine.tests.\(UUID())"))
        view.pasteboard = board
        defer { board.releaseGlobally() }

        view.clipboardCopy(source: view, content: Data("from remote".utf8))
        #expect(board.string(forType: .string) == "from remote")
        #expect(view.clipboardRead(source: view) == Data("from remote".utf8))

        let agentSession = fixture.agentSession(client)
        let agentView = try #require(agentSession.terminalView as? CapturingTerminalView)
        agentView.pasteboard = board
        agentView.clipboardCopy(source: agentView, content: Data("planted".utf8))
        #expect(board.string(forType: .string) == "from remote")
        #expect(agentView.clipboardRead(source: agentView) == nil)
    }
}
