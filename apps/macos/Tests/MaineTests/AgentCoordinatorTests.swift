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
                TerminalSession(host: host, executablePath: "/bin/sh", processArguments: [], capturesOutput: true)
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
}
