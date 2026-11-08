import SwiftUI

/// The sidebar: every saved host, with the ones that have a session marked.
struct HostListView: View {
    @EnvironmentObject private var store: HostStore
    @EnvironmentObject private var sessions: SessionManager
    @EnvironmentObject private var appState: AppState
    @ObservedObject private var sync = SyncCoordinator.shared

    @State private var filter = ""
    @State private var renamingSessionID: UUID?
    @State private var renameText = ""
    @State private var searchFocusRequest = 0
    @State private var hoveredHostID: Host.ID?
    @State private var copiedHostID: Host.ID?
    @State private var renamingGroupID: HostGroup.ID?
    @State private var groupRenameText = ""
    @FocusState private var groupRenameFocused: Bool
    @State private var dropTarget: SidebarDropTarget?
    @FocusState private var renameFocused: Bool
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    private var visibleHosts: [Host] {
        store.hosts.filter(matches)
    }

    private var isFiltering: Bool {
        !filter.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty
    }

    /// While searching, only groups with a match are shown, and they are
    /// shown expanded; otherwise every group, empty ones included.
    private var visibleGroups: [HostGroup] {
        let layout = store.layout
        return isFiltering ? layout.groups.filter { !layout.hosts(in: $0.id).filter(matches).isEmpty } : layout.groups
    }

