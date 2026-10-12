import Foundation

/// The one place that knows how to lock the vault: session history is
/// checkpointed first, and a failed checkpoint leaves the vault open so the
/// user can retry. Both the Host menu and the menu bar item call this.
@MainActor
enum VaultCommands {
    static func lock(appState: AppState, sessions: SessionManager) {
        do {
            try sessions.checkpointSessions()
            appState.lockVault()
        } catch {
            appState.showAlert(title: "Sessions could not be saved", error.localizedDescription)
        }
    }
}
