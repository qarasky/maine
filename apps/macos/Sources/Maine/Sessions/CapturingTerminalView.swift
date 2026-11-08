import AppKit
import Foundation
import SwiftTerm

/// Callbacks from a `CapturingTerminalView` to the session that owns it.
@MainActor
protocol CapturingTerminalViewHost: AnyObject {
    /// The user is in charge: OSC 52 clipboard, link clicks and the bell work. False while an agent drives.
    var isUserDriven: Bool { get }
    func capturedOutput(_ bytes: ArraySlice<UInt8>)
    func capturedUserInput()
    func capturedSizeChange()
    func capturedTermination(exitCode: Int32?)
    func capturedLaunchFailure(_ error: LocalProcessError)
}

/// The terminal for every session. SwiftTerm's `LocalProcessTerminalView`
/// hands PTY bytes straight to its parser, so an agent could not see them;
/// this view owns its own `LocalProcess`, passes every byte to the session
/// (for an agent's output buffer, when one is attached) and then renders it
/// exactly as usual. A terminal view is never replaced, so having it on every
/// session is what lets a session the user opened be handed to an agent.
final class CapturingTerminalView: TerminalView, TerminalViewDelegate, LocalProcessDelegate {
    weak var host: CapturingTerminalViewHost?
    var pasteboard: NSPasteboard = .general
    private var process: LocalProcess!

    override init(frame: CGRect, font: NSFont? = nil, options: TerminalOptions) {
        super.init(frame: frame, font: font, options: options)
        terminalDelegate = self
        process = LocalProcess(delegate: self, dispatchQueue: .main)
    }

    required init?(coder: NSCoder) { fatalError("CapturingTerminalView is created in code") }

    var shellPid: pid_t { process.shellPid }
    var isRunning: Bool { process.running }

    func startProcess(executable: String, args: [String], environment: [String]?) {
        process.startProcess(executable: executable, args: args, environment: environment)
    }

    func terminate() { process.terminate() }

    /// Agent input: written to the process without counting as user typing.
    func sendToProcess(_ bytes: [UInt8]) {
        process.send(data: bytes[...])
    }

    // MARK: - LocalProcessDelegate (delivered on the main queue)

    nonisolated func dataReceived(slice: ArraySlice<UInt8>) {
        MainActor.assumeIsolated {
            host?.capturedOutput(slice)
            feed(byteArray: slice)
        }
    }

    nonisolated func processTerminated(_ source: LocalProcess, exitCode: Int32?) {
        MainActor.assumeIsolated { host?.capturedTermination(exitCode: exitCode) }
    }

    nonisolated func processFailedToStart(_ source: LocalProcess, error: LocalProcessError) {
        MainActor.assumeIsolated { host?.capturedLaunchFailure(error) }
    }

    nonisolated func getWindowSize() -> winsize {
        let dimensions = terminalDimensions
        return winsize(ws_row: UInt16(dimensions.rows), ws_col: UInt16(dimensions.cols), ws_xpixel: 0, ws_ypixel: 0)
    }

    // MARK: - TerminalViewDelegate

    func sizeChanged(source: TerminalView, newCols: Int, newRows: Int) {
        var size = getWindowSize()
        _ = process.updateWindowSize(&size)
        host?.capturedSizeChange()
    }

    func send(source: TerminalView, data: ArraySlice<UInt8>) {
        host?.capturedUserInput()
        process.send(data: data)
    }

    /// OSC 52 behaves like SwiftTerm's stock view for the user's sessions, and
    /// is off while an agent drives one: an agent could otherwise read the
    /// user's clipboard (which may hold copied credentials) or plant text in it.
    func clipboardCopy(source: TerminalView, content: Data) {
        guard host?.isUserDriven == true, let text = String(data: content, encoding: .utf8) else { return }
        pasteboard.clearContents()
        pasteboard.writeObjects([text as NSString])
    }

    func clipboardRead(source: TerminalView) -> Data? {
        guard host?.isUserDriven == true else { return nil }
        return pasteboard.string(forType: .string)?.data(using: .utf8)
    }

    func setTerminalTitle(source: TerminalView, title: String) {}
    func hostCurrentDirectoryUpdate(source: TerminalView, directory: String?) {}
    func scrolled(source: TerminalView, position: Double) {}
    func requestOpenLink(source: TerminalView, link: String, params: [String: String]) {
        guard host?.isUserDriven == true else { return } // a human's link click, not an agent's output
        TerminalView.openDefaultLink(link)
    }
    func bell(source: TerminalView) {
        if host?.isUserDriven == true { NSSound.beep() }
    }
    func iTermContent(source: TerminalView, content: ArraySlice<UInt8>) {}
    func rangeChanged(source: TerminalView, startY: Int, endY: Int) {}
}
