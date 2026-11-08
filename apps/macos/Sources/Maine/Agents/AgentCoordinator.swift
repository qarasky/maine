import Foundation

/// The app's agent switchboard: the master switch, the socket server,
/// paired clients, device-local access, and what locking does to agents.
/// Configured from `RootView` like `SyncCoordinator`.
@MainActor
final class AgentCoordinator: ObservableObject {
    static let shared = AgentCoordinator()
    static let enabledKey = "agentAccessEnabled"

    /// What Settings shows once, right after pairing.
    struct Pairing {
        let client: AgentClient
        let token: String
        let claudeCodeCommand: String
        let desktopJSON: String
    }

    @Published private(set) var isEnabled: Bool
    @Published private(set) var clients: [AgentClient] = []
    @Published private(set) var lastError: String?
    @Published private(set) var cachedAccess = AgentAccess()
    /// What agents did, oldest first. Empty while the vault is locked.
    @Published private(set) var auditEntries: [AgentAuditEntry] = []

    private let defaults: UserDefaults
    private let clientStore: AgentClientStore
    private let socketURL: URL
    private let passwords: PasswordStore
    private var server: AgentServer?
    private(set) weak var store: HostStore?
    private weak var sessions: SessionManager?
    private weak var appState: AppState?
    private var pausedByLock: Set<UUID> = []

    init(defaults: UserDefaults = .standard, clientStore: AgentClientStore = AgentClientStore(),
         socketURL: URL = AgentServer.defaultSocketURL, passwords: PasswordStore = .shared) {
        self.defaults = defaults
        self.clientStore = clientStore
        self.socketURL = socketURL
        self.passwords = passwords
        isEnabled = defaults.bool(forKey: Self.enabledKey)
    }

    func configure(store: HostStore, sessions: SessionManager, appState: AppState) {
        let first = self.store == nil
        self.store = store
        self.sessions = sessions
        self.appState = appState
        guard first else { return }
        clients = (try? clientStore.clients()) ?? []
        refreshAccess()
        refreshAudit()
        let passwords = passwords
        server = AgentServer(socketURL: socketURL, clients: clientStore, handler: AgentToolHandler(
            store: store, sessions: sessions,
            access: { [weak self] in self?.access() ?? AgentAccess() },
            isUnlocked: { [weak appState] in appState?.isVaultUnlocked ?? false },
            hasSavedPassword: { (try? passwords.password(for: $0)) != nil },
            savedPassword: { try? passwords.password(for: $0) },
            audit: { [weak self] in self?.recordAudit($0) }))
        if isEnabled { startServer() }
    }

    // MARK: - Master switch

    func setEnabled(_ enabled: Bool) {
        isEnabled = enabled
        defaults.set(enabled, forKey: Self.enabledKey)
        if enabled {
            startServer()
        } else {
            server?.stop()
            stopAll()
        }
    }

    private func startServer() {
        do {
            try server?.start()
            lastError = nil
        } catch {
            lastError = error.localizedDescription
        }
    }

    // MARK: - Clients

    func addClient(name: String) throws -> Pairing {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        let (client, token) = try clientStore.add(name: trimmed.isEmpty ? "MCP client" : trimmed)
        clients = (try? clientStore.clients()) ?? clients + [client]
        let executable = Bundle.main.executableURL?.path ?? CommandLine.arguments[0]
        let command = "claude mcp add maine --env MAINE_AGENT_TOKEN=\(token) -- \"\(executable)\" --mcp"
        let desktop: JSONValue = ["mcpServers": ["maine": [
            "command": .string(executable), "args": ["--mcp"], "env": ["MAINE_AGENT_TOKEN": .string(token)],
        ]]]
        let encoder = JSONEncoder()
        encoder.outputFormatting = [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]
        let json = (try? String(decoding: encoder.encode(desktop), as: UTF8.self)) ?? desktop.jsonText
        return Pairing(client: client, token: token, claudeCodeCommand: command, desktopJSON: json)
    }

