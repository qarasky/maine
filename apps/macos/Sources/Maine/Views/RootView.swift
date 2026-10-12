import SwiftUI

/// The window: hosts on the left, the selected host's terminal on the right.
struct RootView: View {
    @EnvironmentObject private var store: HostStore
    @EnvironmentObject private var sessions: SessionManager
    @EnvironmentObject private var appState: AppState
    @Environment(\.openWindow) private var openWindow
    @State private var initialized = false

    var body: some View {
        GeometryReader { viewport in
            HSplitView {
                HostListView()
                    .frame(minWidth: 220, idealWidth: DesktopMetrics.sidebarWidth, maxWidth: 340)
                    .layoutPriority(1)
                    .background(InitialSidebarWidth().frame(width: 0, height: 0))
                WorkspaceView()
                    .frame(minWidth: 360, maxWidth: .infinity, maxHeight: .infinity)
            }
            // The window determines the split view's size, never the terminal's
            // fitting size or a newly inserted error banner's ideal height.
            .frame(width: viewport.size.width, height: viewport.size.height, alignment: .topLeading)
            .toolbar(.hidden, for: .windowToolbar)
            .background(MainWindowBridge().frame(width: 0, height: 0))
        }
        .frame(minWidth: 780, minHeight: 520)
        .debugRegion("root")
        .task {
            DebugDiagnostics.sessionManager = sessions
            DebugLog.shared.record("ui.root-task", fields: ["initialized": String(initialized)])
            // The resident menu bar item must always be able to reach these
            // objects, including after the window was closed and reopened.
            MenuBarController.shared.configure(appState: appState, sessions: sessions) {
                openWindow(id: "main")
            }
            guard !initialized else { return }
            initialized = true
            appState.prepareForLaunch()
            store.load()
        }
        .onAppear { DebugLog.shared.record("ui.root-appear") }
        .onDisappear {
            DebugLog.shared.record("ui.root-disappear", flush: true)
            do { try sessions.checkpointSessions() }
            catch PasswordStore.VaultError.locked { }
            catch { store.reportSaveFailure(error.localizedDescription) }
        }
        .sheet(item: $appState.editorDraft) { draft in
            HostEditorSheet(source: draft) { savedHost in
                save(draft: savedHost)
            }
        }
        .onChange(of: appState.isVaultUnlocked) { _, _ in
            store.load()
            if appState.isVaultUnlocked {
                do { try sessions.restoreSavedSessions() }
                catch { appState.showAlert(title: "Saved sessions not restored", error.localizedDescription) }
            }
        }
        .sheet(isPresented: $appState.isVaultSheetPresented) {
            VaultUnlockSheet(initialError: appState.vaultStartupError) {
                if let session = sessions.selectedSession, session.needsVaultUnlock {
                    sessions.reconnectSession(session.id)
                }
            }
        }
        .alert(item: $appState.alert) { message in
            Alert(
                title: Text(message.title),
                message: Text(message.message),
                dismissButton: .cancel(Text("OK"))
            )
        }
    }

    /// The editor is used for both new and existing hosts: saving a host whose
    /// id is already in the list updates it, anything else is added.
    private func save(draft: Host) {
        // The editor has already committed metadata and password atomically.
        appState.dismissEditor()
    }
}
