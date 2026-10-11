import SwiftUI

/// Hover and selection paint exactly the same padded row, including its close control.
struct SidebarRowHighlight: ViewModifier {
    let selected: Bool
    @State private var hovering = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.controlActiveState) private var activeState

    func body(content: Content) -> some View {
        content
            .background(selected ? (activeState == .active ? Color.accentColor.opacity(0.18) : Color.primary.opacity(0.10))
                        : Color.primary.opacity(hovering ? 0.055 : 0),
                        in: RoundedRectangle(cornerRadius: 6))
            .contentShape(RoundedRectangle(cornerRadius: 6))
            .onHover { hovering = $0 }
            .animation(reduceMotion ? nil : .easeOut(duration: 0.12), value: hovering)
    }
}
