import AppKit
import Foundation

/// Application-facing sync: profiles, triggers and status. Everything that
/// touches the network or the vault file goes through `SyncEngine`; this type
/// only decides when, and keeps the app's state in step with the adopted file.
@MainActor
public final class SyncCoordinator: ObservableObject {
    public static let shared = SyncCoordinator()

    public enum Status: Equatable {
        case off
        case idle
        case syncing
        case offline(String)
        case tokenRejected
        case conflicted(copy: String)
        case failed(String)

        public var label: String {
            switch self {
            case .off: return "Sync off"
            case .idle: return "Synced"
            case .syncing: return "Syncing…"
            case .offline(let message): return message
            case .tokenRejected: return "Token rejected"
            case .conflicted(let copy): return "Conflict saved as \(copy)"
            case .failed(let message): return message
            }
        }
    }

    @Published public private(set) var status: Status = .off
    @Published public private(set) var lastSyncedAt: Date?

    public let profiles: SyncProfileStore
    public let passwords: PasswordStore
    public let engine: SyncEngine

    private weak var store: HostStore?
    private weak var sessions: SessionManager?
    private weak var appState: AppState?

    private var vaultObserver: NSObjectProtocol?
    private var activationObserver: NSObjectProtocol?
    private var pushDebounce: Task<Void, Never>?
    private var pollLoop: Task<Void, Never>?
    private var inFlight = false
    private var nextAttempt = Date.distantPast
    private var backoffStep = 0
    private var sizeBlockedSHA: String?

    public convenience init() {
        self.init(profiles: SyncProfileStore(),
                  passwords: .shared,
                  vaultURL: PasswordStore.defaultFileURL,
                  transport: URLSessionSyncTransport())
    }

    public init(profiles: SyncProfileStore,
                passwords: PasswordStore,
                vaultURL: URL,
                transport: any SyncTransport) {
        self.profiles = profiles
        self.passwords = passwords
        engine = SyncEngine(vaultURL: vaultURL, passwords: passwords, transport: transport)
        lastSyncedAt = profiles.activeProfile?.lastSyncedAt
        if profiles.activeProfile != nil { status = .idle }

        vaultObserver = NotificationCenter.default.addObserver(
            forName: .maineVaultDidWrite, object: nil, queue: .main
        ) { [weak self] _ in
            Task { @MainActor [weak self] in self?.localVaultChanged() }
        }
        activationObserver = NotificationCenter.default.addObserver(
            forName: NSApplication.didBecomeActiveNotification, object: nil, queue: .main
        ) { [weak self] _ in
            Task { @MainActor [weak self] in await self?.pullIfPossible(reason: "foreground") }
        }
    }

    public var activeProfile: SyncProfile? { profiles.activeProfile }

    // MARK: - Wiring

    public func configure(store: HostStore, sessions: SessionManager, appState: AppState) {
        self.store = store
        self.sessions = sessions
        self.appState = appState
    }

    // MARK: - Lifecycle triggers

    /// The vault just unlocked: pull (cheaply) and start polling.
    public func vaultDidUnlock() async {
        guard profiles.activeProfile != nil else { status = .off; return }
        status = .idle
        backoffStep = 0
        nextAttempt = .distantPast
        await pullIfPossible(reason: "unlock")
        startPolling()
    }

    public func vaultDidLock() {
        stopPolling()
        pushDebounce?.cancel()
        pushDebounce = nil
    }

    private func localVaultChanged() {
        guard appState?.isVaultUnlocked == true, profiles.activeProfile != nil else { return }
        pushDebounce?.cancel()
        pushDebounce = Task { [weak self] in
            try? await Task.sleep(for: .seconds(2))
            guard !Task.isCancelled else { return }
            await self?.performPush()
        }
    }

    /// Best-effort final push when the vault is about to lock. Fire and
    /// forget: the push reads the file, not the key, so it cannot block.
    public func flushForLock() {
        guard profiles.activeProfile != nil, localIsDirty else { return }
        Task { [weak self] in await self?.performPush(allowingLocked: true) }
    }

    public var wantsFlushBeforeQuit: Bool {
        appState?.isVaultUnlocked == true && profiles.activeProfile != nil && localIsDirty
    }

