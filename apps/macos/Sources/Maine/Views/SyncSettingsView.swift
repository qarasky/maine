import SwiftUI

/// Settings → Sync: the pairings on this device, the active workspace, and the
/// one-time token flows. The status vocabulary mirrors
/// `SyncCoordinator.Status`.
struct SyncSettingsView: View {
    @ObservedObject private var coordinator = SyncCoordinator.shared
    @ObservedObject private var profiles: SyncProfileStore

    @State private var sheet: SyncSheet?
    @State private var renameTarget: SyncProfile?
    @State private var renameText = ""
    @State private var rotateTarget: SyncProfile?
    @State private var deleteTarget: SyncProfile?
    @State private var error: String?

    private enum SyncSheet: Identifiable {
        case create
        case join
        case token(title: String, token: String)

        var id: String {
            switch self {
            case .create: "create"
            case .join: "join"
            case .token(let title, _): "token-\(title)"
            }
        }
    }

    init() {
        _profiles = ObservedObject(wrappedValue: SyncCoordinator.shared.profiles)
    }

    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            Text("Sync")
                .font(.system(size: 15, weight: .semibold))
            Text("Optional and self-hosted. The server holds the encrypted vault byte for byte — never a password, a hostname or a token.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .fixedSize(horizontal: false, vertical: true)

            if profiles.profiles.isEmpty {
                Text("No sync profiles on this device.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .padding(.vertical, 6)
            } else {
                VStack(spacing: 2) {
                    ForEach(profiles.profiles) { profile in
                        profileRow(profile)
                    }
                }
                .padding(.vertical, 4)
            }

            if let error {
                Text(error)
                    .font(.caption)
                    .foregroundStyle(.red)
                    .fixedSize(horizontal: false, vertical: true)
            }

            Divider()

            HStack(spacing: 8) {
                Button("Create New Sync…") { sheet = .create }
                Button("Join Existing…") { sheet = .join }
                Spacer()
                HStack(spacing: 6) {
                    if coordinator.activeProfile != nil {
                        SyncStatusDot(status: coordinator.status)
                    }
                    Text(coordinator.status.label)
                        .font(.system(size: 11))
                        .foregroundStyle(.secondary)
                        .lineLimit(1)
                }
            }
            .buttonStyle(MotionButtonStyle())
        }
        .padding(20)
        .frame(width: 560)
        .sheet(item: $sheet) { item in
            switch item {
            case .create:
                CreateSyncSheet { created in
                    sheet = .token(title: "Sync created — this token is shown once", token: created.token)
                }
            case .join:
                JoinSyncSheet { sheet = nil }
            case .token(let title, let token):
                TokenRevealView(title: title, token: token) { sheet = nil }
            }
        }
        .alert("Rename Sync Profile", isPresented: presenting($renameTarget), presenting: renameTarget) { profile in
            TextField("Name", text: $renameText)
            Button("Rename") {
                let name = renameText.trimmingCharacters(in: .whitespacesAndNewlines)
                if !name.isEmpty {
                    coordinator.profiles.update(id: profile.id) { $0.name = name }
                }
                renameTarget = nil
            }
            Button("Cancel", role: .cancel) { renameTarget = nil }
        }
        .alert("Rotate Token?", isPresented: presenting($rotateTarget), presenting: rotateTarget) { profile in
            Button("Rotate", role: .destructive) { rotate(profile) }
            Button("Cancel", role: .cancel) { rotateTarget = nil }
        } message: { _ in
            Text("The old token stops working immediately. If the response is lost, the slot becomes unreachable — there is no recovery.")
        }
        .alert("Delete Synced Data?", isPresented: presenting($deleteTarget), presenting: deleteTarget) { profile in
            Button("Delete", role: .destructive) { deleteRemote(profile) }
            Button("Cancel", role: .cancel) { deleteTarget = nil }
        } message: { profile in
            Text("This erases the slot for “\(profile.name)” and every stored version on the server. The local vault is not touched.")
        }
    }

    // MARK: - Rows

    private func profileRow(_ profile: SyncProfile) -> some View {
        HStack(spacing: 10) {
            Image(systemName: profiles.activeProfileID == profile.id ? "largecircle.fill.circle" : "circle")
                .foregroundStyle(profiles.activeProfileID == profile.id ? Color.accentColor : Color.secondary)
            VStack(alignment: .leading, spacing: 2) {
                Text(profile.name)
                    .font(.system(size: 13, weight: .medium))
                    .lineLimit(1)
                Text(profile.serverURL.absoluteString)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 8)
            Text(syncedLabel(profile))
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
            Menu {
                Button("Rename…") {
                    renameText = profile.name
                    renameTarget = profile
                }
                Button("Rotate Token…") { rotateTarget = profile }
                Divider()
                Button("Disable Sync") { coordinator.disable(profile.id) }
                Button("Delete Synced Data…", role: .destructive) { deleteTarget = profile }
            } label: {
                Image(systemName: "ellipsis.circle")
            }
            .menuStyle(.borderlessButton)
            .fixedSize()
        }
        .padding(.vertical, 4)
        .padding(.horizontal, 8)
        .contentShape(Rectangle())
        .onTapGesture {
            guard profiles.activeProfileID != profile.id else { return }
            Task { await coordinator.activate(profile.id) }
        }
        .help(profiles.activeProfileID == profile.id ? "Active workspace" : "Switch to this workspace")
    }

    private func syncedLabel(_ profile: SyncProfile) -> String {
        guard let date = profile.lastSyncedAt else { return "never synced" }
        let formatter = RelativeDateTimeFormatter()
        formatter.unitsStyle = .short
        return formatter.localizedString(for: date, relativeTo: Date())
    }

    // MARK: - Actions

    private func rotate(_ profile: SyncProfile) {
        rotateTarget = nil
        Task {
            do {
                let token = try await coordinator.rotateToken(for: profile.id)
                sheet = .token(title: "New token — this is the only time it can be shown", token: token)
            } catch {
                self.error = error.localizedDescription
            }
        }
    }

    private func deleteRemote(_ profile: SyncProfile) {
        deleteTarget = nil
        Task {
            do { try await coordinator.deleteRemoteData(profile.id) }
            catch { self.error = error.localizedDescription }
        }
    }

    private func presenting<T>(_ value: Binding<T?>) -> Binding<Bool> {
        Binding(
            get: { value.wrappedValue != nil },
            set: { if !$0 { value.wrappedValue = nil } }
        )
    }
}

/// The small per-profile status dot used in Settings and the sidebar footer.
struct SyncStatusDot: View {
    let status: SyncCoordinator.Status

    var body: some View {
        Circle()
            .fill(color)
            .frame(width: 6, height: 6)
    }

    private var color: Color {
        switch status {
        case .idle: return .green
        case .syncing: return .accentColor
        case .offline: return .orange
        case .conflicted, .failed, .tokenRejected: return .red
        case .off: return .secondary
        }
    }
}
