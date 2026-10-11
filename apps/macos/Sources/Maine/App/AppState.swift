import Foundation

/// Window level UI state that the menus and the views both need to reach.
@MainActor
public final class AppState: ObservableObject {
    /// The host being edited in the sheet. Non-nil means the sheet is up; the
    /// value is the draft, so closing the sheet discards the user's typing.
    @Published public var editorDraft: Host?
    @Published public var isVaultSheetPresented = false
    @Published public private(set) var isVaultUnlocked = false
    @Published public private(set) var vaultStartupError: String?
    @Published public var alert: AlertMessage?

    private let passwords: PasswordStore
    private var preparedForLaunch = false

    public init(passwords: PasswordStore = .shared) {
        self.passwords = passwords
    }

    /// A fresh app launch always requires the master password, even if a helper
    /// key was left in Keychain by a crash. Re-rendering the window does not.
    public func prepareForLaunch() {
        guard !preparedForLaunch else { return }
        preparedForLaunch = true
        do { try passwords.lock() }
        catch { vaultStartupError = error.localizedDescription }
        isVaultUnlocked = false
        isVaultSheetPresented = true
    }

    public func vaultDidUnlock() {
        isVaultUnlocked = true
        vaultStartupError = nil
    }

    public func lockVault() {
        do {
            try passwords.lock()
            isVaultUnlocked = false
        } catch {
            showAlert(title: "Vault could not be locked", error.localizedDescription)
        }
    }

    public func canSaveHost(_ host: Host, password: String) -> Bool {
        host.isValid && isVaultUnlocked && !password.isEmpty
    }

    public struct AlertMessage: Identifiable, Equatable {
        public let id = UUID()
        public var title: String
        public var message: String

        public init(title: String, message: String) {
            self.title = title
            self.message = message
        }
    }

    // MARK: - Host editor

    public func addHost() {
        editorDraft = Host.blank
    }

    public func edit(_ host: Host) {
        editorDraft = host
    }

    public func dismissEditor() {
        editorDraft = nil
    }

    // MARK: - Alerts

    public func showAlert(title: String = "Something went wrong", _ message: String) {
        alert = AlertMessage(title: title, message: message)
    }

    public func dismissAlert() {
        alert = nil
    }
}
