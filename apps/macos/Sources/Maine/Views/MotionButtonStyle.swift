import SwiftUI

/// Quiet hover/press feedback without changing the button's hit area.
struct MotionButtonStyle: ButtonStyle {
    func makeBody(configuration: Configuration) -> some View {
        Feedback(configuration: configuration)
    }

    private struct Feedback: View {
        let configuration: ButtonStyleConfiguration
        @Environment(\.accessibilityReduceMotion) private var reduceMotion
        @Environment(\.isEnabled) private var isEnabled
        @State private var hovering = false

        var body: some View {
            configuration.label
                .background(Color.primary.opacity(hovering && isEnabled ? 0.055 : 0),
                            in: RoundedRectangle(cornerRadius: 6))
                .opacity(configuration.isPressed ? 0.65 : (isEnabled ? 1 : 0.5))
                .animation(reduceMotion ? nil : .easeOut(duration: 0.12), value: configuration.isPressed)
                .animation(reduceMotion ? nil : .easeOut(duration: 0.12), value: hovering)
                .onHover { hovering = $0 }
        }
    }
}