    /// Waits briefly for one last push; always calls `completion`.
    public func flushBeforeQuit(timeout: TimeInterval = 2, completion: @escaping @MainActor () -> Void) {
        Task { [weak self] in
            guard let self else { completion(); return }
            await self.withTimeout(timeout) {
                await self.performPush(allowingLocked: true)
            }
            completion()
        }
    }

    private var localIsDirty: Bool {
        guard let profile = profiles.activeProfile else { return false }
        return engine.localVaultSHA256() != profile.lastSHA256
    }

    // MARK: - Polling

    private func startPolling() {
        guard pollLoop == nil else { return }
        pollLoop = Task { [weak self] in
            while !Task.isCancelled {
                try? await Task.sleep(for: .seconds(30))
                guard let self, !Task.isCancelled else { return }
                await self.pollTick()
            }
        }
    }

    private func stopPolling() {
        pollLoop?.cancel()
        pollLoop = nil
    }

    private func pollTick() async {
        guard profiles.activeProfile != nil, appState?.isVaultUnlocked == true else { return }
        guard Date() >= nextAttempt else { return }
        await pullIfPossible(reason: "poll")
    }

    private func scheduleBackoff() {
        let delays: [TimeInterval] = [30, 120, 300]
        let delay = delays[min(backoffStep, delays.count - 1)]
        backoffStep += 1
        nextAttempt = Date().addingTimeInterval(delay)
    }

    private func withTimeout(_ seconds: TimeInterval, _ operation: @escaping @MainActor () async -> Void) async {
        await withTaskGroup(of: Void.self) { group in
            group.addTask { await operation() }
            group.addTask { try? await Task.sleep(for: .seconds(seconds)) }
            _ = await group.next()
            group.cancelAll()
        }
    }

    // MARK: - Pull

    private func pullIfPossible(reason: String) async {
        guard let profile = profiles.activeProfile, appState?.isVaultUnlocked == true, !inFlight else { return }
        guard let token = try? profiles.token(for: profile.id), !token.isEmpty else {
            status = .failed("The saved sync token is missing. Pair this device again.")
            return
        }
        inFlight = true
        status = .syncing
        do {
            // Polling and foreground checks use HEAD first; 304 almost always.
            if reason != "conflict" && reason != "unlock" {
                let head = try await engine.head(server: profile.serverURL, token: token, lastVersion: profile.lastVersion)
                switch head {
                case .notModified:
                    inFlight = false
                    status = .idle
                    return
                case .empty:
                    inFlight = false
                    return
                case .current(let state) where state.version == profile.lastVersion:
                    inFlight = false
                    status = .idle
                    return
                default:
                    break
                }
            }
            let result = try await engine.pull(server: profile.serverURL, token: token,
                                               lastVersion: profile.lastVersion, isDirty: localIsDirty)
            inFlight = false
            handlePull(result, profile: profile)
        } catch {
            inFlight = false
            await note(error, during: "pull")
        }
    }

    private func handlePull(_ result: SyncEngine.PullResult, profile: SyncProfile) {
        switch result {
        case .notModified(let state):
            profiles.update(id: profile.id) { $0.lastSHA256 = state.sha256 ?? $0.lastSHA256 }
            status = .idle
        case .empty:
            status = .idle
        case .adopted(let state, let conflictCopy):
            markSynced(profile: profile, state: state)
            adoptInUI()
            if let conflictCopy {
                status = .conflicted(copy: conflictCopy)
                appState?.showAlert(title: "Another device changed the vault",
                                    "Your version was saved as \(conflictCopy). This device now follows the server's copy.")
            } else {
                status = .idle
            }
        }
    }

    /// The live vault file changed under the app; keep hosts and sessions in
    /// step without ever reparenting a terminal that survives.
    private func adoptInUI() {
        store?.load()
        if let hosts = store?.hosts {
            sessions?.closeSessionsMissingFromHosts(hosts)
        }
        do { try sessions?.restoreSavedSessions() }
        catch { appState?.showAlert(title: "Saved sessions not restored", error.localizedDescription) }
    }

    // MARK: - Push

