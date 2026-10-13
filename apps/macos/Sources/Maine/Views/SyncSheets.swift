import AppKit
import SwiftUI

/// Create a slot: this device becomes the origin. The token is shown once.
struct CreateSyncSheet: View {
    @Environment(\.dismiss) private var dismiss
    let onCreated: (SyncCoordinator.CreatedProfile) -> Void

    @State private var server = ""
    @State private var name = ""
    @State private var createSecret = ""
    @State private var needsSecret = false
    @State private var error: String?
    @State private var working = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Create a sync slot").font(.headline)
            Text("This device becomes the origin. The server returns a token exactly once; store it somewhere safe.")
                .font(.callout).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            TextField("http://127.0.0.1:8484", text: $server)
            TextField("Name (optional)", text: $name)
            if needsSecret {
                SecureField("Create secret", text: $createSecret)
                Text("This server requires a create secret (its --create-secret setting).")
                    .font(.caption).foregroundStyle(.secondary)
            }
            if let error {
                Text(error).font(.caption).foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Create") { create() }.keyboardShortcut(.defaultAction)
                    .disabled(working || server.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
            }
            .buttonStyle(MotionButtonStyle())
        }
        .padding(20)
        .frame(width: 460)
    }

    private func create() {
        guard let url = URL.syncBase(from: server) else {
            error = "Enter a server URL, for example http://127.0.0.1:8484"
            return
        }
        working = true
        error = nil
        let trimmedName = name.trimmingCharacters(in: .whitespacesAndNewlines)
        Task {
            do {
                let created = try await SyncCoordinator.shared.createProfile(
                    server: url,
                    name: trimmedName.isEmpty ? (url.host ?? "Sync") : trimmedName,
                    createSecret: needsSecret ? createSecret : nil
                )
                onCreated(created)
            } catch let transportError as SyncTransportError {
                if case .forbidden = transportError { needsSecret = true }
                error = transportError.localizedDescription
            } catch {
                self.error = error.localizedDescription
            }
            working = false
        }
    }
}

/// Join an existing slot: paste the token, then the master password of the
/// vault it holds. Nothing changes until that password has been verified.
struct JoinSyncSheet: View {
    @Environment(\.dismiss) private var dismiss
    let onFinished: () -> Void

    @State private var server = ""
    @State private var token = ""
    @State private var name = ""
    @State private var masterPassword = ""
    @State private var pendingEmptySlot = false
    @State private var error: String?
    @State private var working = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Join a sync slot").font(.headline)
            Text("Enter the server and the token it returned when the slot was created.")
                .font(.callout).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            TextField("http://127.0.0.1:8484", text: $server)
            TextField("maine-XXXX-XXXX-…", text: $token)
                .font(.system(.body, design: .monospaced))
            TextField("Name (optional)", text: $name)
            SecureField("Master password of that vault", text: $masterPassword)
            Text("Two devices can only sync if they open the same vault, which in v1 means the same master password.")
                .font(.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            if let error {
                Text(error).font(.caption).foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
            }
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Join") { join() }.keyboardShortcut(.defaultAction)
                    .disabled(working
                              || server.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                              || token.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
                              || masterPassword.isEmpty)
            }
            .buttonStyle(MotionButtonStyle())
        }
        .padding(20)
        .frame(width: 480)
        .alert("This token points to an empty slot", isPresented: $pendingEmptySlot) {
            Button("Push This Device's Vault") { pushLocalVault() }
            Button("Cancel", role: .cancel) { }
        } message: {
            Text("No vault is stored yet. Push this device's vault as the first version, or cancel and create a new slot instead.")
        }
    }

    private func join() {
        guard let url = URL.syncBase(from: server) else {
            error = "Enter a server URL, for example http://127.0.0.1:8484"
            return
        }
        working = true
        error = nil
        let trimmedToken = token.trimmingCharacters(in: .whitespacesAndNewlines)
        Task {
            do {
                let outcome = try await SyncCoordinator.shared.joinProfile(
                    server: url,
                    token: trimmedToken,
                    name: profileName(for: url),
                    masterPassword: masterPassword
                )
                switch outcome {
                case .joined: onFinished()
                case .emptySlot: pendingEmptySlot = true
                }
            } catch {
                self.error = error.localizedDescription
            }
            working = false
        }
    }

    private func pushLocalVault() {
        guard let url = URL.syncBase(from: server) else { return }
        working = true
        error = nil
        let trimmedToken = token.trimmingCharacters(in: .whitespacesAndNewlines)
        Task {
            do {
                _ = try await SyncCoordinator.shared.pushLocalVaultIntoEmptySlot(
                    server: url,
                    token: trimmedToken,
                    name: profileName(for: url)
                )
                onFinished()
            } catch {
                self.error = error.localizedDescription
            }
            working = false
        }
    }

    private func profileName(for url: URL) -> String {
        let trimmed = name.trimmingCharacters(in: .whitespacesAndNewlines)
        return trimmed.isEmpty ? (url.host ?? "Sync") : trimmed
    }
}

/// The token is shown exactly once; after this view closes, only its hash
/// exists on the server and only the Keychain has the token itself.
struct TokenRevealView: View {
    let title: String
    let token: String
    let onDone: () -> Void

    @State private var copied = false

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(title).font(.headline)
            Text(token)
                .font(.system(.body, design: .monospaced))
                .textSelection(.enabled)
                .padding(8)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Color.secondary.opacity(0.12), in: RoundedRectangle(cornerRadius: 6))
            Text("Store it in a password manager now. The server keeps only its hash, so it cannot be shown again.")
                .font(.caption).foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                Button(copied ? "Copied" : "Copy Token") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(token, forType: .string)
                    copied = true
                }
                Spacer()
                Button("Done") { onDone() }.keyboardShortcut(.defaultAction)
            }
            .buttonStyle(MotionButtonStyle())
        }
        .padding(20)
        .frame(width: 460)
    }
}
