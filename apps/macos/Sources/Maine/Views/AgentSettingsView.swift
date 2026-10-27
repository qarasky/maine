import AppKit
import SwiftUI

/// Settings → Agents: the master switch, paired MCP clients, and which
/// groups and hosts agents may use on this Mac.
struct AgentSettingsView: View {
    @ObservedObject private var coordinator = AgentCoordinator.shared
    @State private var addingClient = false
    @State private var pairing: AgentCoordinator.Pairing?
    @State private var revokeTarget: AgentClient?

    var body: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 14) {
                Text("Agents")
                    .font(.system(size: 15, weight: .semibold))
                Text("Let MCP clients such as Claude Code drive SSH sessions in Maine. Their sessions appear in the sidebar, where you can watch, type, pause or stop them.")
                    .font(.callout)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)

                Toggle("Allow agent access", isOn: Binding(get: { coordinator.isEnabled },
                                                           set: { coordinator.setEnabled($0) }))
                if let error = coordinator.lastError {
                    Label(error, systemImage: "exclamationmark.triangle.fill")
                        .font(.callout).foregroundStyle(.orange)
                }
                Text("Terminal output of agent sessions and the names and addresses of allowed hosts are sent to the agent and its model provider. Allowed hosts are fully usable by agents, including typing their saved passwords into password prompts. Any program running as you can read a client's configuration and reuse its token.")
                    .font(.caption)
                    .foregroundStyle(.secondary)
                    .fixedSize(horizontal: false, vertical: true)

                Divider()
                clientsSection
                Divider()
                accessSection
            }
            .padding(20)
        }
        .frame(width: 560, height: 520)
        .sheet(isPresented: $addingClient) {
            AddAgentClientSheet { pairing = $0 }
        }
        .sheet(item: Binding(get: { pairing.map(PairingBox.init) }, set: { if $0 == nil { pairing = nil } })) { box in
            PairingSheet(pairing: box.pairing) { pairing = nil }
        }
        .confirmationDialog("Revoke \(revokeTarget?.name ?? "client")?", isPresented: Binding(
            get: { revokeTarget != nil }, set: { if !$0 { revokeTarget = nil } })) {
            Button("Revoke", role: .destructive) {
                if let revokeTarget { coordinator.revoke(revokeTarget.id) }
                revokeTarget = nil
            }
        } message: {
            Text("It disconnects now and its token stops working. Its sessions stay open for you.")
        }
    }

    private var clientsSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            HStack {
                Text("Clients").font(.headline)
                Spacer()
                Button("Add Client…") { addingClient = true }
            }
            if coordinator.clients.isEmpty {
                Text("No clients yet. Add one to get its setup command.")
                    .font(.callout).foregroundStyle(.secondary)
            }
            ForEach(coordinator.clients) { client in
                HStack {
                    Image(systemName: "cpu").foregroundStyle(.secondary)
                    Text(client.name)
                    Text(client.createdAt, style: .date).font(.caption).foregroundStyle(.secondary)
                    Spacer()
                    Button("Revoke…") { revokeTarget = client }
                }
            }
        }
    }

    @ViewBuilder
    private var accessSection: some View {
        VStack(alignment: .leading, spacing: 8) {
            Text("Access").font(.headline)
            if let store = coordinator.store {
                AgentAccessList(store: store)
            } else {
                Text("Open the Maine window to choose hosts.").font(.callout).foregroundStyle(.secondary)
            }
        }
    }

    private struct PairingBox: Identifiable {
        let pairing: AgentCoordinator.Pairing
        var id: UUID { pairing.client.id }
    }
}

/// One checkbox per group and per host, in sidebar order. A host inside an
/// allowed group is shown checked and disabled: the group already covers it.
private struct AgentAccessList: View {
    @ObservedObject var store: HostStore
    @ObservedObject private var coordinator = AgentCoordinator.shared

    var body: some View {
        let layout = store.layout
        let access = coordinator.access()
        VStack(alignment: .leading, spacing: 6) {
            if store.hosts.isEmpty && store.groups.isEmpty {
                Text("Unlock the vault to choose hosts.").font(.callout).foregroundStyle(.secondary)
            }
            ForEach(layout.ungrouped) { host in
                hostToggle(host, access: access, coveredByGroup: false)
            }
            ForEach(layout.groups) { group in
                Toggle(isOn: Binding(get: { access.groups.contains(group.id) }, set: { on in
                    var next = access
                    if on { next.groups.insert(group.id) } else { next.groups.remove(group.id) }
                    coordinator.setAccess(next)
                })) {
                    Label(group.name, systemImage: "folder").font(.system(size: 13, weight: .semibold))
                }
                ForEach(layout.hosts(in: group.id)) { host in
                    hostToggle(host, access: access, coveredByGroup: access.groups.contains(group.id))
                        .padding(.leading, 20)
                }
            }
        }
    }

    private func hostToggle(_ host: Host, access: AgentAccess, coveredByGroup: Bool) -> some View {
        Toggle(isOn: Binding(get: { coveredByGroup || access.hosts.contains(host.id) }, set: { on in
            var next = access
            if on { next.hosts.insert(host.id) } else { next.hosts.remove(host.id) }
            coordinator.setAccess(next)
        })) {
            VStack(alignment: .leading, spacing: 1) {
                Text(host.displayName)
                Text(host.subtitle).font(.caption).foregroundStyle(.secondary)
            }
        }
        .disabled(coveredByGroup)
    }
}

private struct AddAgentClientSheet: View {
    @Environment(\.dismiss) private var dismiss
    @State private var name = "Claude Code"
    @State private var error: String?
    let onPaired: (AgentCoordinator.Pairing) -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Add Client").font(.headline)
            TextField("Name", text: $name)
            if let error { Text(error).font(.callout).foregroundStyle(.red) }
            HStack {
                Spacer()
                Button("Cancel") { dismiss() }.keyboardShortcut(.cancelAction)
                Button("Add") {
                    do {
                        let pairing = try AgentCoordinator.shared.addClient(name: name)
                        dismiss()
                        onPaired(pairing)
                    } catch {
                        self.error = error.localizedDescription
                    }
                }
                .keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 380)
    }
}

/// Shown once: the token only exists here and in the client's configuration.
private struct PairingSheet: View {
    let pairing: AgentCoordinator.Pairing
    let onDone: () -> Void

    var body: some View {
        VStack(alignment: .leading, spacing: 12) {
            Text("Set up \(pairing.client.name)").font(.headline)
            Text("This token is shown only once. Copy one of these into the client, then close this sheet.")
                .font(.callout).foregroundStyle(.secondary)
            snippet("Claude Code (run in Terminal)", pairing.claudeCodeCommand)
            snippet("Claude Desktop (claude_desktop_config.json)", pairing.desktopJSON)
            HStack {
                Spacer()
                Button("Done", action: onDone).keyboardShortcut(.defaultAction)
            }
        }
        .padding(20)
        .frame(width: 560)
    }

    private func snippet(_ title: String, _ text: String) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            HStack {
                Text(title).font(.subheadline.weight(.semibold))
                Spacer()
                Button("Copy") {
                    NSPasteboard.general.clearContents()
                    NSPasteboard.general.setString(text, forType: .string)
                }
            }
            Text(text)
                .font(.system(.caption, design: .monospaced))
                .textSelection(.enabled)
                .padding(8)
                .frame(maxWidth: .infinity, alignment: .leading)
                .background(Color.primary.opacity(0.05), in: RoundedRectangle(cornerRadius: 6))
        }
    }
}
