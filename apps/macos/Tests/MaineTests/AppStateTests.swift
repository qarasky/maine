import Foundation
@testable import Maine
import Testing

@MainActor
@Suite
struct AppStateTests {
    @Test
    func everyLaunchLocksTheVaultAndRequestsTheMasterPassword() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let passwords = PasswordStore(fileURL: directory.appendingPathComponent("passwords.maine"),
                                      keychainService: "app.maine.tests.\(UUID())")
        defer {
            try? passwords.lock()
            try? FileManager.default.removeItem(at: directory)
        }
        let host = Host(name: "RPI", hostname: "raspberrypi.local")
        try passwords.unlock(masterPassword: "test launch master password")
        try passwords.save("test host password", for: host.id)

        let state = AppState(passwords: passwords)
        state.prepareForLaunch()
        #expect(state.isVaultSheetPresented)
        #expect(!state.isVaultUnlocked)
        #expect(throws: PasswordStore.VaultError.self) { try passwords.password(for: host.id) }
        try passwords.unlock(masterPassword: "test launch master password")
        state.vaultDidUnlock()
        state.isVaultSheetPresented = false
        // A window/body reappearance must not reload data or re-lock the vault.
        state.prepareForLaunch()
        #expect(state.isVaultUnlocked)
        #expect(!state.isVaultSheetPresented)
        #expect(try passwords.password(for: host.id) == "test host password")
        state.lockVault()
        #expect(!state.isVaultUnlocked)
        #expect(throws: PasswordStore.VaultError.self) { try passwords.password(for: host.id) }
    }

    @Test
    func passwordIsRequiredAndSavingIsDisabledWhileLocked() {
        let state = AppState()
        let host = Host(name: "RPI", hostname: "raspberrypi.local")
        #expect(!state.canSaveHost(host, password: "test password"))
        state.vaultDidUnlock()
        #expect(!state.canSaveHost(host, password: ""))
        #expect(state.canSaveHost(host, password: "test password"))
        #expect(!state.canSaveHost(Host.blank, password: "test password"))
    }
}
