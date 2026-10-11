import SwiftUI

struct VaultUnlockSheet: View {
    @Environment(\.dismiss) private var dismiss
    @EnvironmentObject private var appState: AppState
    let onUnlock: () -> Void
    @State private var password = ""
    @State private var confirmation = ""
    @State private var error: String?
    @State private var working = false
    private let creating = !PasswordStore.shared.exists

    init(initialError: String? = nil, onUnlock: @escaping () -> Void) {
        self.onUnlock = onUnlock
        _error = State(initialValue: initialError)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 16) {
            Text(creating ? "Create Password Vault" : "Unlock Password Vault").font(.headline)
            Text("Enter your master password to unlock your encrypted hosts and passwords for this app session.")
                .font(.callout).foregroundStyle(.secondary)
            SecureField("Master password", text: $password)
            if creating {
                SecureField("Confirm master password", text: $confirmation)
                Text("At least 12 characters. A forgotten master password cannot be recovered.")
                    .font(.caption).foregroundStyle(.secondary)
            }
            if let error { Text(error).foregroundStyle(.red).font(.callout) }
            HStack {
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                    .disabled(working)
                Spacer()
                if working { ProgressView().controlSize(.small) }
                Button(creating ? "Create Vault" : "Unlock") { unlock() }
                    .keyboardShortcut(.defaultAction)
                    .disabled(working || password.isEmpty || (creating && (password.count < 12 || password != confirmation)))
            }
        }
        .padding(24)
        .frame(width: 440)
        .interactiveDismissDisabled(working)
    }

    private func unlock() {
        working = true
        error = nil
        let masterPassword = password
        Task {
            do {
                try await Task.detached {
                    try PasswordStore.shared.unlock(masterPassword: masterPassword)
                }.value
                password = ""
                confirmation = ""
                appState.vaultDidUnlock()
                onUnlock()
                dismiss()
            } catch {
                self.error = error.localizedDescription
            }
            working = false
        }
    }
}
