import AppKit
import Combine
@testable import Maine
import SwiftUI
import Testing

@MainActor
@Suite(.serialized)
struct WorkspaceLifecycleTests {
    private func poll(_ condition: () -> Bool) async {
        let deadline = ContinuousClock.now + .seconds(5)
        while !condition(), ContinuousClock.now < deadline {
            try? await Task.sleep(for: .milliseconds(20))
        }
    }

    @Test(arguments: [false, true])
    func disconnectKeepsTerminalMountedAndReconnectReplacesIt(reportsFailure: Bool) async throws {
        _ = NSApplication.shared
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let executable = directory.appendingPathComponent("test-session")
        let readiness = directory.appendingPathComponent("ready")
        let script = reportsFailure
            ? "#!/bin/sh\ntrap 'exit 255' TERM\n: > '\(readiness.path)'\nread ignored\n"
            : "#!/bin/sh\nexec /bin/sleep 30\n"
        try Data(script.utf8).write(to: executable)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: executable.path)
        let store = HostStore(fileURL: directory.appendingPathComponent("hosts.json"))
        let host = Host(name: "Test", hostname: "localhost")
        try store.add(host).get()
        let manager = SessionManager(store: store) {
            TerminalSession(host: $0, executablePath: executable.path)
        }
        defer { manager.closeAll() }
        let view = NSHostingView(rootView: WorkspaceView()
            .environmentObject(store)
            .environmentObject(manager)
            .environmentObject(AppState()))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 800, height: 520),
                              styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = view
        defer { window.close() }

        manager.open(host)
        let first = try #require(manager.session(for: host.id))
        await poll { first.terminalView.superview != nil }
        #expect(first.terminalView.superview != nil)
        if reportsFailure {
            await poll { FileManager.default.fileExists(atPath: readiness.path) }
            #expect(FileManager.default.fileExists(atPath: readiness.path))
        }
        manager.disconnect(host.id)
        await poll { !first.state.isLive }
        if reportsFailure {
            #expect(first.state == .disconnected(exitCode: 255))
            #expect(first.state.notice == nil)
        }
        #expect(manager.selectedHostID == host.id)
        #expect(manager.openHostIDs == [host.id])
        #expect(manager.session(for: host.id) === first)
        #expect(first.terminalView.superview != nil)
        #expect(first.terminalView.frame.height > 0)

        manager.reconnect(host.id)
        let second = try #require(manager.session(for: host.id))
        #expect(second !== first)
        await poll { second.terminalView.superview != nil && first.terminalView.superview == nil }
        #expect(second.terminalView.superview != nil)
        #expect(first.terminalView.superview == nil)
        #expect(manager.selectedHostID == host.id)
        #expect(manager.openHostIDs == [host.id])
    }

    @Test
    func repeatedSidebarSelectionDisconnectAndReconnectKeepsTheFullWindowIntact() async throws {
        _ = NSApplication.shared
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let executable = directory.appendingPathComponent("test-session")
        try Data("#!/bin/sh\nexec /bin/sleep 30\n".utf8).write(to: executable)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: executable.path)
        let store = HostStore(fileURL: directory.appendingPathComponent("hosts.json"))
        let hosts = [Host(name: "RPI", hostname: "raspberrypi.local"),
                     Host(name: "Server", hostname: "server.local")]
        try store.add(hosts).get()
        let manager = SessionManager(store: store) { TerminalSession(host: $0, executablePath: executable.path) }
        defer { manager.closeAll() }
        let state = AppState(passwords: PasswordStore(fileURL: directory.appendingPathComponent("unused.maine")))
        state.prepareForLaunch()
        state.isVaultSheetPresented = false
        let view = NSHostingView(rootView: RootView()
            .environmentObject(store).environmentObject(manager).environmentObject(state))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1080, height: 720),
                              styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = view
        defer { window.close() }
        #expect(manager.selectedHostID == nil)
        window.makeKeyAndOrderFront(nil)
        manager.open(hosts[0])
        manager.open(hosts[1])

        for cycle in 0..<12 {
            let host = hosts[cycle % hosts.count]
            manager.selectedHostID = host.id
            let session = try #require(manager.session(for: host.id))
            await poll { session.terminalView.superview != nil && !session.terminalView.isHidden }
            let container = try #require(session.terminalView.superview)
            #expect(session.terminalView.window === window)

            manager.selectedHostID = nil
            await poll { session.terminalView.isHidden }
            #expect(session.terminalView.superview === container)
            #expect(session.terminalView.isHidden)

            manager.selectedHostID = host.id
            await poll { !session.terminalView.isHidden }
            manager.disconnect(host.id)
            await poll { !session.state.isLive }
            #expect(session.terminalView.superview === container)
            #expect(!session.terminalView.isHidden)
            #expect(session.terminalView.frame.height > 100)

            var publishedCounts: [Int] = []
            let subscription = manager.$openSessions.dropFirst().sink { publishedCounts.append($0.count) }
            manager.open(host) // A host click reconnects directly, without a details page.
            subscription.cancel()
            #expect(publishedCounts == [2])
            let replacement = try #require(manager.session(for: host.id))
            #expect(replacement !== session)
            window.setContentSize(NSSize(width: cycle % 2 == 0 ? 900 : 1080,
                                         height: cycle % 2 == 0 ? 560 : 720))
            await poll { replacement.terminalView.window === window && replacement.terminalView.frame.height > 100 }
            #expect(replacement.terminalView.window === window)
            #expect(!replacement.terminalView.isHidden)
            #expect(replacement.terminalView.frame.height > 100)
            #expect(manager.openHostIDs.count == 2)
            #expect(store.hosts.count == 2)
        }
        let inactive = try #require(manager.session(for: hosts[0].id))
        let selected = try #require(manager.selectedSession)
        manager.close(hosts[0].id)
        await poll { !inactive.state.isLive && manager.session(for: hosts[0].id) == nil }
        #expect(!inactive.state.isLive)
        #expect(manager.session(for: hosts[0].id) == nil)
        #expect(manager.selectedSession === selected)
        #expect(store.hosts.count == 2) // Closing must not delete a saved host.

        manager.close(hosts[1].id)
        await poll { !selected.state.isLive && manager.openSessions.isEmpty }
        #expect(!selected.state.isLive)
        await poll { manager.selectedHostID == nil && manager.openSessions.isEmpty }
        #expect(manager.selectedHostID == nil)
        #expect(window.contentView === view)
        // A delayed click from a closing row must never select a dead session.
        manager.select(hosts[0].id)
        #expect(manager.selectedHostID == nil)
        #expect(manager.selectedSession == nil)
    }

    @Test
    func multipleSessionsForOneHostStayIndependent() async throws {
        _ = NSApplication.shared
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let executable = directory.appendingPathComponent("test-session")
        try Data("#!/bin/sh\nexec /bin/sleep 30\n".utf8).write(to: executable)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: executable.path)
        let store = HostStore(fileURL: directory.appendingPathComponent("hosts.json"))
        let host = Host(name: "RPI", hostname: "raspberrypi.local")
        let otherHost = Host(name: "Server", hostname: "server.local")
        try store.add([host, otherHost]).get()
        let manager = SessionManager(store: store) { TerminalSession(host: $0, executablePath: executable.path) }
        defer { manager.closeAll() }
        let state = AppState(passwords: PasswordStore(fileURL: directory.appendingPathComponent("unused.maine")))
        let view = NSHostingView(rootView: WorkspaceView()
            .environmentObject(store).environmentObject(manager).environmentObject(state))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1080, height: 720),
                              styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = view
        window.makeKeyAndOrderFront(nil)
        defer { window.close() }

        manager.open(host)
        let first = try #require(manager.selectedSession)
        manager.newSession(host)
        let second = try #require(manager.selectedSession)
        #expect(first.id != second.id)
        #expect(manager.sessions(for: host.id).count == 2)
        #expect(manager.label(for: first) == "Session 1")
        #expect(manager.label(for: second) == "Session 2")
        #expect(first.state.isLive && second.state.isLive)
        await poll { first.terminalView.window === window && second.terminalView.window === window }
        let firstContainer = try #require(first.terminalView.superview)
        let secondContainer = try #require(second.terminalView.superview)
        #expect(firstContainer !== secondContainer)
        await poll { first.terminalView.isHidden && !second.terminalView.isHidden }
        #expect(first.terminalView.isHidden)
        #expect(!second.terminalView.isHidden)

        for session in [first, second, first] {
            manager.selectSession(session.id)
            await poll { !session.terminalView.isHidden }
            #expect(manager.selectedSession === session)
            #expect(first.terminalView.isHidden == (session !== first))
            #expect(second.terminalView.isHidden == (session !== second))
            #expect(first.terminalView.superview === firstContainer)
            #expect(second.terminalView.superview === secondContainer)
        }
        manager.open(host) // A normal host click must not add a third session.
        #expect(manager.selectedSession === first)
        #expect(manager.sessions(for: host.id).count == 2)
        manager.reconnectSession(first.id)
        let replacement = try #require(manager.selectedSession)
        #expect(replacement !== first && replacement !== second)
        #expect(manager.label(for: replacement) == "Session 1")
        #expect(manager.sessions(for: host.id).count == 2)
        #expect(second.state.isLive)
        manager.selectSession(second.id)
        manager.closeSession(replacement.id)
        await poll { !first.state.isLive && !replacement.state.isLive }
        #expect(manager.selectedSession === second)
        #expect(second.state.isLive)
        #expect(manager.sessions(for: host.id).count == 1)
        manager.selectSession(replacement.id) // Ignore stale child-row actions.
        #expect(manager.selectedSession === second)

        manager.open(otherHost)
        let other = try #require(manager.selectedSession)
        manager.selectSession(second.id)
        manager.closeSession(second.id)
        #expect(manager.selectedSession === other)
        #expect(store.hosts.count == 2)
        manager.newSession(host)
        let third = try #require(manager.selectedSession)
        manager.newSession(host)
        let fourth = try #require(manager.selectedSession)
        manager.delete(host)
        await poll { !third.state.isLive && !fourth.state.isLive }
        #expect(manager.sessions(for: host.id).isEmpty)
        #expect(manager.selectedSession === other)
        #expect(other.state.isLive)
        #expect(store.hosts == [otherHost])
    }

    @Test
    func sessionsAutosaveAcrossLaunchesAndOnlyExplicitCloseRemovesThem() async throws {
        _ = NSApplication.shared
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let vault = PasswordStore(fileURL: directory.appendingPathComponent("vault.maine"),
                                  keychainService: "app.maine.tests.\(UUID())")
        defer { try? vault.lock() }
        try vault.unlock(masterPassword: "saved sessions master password")
        let store = HostStore(vault: vault)
        let host = Host(name: "Server", hostname: "server.example")
        try store.saveHost(host, password: "host credential")
        let executable = directory.appendingPathComponent("test-session")
        try Data("#!/bin/sh\nexec /bin/sleep 30\n".utf8).write(to: executable)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: executable.path)
        let manager = SessionManager(store: store) { TerminalSession(host: $0, executablePath: executable.path) }
        defer { manager.closeAll() }
        manager.newSession(host)
        let session = try #require(manager.selectedSession)
        #expect(try vault.savedSessions().count == 1) // Created automatically.
        #expect(throws: HostStoreError.self) { try manager.renameSession(session.id, to: "  ") }
        try manager.renameSession(session.id, to: "  Deployment  ")
        #expect(manager.label(for: session) == "Deployment")
        session.terminalView.feed(text: "private deployment output\r\nUnicode: café 🔐\r\n")
        await poll { session.historySnapshot().contains("private deployment output") }
        try manager.checkpointSessions()
        let savedID = try #require(session.savedSessionID)
        #expect(try vault.savedSessions().count == 1)
        try manager.renameSession(session.id, to: "Production")
        #expect(try vault.savedSessions().first?.name == "Production")
        session.terminalView.feed(text: "updated snapshot\r\n")
        await poll { session.historySnapshot().contains("updated snapshot") }
        // No Save action: the periodic checkpoint must pick up new output.
        await poll { (try? vault.savedSessions().first?.history.contains("updated snapshot")) == true }
        let saved = try #require(vault.savedSessions().first)
        #expect(saved.id == savedID)
        #expect(saved.history.contains("updated snapshot"))
        #expect(saved.history.contains("café 🔐"))
        let fileText = try String(contentsOf: vault.fileURL, encoding: .utf8)
        #expect(!fileText.contains("Production"))
        #expect(!fileText.contains("deployment output"))
        let before = try Data(contentsOf: vault.fileURL)
        try vault.lock()
        #expect(throws: PasswordStore.VaultError.self) { try manager.checkpointSessions() }
        #expect(throws: PasswordStore.VaultError.self) { try manager.renameSession(session.id, to: "Locked change") }
        #expect(manager.label(for: session) == "Production")
        #expect(try Data(contentsOf: vault.fileURL) == before)
        manager.closeSession(session.id)
        #expect(manager.openSessions.count == 1) // Cannot silently resurrect a close that failed while locked.
        try vault.unlock(masterPassword: "saved sessions master password")
        session.terminalView.feed(text: "quit-time output\r\n")
        await poll { session.historySnapshot().contains("quit-time output") }
        try manager.shutdown()
        #expect(try vault.savedSessions().count == 1) // Quitting is not closing.
        #expect(try vault.savedSessions().first?.history.contains("quit-time output") == true)

        let restartedStore = HostStore(vault: vault)
        restartedStore.load()
        let restarted = SessionManager(store: restartedStore) { TerminalSession(host: $0, executablePath: executable.path) }
        defer { restarted.closeAll() }
        try restarted.restoreSavedSessions()
        try restarted.restoreSavedSessions()
        #expect(restarted.openSessions.count == 1)
        let restored = try #require(restarted.openSessions.first)
        #expect(restored.state.isLive) // Restore reconnects automatically.
        #expect(restored.savedSessionID == savedID)
        #expect(restarted.label(for: restored) == "Production")
        #expect(restarted.selectedSession === restored)
        await poll { restored.historySnapshot().contains("updated snapshot") }
        #expect(restored.historySnapshot().contains("updated snapshot"))
        restarted.open(host) // Already live, so this just reveals the session.
        #expect(restarted.selectedSession === restored)
        #expect(restored.state.isLive)
        restarted.reconnectSession(restored.id)
        let replacement = try #require(restarted.selectedSession)
        #expect(replacement !== restored)
        #expect(replacement.state.isLive)
        #expect(replacement.customName == "Production")
        #expect(replacement.savedSessionID == savedID)
        await poll { replacement.historySnapshot().contains("updated snapshot") }
        #expect(replacement.historySnapshot().contains("updated snapshot"))
        restarted.closeSession(replacement.id)
        #expect(restarted.openSessions.isEmpty)
        #expect(try vault.savedSessions().isEmpty)
        restarted.newSession(host)
        #expect(try vault.savedSessions().count == 1)
        restarted.delete(host)
        #expect(try vault.savedSessions().isEmpty)
        #expect(restarted.openSessions.isEmpty)
    }

    @Test
    func desktopLayoutKeepsItsRhythmAcrossSizesAndAppearances() async throws {
        _ = NSApplication.shared
        NSApplication.shared.activate(ignoringOtherApps: true)
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let executable = directory.appendingPathComponent("test-session")
        try Data("#!/bin/sh\n/bin/sleep 0.2\nprintf 'Linux raspberrypi 6.12\\n\\nLast login: Sun Oct 11 10:41\\npi@raspberrypi:~ $ '\nexec /bin/sleep 30\n".utf8).write(to: executable)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: executable.path)
        let store = HostStore(fileURL: directory.appendingPathComponent("hosts.json"))
        let host = Host(name: "raspberry-pi", hostname: "192.168.1.42", username: "pi")
        let other = Host(name: "Development", hostname: "dev.example", username: "deploy")
        try store.add([host, other]).get()
        let manager = SessionManager(store: store) { TerminalSession(host: $0, executablePath: executable.path) }
        defer { manager.closeAll() }
        let state = AppState(passwords: PasswordStore(fileURL: directory.appendingPathComponent("unused.maine")))
        state.prepareForLaunch()
        state.isVaultSheetPresented = false
        state.vaultDidUnlock()
        let view = NSHostingView(rootView: RootView()
            .environmentObject(store).environmentObject(manager).environmentObject(state))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1080, height: 720),
                              styleMask: [.titled, .resizable, .fullSizeContentView], backing: .buffered, defer: false)
        window.titleVisibility = .hidden
        window.titlebarAppearsTransparent = true
        window.isReleasedWhenClosed = false
        window.contentView = view
        window.makeKeyAndOrderFront(nil)
        defer { window.close() }
        for size in [NSSize(width: 1080, height: 720), NSSize(width: 780, height: 520), NSSize(width: 1440, height: 900)] {
            window.setContentSize(size)
            try await Task.sleep(for: .milliseconds(150))
            let workspace = try #require(DebugDiagnostics.frameForRegion("workspace"))
            let empty = try #require(DebugDiagnostics.frameForRegion("workspace-empty-state"))
            #expect(abs(empty.midX - workspace.midX) < 1)
            #expect(abs(empty.midY - workspace.midY) < 1)
        }
        window.setContentSize(NSSize(width: 1080, height: 720))
        manager.open(host)
        let first = try #require(manager.selectedSession)
        for dark in [true, false] {
            window.appearance = NSAppearance(named: dark ? .darkAqua : .aqua)
            for size in [NSSize(width: 1080, height: 720), NSSize(width: 780, height: 520), NSSize(width: 1440, height: 900)] {
                window.setContentSize(size)
                try await Task.sleep(for: .milliseconds(250))
                assertVisibleLayout(window: window, view: view, session: first)
                let header = try #require(DebugDiagnostics.frameForRegion("session-header", sessionID: first.id))
                #expect(abs(header.height - DesktopMetrics.headerHeight) < 1)
                let sidebar = try #require(DebugDiagnostics.frameForRegion("sidebar"))
                #expect(sidebar.width >= 219 && sidebar.width <= 341)
                let footer = try #require(DebugDiagnostics.frameForRegion("sidebar-footer"))
                #expect(abs(footer.height - DesktopMetrics.footerHeight) < 1)
                let row = try #require(DebugDiagnostics.frameForRegion("host-row", sessionID: host.id))
                #expect(abs(row.height - DesktopMetrics.hostRowHeight) < 1)
                #expect(sidebar.insetBy(dx: -1, dy: -1).contains(row))
                #expect(first.terminalView.frame.minX == DesktopMetrics.terminalInset)
                if size.width == 1080 { try writePreview(view: view, name: dark ? "single-dark" : "single-light") }
            }
        }
        manager.newSession(host)
        let second = try #require(manager.selectedSession)
        try manager.renameSession(second.id, to: "Deployment")
        window.setContentSize(NSSize(width: 1080, height: 720))
        window.appearance = NSAppearance(named: .darkAqua)
        try await Task.sleep(for: .milliseconds(300))
        await poll { second.historySnapshot().contains("Last login") }
        try await Task.sleep(for: .milliseconds(200))
        assertVisibleLayout(window: window, view: view, session: second)
        let child = try #require(DebugDiagnostics.frameForRegion("session-row", sessionID: second.id))
        #expect(abs(child.height - DesktopMetrics.sessionRowHeight) < 1)
        try writePreview(view: view, name: "multiple-dark")
        let search = try #require(searchField(in: view))
        NotificationCenter.default.post(name: .maineFocusSearch, object: nil)
        await poll { search.currentEditor() != nil }
        #expect(search.currentEditor() != nil)
        search.stringValue = "dev.example"
        search.sendAction(search.action, to: search.target)
        await poll { DebugDiagnostics.frameForRegion("host-row", sessionID: host.id) == nil }
        #expect(DebugDiagnostics.frameForRegion("host-row", sessionID: host.id) == nil)
        #expect(DebugDiagnostics.frameForRegion("host-row", sessionID: other.id) != nil)
        search.stringValue = ""
        search.sendAction(search.action, to: search.target)
        await poll { DebugDiagnostics.frameForRegion("host-row", sessionID: host.id) != nil }
        #expect(DebugDiagnostics.frameForRegion("host-row", sessionID: host.id) != nil)
    }

    private func searchField(in view: NSView) -> NSSearchField? {
        if let field = view as? NSSearchField { return field }
        return view.subviews.lazy.compactMap { searchField(in: $0) }.first
    }

    private func writePreview(view: NSView, name: String) throws {
        guard let directory = ProcessInfo.processInfo.environment["MAINE_UI_PREVIEW_DIR"] else { return }
        let url = URL(fileURLWithPath: directory, isDirectory: true)
        try FileManager.default.createDirectory(at: url, withIntermediateDirectories: true)
        guard let window = view.window else { return }
        let capture = Process()
        capture.executableURL = URL(fileURLWithPath: "/usr/sbin/screencapture")
        capture.arguments = ["-x", "-o", "-l", String(window.windowNumber), url.appendingPathComponent(name + ".png").path]
        try capture.run()
        capture.waitUntilExit()
    }

    @Test
    func defaultSessionNamesDoNotChangeSidebarHierarchyAfterRestore() throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        let vault = PasswordStore(fileURL: directory.appendingPathComponent("vault.maine"),
                                  keychainService: "app.maine.tests.\(UUID())")
        defer { try? vault.lock(); try? FileManager.default.removeItem(at: directory) }
        try vault.unlock(masterPassword: "hierarchy test master password")
        let store = HostStore(vault: vault)
        let host = Host(name: "Server", hostname: "server.example")
        try store.saveHost(host, password: "credential")
        let manager = SessionManager(store: store) { TerminalSession(host: $0, executablePath: "/not/an/executable") }
        manager.newSession(host)
        let first = try #require(manager.selectedSession)
        #expect(first.customName == nil)
        try manager.shutdown()
        let restored = SessionManager(store: store) { TerminalSession(host: $0, executablePath: "/not/an/executable") }
        defer { restored.closeAll() }
        try restored.restoreSavedSessions()
        let session = try #require(restored.selectedSession)
        #expect(session.customName == nil)
        #expect(restored.label(for: session) == "Session 1")
        try restored.renameSession(session.id, to: "Session 4") // A custom name can look generated.
        let again = SessionManager(store: store) { TerminalSession(host: $0, executablePath: "/not/an/executable") }
        defer { again.closeAll() }
        try again.restoreSavedSessions()
        #expect(again.selectedSession?.customName == "Session 4")
    }

    @Test
    func passwordInputIsDisabledUntilUnlockedAndDisabledAgainOnLock() async throws {
        _ = NSApplication.shared
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        defer { try? FileManager.default.removeItem(at: directory) }
        let state = AppState(passwords: PasswordStore(fileURL: directory.appendingPathComponent("unused.maine")))
        let store = HostStore(fileURL: directory.appendingPathComponent("hosts.json"))
        let host = Host(name: "RPI", hostname: "raspberrypi.local")
        state.edit(host)
        let view = NSHostingView(rootView: HostEditorSheet(source: host, onSave: { _ in })
            .environmentObject(store).environmentObject(state))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 460, height: 480),
                              styleMask: [.titled], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = view
        defer { window.close() }
        await poll { self.secureField(in: view) != nil }
        let input = try #require(secureField(in: view))
        #expect(!input.isEnabled)
        state.vaultDidUnlock()
        await poll { input.isEnabled }
        #expect(input.isEnabled)
        state.lockVault()
        await poll { !input.isEnabled }
        #expect(!input.isEnabled)
    }

    @Test(arguments: [false, true])
    func exit255KeepsSplitViewHeaderAndTerminalInsideTheWindow(requestedDisconnect: Bool) async throws {
        _ = NSApplication.shared
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let executable = directory.appendingPathComponent("test-session")
        let ready = directory.appendingPathComponent("ready")
        let exit = directory.appendingPathComponent("exit")
        let script = """
        #!/bin/sh
        trap 'exit 255' TERM
        : > '\(ready.path)'
        while [ ! -e '\(exit.path)' ]; do /bin/sleep 0.02; done
        exit 255
        """
        try Data(script.utf8).write(to: executable)
        try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: executable.path)
        let store = HostStore(fileURL: directory.appendingPathComponent("hosts.json"))
        let host = Host(name: "RPI", hostname: "raspberrypi.local")
        try store.add(host).get()
        let manager = SessionManager(store: store) { TerminalSession(host: $0, executablePath: executable.path) }
        defer { manager.closeAll() }
        let state = AppState(passwords: PasswordStore(fileURL: directory.appendingPathComponent("unused.maine")))
        state.prepareForLaunch()
        state.isVaultSheetPresented = false
        let view = NSHostingView(rootView: RootView()
            .environmentObject(store).environmentObject(manager).environmentObject(state))
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 1080, height: 720),
                              styleMask: [.titled, .resizable], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView = view
        window.makeKeyAndOrderFront(nil)
        defer { window.close() }

        manager.open(host)
        let session = try #require(manager.session(for: host.id))
        await poll {
            FileManager.default.fileExists(atPath: ready.path)
                && DebugDiagnostics.frameForRegion("session-header", sessionID: session.id) != nil
        }
        let initialHeight = window.contentView!.bounds.height
        if requestedDisconnect { manager.disconnect(host.id) }
        else { try Data().write(to: exit) }
        await poll { !session.state.isLive }
        // Give both layout and AppKit's deferred fitting-size invalidation time
        // to settle. The old tests returned before the overflow could occur.
        try await Task.sleep(for: .milliseconds(500))
        if requestedDisconnect {
            #expect(session.state == .disconnected(exitCode: 255))
            #expect(session.state.notice == nil)
            #expect(DebugDiagnostics.frameForRegion("notice", sessionID: session.id) == nil)
        } else {
            #expect(session.state == .closed(exitCode: 255))
            #expect(session.state.notice != nil)
            let notice = try #require(DebugDiagnostics.frameForRegion("notice", sessionID: session.id))
            let root = try #require(DebugDiagnostics.frameForRegion("root"))
            #expect(root.insetBy(dx: -1, dy: -1).contains(notice))
        }
        assertVisibleLayout(window: window, view: view, session: session)
        #expect(abs(window.contentView!.bounds.height - initialHeight) < 1)
        #expect(manager.selectedSession === session)

        window.setContentSize(NSSize(width: 900, height: 560))
        try await Task.sleep(for: .milliseconds(200))
        assertVisibleLayout(window: window, view: view, session: session)
    }

    private func assertVisibleLayout(window: NSWindow, view: NSView, session: TerminalSession,
                                     sourceLocation: SourceLocation = #_sourceLocation) {
        guard let root = DebugDiagnostics.frameForRegion("root"),
              let header = DebugDiagnostics.frameForRegion("session-header", sessionID: session.id),
              let terminal = DebugDiagnostics.frameForRegion("terminal", sessionID: session.id) else {
            Issue.record("Missing root/header/terminal geometry", sourceLocation: sourceLocation)
            return
        }
        #expect(root.insetBy(dx: -1, dy: -1).contains(header), sourceLocation: sourceLocation)
        #expect(root.insetBy(dx: -1, dy: -1).contains(terminal), sourceLocation: sourceLocation)
        #expect(header.height > 20, sourceLocation: sourceLocation)
        #expect(terminal.height > 100, sourceLocation: sourceLocation)
        #expect(terminal.maxY <= root.maxY + 1, sourceLocation: sourceLocation)
        let splits = splitViews(in: view)
        #expect(!splits.isEmpty, sourceLocation: sourceLocation)
        for split in splits {
            #expect(split.bounds.height <= window.contentView!.bounds.height + 1, sourceLocation: sourceLocation)
        }
    }

    private func splitViews(in view: NSView) -> [NSSplitView] {
        (view as? NSSplitView).map { [$0] } ?? view.subviews.flatMap { splitViews(in: $0) }
    }

    private func secureField(in view: NSView) -> NSSecureTextField? {
        if let field = view as? NSSecureTextField { return field }
        return view.subviews.lazy.compactMap { secureField(in: $0) }.first
    }

}
