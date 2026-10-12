import SwiftUI

/// Hands the scene's real `NSWindow` to the menu bar controller so the status
/// item can restore the resident window rather than merely reopen it.
struct MainWindowBridge: NSViewRepresentable {
    func makeNSView(context: Context) -> Probe { Probe() }
    func updateNSView(_ nsView: Probe, context: Context) { }

    final class Probe: NSView {
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            MenuBarController.shared.trackMainWindow(window)
        }
    }
}
