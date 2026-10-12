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
        statusItem = item
    }

    /// Connects the resident UI to the vault and session state. Idempotent:
    /// the window calls this again every time it is reopened.
    func configure(appState: AppState, sessions: SessionManager, openWindow: @escaping () -> Void) {
        self.appState = appState
        self.sessions = sessions
        openMainWindow = openWindow
    }

    /// Tracks the main window, including the nil that SwiftUI reports when it
    /// destroys the window on close.
    func trackMainWindow(_ window: NSWindow?) {
        mainWindow = window
    }

    /// Brings the main window back, recreating it when it was closed.
    func showMainWindow() {
        openMainWindow?()
        if let mainWindow {
            if mainWindow.isMiniaturized { mainWindow.deminiaturize(nil) }
            mainWindow.makeKeyAndOrderFront(nil)
        }
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
