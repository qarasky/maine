import AppKit
import Foundation

/// The one place that copies a host's credentials to the pasteboard. The
/// password comes from the vault, so a locked vault asks for the master
/// password first; nothing beyond the host ID is ever logged, and the copied
/// text is never written to the debug log.
@MainActor
enum CredentialCommands {
    /// Returns true when the block reached the pasteboard.
    @discardableResult
    static func copy(host: Host, appState: AppState,
                     passwords: PasswordStore = .shared,
                     pasteboard: NSPasteboard = .general) -> Bool {
        guard appState.isVaultUnlocked else {
            appState.isVaultSheetPresented = true
            return false
        }
        do {
            let password = try passwords.password(for: host.id)
            let text = CredentialText.make(host: host, password: password)
            pasteboard.clearContents()
            pasteboard.setString(text, forType: .string)
            DebugLog.shared.record("ui.copy-credentials", fields: ["host": host.id.uuidString])
            return true
        } catch {
            appState.showAlert(title: "Credentials not copied", error.localizedDescription)
            return false
        }
    }
}
