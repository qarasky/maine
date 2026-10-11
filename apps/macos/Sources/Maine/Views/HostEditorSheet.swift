import SwiftUI

/// The sheet that adds and edits a single host.
///
/// The same sheet is used for both: a host whose id is already in the store is
/// updated, a new one is added. The draft lives in `AppState` rather than in the
/// view, so the menu commands and the sheet cannot disagree about what is
/// being edited.
struct HostEditorSheet: View {
    @EnvironmentObject private var store: HostStore
    @EnvironmentObject private var appState: AppState

    /// The host this presentation of the sheet was opened for.
    let source: Host
    private let onSave: (Host) -> Void

    @FocusState private var focusedField: Field?
    @State private var password = ""
    @State private var passwordError: String?
    @State private var showVaultUnlock = false

    private enum Field: Hashable {
        case name, hostname, port, username
    }

    init(source: Host, onSave: @escaping (Host) -> Void) {
        self.source = source
        self.onSave = onSave
    }

    /// The draft being edited. Reads and writes go straight to `AppState`.
    private var draft: Binding<Host> {
        Binding(
            get: { appState.editorDraft ?? source },
            set: { appState.editorDraft = $0 }
        )
    }

    private var isNewHost: Bool { store.host(id: draft.wrappedValue.id) == nil }

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            form
            Divider()
            buttons
        }
        .frame(width: 480)
        .onAppear {
            focusedField = .name
            loadPassword()
        }
        .sheet(isPresented: $showVaultUnlock) {
            VaultUnlockSheet { loadPassword() }
        }
        .onChange(of: appState.isVaultUnlocked) { _, unlocked in
            if unlocked { loadPassword() }
            else { password = ""; passwordError = nil }
        }
    }

    // MARK: - Pieces

    private var header: some View {
        HStack {
            Text(isNewHost ? "New Host" : "Edit Host")
                .font(.headline)
            Spacer()
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 14)
    }

    private var form: some View {
        Form {
            Section {
                TextField("Name", text: draft.name)
                    .focused($focusedField, equals: .name)

                LabeledContent("IP / Hostname") {
                    HStack(spacing: 8) {
                        TextField("192.168.x.x", text: draft.hostname)
                            .labelsHidden()
                            .accessibilityLabel("IP / Hostname")
                            .focused($focusedField, equals: .hostname)
                        TextField("22", text: portBinding)
                            .labelsHidden()
                            .accessibilityLabel("Port")
                            .focused($focusedField, equals: .port)
                            .frame(width: 64)
                    }
                }

                TextField("Username", text: draft.username)
                    .focused($focusedField, equals: .username)

                SecureField("Password", text: $password)
                    .disabled(!appState.isVaultUnlocked)
                if !appState.isVaultUnlocked {
                    Button(PasswordStore.shared.exists ? "Unlock Password Vault…" : "Create Password Vault…") {
                        showVaultUnlock = true
                    }
                }
                if let passwordError {
                    Text(passwordError).font(.callout).foregroundStyle(.red)
                }
            } footer: {
                if draft.wrappedValue.issues.isEmpty == false {
                    VStack(alignment: .leading, spacing: 4) {
                        ForEach(draft.wrappedValue.issues, id: \.self) { issue in
                            Label(issue, systemImage: "exclamationmark.circle.fill")
                                .foregroundStyle(.orange)
                                .font(.callout)
                        }
                    }
                    .padding(.top, 6)
                }
            }
        }
        .formStyle(.grouped)
    }

    /// A text view of the port, so partial input is visible while typing.
    private var portBinding: Binding<String> {
        Binding(
            get: { String(draft.wrappedValue.port) },
            set: { newValue in
                let digits = newValue.filter { $0.isNumber }
                draft.wrappedValue.port = Int(digits) ?? Host.defaultPort
            }
        )
    }

    private var buttons: some View {
        HStack {
            Button("Cancel") { appState.dismissEditor() }
                .keyboardShortcut(.cancelAction)

            Spacer()

            Button(isNewHost ? "Add Host" : "Save Changes") {
                do {
                    guard appState.canSaveHost(draft.wrappedValue, password: password) else { return }
                    try store.saveHost(draft.wrappedValue, password: password)
                    onSave(draft.wrappedValue)
                } catch {
                    if case PasswordStore.VaultError.locked = error {
                        showVaultUnlock = true
                    } else {
                        appState.showAlert(title: "Password not saved", error.localizedDescription)
                    }
                }
            }
            .buttonStyle(.borderedProminent)
            .disabled(!appState.canSaveHost(draft.wrappedValue, password: password) || passwordError != nil)
            .keyboardShortcut(.defaultAction)
        }
        .padding(.horizontal, 20)
        .padding(.vertical, 14)
    }

    private func loadPassword() {
        guard appState.isVaultUnlocked else { password = ""; return }
        do {
            password = try PasswordStore.shared.password(for: source.id) ?? ""
            passwordError = nil
        } catch {
            password = ""
            passwordError = error.localizedDescription
        }
    }
}
