import Foundation
@testable import Maine
import Testing

/// The MCP tools end to end, minus the socket: a vault-backed host list and
/// real `/bin/sh` sessions standing in for ssh. No window.
@MainActor
@Suite(.serialized)
struct AgentToolsTests {
    @MainActor
    private final class Fixture {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let vault: PasswordStore
        let store: HostStore
        let sessions: SessionManager
        let handler: AgentToolHandler
        var access = AgentAccess()
        var unlocked = true
        let prod = HostGroup(name: "Prod"), lab = HostGroup(name: "Lab")
        let web: Maine.Host, db: Maine.Host, loose: Maine.Host
        let alice = AgentClient(id: UUID(), name: "Alice agent", createdAt: Date())
        let bob = AgentClient(id: UUID(), name: "Bob agent", createdAt: Date())

        let log: DebugLog

        init(executable: String = "/bin/sh") throws {
            log = DebugLog(fileURL: directory.appendingPathComponent("debug.jsonl"), enabled: true)
            vault = PasswordStore(fileURL: directory.appendingPathComponent("vault.maine"),
                                  keychainService: "app.maine.tests.\(UUID())")
            try vault.unlock(masterPassword: "agent tools master password")
            store = HostStore(vault: vault)
            web = Maine.Host(name: "Web", hostname: "web.example", username: "deploy", groupID: prod.id)
            db = Maine.Host(name: "DB", hostname: "db.example", groupID: lab.id)
            loose = Maine.Host(name: "Loose", hostname: "loose.example")
            for host in [web, db, loose] { try store.saveHost(host, password: "secret-\(host.name)") }
            try vault.saveLayout(groups: [prod, lab], hosts: [loose, web, db])
            store.load()
            let shell: (Maine.Host) -> TerminalSession = { host in
                TerminalSession(host: host, executablePath: executable, processArguments: [], capturesOutput: true)
            }
            sessions = SessionManager(store: store, makeSession: shell, makeAgentSession: shell)
            var accessBox: () -> AgentAccess = { AgentAccess() }
            var unlockedBox: () -> Bool = { true }
            handler = AgentToolHandler(store: store, sessions: sessions,
                                       access: { accessBox() }, isUnlocked: { unlockedBox() },
                                       hasSavedPassword: { [vault] id in (try? vault.password(for: id)) != nil },
                                       log: log)
            accessBox = { [unowned self] in self.access }
            unlockedBox = { [unowned self] in self.unlocked }
            access = AgentAccess(groups: [prod.id])
        }

        func call(_ tool: String, _ arguments: JSONValue = [:], as client: AgentClient? = nil) async -> AgentReply {
            await handler.handle(AgentCall(id: 1, tool: tool, arguments: arguments), client: client ?? alice)
        }

        func open(_ host: Maine.Host, as client: AgentClient? = nil) async throws -> String {
            let reply = await call("open_session", ["host_id": .string(host.id.uuidString)], as: client)
            return try #require(reply.result?.string("session_id"), "\(reply.error ?? "")")
        }

        func tearDown() {
            for session in sessions.openSessions { session.sendInput(Array("exit\n".utf8)) }
            sessions.closeAll()
            try? vault.lock()
            try? FileManager.default.removeItem(at: directory)
        }
    }

    @Test
    func listHostsShowsOnlyAllowedHosts() async throws {
        let fixture = try Fixture()
        defer { fixture.tearDown() }
        let reply = await fixture.call("list_hosts")
        let hosts = try #require(reply.result?.array("hosts"))
        #expect(hosts.compactMap { $0.string("name") } == ["Web"])
        let web = try #require(hosts.first)
        #expect(web.string("group") == "Prod")
        #expect(web.string("hostname") == "web.example")
        #expect(web.string("username") == "deploy")
        #expect(web.int("port") == 22)
        #expect(web["has_saved_password"] == true)
        if case .object(let keys) = web {
            #expect(Set(keys.keys) == ["id", "name", "group", "hostname", "port", "username", "has_saved_password"])
        }
        let json = reply.result?.jsonText ?? ""
        let leaksPassword = json.contains("secret")
        #expect(!leaksPassword, "\(json)")

        fixture.access = AgentAccess(hosts: [fixture.loose.id])
        let loose = await fixture.call("list_hosts")
        #expect(loose.result?.array("hosts")?.first?["group"] == .null)
    }

