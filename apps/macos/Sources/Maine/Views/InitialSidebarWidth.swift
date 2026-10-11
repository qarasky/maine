import SwiftUI

/// HSplitView otherwise starts its flexible sidebar at its maximum width.
/// Set the divider once; subsequent user resizing remains AppKit-owned.
struct InitialSidebarWidth: NSViewRepresentable {
    func makeNSView(context: Context) -> Probe { Probe() }
    func updateNSView(_ nsView: Probe, context: Context) { }

    final class Probe: NSView {
        private var applied = false
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            guard window != nil, !applied else { return }
            Task { @MainActor [weak self] in
                guard let self, !self.applied else { return }
                var ancestor = self.superview
                while let view = ancestor {
                    if let split = view as? NSSplitView, split.isVertical, split.subviews.count > 1 {
                        split.setPosition(DesktopMetrics.sidebarWidth, ofDividerAt: 0)
                        self.applied = true
                        return
                    }
                    ancestor = view.superview
                }
            }
        }
    }
}
