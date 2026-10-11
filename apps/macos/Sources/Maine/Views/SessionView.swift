import SwiftUI
import SwiftTerm

/// Hosts the SwiftTerm view for one session.
///
/// The terminal view is owned by the session, not by this representable, so
/// switching sessions keeps every terminal alive with its scrollback intact.
struct TerminalContainerView: NSViewRepresentable {
    let session: TerminalSession
    let isActive: Bool

    func makeNSView(context: Context) -> TerminalHostView {
        let view = TerminalHostView(terminal: session.terminalView, sessionID: session.id)
        view.setActive(isActive)
        return view
    }

    func updateNSView(_ nsView: TerminalHostView, context: Context) {
        nsView.setActive(isActive)
    }

    static func dismantleNSView(_ nsView: TerminalHostView, coordinator: ()) {
        DebugLog.shared.record("terminal.container-dismantle", fields: nsView.debugFields, flush: true)
    }

    func sizeThatFits(_ proposal: ProposedViewSize, nsView: TerminalHostView, context: Context) -> CGSize? {
        let width = proposal.width.flatMap { $0.isFinite ? $0 : nil } ?? 800
        let height = proposal.height.flatMap { $0.isFinite ? $0 : nil } ?? 480
        return CGSize(width: width, height: height)
    }
}

/// SwiftUI manages this neutral container, not SwiftTerm's mutable terminal
/// view. Explicit constraints prevent terminal sizing from collapsing the UI.
final class TerminalHostView: NSView {
    let terminal: LocalProcessTerminalView
    let sessionID: UUID
    private var active = false
    private var lastLoggedGeometry: String?

    init(terminal: LocalProcessTerminalView, sessionID: UUID) {
        self.terminal = terminal
        self.sessionID = sessionID
        super.init(frame: .zero)
        wantsLayer = true
        layer?.backgroundColor = terminal.nativeBackgroundColor.cgColor
        terminal.translatesAutoresizingMaskIntoConstraints = false
        addSubview(terminal)
        NSLayoutConstraint.activate([
            terminal.leadingAnchor.constraint(equalTo: leadingAnchor, constant: DesktopMetrics.terminalInset),
            terminal.trailingAnchor.constraint(equalTo: trailingAnchor, constant: -DesktopMetrics.terminalInset),
            terminal.topAnchor.constraint(equalTo: topAnchor, constant: DesktopMetrics.terminalInset),
            terminal.bottomAnchor.constraint(equalTo: bottomAnchor, constant: -DesktopMetrics.terminalInset)
        ])
        DebugLog.shared.record("terminal.container-created", fields: debugFields)
    }

    required init?(coder: NSCoder) { fatalError("init(coder:) has not been implemented") }

    override var intrinsicContentSize: NSSize {
        NSSize(width: NSView.noIntrinsicMetric, height: NSView.noIntrinsicMetric)
    }

    func setActive(_ value: Bool) {
        let changed = active != value
        active = value
        terminal.isHidden = !value
        if changed { DebugLog.shared.record("terminal.container-active", fields: debugFields) }
        if changed && value { focusIfActive() }
        if !value, window?.firstResponder === terminal { window?.makeFirstResponder(nil) }
    }

    override func viewDidMoveToWindow() {
        super.viewDidMoveToWindow()
        DebugLog.shared.record("terminal.container-window", fields: debugFields)
        focusIfActive()
    }

    override func viewDidChangeEffectiveAppearance() {
        super.viewDidChangeEffectiveAppearance()
        effectiveAppearance.performAsCurrentDrawingAppearance {
            layer?.backgroundColor = terminal.nativeBackgroundColor.cgColor
        }
    }

    override func viewDidMoveToSuperview() {
        super.viewDidMoveToSuperview()
        DebugLog.shared.record("terminal.container-superview", fields: debugFields)
    }

    override func layout() {
        super.layout()
        guard DebugLog.shared.isEnabled else { return }
        let geometry = "\(frame)|\(bounds)|\(terminal.frame)|\(terminal.isHidden)"
        guard geometry != lastLoggedGeometry else { return }
        lastLoggedGeometry = geometry
        DebugLog.shared.record("terminal.container-layout", fields: debugFields)
    }

    var debugFields: [String: String] {
        ["session": sessionID.uuidString, "container": String(describing: ObjectIdentifier(self)),
         "terminal": String(describing: ObjectIdentifier(terminal)), "active": String(active),
         "frame": NSStringFromRect(frame), "bounds": NSStringFromRect(bounds),
         "terminalFrame": NSStringFromRect(terminal.frame), "terminalHidden": String(terminal.isHidden),
         "superview": superview.map { String(reflecting: type(of: $0)) } ?? "none",
         "window": window.map { String($0.windowNumber) } ?? "none"]
    }

