import Foundation
import Combine

/// Tracks the open terminal sessions and which one the detail pane shows.
///
/// Sessions have independent identities, even when they connect to the same host.
@MainActor
public final class SessionManager: ObservableObject {
    @Published public private(set) var openSessions: [TerminalSession] = []
    @Published public private(set) var savedSessions: [SavedSession] = []
    public var openHostIDs: [Host.ID] { openSessions.map(\.host.id) }
    @Published public private(set) var selectedSessionID: UUID? {
        didSet {
            DebugLog.shared.record("workspace.selection", fields: [
                "previousSession": oldValue?.uuidString ?? "none", "selectedHost": selectedHostID?.uuidString ?? "none",
                "selectedSession": selectedSession?.id.uuidString ?? "none", "openSessions": String(openSessions.count)
            ])
        }
    }

    /// Host-level navigation remains convenient for the sidebar and menus.
    public var selectedHostID: Host.ID? {
        get { selectedSession?.host.id }
        set {
            guard let newValue else { selectedSessionID = nil; return }
            selectedSessionID = session(for: newValue)?.id
        }
    }

    private var subscriptions: [UUID: AnyCancellable] = [:]
    private var sessionNumbers: [UUID: Int] = [:]
    private var autosaveTask: Task<Void, Never>?
    private let store: HostStore
    private let makeSession: (Host) -> TerminalSession

    public init(store: HostStore, makeSession: @escaping (Host) -> TerminalSession = { TerminalSession(host: $0) }) {
        self.store = store
        self.makeSession = makeSession
    }

    // MARK: - Queries

    public func session(for hostID: Host.ID) -> TerminalSession? {
        if let selectedSession, selectedSession.host.id == hostID { return selectedSession }
        return sessions(for: hostID).first(where: { $0.state.isLive }) ?? sessions(for: hostID).first
    }

    public func sessions(for hostID: Host.ID) -> [TerminalSession] {
        openSessions.filter { $0.host.id == hostID }
    }

    public func label(for session: TerminalSession) -> String {
        session.customName ?? "Session \(sessionNumbers[session.id] ?? 1)"
    }

    public func renameSession(_ id: UUID, to name: String) throws {
        let name = name.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !name.isEmpty else { throw HostStoreError("Enter a session name.") }
        guard let session = openSessions.first(where: { $0.id == id }) else { return }
        if let savedID = session.savedSessionID,
           var saved = try store.savedSessions().first(where: { $0.id == savedID }) {
            saved.name = name
            saved.usesDefaultName = false
            try store.saveSession(saved)
            savedSessions = try store.savedSessions()
        }
        session.customName = name
    }

    public func checkpointSessions() throws {
        guard store.persistsSessions, !openSessions.isEmpty else { return }
        let stored = try store.savedSessions()
        var changes: [SavedSession] = []
        for session in openSessions {
            let id = session.savedSessionID ?? session.id
            let history = session.historySnapshot()
            let name = label(for: session)
            if let previous = stored.first(where: { $0.id == id }),
               previous.name == name, previous.history == history { continue }
            changes.append(SavedSession(id: id, hostID: session.host.id, name: name, history: history,
                                        usesDefaultName: session.customName == nil, number: sessionNumbers[session.id]))
        }
        if !changes.isEmpty {
            try store.saveSessions(changes)
            store.clearSaveFailure()
        }
        for session in openSessions where session.savedSessionID == nil { session.savedSessionID = session.id }
        savedSessions = try store.savedSessions()
    }

    private func checkpointAutomatically() {
        do { try checkpointSessions() }
        catch PasswordStore.VaultError.locked { /* Resume on unlock; no plaintext fallback. */ }
        catch { store.reportSaveFailure(error.localizedDescription) }
    }

    private func startAutosave() {
        guard store.persistsSessions, autosaveTask == nil else { return }
        autosaveTask = Task { @MainActor [weak self] in
            while !Task.isCancelled {
                do { try await Task.sleep(for: .seconds(2)) } catch { return }
                guard let self else { return }
                self.checkpointAutomatically()
            }
        }
    }

    deinit { autosaveTask?.cancel() }

    /// Quit ends processes but keeps their records, unlike an explicit close.
    public func shutdown() throws {
        try checkpointSessions()
        stopProcessesForQuit()
    }

    public func stopProcessesForQuit() {
        autosaveTask?.cancel()
        autosaveTask = nil
        for session in openSessions { session.disconnect() }
    }

    /// Restores snapshots and reconnects them, including after a fresh launch.
    public func restoreSavedSessions(for hostID: UUID? = nil) throws {
        savedSessions = try store.savedSessions()
        for saved in savedSessions where hostID == nil || saved.hostID == hostID {
            guard !openSessions.contains(where: { $0.savedSessionID == saved.id }),
                  let host = store.host(id: saved.hostID) else { continue }
            let session = makeSession(host)
            let generatedName = saved.usesDefaultName ?? (saved.name.range(of: "^Session [0-9]+$", options: .regularExpression) != nil)
            session.customName = generatedName ? nil : saved.name
            session.savedSessionID = saved.id
            session.restoreHistory(saved.history)
            sessionNumbers[session.id] = saved.number ?? (generatedName ? Int(saved.name.split(separator: " ").last ?? "") : nil)
                ?? (sessions(for: host.id).count + 1)
            observe(session)
            openSessions.append(session)
            // Restored history stays as scrollback; the session reconnects
            // on its own so the user never has to press Connect after unlock.
            session.connect()
        }
        if selectedSessionID == nil, let first = openSessions.first { selectSession(first.id) }
        startAutosave()
    }

