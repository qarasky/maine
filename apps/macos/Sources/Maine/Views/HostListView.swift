import SwiftUI

/// The sidebar: every saved host, with the ones that have a session marked.
struct HostListView: View {
    @EnvironmentObject private var store: HostStore
    @EnvironmentObject private var sessions: SessionManager
    @EnvironmentObject private var appState: AppState

    @State private var filter = ""
    @State private var renamingSessionID: UUID?
    @State private var renameText = ""
    @State private var searchFocusRequest = 0
    @FocusState private var renameFocused: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var visibleHosts: [Host] {
        store.hosts.filter(matches)
    }

    private func matches(_ host: Host) -> Bool {
        let query = filter.trimmingCharacters(in: .whitespacesAndNewlines)
        guard query.isEmpty == false else { return true }
        return host.displayName.localizedCaseInsensitiveContains(query)
            || host.hostname.localizedCaseInsensitiveContains(query)
            || host.username.localizedCaseInsensitiveContains(query)
    }

    var body: some View {
        VStack(spacing: 0) {
            HostSearchField(text: $filter, focusRequest: searchFocusRequest)
                .frame(height: 28)
                .padding(.horizontal, DesktopMetrics.sidebarInset)
                .padding(.top, 10)
                .padding(.bottom, 8)
                .debugRegion("sidebar-search")
            if let failure = store.loadFailure {
                loadFailureBanner(failure)
            } else if let saveFailure = store.saveFailure {
                saveFailureBanner(saveFailure)
            }

            List {
                ForEach(visibleHosts) { host in
                    let hostSessions = sessions.sessions(for: host.id)
                    let showsChildren = hostSessions.count > 1 || hostSessions.contains {
                        $0.customName != nil || renamingSessionID == $0.id
                    }
                    VStack(spacing: 2) {
                        HStack(spacing: 0) {
                            Button {
                                if let session = sessions.session(for: host.id) { sessions.selectSession(session.id) }
                                else { sessions.open(host) }
                            } label: {
                                row(for: host)
                                    .frame(maxWidth: .infinity, alignment: .leading)
                                    .padding(.horizontal, DesktopMetrics.rowInset)
                                    .frame(height: DesktopMetrics.hostRowHeight)
                                    .contentShape(Rectangle())
                            }
                            .buttonStyle(.plain)
                            if !showsChildren, let session = hostSessions.first {
                                closeButton(session)
                                    .padding(.trailing, DesktopMetrics.rowInset)
                                    .transition(.opacity)
                            }
                        }
                        .modifier(SidebarRowHighlight(selected: !showsChildren && sessions.selectedHostID == host.id))
                        .debugRegion("host-row", sessionID: host.id)
                        .contextMenu { contextMenu(for: host) }

                        if showsChildren {
                            ForEach(hostSessions) { session in
                                sessionRow(session)
                                    .padding(.leading, DesktopMetrics.childIndent)
                                    .transition(reduceMotion ? .opacity : .move(edge: .top).combined(with: .opacity))
                            }
                        }
                    }
                    .listRowInsets(EdgeInsets(top: 1, leading: 0, bottom: 1, trailing: 0))
                    .listRowBackground(Color.clear)
                }
            }
            .listStyle(.sidebar)
            .contentMargins(.horizontal, DesktopMetrics.sidebarInset, for: .scrollContent)
            .contentMargins(.vertical, 0, for: .scrollContent)
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.22), value: sessions.openSessions.map(\.id))
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.22), value: sessions.openSessions.map { sessions.label(for: $0) })
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.22), value: sessions.openSessions.map(\.savedSessionID))
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.16), value: sessions.selectedSessionID)
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.16), value: renamingSessionID)
            .overlay {
                if visibleHosts.isEmpty {
                    VStack(spacing: 8) {
                        Text(!appState.isVaultUnlocked ? "Vault Locked" : (filter.isEmpty ? "No Hosts" : "No Results"))
                            .font(.system(size: 13, weight: .medium))
                        Text(!appState.isVaultUnlocked ? "Unlock to see your hosts." : (filter.isEmpty ? "Add a host to get started." : "Try another name or address."))
                            .font(.system(size: 12)).foregroundStyle(.secondary)
                        if !appState.isVaultUnlocked {
                            Button("Unlock Vault…") { appState.isVaultSheetPresented = true }
                        } else if filter.isEmpty {
                            Button("New Host…") { appState.addHost() }
                        }
                    }
                    .padding(16)
                    .frame(maxWidth: .infinity, maxHeight: .infinity)
                }
            }
            .onDeleteCommand {
                guard renamingSessionID == nil else { return }
                if let session = sessions.selectedSession { closeSession(session) }
            }

            Divider()

            footer
        }
        .background(.regularMaterial)
        .debugRegion("sidebar")
        .onReceive(NotificationCenter.default.publisher(for: .maineFocusSearch)) { _ in
            searchFocusRequest += 1
        }
        .onChange(of: renameFocused) { _, focused in
            if !focused { commitRename() }
        }
    }

    // MARK: - Rows

    private func closeButton(_ session: TerminalSession) -> some View {
        Button {
            DebugLog.shared.record("ui.sidebar-close-click", fields: session.debugFields, flush: true)
            closeSession(session)
        } label: {
            Image(systemName: "xmark")
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
                .frame(width: 24, height: 24)
                .contentShape(Rectangle())
        }
        .buttonStyle(MotionButtonStyle())
        .help("Close session")
        .accessibilityLabel("Close \(sessions.label(for: session)) for \(session.host.displayName)")
        .accessibilityIdentifier("close-session-\(session.id.uuidString)")
    }

    private func sessionRow(_ session: TerminalSession) -> some View {
        HStack(spacing: 0) {
            if renamingSessionID == session.id {
                sessionLabel(session)
            } else {
                Button { sessions.selectSession(session.id) } label: { sessionLabel(session) }
                    .buttonStyle(.plain)
                    .accessibilityIdentifier("select-session-\(session.id.uuidString)")
            }
            closeButton(session)
                .padding(.trailing, DesktopMetrics.rowInset)
        }
        .modifier(SidebarRowHighlight(selected: sessions.selectedSessionID == session.id))
        .debugRegion("session-row", sessionID: session.id)
        .help(session.state.label)
        .contextMenu {
            Button("Show Session") { sessions.selectSession(session.id) }
            sessionActions(session)
            Button("Reconnect") { sessions.reconnectSession(session.id) }
            Button("Close Session") { closeSession(session) }
        }
    }

    private func sessionLabel(_ session: TerminalSession) -> some View {
        HStack(spacing: 8) {
            Image(systemName: "terminal").font(.system(size: 12))
                .foregroundStyle(.secondary).frame(width: 16)
            if renamingSessionID == session.id {
                TextField("Session name", text: $renameText)
                    .textFieldStyle(.plain)
                    .focused($renameFocused)
                    .onAppear { renameFocused = true }
                    .onSubmit { commitRename() }
                    .onExitCommand { renamingSessionID = nil; renameFocused = false }
                    .accessibilityIdentifier("session-name-editor")
            } else {
                Text(sessions.label(for: session)).lineLimit(1)
                    .simultaneousGesture(TapGesture(count: 2).onEnded { beginRename(session) })
            }
            Spacer(minLength: 4)
            stateIndicator(for: session.state)
        }
        .font(.system(size: 12))
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(.horizontal, DesktopMetrics.rowInset)
        .frame(height: DesktopMetrics.sessionRowHeight)
        .contentShape(Rectangle())
    }

    private func commitRename() {
        guard let id = renamingSessionID else { return }
        let name = renameText.trimmingCharacters(in: .whitespacesAndNewlines)
        renamingSessionID = nil
        renameFocused = false
        if !name.isEmpty { performSessionAction { try sessions.renameSession(id, to: name) } }
    }

    @ViewBuilder
    private func sessionActions(_ session: TerminalSession) -> some View {
        Button("Rename Session…") {
            beginRename(session)
        }
    }

    private func beginRename(_ session: TerminalSession) {
        renamingSessionID = session.id
        renameText = sessions.label(for: session)
        sessions.selectSession(session.id)
    }

    private func closeSession(_ session: TerminalSession) {
        guard appState.isVaultUnlocked || session.savedSessionID == nil else {
            appState.isVaultSheetPresented = true
            return
        }
        sessions.closeSession(session.id)
    }

    private func performSessionAction(_ action: () throws -> Void) {
        do { try action() }
        catch { appState.showAlert(title: "Session not saved", error.localizedDescription) }
    }

    private func row(for host: Host) -> some View {
        let session = sessions.session(for: host.id)
        return HStack(spacing: 8) {
            Image(systemName: "terminal")
                .font(.system(size: 14))
                .foregroundStyle(session?.state.isLive == true ? Color.accentColor : Color.secondary)
                .frame(width: 16)

            VStack(alignment: .leading, spacing: 2) {
                Text(host.displayName)
                    .font(.system(size: 13, weight: .medium))
                    .lineLimit(1)
                    .simultaneousGesture(TapGesture(count: 2).onEnded {
                        if sessions.sessions(for: host.id).count == 1, let session { beginRename(session) }
                    })
                Text(host.subtitle)
                    .font(.system(size: 11))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }

            Spacer(minLength: 4)

            if let session {
                stateIndicator(for: session.state)
            }
        }
        .help(help(for: host))
    }

    @ViewBuilder
    private func stateIndicator(for state: TerminalSession.State) -> some View {
        let color: Color = switch state {
        case .connecting, .connected: .green
        case .failed: .red
        case .closed, .disconnected: .secondary
        case .idle: .secondary
        }
        Circle()
            .fill(color)
            .frame(width: 6, height: 6)
            .help(state.label)
    }

    private func help(for host: Host) -> String {
        guard let session = sessions.session(for: host.id) else {
            return "Connect to \(host.displayName)"
        }
        return "\(host.displayName): \(session.state.label)"
    }

    private func contextMenu(for host: Host) -> some View {
        let session = sessions.session(for: host.id)
        let isLive = sessions.sessions(for: host.id).contains { $0.state.isLive }
        return Group {
            if isLive {
                Button("Show Session") { if let session { sessions.selectSession(session.id) } }
                Button("New Session") { sessions.newSession(host) }
                Button("Reconnect") { sessions.reconnect(host.id) }
            } else {
                Button("Connect") {
                    if let session { sessions.reconnectSession(session.id) }
                    else { sessions.open(host) }
                }
            }
            if let session {
                sessionActions(session)
                Button("Close Session") { closeSession(session) }
            }
            Divider()
            Button("Edit Host…") { appState.edit(host) }
            Button("Duplicate Host") { duplicate(host) }
            Divider()
            Button("Delete Host", role: .destructive) { sessions.delete(host) }
        }
    }

    private func duplicate(_ host: Host) {
        var copy = host
        copy.id = UUID()
        copy.name = unique(name: host.name)
        do {
            guard appState.isVaultUnlocked else { appState.isVaultSheetPresented = true; return }
            guard let password = try PasswordStore.shared.password(for: host.id) else {
                appState.edit(host)
                return
            }
            try store.saveHost(copy, password: password)
        } catch {
            appState.showAlert(title: "Host not duplicated", error.localizedDescription)
        }
    }

    private func unique(name: String) -> String {
        var candidate = name + " copy"
        var suffix = 2
        while store.hosts.contains(where: { $0.displayName == candidate }) {
            candidate = "\(name) copy \(suffix)"
            suffix += 1
        }
        return candidate
    }

    // MARK: - Chrome

    private func loadFailureBanner(_ failure: HostStore.LoadFailure) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Label("Host list not loaded", systemImage: "exclamationmark.triangle.fill")
                .font(.caption.weight(.semibold))
            Text(failure.message)
                .font(.caption)
                .fixedSize(horizontal: false, vertical: true)
            HStack {
                Button("Reveal File") { store.revealInFinder() }
                Button("Try Again") { store.load() }
            }
            .font(.caption)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(8)
        .background(Color.orange.opacity(0.15), in: RoundedRectangle(cornerRadius: 6))
        .padding(8)
    }

    private func saveFailureBanner(_ message: String) -> some View {
        VStack(alignment: .leading, spacing: 6) {
            Label("Changes not saved", systemImage: "externaldrive.badge.exclamationmark")
                .font(.caption.weight(.semibold))
            Text(message)
                .font(.caption)
                .fixedSize(horizontal: false, vertical: true)
                .textSelection(.enabled)
        }
        .frame(maxWidth: .infinity, alignment: .leading)
        .padding(8)
        .background(Color.red.opacity(0.12), in: RoundedRectangle(cornerRadius: 6))
        .padding(8)
    }

    private var footer: some View {
        HStack(spacing: 12) {
            Button {
                appState.addHost()
            } label: {
                Label("New Host", systemImage: "plus")
                    .font(.system(size: 12))
                    .padding(.horizontal, 6)
                    .frame(height: 24)
            }
            .help("New host")

            Spacer(minLength: 0)

            Text(Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "1.0.0")
                .font(.system(size: 11))
                .foregroundStyle(.secondary)
                .monospacedDigit()
        }
        .padding(.horizontal, DesktopMetrics.sidebarInset)
        .frame(height: DesktopMetrics.footerHeight)
        .debugRegion("sidebar-footer")
        .buttonStyle(MotionButtonStyle())
    }
}
