import AppKit
@testable import Maine
import Testing

@MainActor
@Suite
struct CredentialCommandsTests {
    private func temporaryStore() -> (PasswordStore, URL) {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let store = PasswordStore(fileURL: directory.appendingPathComponent("passwords.maine"),
                                  keychainService: "app.maine.tests.\(UUID())")
        return (store, directory)
    }

    /// A named pasteboard, so tests never touch the user's real clipboard.
    private func temporaryPasteboard() -> NSPasteboard {
        NSPasteboard(name: NSPasteboard.Name("app.maine.tests.\(UUID().uuidString)"))
    }

    @Test
    func copyWritesTheCredentialBlockIncludingTheSavedPassword() throws {
        let (passwords, directory) = temporaryStore()
        defer {
            try? passwords.lock()
            try? FileManager.default.removeItem(at: directory)
        }
        let host = Host(name: "Web", hostname: "10.0.0.4", port: 2222, username: "root")
        try passwords.unlock(masterPassword: "test copy credentials master")
        try passwords.save("abcdef123", for: host.id)

        let state = AppState(passwords: passwords)
        state.vaultDidUnlock()
        let pasteboard = temporaryPasteboard()

        CredentialCommands.copy(host: host, appState: state, passwords: passwords, pasteboard: pasteboard)

        #expect(pasteboard.string(forType: .string) == """
        [ip] 10.0.0.4
        [port] 2222
        [username] root
        [password] abcdef123
        """)
    }

    @Test
    func copyRequestsTheMasterPasswordWhileLocked() throws {
        let (passwords, directory) = temporaryStore()
        defer { try? FileManager.default.removeItem(at: directory) }
        let state = AppState(passwords: passwords)
        let pasteboard = temporaryPasteboard()

        CredentialCommands.copy(host: Host(name: "Web", hostname: "example.com"),
                                appState: state, passwords: passwords, pasteboard: pasteboard)

        #expect(state.isVaultSheetPresented)
        #expect(pasteboard.string(forType: .string) == nil)
    }
}