    private func performPush(allowingLocked: Bool = false) async {
        guard let profile = profiles.activeProfile, !inFlight else { return }
        guard allowingLocked || appState?.isVaultUnlocked == true else { return }
        guard let token = try? profiles.token(for: profile.id), !token.isEmpty else { return }
        let currentSHA = engine.localVaultSHA256()
        if let blocked = sizeBlockedSHA, blocked == currentSHA { return }
        if currentSHA == profile.lastSHA256 { return }
        inFlight = true
        status = .syncing
        do {
            let result = try await engine.push(server: profile.serverURL, token: token,
                                               lastVersion: profile.lastVersion, lastSHA256: profile.lastSHA256)
            inFlight = false
            switch result {
            case .skipped(let state), .stored(let state):
                markSynced(profile: profile, state: state)
            }
            status = .idle
        } catch {
            inFlight = false
            await note(error, during: "push")
        }
    }

    private func markSynced(profile: SyncProfile, state: SyncSlotState) {
        let now = Date()
        profiles.update(id: profile.id) {
            $0.lastVersion = state.version
            if let sha = state.sha256 { $0.lastSHA256 = sha }
            $0.lastSyncedAt = now
        }
        lastSyncedAt = now
        backoffStep = 0
        nextAttempt = .distantPast
        sizeBlockedSHA = nil
    }

    // MARK: - Failures

    private func note(_ error: Error, during operation: String) async {
        if error is CancellationError { return }
        if let transportError = error as? SyncTransportError {
            switch transportError {
            case .conflict:
                // The local file is dirty by definition here; the pull keeps
                // a copy of it before adopting the server's version.
                await pullIfPossible(reason: "conflict")
            case .unauthorized:
                status = .tokenRejected
                stopPolling()
            case .offline(let message):
                status = .offline(message)
                scheduleBackoff()
            case .tooLarge:
                sizeBlockedSHA = engine.localVaultSHA256()
                status = .failed("The vault is larger than this server accepts.")
            case .preconditionRequired:
                status = .failed("The sync client sent an incomplete request. Nothing was changed.")
            case .server, .malformed, .notFound, .forbidden:
                status = .failed(transportError.localizedDescription)
                if case .server = transportError { scheduleBackoff() }
            }
            return
        }
        if let engineError = error as? SyncEngineError {
            status = .failed(engineError.localizedDescription)
            appState?.showAlert(title: "Sync stopped", engineError.localizedDescription)
            stopPolling()
            return
        }
        status = .failed(error.localizedDescription)
        scheduleBackoff()
        _ = operation
    }

    // MARK: - Pairing

    public struct CreatedProfile: Sendable {
        public let profile: SyncProfile
        /// Returned by the server exactly once; already in the Keychain.
        public let token: String
    }

    public enum JoinOutcome: Sendable {
        case joined(SyncProfile)
        case emptySlot
    }

    public func createProfile(server: URL, name: String, createSecret: String?) async throws -> CreatedProfile {
        let outgoing = profiles.activeProfile
        let created = try await engine.create(at: server, createSecret: createSecret)
        if let outgoing {
            // The live vault now belongs to the new profile; keep the previous
            // profile's copy so switching back is lossless.
            try? engine.copyLocalVault(to: engine.workspaceURL(for: outgoing.id))
        }
        let profile = SyncProfile(name: name, serverURL: server,
                                  lastVersion: created.state.version, lastSHA256: created.state.sha256)
        try profiles.add(profile, token: created.token)
        profiles.setActive(profile.id)
        markSynced(profile: profile, state: created.state)
        status = .idle
        return CreatedProfile(profile: profile, token: created.token)
    }

