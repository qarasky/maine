import SwiftUI

/// The detail pane. Session navigation and closing live in the sidebar.
struct WorkspaceView: View {
    @EnvironmentObject private var sessions: SessionManager
    @EnvironmentObject private var appState: AppState
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        ZStack {
            ContentUnavailableView {
                Label(appState.isVaultUnlocked ? "Select a Host" : "Vault Locked", systemImage: appState.isVaultUnlocked ? "terminal" : "lock")
            } description: {
                Text(appState.isVaultUnlocked ? "Choose a host in the sidebar to open a terminal." : "Unlock your vault to access hosts and sessions.")
            } actions: {
                if appState.isVaultUnlocked {
                    Button("New Host…") { appState.addHost() }
                } else {
                    Button("Unlock Vault…") { appState.isVaultSheetPresented = true }
                }
            }
                .debugRegion("workspace-empty-state")
                .frame(maxWidth: .infinity, maxHeight: .infinity, alignment: .center)
                .opacity(sessions.selectedSession == nil ? 1 : 0)
                .allowsHitTesting(sessions.selectedSession == nil)
                .accessibilityHidden(sessions.selectedSession != nil)
            // Sidebar session switches never reparent a terminal NSView.
            ForEach(sessions.openSessions) { session in
                let active = sessions.selectedSessionID == session.id
                SessionView(session: session, isActive: active)
                    .opacity(active ? 1 : 0)
                    .allowsHitTesting(active)
                    .accessibilityHidden(!active)
            }
        }
        .frame(minWidth: 0, maxWidth: .infinity, minHeight: 0, maxHeight: .infinity, alignment: .topLeading)
        // Animate visibility, not terminal identity or its AppKit parent.
        .animation(reduceMotion ? nil : .easeInOut(duration: 0.18), value: sessions.selectedSessionID)
        .background(Color(NSColor.textBackgroundColor))
        .debugRegion("workspace")
    }
}
