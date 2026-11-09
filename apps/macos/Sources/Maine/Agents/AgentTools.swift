import Foundation

/// The tools behind the MCP surface (`docs/mcp.md` § Tools). Runs in the app
/// on the main actor, next to the sessions it drives. Every id an agent
/// passes is checked against device-local access and the calling client; a
/// host or session it may not use answers exactly like one that doesn't exist.
@MainActor
final class AgentToolHandler: AgentCallHandling {
    static let locked = "Maine is locked. Ask the user to unlock it."
    static let notFound = "not found"

    private let store: HostStore
    private let sessions: SessionManager
    private let access: @MainActor () -> AgentAccess
    private let isUnlocked: @MainActor () -> Bool
    private let hasSavedPassword: @MainActor (Host.ID) -> Bool
    private let savedPassword: @MainActor (Host.ID) -> String?
    private let audit: @MainActor (AgentAuditEntry) -> Void
    private let log: DebugLog

    init(store: HostStore, sessions: SessionManager,
         access: @escaping @MainActor () -> AgentAccess,
         isUnlocked: @escaping @MainActor () -> Bool,
         hasSavedPassword: @escaping @MainActor (Host.ID) -> Bool = { (try? PasswordStore.shared.password(for: $0)) != nil },
         savedPassword: @escaping @MainActor (Host.ID) -> String? = { try? PasswordStore.shared.password(for: $0) },
         audit: @escaping @MainActor (AgentAuditEntry) -> Void = { _ in },
         log: DebugLog = .shared) {
        self.savedPassword = savedPassword
        self.audit = audit
        self.log = log
        self.store = store
        self.sessions = sessions
        self.access = access
        self.isUnlocked = isUnlocked
        self.hasSavedPassword = hasSavedPassword
    }

    // MARK: - Limits

    static func runTimeout(_ seconds: Int?) -> Duration { .seconds(min(max(seconds ?? 60, 1), 600)) }
    static func readWait(_ seconds: Int?) -> Duration? { seconds.map { .seconds(min(max($0, 0), 60)) } }
    static func maxBytes(_ value: Int?) -> Int { min(max(value ?? 65_536, 1), 65_536) }

    // MARK: - Dispatch

    func handle(_ call: AgentCall, client: AgentClient) async -> AgentReply {
        let started = ContinuousClock.now
        let reply: AgentReply
        do {
            reply = AgentReply(id: call.id, result: try await perform(call, client: client), error: nil)
        } catch let error as AgentDriverError {
            reply = AgentReply(id: call.id, result: nil, error: error.errorDescription)
        } catch let error as ToolError {
            reply = AgentReply(id: call.id, result: nil, error: error.message)
        } catch {
            reply = AgentReply(id: call.id, result: nil, error: error.localizedDescription)
        }
        // Metadata only: never arguments, output, host names or tokens.
        log.record("agent.tool", fields: [
            "tool": AgentToolCatalog.contains(call.tool) ? call.tool : "unknown",
            "client": client.id.uuidString,
            "session": call.arguments.string("session_id").flatMap(UUID.init(uuidString:))?.uuidString ?? "",
            "outcome": reply.error == nil ? "ok" : "error",
            "ms": String((ContinuousClock.now - started).components.seconds * 1000
                + (ContinuousClock.now - started).components.attoseconds / 1_000_000_000_000_000),
            "bytes": String(reply.result?.jsonText.utf8.count ?? 0),
        ])
        return reply
    }

    private struct ToolError: Error { let message: String }