    @Test
    func disallowedAndMissingIdsLookTheSame() async throws {
        let fixture = try Fixture()
        defer { fixture.tearDown() }
        #expect(await fixture.call("open_session", ["host_id": .string(fixture.db.id.uuidString)]).error == "not found")
        #expect(await fixture.call("open_session", ["host_id": .string(UUID().uuidString)]).error == "not found")
        #expect(await fixture.call("run", ["session_id": .string(UUID().uuidString), "command": "ls"]).error == "not found")
    }

    @Test
    func lockedVaultRefusesEveryTool() async throws {
        let fixture = try Fixture()
        defer { fixture.tearDown() }
        fixture.unlocked = false
        for tool in AgentToolCatalog.tools.map(\.name) {
            #expect(await fixture.call(tool).error == "Maine is locked. Ask the user to unlock it.", "\(tool)")
        }
    }

    @Test
    func sessionsAreScopedToTheirClient() async throws {
        let fixture = try Fixture()
        defer { fixture.tearDown() }
        let id = try await fixture.open(fixture.web)
        #expect(await fixture.call("list_sessions").result?.array("sessions")?.compactMap { $0.string("session_id") } == [id])
        #expect(await fixture.call("list_sessions", as: fixture.bob).result?.array("sessions") == [])
        #expect(await fixture.call("run", ["session_id": .string(id), "command": "true"], as: fixture.bob).error == "not found")
    }

    @Test
    func runOverTheToolSurface() async throws {
        let fixture = try Fixture()
        defer { fixture.tearDown() }
        let id = try await fixture.open(fixture.web)
        let reply = await fixture.call("run", ["session_id": .string(id), "command": "echo hi; false"])
        let result = try #require(reply.result, "\(reply.error ?? "")")
        #expect(result["completed"] == true)
        #expect(result.int("exit_code") == 1)
        #expect(result.string("output")?.trimmingCharacters(in: .whitespacesAndNewlines) == "hi")
        #expect(result.string("mode") == "marker")

        let screen = await fixture.call("read_screen", ["session_id": .string(id)])
        #expect(screen.result?.array("rows")?.contains { $0.stringValue?.contains("hi") == true } == true)
        let read = await fixture.call("read_output", ["session_id": .string(id)])
        #expect(read.result?.string("output")?.contains("hi") == true)
        #expect(await fixture.call("send_keys", ["session_id": .string(id), "keys": ["text:echo typed", "enter"]]).result?.int("sent") == 2)
        #expect(await fixture.call("send_keys", ["session_id": .string(id), "keys": ["bogus"]]).error == "unknown key: bogus")
    }

    @Test
    func runOnADeadSessionFailsFast() async throws {
        let fixture = try Fixture()
        defer { fixture.tearDown() }
        let id = try await fixture.open(fixture.web)
        _ = await fixture.call("send_keys", ["session_id": .string(id), "keys": ["text:exit", "enter"]])
        let deadline = ContinuousClock.now + .seconds(5)
        while fixture.sessions.openSessions.first?.state.isLive == true, ContinuousClock.now < deadline {
            try? await Task.sleep(for: .milliseconds(20))
        }
        let started = ContinuousClock.now
        let reply = await fixture.call("run", ["session_id": .string(id), "command": "true"])
        #expect(reply.error == "session is not connected")
        #expect(ContinuousClock.now - started < .seconds(1))
    }

    @Test
    func hugeOutputIsTruncated() async throws {
        let fixture = try Fixture()
        defer { fixture.tearDown() }
        let id = try await fixture.open(fixture.web)
        let reply = await fixture.call("run", ["session_id": .string(id), "command": "yes | head -n 200000",
                                               "timeout_seconds": 60])
        let result = try #require(reply.result, "\(reply.error ?? "")")
        #expect(result["truncated"] == true)
        #expect((result.string("output") ?? "").utf8.count <= 65_536)
        #expect(result.int("exit_code") == 0)
    }

