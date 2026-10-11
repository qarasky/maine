import SwiftUI

/// Geometry-only probes that do not participate in layout or intercept input.
extension View {
    func debugRegion(_ name: String, sessionID: UUID? = nil) -> some View {
        background {
            GeometryReader { geometry in
                let frame = geometry.frame(in: .global)
                Color.clear
                    .onAppear { logRegion("appear", name: name, sessionID: sessionID, frame: frame) }
                    .onChange(of: frame) { _, newFrame in
                        logRegion("geometry", name: name, sessionID: sessionID, frame: newFrame)
                    }
                    .onDisappear { logRegion("disappear", name: name, sessionID: sessionID, frame: frame) }
            }
            .allowsHitTesting(false)
            .accessibilityHidden(true)
        }
    }
}

@MainActor
private func logRegion(_ event: String, name: String, sessionID: UUID?, frame: CGRect) {
    DebugDiagnostics.recordRegion(event, name: name, sessionID: sessionID, frame: frame)
}