    private func focusIfActive() {
        guard active, window?.attachedSheet == nil else { return }
        window?.makeFirstResponder(terminal)
    }
}

/// A live session: header, any problem to report, then the terminal.
struct SessionView: View {
    @ObservedObject var session: TerminalSession
    var isActive = true

    var body: some View {
        VStack(spacing: 0) {
            SessionHeader(session: session)
                .fixedSize(horizontal: false, vertical: true)
                .layoutPriority(1)
                .debugRegion("session-header", sessionID: session.id)
            Divider()

            if let notice = session.state.notice {
                NoticeBanner(session: session, message: notice)
                    .fixedSize(horizontal: false, vertical: true)
                    .layoutPriority(1)
                    .debugRegion("notice", sessionID: session.id)
                Divider()
            }

            GeometryReader { viewport in
                TerminalContainerView(session: session, isActive: isActive)
                    .frame(width: viewport.size.width, height: viewport.size.height)
                    .debugRegion("terminal", sessionID: session.id)
            }
            // Take only the space left after the controls/banner. A terminal
            // must not contribute its previous AppKit height to window fitting.
            .frame(minWidth: 0, maxWidth: .infinity, minHeight: 0, maxHeight: .infinity)
            .clipped()
        }
        .debugRegion("session", sessionID: session.id)
        .onAppear { DebugLog.shared.record("ui.session-appear", fields: session.debugFields) }
        .onDisappear { DebugLog.shared.record("ui.session-disappear", fields: session.debugFields) }
        .onChange(of: isActive) { _, active in
            DebugLog.shared.record("ui.session-active", fields: ["session": session.id.uuidString, "active": String(active)])
        }
    }
}

/// The bar above a terminal: what is running and its status.
/// Sessions connect on their own, so the header carries no Connect button;
///
/// connection problems surface through the notice banner below it.
struct SessionHeader: View {
    @ObservedObject var session: TerminalSession

    private var target: SSHTarget {
        SSHTarget.make(for: session.host)
    }

    var body: some View {
        HStack(alignment: .center, spacing: 12) {
            VStack(alignment: .leading, spacing: 3) {
                Text(session.customName.map { "\(session.host.displayName) — \($0)" } ?? session.host.displayName)
                    .font(.system(size: 13, weight: .semibold))
                    .lineLimit(1)
                Text(session.host.subtitle)
                    .font(.system(.caption, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .lineLimit(1)
                    .truncationMode(.middle)
                    .textSelection(.enabled)
                    .help(target.commandLine)
            }

            Spacer(minLength: 12)

            StatusPill(state: session.state)
        }
        .padding(.horizontal, 16)
        .frame(height: DesktopMetrics.headerHeight)
        .background(Color(NSColor.windowBackgroundColor))
    }

}

/// The little coloured status indicator.
struct StatusPill: View {
    let state: TerminalSession.State

    /// SwiftTerm also brings a `Color` type into scope, so qualify ours.
    private var tint: SwiftUI.Color {
        switch state {
        case .connecting: .orange
        case .connected: .green
        case .closed, .disconnected: .secondary
        case .failed: .red
        case .idle: .secondary
        }
    }

    private var icon: String {
        switch state {
        case .connecting: "hourglass"
        case .connected: "circle.fill"
        case .closed, .disconnected: "moon.zzz.fill"
        case .failed: "exclamationmark.triangle.fill"
        case .idle: "circle"
        }
    }

    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: icon).font(.system(size: 7)).foregroundStyle(tint)
            Text(state == .idle ? "Disconnected" : state.label)
                .font(.system(size: 11)).foregroundStyle(.secondary)
        }
        .fixedSize()
        .accessibilityElement(children: .combine)
    }
}

/// Shown when a session could not start, or ssh exited with an error.
struct NoticeBanner: View {
    @ObservedObject var session: TerminalSession
    @EnvironmentObject private var sessions: SessionManager
    @EnvironmentObject private var appState: AppState
    let message: String

    var body: some View {
        HStack(alignment: .top, spacing: 10) {
            Image(systemName: "exclamationmark.triangle.fill")
                .foregroundStyle(.orange)
                .padding(.top, 1)

            Text(message)
                .font(.callout)
                .fixedSize(horizontal: false, vertical: true)
                .frame(maxWidth: .infinity, alignment: .leading)
                .textSelection(.enabled)

            if session.needsVaultUnlock {
                Button("Unlock Vault…") { appState.isVaultSheetPresented = true }
                    .fixedSize()
            } else if session.needsPassword {
                Button("Edit Host…") { appState.edit(session.host) }
                    .fixedSize()
            } else {
                Button("Try Again") { sessions.reconnectSession(session.id) }
                    .fixedSize()
            }
        }
        .padding(.horizontal, 16)
        .padding(.vertical, 10)
        .background(SwiftUI.Color.orange.opacity(0.12))
    }
}
