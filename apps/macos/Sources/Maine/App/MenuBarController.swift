import AppKit

/// Owns the menu bar item that keeps Maine reachable while its window is
/// closed. The window's close button only hides it: the app stays resident so
/// open sessions keep running, and the vault stays unlocked while the process
/// lives. Quitting is explicit — right-click the item or press Command-Q — and
/// still checkpoints through `applicationShouldTerminate`.
@MainActor
final class MenuBarController: NSObject {
    static let shared = MenuBarController()

    private var statusItem: NSStatusItem?
    private var closeKeyMonitor: Any?
    private weak var mainWindow: NSWindow?
    private weak var appState: AppState?
    private weak var sessions: SessionManager?
    private var openMainWindow: (() -> Void)?

    private override init() { super.init() }

    /// Creates the status item at launch, before any window exists.
    func install() {
        guard statusItem == nil else { return }
        let item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        if let button = item.button {
            button.image = Self.menuBarMark()
            button.target = self
            button.action = #selector(statusItemClicked)
            button.sendAction(on: [.leftMouseUp, .rightMouseUp])
            button.toolTip = "Maine"
            button.setAccessibilityLabel("Maine")
        }
        // Command-W hides the window instead of closing it. A local monitor
        // runs before menu key equivalents, so this also covers a standard
        // Close command; ⇧⌘W (Close Session) and other combinations pass
        // through untouched.
        closeKeyMonitor = NSEvent.addLocalMonitorForEvents(matching: .keyDown) { [weak self] event in
            self?.handleCloseKey(event) ?? event
        }
        statusItem = item
    }

    /// The close shortcut's rule, separated from the monitor so it can be
    /// tested: exactly ⌘W with the window tracked; every other combination
    /// (including ⇧⌘W) is returned to its normal handler.
    func handleCloseKey(_ event: NSEvent) -> NSEvent? {
        guard mainWindow != nil,
              event.charactersIgnoringModifiers == "w",
              event.modifierFlags.contains(.command),
              event.modifierFlags.intersection([.shift, .option, .control]).isEmpty else {
            return event
        }
        hideMainWindow()
        return nil
    }

    /// Connects the resident UI to the vault and session state. Idempotent:
    /// the window calls this again every time it is reopened.
    func configure(appState: AppState, sessions: SessionManager, openWindow: @escaping () -> Void) {
        self.appState = appState
        self.sessions = sessions
        openMainWindow = openWindow
    }

    /// Tracks the main window and makes its close button hide it instead of
    /// destroying it. Idempotent: it runs again if SwiftUI ever has to
    /// recreate the window.
    func trackMainWindow(_ window: NSWindow?) {
        mainWindow = window
        guard let window else { return }
        let close = window.standardWindowButton(.closeButton)
        close?.target = self
        close?.action = #selector(hideMainWindow(_:))
    }

    /// Hides the window. The process, its sessions and the unlocked vault
    /// keep running; the status item brings the same window back.
    @objc func hideMainWindow(_ sender: Any? = nil) {
        guard let mainWindow, mainWindow.isVisible else { return }
        DebugLog.shared.record("ui.window-hidden")
        mainWindow.orderOut(nil)
    }

    /// Brings the main window back, recreating it only if it was destroyed.
    func showMainWindow() {
        if let mainWindow {
            if mainWindow.isMiniaturized { mainWindow.deminiaturize(nil) }
            mainWindow.makeKeyAndOrderFront(nil)
        } else {
            openMainWindow?()
        }
        if NSApp.isHidden { NSApp.unhide(nil) }
        NSApp.activate()
    }

    // MARK: - Status item

    @objc private func statusItemClicked() {
        if NSApp.currentEvent?.type == .rightMouseUp {
            presentMenu()
        } else {
            showMainWindow()
        }
    }

    private func presentMenu() {
        guard let statusItem else { return }
        let menu = NSMenu()

        menu.addItem(item("Open Maine", action: #selector(openMaine)))
        menu.addItem(.separator())

        if let appState {
            if appState.isVaultUnlocked {
                menu.addItem(item("Lock Password Vault", action: #selector(lockVault)))
            } else {
                menu.addItem(item("Unlock Password Vault…", action: #selector(unlockVault)))
            }
            menu.addItem(.separator())
        }

        menu.addItem(item("Quit Maine", action: #selector(quitMaine)))

        // A status item shows its menu on left-click while one is installed,
        // so it is attached just for this click and removed right after the
        // menu closes: the next left-click must open the window again.
        statusItem.menu = menu
        statusItem.button?.performClick(nil)
        statusItem.menu = nil
    }

    private func item(_ title: String, action: Selector) -> NSMenuItem {
        let item = NSMenuItem(title: title, action: action, keyEquivalent: "")
        item.target = self
        return item
    }

    @objc private func openMaine() { showMainWindow() }

    @objc private func lockVault() {
        guard let appState, let sessions else { return }
        VaultCommands.lock(appState: appState, sessions: sessions)
    }

    @objc private func unlockVault() {
        showMainWindow()
        appState?.isVaultSheetPresented = true
    }

    @objc private func quitMaine() { NSApp.terminate(nil) }

    // MARK: - Menu bar mark

    /// The angular M from `scripts/render-mark.swift`, redrawn small and as a
    /// template image so the menu bar picks its own light or dark color.
    private static func menuBarMark() -> NSImage {
        let image = NSImage(size: NSSize(width: 18, height: 18), flipped: false) { _ in
            let width: CGFloat = 13
            let height = width * 430 / 540
            let x = (18 - width) / 2
            let y = (18 - height) / 2
            let path = NSBezierPath()
            path.move(to: NSPoint(x: x, y: y))
            path.line(to: NSPoint(x: x, y: y + height))
            path.line(to: NSPoint(x: x + width / 2, y: y + height * 105 / 430))
            path.line(to: NSPoint(x: x + width, y: y + height))
            path.line(to: NSPoint(x: x + width, y: y))
            path.lineJoinStyle = .bevel
            path.lineCapStyle = .square
            path.lineWidth = width * 112 / 540
            NSColor.black.setStroke()
            path.stroke()
            return true
        }
        image.isTemplate = true
        return image
    }
}
