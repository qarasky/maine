import AppKit

@MainActor
enum DebugDiagnostics {
    static weak var sessionManager: SessionManager?
    private struct Region: Hashable {
        let name: String
        let sessionID: UUID?
    }
    private static var regions: [Region: CGRect] = [:]

    static func recordRegion(_ event: String, name: String, sessionID: UUID?, frame: CGRect) {
        let region = Region(name: name, sessionID: sessionID)
        if event == "disappear" { regions[region] = nil }
        else { regions[region] = frame }
        DebugLog.shared.record("ui.region.\(event)", fields: [
            "region": name, "session": sessionID?.uuidString ?? "none", "frame": NSStringFromRect(frame)
        ])
    }

    /// Last measured geometry, also usable by native-window regression tests.
    static func frameForRegion(_ name: String, sessionID: UUID? = nil) -> CGRect? {
        regions[Region(name: name, sessionID: sessionID)]
    }

    static func capture(_ reason: String, sessions: SessionManager? = nil, log: DebugLog = .shared) {
        guard log.isEnabled else { return }
        let manager = sessions ?? sessionManager
        let snapshot = UUID().uuidString
        log.record("snapshot.begin", fields: [
            "snapshot": snapshot, "reason": reason,
            "selectedHost": manager?.selectedHostID?.uuidString ?? "none",
            "selectedSession": manager?.selectedSession?.id.uuidString ?? "none",
            "openSessions": String(manager?.openSessions.count ?? 0),
            "windows": String(NSApp?.windows.count ?? 0)
        ], flush: true)
        for (region, frame) in regions {
            log.record("snapshot.region", fields: [
                "snapshot": snapshot, "region": region.name,
                "session": region.sessionID?.uuidString ?? "none", "frame": NSStringFromRect(frame)
            ])
        }
        for session in manager?.openSessions ?? [] {
            log.record("snapshot.session", fields: session.debugFields.merging(["snapshot": snapshot]) { _, new in new })
        }
        for window in NSApp?.windows ?? [] {
            log.record("snapshot.window", fields: [
                "snapshot": snapshot, "window": String(window.windowNumber),
                "frame": NSStringFromRect(window.frame), "visible": String(window.isVisible),
                "key": String(window.isKeyWindow), "main": String(window.isMainWindow),
                "miniaturized": String(window.isMiniaturized),
                "occlusion": String(window.occlusionState.rawValue),
                "sheet": String(window.attachedSheet != nil),
                "firstResponder": window.firstResponder.map { String(reflecting: type(of: $0)) } ?? "none"
            ])
            var count = 0
            if let root = window.contentView {
                captureView(root, parent: -1, depth: 0, window: window.windowNumber,
                            snapshot: snapshot, count: &count, log: log)
            }
            if count >= 250 { log.record("snapshot.tree-limit", fields: ["snapshot": snapshot]) }
        }
        log.record("snapshot.end", fields: ["snapshot": snapshot], flush: true)
    }

    static func scheduleDisconnectSnapshots(session: TerminalSession, correlation: String) {
        guard DebugLog.shared.isEnabled else { return }
        // If the main thread stops progressing, these snapshots will be absent.
        // The last flushed event then tells us where execution stopped.
        for delay in [100, 500, 2000] {
            Task { @MainActor [weak session] in
                try? await Task.sleep(for: .milliseconds(delay))
                DebugLog.shared.record("disconnect.followup", fields: (session?.debugFields ?? [:])
                    .merging(["correlation": correlation, "delayMS": String(delay)]) { _, new in new })
                capture("disconnect.\(correlation).after-\(delay)ms")
            }
        }
    }

    private static func captureView(_ view: NSView, parent: Int, depth: Int, window: Int,
                                    snapshot: String, count: inout Int, log: DebugLog) {
        guard count < 250, depth < 30 else { return }
        let index = count
        count += 1
        // Deliberately do not read text, titles, accessibility values or images.
        var fields: [String: String] = [
            "snapshot": snapshot, "window": String(window), "index": String(index),
            "parent": String(parent), "depth": String(depth),
            "class": String(reflecting: type(of: view)), "object": String(describing: ObjectIdentifier(view)),
            "frame": NSStringFromRect(view.frame), "bounds": NSStringFromRect(view.bounds),
            "visibleRect": NSStringFromRect(view.visibleRect),
            "hidden": String(view.isHidden), "hiddenAncestor": String(view.isHiddenOrHasHiddenAncestor),
            "alpha": String(describing: view.alphaValue), "needsLayout": String(view.needsLayout),
            "needsDisplay": String(view.needsDisplay), "constraints": String(view.constraints.count)
        ]
        fields["layerOpacity"] = view.layer.map { String($0.opacity) } ?? "none"
        fields["layerHidden"] = view.layer.map { String($0.isHidden) } ?? "none"
        log.record("snapshot.view", fields: fields)
        for child in view.subviews {
            captureView(child, parent: index, depth: depth + 1, window: window, snapshot: snapshot, count: &count, log: log)
        }
    }
}

extension TerminalSession {
    var debugFields: [String: String] {
        let counters = terminalView.diagnostics
        var fields: [String: String] = ["session": id.uuidString, "host": host.id.uuidString, "state": state.debugName,
         "childPID": String(terminalView.process.shellPid), "running": String(terminalView.process.running),
         "terminal": String(describing: ObjectIdentifier(terminalView)),
         "frame": NSStringFromRect(terminalView.frame), "bounds": NSStringFromRect(terminalView.bounds),
         "attached": String(terminalView.superview != nil),
         "window": terminalView.window.map { String($0.windowNumber) } ?? "none",
         "hidden": String(terminalView.isHidden), "needsVaultUnlock": String(needsVaultUnlock)]
        fields["bytesFed"] = String(counters.bytesFed)
        fields["frames"] = String(counters.frames)
        fields["renders"] = String(counters.renders)
        fields["mainHops"] = String(counters.mainHops)
        return fields
    }
}

extension TerminalSession.State {
    var debugName: String {
        switch self {
        case .idle: "idle"
        case .connecting: "connecting"
        case .connected: "connected"
        case .closed(let code): "closed:\(code.map(String.init) ?? "signal")"
        case .disconnected(let code): "disconnected:\(code.map(String.init) ?? "signal")"
        case .failed: "failed" // The associated message may contain private host data.
        }
    }
}