    @Test
    func concurrentRunsFollowTheBusyRule() async throws {
        let fixture = try Fixture()
        defer { fixture.tearDown() }
        let first = try await fixture.open(fixture.web)
        let second = try await fixture.open(fixture.web)
        async let a = fixture.call("run", ["session_id": .string(first), "command": "sleep 1; echo A"])
        try? await Task.sleep(for: .milliseconds(100))
        let busy = await fixture.call("run", ["session_id": .string(first), "command": "true"])
        #expect(busy.error == "a command is still running; use read_output, or send_keys ctrl-c")
        async let b = fixture.call("run", ["session_id": .string(second), "command": "sleep 1; echo B"])
        let started = ContinuousClock.now
        let (resultA, resultB) = await (a, b)
        #expect(resultA.result?.string("output")?.contains("A") == true)
        #expect(resultB.result?.string("output")?.contains("B") == true)
        #expect(ContinuousClock.now - started < .seconds(3))
    }

    @Test
    func revokingAccessMidRunEndsTheCall() async throws {
        let fixture = try Fixture()
        defer { fixture.tearDown() }
        let id = try await fixture.open(fixture.web)
        async let run = fixture.call("run", ["session_id": .string(id), "command": "sleep 5", "timeout_seconds": 30])
        try? await Task.sleep(for: .milliseconds(300))
        fixture.access = AgentAccess()
        let started = ContinuousClock.now
        #expect(await run.error == "not found")
        #expect(ContinuousClock.now - started < .seconds(2))
        let session = try #require(fixture.sessions.openSessions.first)
        #expect(session.agent == nil)
        #expect(session.formerAgentName == "Alice agent")
    }

    @Test
    func closeSessionClosesIt() async throws {
        let fixture = try Fixture()
        defer { fixture.tearDown() }
        let id = try await fixture.open(fixture.web)
        _ = await fixture.call("send_keys", ["session_id": .string(id), "keys": ["text:exit", "enter"]])
        #expect(await fixture.call("close_session", ["session_id": .string(id)]).result?["closed"] == true)
        #expect(fixture.sessions.openSessions.isEmpty)
    }

    @Test
    func argumentLimitsAreClamped() {
        #expect(AgentToolHandler.runTimeout(nil) == .seconds(60))
        #expect(AgentToolHandler.runTimeout(900) == .seconds(600))
        #expect(AgentToolHandler.runTimeout(0) == .seconds(1))
        #expect(AgentToolHandler.readWait(120) == .seconds(60))
        #expect(AgentToolHandler.readWait(nil) == nil)
        #expect(AgentToolHandler.maxBytes(1_000_000) == 65_536)
        #expect(AgentToolHandler.maxBytes(nil) == 65_536)
    }

    // MARK: - Final review fixes

    @Test
    func pausedSessionsRefuseEveryCall() async throws {
        let fixture = try Fixture()
        defer { fixture.tearDown() }
        let id = try await fixture.open(fixture.web)
        fixture.sessions.openSessions.first?.agent?.isPaused = true
        for tool in ["read_output", "read_screen", "send_keys", "run", "close_session"] {
            let reply = await fixture.call(tool, ["session_id": .string(id), "command": "true", "keys": ["enter"]])
            #expect(reply.error == "paused by the user", "\(tool)")
        }
        #expect(fixture.sessions.openSessions.count == 1)
    }

    @Test
    func aFailedOpenLeavesNoAgentAttached() async throws {
        let fixture = try Fixture(executable: "/nonexistent/ssh")
        defer { fixture.tearDown() }
        let reply = await fixture.call("open_session", ["host_id": .string(fixture.web.id.uuidString)])
        #expect(reply.error?.contains("/nonexistent/ssh") == true)
        let session = try #require(fixture.sessions.openSessions.first)
        #expect(session.agent == nil)
        #expect(session.formerAgentName == "Alice agent")
    }

    @Test
    func theDebugLogNeverRecordsAgentSuppliedText() async throws {
        let fixture = try Fixture()
        defer { fixture.tearDown() }
        _ = await fixture.call("run --secret-flag", ["session_id": "prod-db-password-hunter2"])
        _ = await fixture.call("run", ["session_id": "rm -rf /home/alice", "command": "cat /etc/shadow"])
        let text = (try? String(contentsOf: fixture.log.fileURL, encoding: .utf8)) ?? ""
        #expect(text.contains("agent.tool"))
        for secret in ["secret-flag", "hunter2", "rm -rf", "shadow", "alice"] {
            let found = text.contains(secret)
            #expect(!found, "\(secret)")
        }
    }
}