    private func isExpanded(_ group: HostGroup) -> Bool {
        isFiltering || !store.isCollapsed(group.id)
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
                let layout = store.layout
                ForEach(layout.ungrouped.filter(matches)) { host in
                    hostEntry(host)
                }
                ForEach(visibleGroups) { group in
                    groupHeader(group, count: layout.hosts(in: group.id).count)
                    if isExpanded(group) {
                        let groupHosts = layout.hosts(in: group.id).filter(matches)
                        if groupHosts.isEmpty {
                            emptyGroupRow(group)
                        }
                        ForEach(groupHosts) { host in
                            hostEntry(host)
                        }
                    }
                }
                if !store.groups.isEmpty && !isFiltering {
                    ungroupDropZone
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
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.18), value: store.collapsedGroupIDs)
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.18), value: store.layout)
            .overlay {
                if visibleHosts.isEmpty && (isFiltering || store.groups.isEmpty) {
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
        .onReceive(NotificationCenter.default.publisher(for: .maineNewGroup)) { _ in
            createGroup()
        }
        .onChange(of: renameFocused) { _, focused in
            if !focused { commitRename() }
        }
        .onChange(of: groupRenameFocused) { _, focused in
            if !focused { commitGroupRename() }
        }
    }

    // MARK: - Hosts and groups

    /// One host: its row and, when it has several or renamed sessions, the
    /// session rows beneath it. The row is draggable and accepts drops
    /// (another host lands just above it, in the same section).
    private func hostEntry(_ host: Host) -> some View {
        let hostSessions = sessions.sessions(for: host.id)
        let showsChildren = hostSessions.count > 1 || hostSessions.contains {
            $0.customName != nil || renamingSessionID == $0.id || $0.isEphemeral
        }
        return VStack(spacing: 2) {
            HStack(spacing: 0) {
                row(for: host)
                    .frame(maxWidth: .infinity, alignment: .leading)
                    .padding(.horizontal, DesktopMetrics.rowInset)
                    .frame(height: DesktopMetrics.hostRowHeight)
                    .contentShape(Rectangle())
                    .onTapGesture {
                        if let session = sessions.session(for: host.id) { sessions.selectSession(session.id) }
                        else { sessions.open(host) }
                    }
                    .accessibilityElement(children: .combine)
                    .accessibilityAddTraits(.isButton)
                    .accessibilityAction {
                        if let session = sessions.session(for: host.id) { sessions.selectSession(session.id) }
                        else { sessions.open(host) }
                    }
                if hoveredHostID == host.id || copiedHostID == host.id {
                    copyButton(host)
                        .padding(.trailing, !showsChildren && !hostSessions.isEmpty ? 0 : DesktopMetrics.rowInset)
                        .transition(.opacity)
                }
                if !showsChildren, let session = hostSessions.first {
                    closeButton(session)
                        .padding(.trailing, DesktopMetrics.rowInset)
                        .transition(.opacity)
                }
            }
            .modifier(SidebarRowHighlight(selected: !showsChildren && sessions.selectedHostID == host.id))
            .overlay(alignment: .top) { insertionLine(visible: dropTarget == .host(host.id)) }
            .onHover { inside in
                if inside { hoveredHostID = host.id }
                else if hoveredHostID == host.id { hoveredHostID = nil }
            }
            .animation(reduceMotion ? nil : .easeInOut(duration: 0.12), value: hoveredHostID == host.id)
            .debugRegion("host-row", sessionID: host.id)
            .contextMenu { contextMenu(for: host) }
            .draggable(SidebarDragItem.host(host.id).payload) {
                Label(host.displayName, systemImage: "terminal")
                    .font(.system(size: 13, weight: .medium))
                    .padding(.horizontal, 8).padding(.vertical, 4)
            }
            .dropDestination(for: String.self) { items, _ in
                drop(items, toGroup: host.groupID, before: host.id)
            } isTargeted: { setDropTarget(.host(host.id), $0) }

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

    /// A collapsible section header. Click toggles; hosts dropped on it join
    /// the group at its end; a group dropped on it moves above it.
    private func groupHeader(_ group: HostGroup, count: Int) -> some View {
        let expanded = isExpanded(group)
        return HStack(spacing: 6) {
            Image(systemName: "chevron.right")
                .font(.system(size: 10, weight: .semibold))
                .rotationEffect(.degrees(expanded ? 90 : 0))
                .foregroundStyle(.secondary)
                .frame(width: 16)
            if renamingGroupID == group.id {
                TextField("Group name", text: $groupRenameText)
                    .textFieldStyle(.plain)
                    .font(.system(size: 12, weight: .semibold))
                    .focused($groupRenameFocused)
                    .onAppear { groupRenameFocused = true }
                    .onSubmit { commitGroupRename() }
                    .onExitCommand { renamingGroupID = nil; groupRenameFocused = false }
                    .accessibilityIdentifier("group-name-editor")
            } else {
                Text(group.name)
                    .font(.system(size: 12, weight: .semibold))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
            }
            Spacer(minLength: 4)
            if !expanded && count > 0 {
                Text("\(count)")
                    .font(.system(size: 11).monospacedDigit())
                    .foregroundStyle(.tertiary)
            }
        }
        .padding(.horizontal, DesktopMetrics.rowInset)
        .frame(height: DesktopMetrics.groupRowHeight)
        .contentShape(Rectangle())
        .onTapGesture { toggle(group) }
        .modifier(SidebarRowHighlight(selected: false))
        .overlay {
            if dropTarget == .group(group.id) {
                RoundedRectangle(cornerRadius: 6).strokeBorder(Color.accentColor, lineWidth: 2)
            }
        }
        .accessibilityElement(children: .combine)
        .accessibilityAddTraits(.isButton)
        .accessibilityLabel("\(group.name), group, \(expanded ? "expanded" : "collapsed")")
        .accessibilityAction { toggle(group) }
        .accessibilityIdentifier("group-\(group.id.uuidString)")
        .debugRegion("group-header", sessionID: group.id)
        .contextMenu {
            Button(expanded ? "Collapse Group" : "Expand Group") { toggle(group) }
            Button("Rename Group…") { beginGroupRename(group) }
            Divider()
            Button("New Group") { createGroup() }
            Divider()
            Button("Delete Group", role: .destructive) { deleteGroup(group) }
        }
        .draggable(SidebarDragItem.group(group.id).payload) {
            Label(group.name, systemImage: "folder")
                .font(.system(size: 12, weight: .semibold))
                .padding(.horizontal, 8).padding(.vertical, 4)
        }
        .dropDestination(for: String.self) { items, _ in
            drop(items, toGroup: group.id, before: nil, groupBefore: group.id)
        } isTargeted: { setDropTarget(.group(group.id), $0) }
        .listRowInsets(EdgeInsets(top: 6, leading: 0, bottom: 1, trailing: 0))
        .listRowBackground(Color.clear)
    }

    private func emptyGroupRow(_ group: HostGroup) -> some View {
        Text("Drag hosts here")
            .font(.system(size: 11))
            .foregroundStyle(.tertiary)
            .frame(maxWidth: .infinity, alignment: .leading)
            .padding(.leading, DesktopMetrics.rowInset + 22)
            .frame(height: DesktopMetrics.sessionRowHeight)
            .contentShape(Rectangle())
            .overlay {
                if dropTarget == .groupEnd(group.id) {
                    RoundedRectangle(cornerRadius: 6).strokeBorder(Color.accentColor, lineWidth: 2)
                }
            }
            .dropDestination(for: String.self) { items, _ in
                drop(items, toGroup: group.id, before: nil)
            } isTargeted: { setDropTarget(.groupEnd(group.id), $0) }
            .listRowInsets(EdgeInsets(top: 1, leading: 0, bottom: 1, trailing: 0))
            .listRowBackground(Color.clear)
    }

    /// The end of the list: a host dropped here leaves its group and goes to
    /// the end of the ungrouped hosts; a group dropped here moves last.
    private var ungroupDropZone: some View {
        Text(dropTarget == .ungroupedEnd ? "Remove from group" : " ")
            .font(.system(size: 11))
            .foregroundStyle(.secondary)
            .frame(maxWidth: .infinity)
            .frame(height: DesktopMetrics.sessionRowHeight)
            .contentShape(Rectangle())
            .overlay {
                if dropTarget == .ungroupedEnd {
                    RoundedRectangle(cornerRadius: 6).strokeBorder(Color.accentColor, lineWidth: 2)
                }
            }
            .dropDestination(for: String.self) { items, _ in
                drop(items, toGroup: nil, before: nil, groupToEnd: true)
            } isTargeted: { setDropTarget(.ungroupedEnd, $0) }
            .listRowInsets(EdgeInsets(top: 4, leading: 0, bottom: 1, trailing: 0))
            .listRowBackground(Color.clear)
            .accessibilityHidden(true)
    }

    private func insertionLine(visible: Bool) -> some View {
        Capsule()
            .fill(Color.accentColor)
            .frame(height: 2)
            .padding(.horizontal, 4)
            .offset(y: -2)
            .opacity(visible ? 1 : 0)
    }

    private func setDropTarget(_ target: SidebarDropTarget, _ targeted: Bool) {
        if targeted { dropTarget = target }
        else if dropTarget == target { dropTarget = nil }
    }

    /// Applies a sidebar drop. Hosts go to `group` (before `target` when
    /// given); groups move before `groupBefore`, or last with `groupToEnd`.
    private func drop(_ items: [String], toGroup group: HostGroup.ID?, before target: Host.ID?,
                      groupBefore: HostGroup.ID? = nil, groupToEnd: Bool = false) -> Bool {
        dropTarget = nil
        let parsed = items.compactMap(SidebarDragItem.init(payload:))
        guard !parsed.isEmpty else { return false }
        var target = target
        var groupID = group
        // Dropping a group on a host in another group places it above that group.
        let groupAnchor = groupBefore ?? (target.flatMap { store.host(id: $0)?.groupID })
        let result = store.updateLayout { layout in
            for item in parsed {
                switch item {
                case .host(let id):
                    layout.moveHost(id, toGroup: groupID, before: target)
                    target = nil
                    groupID = layout.hosts.first { $0.id == id }?.groupID
                case .group(let id):
                    if groupToEnd { layout.moveGroup(id, before: nil) }
                    else if let groupAnchor { layout.moveGroup(id, before: groupAnchor) }
                }
            }
        }
        if case .failure(let error) = result { appState.showAlert(title: "Hosts not rearranged", error.localizedDescription) }
        DebugLog.shared.record("ui.sidebar-drop", fields: ["items": String(parsed.count)])
        return true
    }

    private func toggle(_ group: HostGroup) {
        guard !isFiltering, renamingGroupID != group.id else { return }
        store.setCollapsed(group.id, !store.isCollapsed(group.id))
    }

    private func createGroup(moving host: Host? = nil) {
        guard appState.isVaultUnlocked else { appState.isVaultSheetPresented = true; return }
        var created: HostGroup?
        let result = store.updateLayout { layout in
            let group = layout.addGroup()
            if let host { layout.moveHost(host.id, toGroup: group.id, before: nil) }
            created = group
        }
        switch result {
        case .success:
            if let created {
                store.setCollapsed(created.id, false)
                beginGroupRename(created)
            }
        case .failure(let error):
            appState.showAlert(title: "Group not created", error.localizedDescription)
        }
    }

    private func beginGroupRename(_ group: HostGroup) {
        renamingGroupID = group.id
        groupRenameText = group.name
    }

    private func commitGroupRename() {
        guard let id = renamingGroupID else { return }
        let name = groupRenameText
        renamingGroupID = nil
        groupRenameFocused = false
        guard !name.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { return }
        if case .failure(let error) = store.updateLayout({ try $0.renameGroup(id, to: name) }) {
            appState.showAlert(title: "Group not renamed", error.localizedDescription)
        }
    }

    private func deleteGroup(_ group: HostGroup) {
        if case .failure(let error) = store.updateLayout({ $0.deleteGroup(group.id) }) {
            appState.showAlert(title: "Group not deleted", error.localizedDescription)
        }
    }

    private func move(_ host: Host, to group: HostGroup.ID?) {
        if case .failure(let error) = store.updateLayout({ $0.moveHost(host.id, toGroup: group, before: nil) }) {
            appState.showAlert(title: "Host not moved", error.localizedDescription)
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

    /// Copies the host's IP, username and saved password as plain text. Shown
    /// only while the row is hovered; briefly turns into a checkmark.
    private func copyButton(_ host: Host) -> some View {
        Button {
            guard CredentialCommands.copy(host: host, appState: appState) else { return }
            copiedHostID = host.id
            Task { @MainActor in
                try? await Task.sleep(for: .seconds(1.2))
                if copiedHostID == host.id { copiedHostID = nil }
            }
        } label: {
            Image(systemName: copiedHostID == host.id ? "checkmark" : "doc.on.doc")
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
                .frame(width: 24, height: 24)
                .contentShape(Rectangle())
        }
        .buttonStyle(MotionButtonStyle())
        .help("Copy IP, username and password")
        .accessibilityLabel("Copy credentials for \(host.displayName)")
        .accessibilityIdentifier("copy-credentials-\(host.id.uuidString)")
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
            handToAgentMenu(session)
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
            if let agent = session.agent {
                AgentActivityBadge(agent: agent)
            } else if session.formerAgentName != nil {
                Image(systemName: "cpu").font(.system(size: 10)).foregroundStyle(.tertiary)
                    .help("Agent disconnected")
            }
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
    private func handToAgentMenu(_ session: TerminalSession) -> some View {
        let agents = AgentCoordinator.shared
        if agents.canHandOff(session) {
            Menu("Hand to Agent") {
                ForEach(agents.clients) { client in
                    Button(client.name) {
                        do { try agents.handToAgent(sessionID: session.id, clientID: client.id) }
                        catch { appState.showAlert(title: "Can't hand off this session", error.localizedDescription) }
                    }
                }
            }
        }
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
            Button("Copy Credentials") { CredentialCommands.copy(host: host, appState: appState) }
            Menu("Move to Group") {
                Button("None") { move(host, to: nil) }
                    .disabled(host.groupID == nil)
                if !store.groups.isEmpty { Divider() }
                ForEach(store.groups) { group in
                    Button(group.name) { move(host, to: group.id) }
                        .disabled(host.groupID == group.id)
                }
                Divider()
                Button("New Group…") { createGroup(moving: host) }
            }
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

            Button {
                createGroup()
            } label: {
                Image(systemName: "folder.badge.plus")
                    .font(.system(size: 12))
                    .frame(width: 24, height: 24)
            }
            .disabled(!appState.isVaultUnlocked)
            .help("New group")
            .accessibilityLabel("New group")

            Spacer(minLength: 0)

            if let profile = sync.activeProfile {
                SyncStatusDot(status: sync.status)
                    .help("\(profile.name): \(sync.status.label)")
            }

            Text(AppVersion.current.description)
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

/// What a sidebar drag carries: a plain string, so no custom pasteboard type
/// has to be declared. Only IDs travel, never names.
enum SidebarDragItem: Equatable {
    case host(Host.ID)
    case group(HostGroup.ID)

    private static let hostPrefix = "maine-host:"
    private static let groupPrefix = "maine-group:"

    var payload: String {
        switch self {
        case .host(let id): Self.hostPrefix + id.uuidString
        case .group(let id): Self.groupPrefix + id.uuidString
        }
    }

    init?(payload: String) {
        if payload.hasPrefix(Self.hostPrefix), let id = UUID(uuidString: String(payload.dropFirst(Self.hostPrefix.count))) {
            self = .host(id)
        } else if payload.hasPrefix(Self.groupPrefix),
                  let id = UUID(uuidString: String(payload.dropFirst(Self.groupPrefix.count))) {
            self = .group(id)
        } else {
            return nil
        }
    }
}

enum SidebarDropTarget: Hashable {
    case host(Host.ID)
    case group(HostGroup.ID)
    case groupEnd(HostGroup.ID)
    case ungroupedEnd
}

/// Marks an agent-driven session row; spins while the agent is typing or a
/// command runs. A paused agent shows a pause glyph instead.
struct AgentActivityBadge: View {
    @ObservedObject var agent: AgentDriver

    var body: some View {
        HStack(spacing: 4) {
            if agent.isBusy || agent.isActive {
                ProgressView().controlSize(.mini)
            }
            Image(systemName: agent.isPaused ? "pause.circle" : "cpu")
                .font(.system(size: 10))
                .foregroundStyle(.secondary)
        }
        .help(agent.isPaused ? "\(agent.clientName) is paused" : "Driven by \(agent.clientName)")
        .accessibilityElement(children: .ignore)
        .accessibilityLabel(agent.isPaused ? "Agent paused" : "Driven by \(agent.clientName)")
    }
}