    private func perform(_ call: AgentCall, client: AgentClient) async throws -> JSONValue {
        guard isUnlocked() else { throw ToolError(message: Self.locked) }
        let arguments = call.arguments
        switch call.tool {
        case "list_hosts":
            let layout = store.layout
            let hosts = access().allowedHosts(in: layout).map { host -> JSONValue in
                ["id": .string(host.id.uuidString), "name": .string(host.displayName),
                 "group": layout.group(id: host.groupID).map { .string($0.name) } ?? .null,
                 "hostname": .string(host.hostname), "port": .number(Double(host.port)),
                 "username": .string(host.username), "has_saved_password": .bool(hasSavedPassword(host.id))]
            }
            return ["hosts": .array(hosts)]

        case "list_sessions":
            let mine = sessions.openSessions.filter { $0.agent?.clientID == client.id && allowed($0) }
            return ["sessions": .array(mine.map { session in
                ["session_id": .string(session.id.uuidString), "host_id": .string(session.host.id.uuidString),
                 "name": .string(sessions.label(for: session)), "state": .string(session.agent?.isReady == false && session.state.isLive
                    ? TerminalSession.State.connecting.label : session.state.label),
                 "busy": .bool(session.agent?.isBusy ?? false)]
            })]

        case "open_session":
            guard let id = arguments.string("host_id").flatMap(UUID.init(uuidString:)),
                  let host = store.host(id: id), access().allows(host) else { throw ToolError(message: Self.notFound) }
            let session = sessions.openAgentSession(host, client: client, name: arguments.string("name"))
            guard let driver = session.agent else { throw ToolError(message: Self.notFound) }
            let login = await driver.settleAfterLogin(timeout: .seconds(30))
            if let notice = session.state.notice, !session.state.isLive {
                // Nothing for the agent to drive; the user sees the failure.
                sessions.endAgentControl(session.id)
                throw ToolError(message: notice)
            }
            record(.open, session, client)
            let ready = driver.isReady
            var result: [String: JSONValue] = [
                "session_id": .string(session.id.uuidString),
                "state": .string(ready ? session.state.label : TerminalSession.State.connecting.label),
                "ready": .bool(ready), "output": .string(login.output)]
            if !ready {
                result["hint"] = .string("The remote shell is not ready, so run is refused. Use read_screen; if ssh "
                    + "waits at a password or host-key prompt, answer it with type_password or send_keys.")
            }
            return .object(result)

        case "run":
            let (session, driver) = try resolve(arguments, client: client)
            guard let command = arguments.string("command") else { throw ToolError(message: "command is required") }
            let result: AgentDriver.RunResult
            do {
                result = try await driver.run(command, timeout: Self.runTimeout(arguments.int("timeout_seconds")),
                                              shouldStop: { [weak self] in self?.allowed(session) != true })
            } catch AgentDriverError.stopped {
                sessions.endAgentControl(session.id)
                throw ToolError(message: Self.notFound)
            }
            guard allowed(session) else {
                sessions.endAgentControl(session.id)
                throw ToolError(message: Self.notFound)
            }
            record(.run, session, client, detail: command, exitCode: result.exitCode)
            return ["completed": .bool(result.completed),
                    "exit_code": result.exitCode.map { .number(Double($0)) } ?? .null,
                    "output": .string(result.output), "cursor": .number(Double(result.cursor)),
                    "truncated": .bool(result.truncated), "mode": .string(result.mode)]

        case "read_output":
            let (_, driver) = try resolve(arguments, client: client)
            let read = await driver.readOutput(since: arguments.int("since"), wait: Self.readWait(arguments.int("wait_seconds")),
                                               maxBytes: Self.maxBytes(arguments.int("max_bytes")))
            return ["output": .string(read.output), "cursor": .number(Double(read.cursor)),
                    "busy": .bool(read.busy), "truncated": .bool(read.truncated)]

        case "read_screen":
            let (_, driver) = try resolve(arguments, client: client)
            let screen = await driver.readScreen(wait: Self.readWait(arguments.int("wait_seconds")))
            return ["rows": .array(screen.rows.map(JSONValue.string)), "cursor_row": .number(Double(screen.cursorRow)),
                    "cursor_col": .number(Double(screen.cursorCol)), "alternate_screen": .bool(screen.alternateScreen),
                    "changed": .bool(screen.changed), "busy": .bool(screen.busy)]

        case "send_keys":
            let (session, driver) = try resolve(arguments, client: client)
            let raw = (arguments.array("keys") ?? []).compactMap(\.stringValue)
            guard !raw.isEmpty else { throw ToolError(message: "keys is required") }
            var keys: [AgentKey] = []
            for name in raw {
                guard let key = AgentKey(name) else { throw ToolError(message: "unknown key: \(name)") }
                keys.append(key)
            }
            try await driver.send(keys: keys)
            // Names only: literal text may itself be a secret the agent typed.
            record(.keys, session, client, detail: raw.map { $0.hasPrefix("text:") ? "text(\($0.count - 5) chars)" : $0 }
                .joined(separator: ", "))
            return ["sent": .number(Double(keys.count))]

        case "type_password":
            let (session, driver) = try resolve(arguments, client: client)
            guard let password = savedPassword(session.host.id), !password.isEmpty else {
                throw ToolError(message: "no saved password for this host")
            }
            try await driver.typePassword(password)
            record(.password, session, client)
            return ["typed": true]

        case "close_session":
            let (session, _) = try resolve(arguments, client: client)
            record(.close, session, client)
            sessions.closeSession(session.id)
            return ["closed": true]

        default:
            throw ToolError(message: "unknown tool: \(call.tool)")
        }
    }

    private func record(_ kind: AgentAuditEntry.Kind, _ session: TerminalSession, _ client: AgentClient,
                        detail: String? = nil, exitCode: Int32? = nil) {
        audit(AgentAuditEntry(client: client.name, hostName: session.host.displayName, sessionID: session.id,
                              kind: kind, detail: detail, exitCode: exitCode))
    }

    /// The caller's own session on a host it may still use, or `not found`.
    private func resolve(_ arguments: JSONValue, client: AgentClient) throws -> (TerminalSession, AgentDriver) {
        guard let id = arguments.string("session_id").flatMap(UUID.init(uuidString:)),
              let session = sessions.openSessions.first(where: { $0.id == id }),
              let driver = session.agent, driver.clientID == client.id else { throw ToolError(message: Self.notFound) }
        guard allowed(session) else {
            sessions.endAgentControl(session.id)
            throw ToolError(message: Self.notFound)
        }
        guard !driver.isPaused else { throw AgentDriverError.paused }
        return (session, driver)
    }

    /// Access is judged on the host as it is now (its group may have changed).
    private func allowed(_ session: TerminalSession) -> Bool {
        guard let host = store.host(id: session.host.id) else { return false }
        return access().allows(host)
    }
}