    public var selectedSession: TerminalSession? {
        openSessions.first { $0.id == selectedSessionID }
    }

    public func selectSession(_ sessionID: UUID) {
        guard let session = openSessions.first(where: { $0.id == sessionID }) else { return }
        selectedSessionID = sessionID
        session.focus()
    }

    public func select(_ hostID: Host.ID?) {
        selectedHostID = hostID.flatMap { session(for: $0) == nil ? nil : $0 }
    }

    // MARK: - Opening and closing

    /// Connects to `host`, or reveals the existing session when there is one.
    public func open(_ host: Host) {
        DebugLog.shared.record("manager.open", fields: ["host": host.id.uuidString])
        if let existing = session(for: host.id) {
            if !existing.state.isLive {
                reconnect(host.id)
                return
            }
            selectedSessionID = existing.id
            existing.focus()
            return
        }

        newSession(host)
    }

    /// Explicitly opens another independent connection, without replacing any.
    public func newSession(_ host: Host) {
        DebugLog.shared.record("manager.new-session", fields: ["host": host.id.uuidString])
        let number = sessions(for: host.id).compactMap { sessionNumbers[$0.id] }.max().map { $0 + 1 } ?? 1
        let session = makeSession(host)
        sessionNumbers[session.id] = number
        observe(session)
        openSessions.append(session)
        selectedSessionID = session.id
        session.connect()
        checkpointAutomatically()
        startAutosave()
    }

    /// Starts a brand new session: the old process is stopped and the terminal
    /// starts from a clean screen.
    public func reconnect(_ hostID: Host.ID) {
        if let session = session(for: hostID) { reconnectSession(session.id); return }
        guard let host = store.host(id: hostID) else { return }
        open(host)
    }

    public func reconnectSession(_ sessionID: UUID) {
        DebugLog.shared.record("manager.reconnect", fields: ["session": sessionID.uuidString])
        guard let index = openSessions.firstIndex(where: { $0.id == sessionID }) else { return }
        let previous = openSessions[index]
        let host = store.host(id: previous.host.id) ?? previous.host
        previous.disconnect()
        subscriptions[previous.id] = nil
        let replacement = makeSession(host)
        replacement.customName = previous.customName
        replacement.savedSessionID = previous.savedSessionID
        if previous.savedSessionID != nil {
            replacement.restoreHistory(previous.historySnapshot() + "\n— New connection —")
        }
        sessionNumbers[replacement.id] = sessionNumbers.removeValue(forKey: previous.id)
        observe(replacement)
        // Publish one replacement, not an empty workspace followed by an open.
        openSessions[index] = replacement
        selectedSessionID = replacement.id
        replacement.connect()
        checkpointAutomatically()
    }

    public func disconnect(_ hostID: Host.ID) {
        DebugLog.shared.record("manager.disconnect", fields: ["host": hostID.uuidString])
        session(for: hostID)?.disconnect()
    }

    public func close(_ hostID: Host.ID) {
        guard let session = session(for: hostID) else { return }
        closeSession(session.id)
    }

    public func closeSession(_ sessionID: UUID) {
        DebugLog.shared.record("manager.close", fields: ["session": sessionID.uuidString])
        guard let session = openSessions.first(where: { $0.id == sessionID }) else { return }
        if let savedID = session.savedSessionID {
            do {
                try store.deleteSavedSession(savedID)
                savedSessions.removeAll { $0.id == savedID }
            } catch {
                store.reportSaveFailure("Session not closed: \(error.localizedDescription)")
                return
            }
            store.clearSaveFailure()
        }
        // Always stop the process before dropping the view that owns it,
        // otherwise the ssh child would keep running.
        session.disconnect()
        subscriptions[session.id] = nil
        sessionNumbers[session.id] = nil
        let wasSelected = selectedSessionID == sessionID
        openSessions.removeAll { $0.id == sessionID }
        if wasSelected {
            selectedSessionID = sessions(for: session.host.id).last?.id ?? openSessions.last?.id
        }
    }

    public func closeAll() {
        DebugLog.shared.record("manager.close-all", fields: ["count": String(openSessions.count)])
        for session in openSessions { closeSession(session.id) }
        if openSessions.isEmpty {
            autosaveTask?.cancel()
            autosaveTask = nil
        }
    }

    /// Removes the host and stops whatever it had running.
    public func delete(_ host: Host) {
        store.delete(host)
        if store.host(id: host.id) == nil {
            savedSessions.removeAll { $0.hostID == host.id }
            for session in sessions(for: host.id) { closeSession(session.id) }
        }
    }

    private func observe(_ session: TerminalSession) {
        subscriptions[session.id] = session.objectWillChange.sink { [weak self] _ in
            self?.objectWillChange.send()
        }
    }
}
