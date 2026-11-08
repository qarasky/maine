import AppKit
import Foundation

/// Why a check or an install could not finish. The UI shows the message; the
/// log only records the outcome.
enum UpdateCheckError: LocalizedError {
    case unexpectedResponse(Int)
    case checksumMissing

    var errorDescription: String? {
        switch self {
        case .unexpectedResponse(let status):
            "The update server answered with status \(status)."
        case .checksumMissing:
            "The release does not publish a readable SHA-256 checksum, so Maine will not install it."
        }
    }
}

/// Application-facing updating: when to check, what to offer, and the hand-off
/// to `UpdateInstaller`. Configured from `RootView` like `SyncCoordinator`.
@MainActor
final class UpdateCoordinator: ObservableObject {
    static let shared = UpdateCoordinator()
    static let automaticKey = "app.maine.update.automatic"
    /// Two hours, as agreed in the spec.
    static let interval: Duration = .seconds(7200)

    enum Phase: Equatable {
        case idle
        case checking
        case downloading
        case waitingForIdle
        case failed(String)

        var isBusy: Bool {
            switch self {
            case .checking, .downloading, .waitingForIdle: true
            case .idle, .failed: false
            }
        }
    }

    /// Persisted preference: automatic checks happen only when this is on and
    /// the running build is a real release.
    @Published var automaticChecks: Bool {
        didSet {
            guard automaticChecks != oldValue else { return }
            defaults.set(automaticChecks, forKey: Self.automaticKey)
        }
    }
    @Published private(set) var lastChecked: Date?
    @Published private(set) var phase: Phase = .idle

    let current: AppVersion
    private let fetch: () async throws -> Data
    private let defaults: UserDefaults
    private let skipped: SkippedVersionStore
    private var isIdle: @MainActor () -> Bool
    private weak var appState: AppState?
    private var timer: Task<Void, Never>?
    private var popupIsOpen = false

    init(current: AppVersion = .current,
         fetch: @escaping () async throws -> Data = UpdateCoordinator.fetchLatest,
         defaults: UserDefaults = .standard,
         isIdle: @escaping @MainActor () -> Bool = { true }) {
        self.current = current
        self.fetch = fetch
        self.defaults = defaults
        skipped = SkippedVersionStore(defaults: defaults)
        self.isIdle = isIdle
        automaticChecks = defaults.object(forKey: Self.automaticKey) as? Bool ?? true
    }

    // MARK: - Wiring

    /// Ties the updater to the window: an install waits while an agent is
    /// working or a sync push is in flight, and popups need an unlocked vault.
    func configure(appState: AppState, sessions: SessionManager) {
        self.appState = appState
        isIdle = { [weak sessions] in
            let agentBusy = sessions?.agentSessions.contains { $0.agent?.isBusy == true } ?? false
            return !agentBusy && SyncCoordinator.shared.status != .syncing
        }
    }

    /// True while the two-hour check loop is scheduled.
    var isScheduling: Bool { timer != nil }

    // MARK: - Schedule

    /// Checks now, then every two hours until the vault locks.
    func vaultDidUnlock() {
        guard timer == nil else { return }
        timer = Task { [weak self] in
            guard let self else { return }
            await self.check(manual: false)
            while !Task.isCancelled {
                try? await Task.sleep(for: Self.interval)
                guard !Task.isCancelled else { return }
                await self.check(manual: false)
            }
        }
    }

    func vaultDidLock() {
        timer?.cancel()
        timer = nil
    }

    // MARK: - Checking

    /// Runs one check. Automatic checks never touch the network when the
    /// toggle is off or the build is a development build, skip a version the
    /// user skipped, and stay silent on failure. Manual checks always answer.
    @discardableResult
    func check(manual: Bool) async -> UpdateOffer? {
        if !manual {
            guard automaticChecks, !current.isDevelopment else { return nil }
        }
        guard !phase.isBusy else { return nil }

        phase = .checking
        let data: Data
        do {
            data = try await fetch()
        } catch {
            return failed(error, manual: manual)
        }

        let offer: UpdateOffer?
        do {
            offer = try UpdateFeed.offer(from: data, current: current)
        } catch {
            return failed(error, manual: manual)
        }
        lastChecked = Date()
        phase = .idle

        guard let offer else {
            DebugLog.shared.record("update.check", fields: [
                "outcome": "upToDate", "version": current.description,
            ])
            if manual { presentUpToDate() }
            return nil
        }
        if !manual, skipped.skipped == offer.version {
            DebugLog.shared.record("update.check", fields: [
                "outcome": "skipped", "version": offer.version.description,
            ])
            return nil
        }

        DebugLog.shared.record("update.check", fields: [
            "outcome": "offered", "version": offer.version.description,
            "current": current.description,
        ])
        presentIfNeeded(offer)
        return offer
    }

    private func failed(_ error: Error, manual: Bool) -> UpdateOffer? {
        DebugLog.shared.record("update.check", fields: [
            "outcome": "failed", "version": current.description,
        ])
        guard manual else {
            phase = .idle
            return nil
        }
        let message = "The update check failed. \(error.localizedDescription)"
        phase = .failed(message)
        appState?.showAlert(title: "Update check failed", message)
        return nil
    }

    /// Remembers a version the user does not want to be offered again.
    func skip(_ offer: UpdateOffer) {
        skipped.skipped = offer.version
        DebugLog.shared.record("update.skip", fields: ["version": offer.version.description])
    }

    // MARK: - Popup

    private func presentUpToDate() {
        guard let appState, appState.isVaultUnlocked else { return }
        appState.showAlert(title: "Maine is up to date (\(current.description))",
                           "You are running the latest version.")
    }