    func revoke(_ id: UUID) {
        do { try clientStore.revoke(id) } catch { lastError = error.localizedDescription }
        server?.disconnect(clientID: id)
        sessions?.endAgentControl(clientID: id)
        clients = (try? clientStore.clients()) ?? clients.filter { $0.id != id }
    }

    // MARK: - Access

    /// The current rules, pruned to the hosts and groups that still exist.
    func access() -> AgentAccess {
        guard let layout = store?.layout else { return cachedAccess }
        return cachedAccess.pruned(to: layout)
    }

    func setAccess(_ access: AgentAccess) {
        let pruned = store.map { access.pruned(to: $0.layout) } ?? access
        do {
            try passwords.setAgentAccess(pruned)
            cachedAccess = pruned
        } catch {
            lastError = error.localizedDescription
            return
        }
        // Revocation is immediate: sessions on hosts no longer allowed lose
        // their agent (the session stays open for the user).
        for session in sessions?.agentSessions ?? [] {
            let host = store?.host(id: session.host.id)
            if host.map(pruned.allows) != true { sessions?.endAgentControl(session.id) }
        }
    }

    // MARK: - Hand to agent

    /// Lets `clientID` drive a session the user opened. Needs the master
    /// switch on, an unlocked vault and a host the rules allow, exactly like
    /// every other way an agent reaches a host.
    func handToAgent(sessionID: UUID, clientID: UUID) throws {
        guard let client = clients.first(where: { $0.id == clientID }),
              let session = sessions?.openSessions.first(where: { $0.id == sessionID }) else {
            throw AgentHandOffError.notFound
        }
        guard mayReach(session), let host = store?.host(id: session.host.id) else { throw AgentHandOffError.notAllowed }
        try sessions?.handToAgent(sessionID, client: client)
        recordAudit(AgentAuditEntry(client: client.name, hostName: host.displayName, sessionID: sessionID, kind: .handoff))
    }

    /// Whether the menu may offer hand-off for this session.
    func canHandOff(_ session: TerminalSession) -> Bool {
        session.agent == nil && !clients.isEmpty && mayReach(session)
    }

    private func mayReach(_ session: TerminalSession) -> Bool {
        guard isEnabled, appState?.isVaultUnlocked == true, let host = store?.host(id: session.host.id) else { return false }
        return access().allows(host)
    }

    // MARK: - Audit

    func recordAudit(_ entry: AgentAuditEntry) {
        do {
            try passwords.appendAgentAudit([entry])
            refreshAudit()
        } catch {
            lastError = error.localizedDescription
        }
    }

    func clearAudit() {
        do {
            try passwords.clearAgentAudit()
            auditEntries = []
        } catch {
            lastError = error.localizedDescription
        }
    }

    private func refreshAudit() {
        auditEntries = (try? passwords.agentAudit()) ?? []
    }

    private func refreshAccess() {
        cachedAccess = (try? passwords.agentAccess()) ?? AgentAccess()
    }

    // MARK: - Lock and stop

    var activeCount: Int { sessions?.agentSessions.count ?? 0 }

    func vaultDidLock() {
        auditEntries = []
        for session in sessions?.agentSessions ?? [] {
            guard let agent = session.agent, !agent.isPaused else { continue }
            agent.isPaused = true
            pausedByLock.insert(session.id)
        }
    }

    func vaultDidUnlock() {
        refreshAccess()
        refreshAudit()
        for session in sessions?.agentSessions ?? [] where pausedByLock.contains(session.id) {
            session.agent?.isPaused = false
        }
        pausedByLock = []
    }

    /// Ends agent control everywhere; every session stays open.
    func stopAll() {
        for session in sessions?.agentSessions ?? [] { sessions?.endAgentControl(session.id) }
        pausedByLock = []
    }
}
