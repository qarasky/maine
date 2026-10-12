import AppKit
@testable import Maine
import Testing

/// The menu bar resident: closing the window must never end the process, and
/// the window must be hidden and restored rather than destroyed.
@MainActor
@Suite(.serialized)
struct MenuBarControllerTests {
    private func makeWindow() -> NSWindow {
        let window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 400, height: 300),
                              styleMask: [.titled, .closable], backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        return window
    }

    @Test
    func theCloseButtonHidesTheWindowInsteadOfClosingIt() throws {
        _ = NSApplication.shared
        let controller = MenuBarController.shared
        let window = makeWindow()
        defer {
            controller.trackMainWindow(nil)
            window.close()
        }

        window.makeKeyAndOrderFront(nil)
        controller.trackMainWindow(window)
        try #require(window.isVisible)

        let closeButton = try #require(window.standardWindowButton(.closeButton))
        #expect(closeButton.target === controller)
        #expect(closeButton.action == #selector(MenuBarController.hideMainWindow(_:)))

        closeButton.performClick(nil)
        #expect(!window.isVisible)
    }

    @Test
    func hidingAndRestoringKeepsTheSameWindow() throws {
        _ = NSApplication.shared
        let controller = MenuBarController.shared
        let window = makeWindow()
        defer {
            controller.trackMainWindow(nil)
            window.close()
        }

        window.makeKeyAndOrderFront(nil)
        controller.trackMainWindow(window)
        try #require(window.isVisible)

        controller.hideMainWindow()
        #expect(!window.isVisible)

        controller.showMainWindow()
        #expect(window.isVisible)
        #expect(window.standardWindowButton(.closeButton)?.target === controller)
    }

    @Test
    func commandWHidesTheWindowAndShiftCommandWIsLeftAlone() throws {
        _ = NSApplication.shared
        let controller = MenuBarController.shared
        let window = makeWindow()
        defer {
            controller.trackMainWindow(nil)
            window.close()
        }

        window.makeKeyAndOrderFront(nil)
        controller.trackMainWindow(window)
        try #require(window.isVisible)

        let shiftCommandW = try #require(NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: [.command, .shift], timestamp: 0,
            windowNumber: window.windowNumber, context: nil,
            characters: "w", charactersIgnoringModifiers: "w", isARepeat: false, keyCode: 13))
        let commandW = try #require(NSEvent.keyEvent(
            with: .keyDown, location: .zero, modifierFlags: .command, timestamp: 0,
            windowNumber: window.windowNumber, context: nil,
            characters: "w", charactersIgnoringModifiers: "w", isARepeat: false, keyCode: 13))

        // ⇧⌘W belongs to Close Session; it must reach its normal handler.
        #expect(controller.handleCloseKey(shiftCommandW) === shiftCommandW)

        // ⌘W hides: the event is swallowed and the same window can return.
        #expect(controller.handleCloseKey(commandW) == nil)
        #expect(!window.isVisible)
    }

    @Test
    func theAppDoesNotQuitWhenItsLastWindowCloses() {
        _ = NSApplication.shared
        let delegate = MaineAppDelegate()
        #expect(delegate.applicationShouldTerminateAfterLastWindowClosed(NSApp) == false)
    }
}