    public func joinProfile(server: URL, token: String, name: String, masterPassword: String) async throws -> JoinOutcome {
        switch try await engine.prepareJoin(server: server, token: token, masterPassword: masterPassword) {
        case .emptySlot:
            return .emptySlot
        case .ready(let bytes, let state):
            let outgoing = profiles.activeProfile
            do {
                try sessions?.detachAllForVaultSwitch()
                if let outgoing {
                    await withTimeout(3) { await self.performPush(allowingLocked: true) }
                    try engine.moveLocalVault(to: engine.workspaceURL(for: outgoing.id))
                }
            } catch {
                throw error
            }
            try engine.adoptPreparedJoin(bytes, state: state)
            let profile = SyncProfile(name: name, serverURL: server,
                                      lastVersion: state.version, lastSHA256: state.sha256)
            try profiles.add(profile, token: token)
            profiles.setActive(profile.id)
            markSynced(profile: profile, state: state)
            status = .idle
            // The password was just verified against the adopted file; use it
            // so the user lands in a working app instead of the sheet again.
            do { try passwords.unlock(masterPassword: masterPassword) }
            catch { /* The unlock sheet will ask for it again. */ }
            appState?.vaultDidUnlock()
            appState?.isVaultSheetPresented = false
            adoptInUI()
            startPolling()
            return .joined(profile)
        }
    }

    /// Join an empty slot by pushing this device's vault as its first version.
    public func pushLocalVaultIntoEmptySlot(server: URL, token: String, name: String) async throws -> SyncProfile {
        guard engine.localVaultExists else { throw SyncEngineError.noLocalVault }
        let result = try await engine.push(server: server, token: token, lastVersion: 0, lastSHA256: nil)
        let state: SyncSlotState
        switch result {
        case .stored(let slotState), .skipped(let slotState):
            state = slotState
        }
        let profile = SyncProfile(name: name, serverURL: server,
                                  lastVersion: state.version, lastSHA256: state.sha256)
        try profiles.add(profile, token: token)
        profiles.setActive(profile.id)
        markSynced(profile: profile, state: state)
        status = .idle
        return profile
    }

    // MARK: - Token and slot management

    public func rotateToken(for id: UUID) async throws -> String {
        guard let profile = profiles.profile(for: id),
              let rawToken = try profiles.token(for: id), !rawToken.isEmpty else {
            throw SyncTransportError.unauthorized
        }
        let newToken = try await engine.rotate(server: profile.serverURL, token: rawToken)
        // Persist before acknowledging: if this is lost, the slot is gone.
        try profiles.replaceToken(newToken, for: id)
        if id == profiles.activeProfileID { status = .idle }
        return newToken
    }

    public func disable(_ id: UUID) {
        do {
            try profiles.remove(id)
        } catch {
            appState?.showAlert(title: "Sync not disabled", error.localizedDescription)
            return
        }
        if profiles.activeProfile == nil {
            status = .off
            lastSyncedAt = nil
            stopPolling()
        }
    }

    public func deleteRemoteData(_ id: UUID) async throws {
        guard let profile = profiles.profile(for: id),
              let rawToken = try profiles.token(for: id), !rawToken.isEmpty else {
            throw SyncTransportError.unauthorized
        }
        try await engine.delete(server: profile.serverURL, token: rawToken)
        disable(id)
    }

    // MARK: - Workspaces

    /// Makes `id` the active profile: the current vault is parked in the
    /// outgoing profile's workspace file, the target's workspace file becomes
    /// the live vault, and the app asks for that vault's master password.
    public func activate(_ id: UUID) async {
        let currentID = profiles.activeProfileID
        guard id != currentID, let target = profiles.profile(for: id) else { return }
        let outgoing = currentID.flatMap { profiles.profile(for: $0) }

        if outgoing != nil || engine.localVaultExists {
            do {
                try sessions?.detachAllForVaultSwitch()
                if let outgoing {
                    await withTimeout(3) { await self.performPush(allowingLocked: true) }
                    try engine.moveLocalVault(to: engine.workspaceURL(for: outgoing.id))
                } else {
                    try engine.moveLocalVaultAside(kind: "pre-sync")
                }
            } catch {
                appState?.showAlert(title: "Workspace not switched", error.localizedDescription)
                return
            }
        }

        stopPolling()
        pushDebounce?.cancel()
        pushDebounce = nil

        let workspace = engine.workspaceURL(for: id)
        if FileManager.default.fileExists(atPath: workspace.path) {
            do { try engine.adoptLocalVault(from: workspace) }
            catch { appState?.showAlert(title: "Workspace not switched", error.localizedDescription) }
        }
        profiles.setActive(id)
        lastSyncedAt = target.lastSyncedAt
        status = .idle

        store?.load()
        appState?.lockVault()
        appState?.isVaultSheetPresented = true
    }
}