    /// Offers the release once. Never while the vault is locked, its unlock
    /// sheet is still up, or another update popup is open.
    private func presentIfNeeded(_ offer: UpdateOffer) {
        guard let appState, appState.isVaultUnlocked, !appState.isVaultSheetPresented,
              !popupIsOpen else { return }
        popupIsOpen = true
        defer { popupIsOpen = false }

        let alert = NSAlert()
        alert.messageText = "Maine \(offer.version) is available"
        alert.informativeText = "You have \(current)."
        alert.addButton(withTitle: "Update")
        alert.addButton(withTitle: "Later")
        alert.addButton(withTitle: "Skip This Version")
        if !offer.notes.isEmpty {
            alert.accessoryView = Self.notesView(offer.notes)
        }

        switch alert.runModal() {
        case .alertFirstButtonReturn:
            DebugLog.shared.record("update.popup", fields: [
                "choice": "update", "version": offer.version.description,
            ])
            Task { await install(offer) }
        case .alertThirdButtonReturn:
            DebugLog.shared.record("update.popup", fields: [
                "choice": "skip", "version": offer.version.description,
            ])
            skip(offer)
        default:
            DebugLog.shared.record("update.popup", fields: [
                "choice": "later", "version": offer.version.description,
            ])
        }
    }

    /// Release notes in a scroll view, capped so a huge body stays readable.
    private static func notesView(_ notes: String) -> NSView {
        let scroll = NSScrollView(frame: NSRect(x: 0, y: 0, width: 420, height: 120))
        scroll.hasVerticalScroller = true
        scroll.drawsBackground = false
        scroll.borderType = .bezelBorder

        let text = NSTextView(frame: scroll.bounds)
        text.isEditable = false
        text.isSelectable = true
        text.drawsBackground = false
        text.font = .systemFont(ofSize: NSFont.smallSystemFontSize)
        text.string = String(notes.prefix(4000))
        text.textContainerInset = NSSize(width: 6, height: 6)
        scroll.documentView = text
        return scroll
    }

    // MARK: - Install

    /// A SHA-256 digest is exactly 64 hex characters; anything else is refused
    /// before a byte is downloaded.
    static func isValidSHA256(_ text: String) -> Bool {
        text.count == 64 && text.allSatisfy { $0.isASCII && $0.isHexDigit }
    }

    /// Downloads, verifies and stages the release, then quits so the detached
    /// helper can swap the bundle and relaunch.
    func install(_ offer: UpdateOffer) async {
        guard !phase.isBusy else { return }
        let target = Bundle.main.bundleURL
        guard UpdateInstaller.canInstall(over: target) else {
            DebugLog.shared.record("update.install", fields: [
                "outcome": "notWritable", "version": offer.version.description,
            ])
            appState?.showAlert(title: "Maine \(offer.version) cannot be installed here",
                                UpdateInstallError.notWritable.errorDescription ?? "")
            NSWorkspace.shared.open(offer.pageURL)
            return
        }

        phase = .downloading
        do {
            let checksumFile = try await Self.fetchChecksum(for: offer)
            let fileName = "Maine-\(offer.version).dmg"
            guard let expected = UpdateFeed.expectedSHA256(from: checksumFile, fileName: fileName),
                  Self.isValidSHA256(expected) else {
                throw UpdateCheckError.checksumMissing
            }

            if !isIdle() { phase = .waitingForIdle }
            while !isIdle() {
                try await Task.sleep(for: .seconds(2))
            }

            phase = .downloading
            let staged = try await UpdateInstaller.stage(offer, expectedSHA256: expected, over: target)
            try UpdateInstaller.launchSwap(parentPID: ProcessInfo.processInfo.processIdentifier,
                                           staged: staged, target: target)
            DebugLog.shared.record("update.install", fields: [
                "outcome": "staged", "version": offer.version.description,
            ], flush: true)
            NSApp.terminate(nil)
        } catch {
            let message = (error as? LocalizedError)?.errorDescription ?? error.localizedDescription
            phase = .failed(message)
            DebugLog.shared.record("update.install", fields: [
                "outcome": "failed", "version": offer.version.description,
            ])
            appState?.showAlert(title: "Update not installed", message)
        }
    }

    private static func fetchChecksum(for offer: UpdateOffer) async throws -> String {
        var request = URLRequest(url: offer.sha256URL)
        request.setValue("Maine-Updater", forHTTPHeaderField: "User-Agent")
        request.timeoutInterval = 15
        let (data, response) = try await URLSession.shared.data(for: request)
        guard let http = response as? HTTPURLResponse, http.statusCode == 200 else {
            throw UpdateCheckError.unexpectedResponse((response as? HTTPURLResponse)?.statusCode ?? 0)
        }
        return String(decoding: data, as: UTF8.self)
    }

    /// The only automatic network call besides sync. A plain GET: no
    /// identifiers, no cookies, a generic user agent.
    nonisolated static func fetchLatest() async throws -> Data {
        var request = URLRequest(url: UpdateFeed.latestReleaseURL)
        request.setValue("application/vnd.github+json", forHTTPHeaderField: "Accept")
        request.setValue("Maine-Updater", forHTTPHeaderField: "User-Agent")
        request.timeoutInterval = 15
        let (data, response) = try await URLSession.shared.data(for: request)
        guard let http = response as? HTTPURLResponse else {
            throw UpdateCheckError.unexpectedResponse(0)
        }
        guard http.statusCode == 200 else {
            throw UpdateCheckError.unexpectedResponse(http.statusCode)
        }
        return data
    }
}
