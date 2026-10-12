import SwiftUI

@main
enum MaineEntryPoint {
    @MainActor
    static func main() {
        if ProcessInfo.processInfo.environment["MAINE_SSH_ASKPASS"] == "1" {
            exit(SSHAskPass.run())
        }
        MaineApp.main()
    }
}

struct MaineApp: App {
    @NSApplicationDelegateAdaptor(MaineAppDelegate.self) private var appDelegate
    @StateObject private var store: HostStore
    @StateObject private var sessions: SessionManager
    @StateObject private var appState = AppState()
    @StateObject private var debugLog = DebugLog.shared

    init() {
        let store = HostStore()
        _store = StateObject(wrappedValue: store)
        _sessions = StateObject(wrappedValue: SessionManager(store: store))
    }

    var body: some Scene {
        Window("Maine", id: "main") {
            RootView()
                .environmentObject(store)
                .environmentObject(sessions)
                .environmentObject(appState)
        }
        .defaultSize(width: 1080, height: 720)
        .windowStyle(.hiddenTitleBar)
        .commands {
            CommandGroup(after: .textEditing) {
                Button("Search Hosts") { NotificationCenter.default.post(name: .maineFocusSearch, object: nil) }
                    .keyboardShortcut("f", modifiers: .command)
            }
            CommandMenu("Host") {
                Button("New Host…") { appState.addHost() }
                    .keyboardShortcut("n", modifiers: .command)

                Divider()

                Button("Connect to Selected Host") { connectSelectedHost() }
                    .keyboardShortcut(.return, modifiers: .command)

                Button("Reconnect to Selected Host") { reconnectSelectedHost() }
                    .keyboardShortcut("r", modifiers: [.command, .shift])

                Divider()

                Button("Unlock Password Vault…") { appState.isVaultSheetPresented = true }
                Button("Lock Password Vault") { VaultCommands.lock(appState: appState, sessions: sessions) }
                    .disabled(!appState.isVaultUnlocked)
                Button("Show Encrypted Vault") {
                    NSWorkspace.shared.activateFileViewerSelecting([PasswordStore.defaultFileURL])
                }

                Divider()

                Button("Delete Selected Host", role: .destructive) { deleteSelectedHost() }
            }

            CommandMenu("Session") {
                Button("Close Session") { closeSelectedSession() }
                    .keyboardShortcut("w", modifiers: [.command, .shift])

                Divider()

                Button("Close Every Session") {
                    guard appState.isVaultUnlocked else { appState.isVaultSheetPresented = true; return }
                    sessions.closeAll()
                }
            }

            CommandMenu("Debug") {
                Toggle("Enable Debug Logging", isOn: Binding(
                    get: { debugLog.isEnabled },
                    set: { enabled in
                        debugLog.setEnabled(enabled)
                        if enabled { DebugDiagnostics.capture("debug-enabled", sessions: sessions) }
                        if let error = debugLog.lastError { appState.showAlert(title: "Debug logging failed", error) }
                    }
                ))
                Button("Capture Diagnostic Snapshot") {
                    if !debugLog.isEnabled { debugLog.setEnabled(true) }
                    DebugDiagnostics.capture("manual", sessions: sessions)
                    if let error = debugLog.lastError { appState.showAlert(title: "Debug logging failed", error) }
                }
                .keyboardShortcut("l", modifiers: [.command, .option, .shift])
                Button("Show Debug Logs") {
                    let files = [debugLog.fileURL, debugLog.previousFileURL]
                        .filter { FileManager.default.fileExists(atPath: $0.path) }
                    if files.isEmpty {
                        appState.showAlert(title: "No debug log yet", "Enable Debug Logging first, then reproduce the problem.")
                    } else {
                        NSWorkspace.shared.activateFileViewerSelecting(files)
                    }
                }
                Button("Copy Debug Log Path") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(debugLog.fileURL.path, forType: .string)
                }
            }
        }
    }

    // MARK: - Menu actions

    private func connectSelectedHost() {
        if let session = sessions.selectedSession, !session.state.isLive {
            sessions.reconnectSession(session.id)
            return
        }
        guard let host = store.host(id: sessions.selectedHostID) else { return }
        sessions.open(host)
    }

    private func reconnectSelectedHost() {
        guard let id = sessions.selectedSessionID else { return }
        sessions.reconnectSession(id)
    }

    private func deleteSelectedHost() {
        guard let host = store.host(id: sessions.selectedHostID) else { return }
        sessions.delete(host)
    }

    private func closeSelectedSession() {
        guard let id = sessions.selectedSessionID else { return }
        guard appState.isVaultUnlocked else { appState.isVaultSheetPresented = true; return }
        sessions.closeSession(id)
    }
}

final class MaineAppDelegate: NSObject, NSApplicationDelegate {
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        do { try DebugDiagnostics.sessionManager?.shutdown() }
        catch PasswordStore.VaultError.locked {
            // Locking checkpoints first. Keep that encrypted snapshot on quit.
            DebugDiagnostics.sessionManager?.stopProcessesForQuit()
        } catch {
            let alert = NSAlert()
            alert.messageText = "Sessions could not be saved"
            alert.informativeText = error.localizedDescription + "\nMaine will stay open so you can retry."
            alert.runModal()
            return .terminateCancel
        }
        return .terminateNow
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        // Maine is a menu bar resident: the window can close while sessions
        // stay connected, so the process must not be reaped while idle. The
        // runtime opt-out also covers development runs without the bundle,
        // whose Info.plist is not read.
        ProcessInfo.processInfo.disableAutomaticTermination("Maine stays in the menu bar")
        NSApp.setActivationPolicy(.accessory)
        MenuBarController.shared.install()
        DebugLog.shared.record("application.launch", fields: [
            "format": "disconnect-diagnostics-v1",
            "executable": Bundle.main.executableURL?.path ?? CommandLine.arguments[0],
            "osVersion": ProcessInfo.processInfo.operatingSystemVersionString,
            "version": Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "development"
        ], flush: true)
    }

    func applicationShouldHandleReopen(_ sender: NSApplication, hasVisibleWindows flag: Bool) -> Bool {
        MenuBarController.shared.showMainWindow()
        return true
    }

    /// Maine outlives its window: closing it hides it so sessions and the
    /// unlocked vault keep running. Quitting is explicit (⌘Q or the status
    /// item's menu).
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool {
        false
    }

    func applicationWillTerminate(_ notification: Notification) {
        DebugLog.shared.record("application.quit", flush: true)
        try? PasswordStore.shared.lock()
    }
}
