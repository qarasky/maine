# Maine: desktop design references and decisions

Reviewed October 11, 2026. The objective is a well-behaved Mac SSH utility,
not a web dashboard, a task manager skin, or a copy of another app's branding.

## References examined

- [Apple's 2017 Design Award announcement](https://www.apple.com/newsroom/2017/06/apple-design-awards-celebrate-the-best-in-innovation-and-creativity/)
  verifies **Things 3** and **Bear** as winners, including their Mac versions.
- [Things](https://culturedcode.com/things/): examined the official Mac screenshot
  in its product collage. Compact sidebar groups, consistent icon/text columns,
  restrained selection, and an explicitly labeled bottom creation action. Its
  expansive document whitespace is not appropriate for a terminal.
- [Bear](https://bear.app/): examined the official Mac hero screenshot. Clear
  nested sidebar indentation, subdued secondary information, small controls,
  and content that dominates the window instead of the chrome.
- [Nova](https://nova.app/): examined the official Nova 14 Mac screenshot. Useful
  developer-tool reference for dense navigation and content boundaries. Nova
  was a **2021 finalist**, not a winner ([Apple's award archive](https://developer.apple.com/design/awards/2021)).
  Its purple theme and extensive toolbar are deliberately not copied.
- Apple's [Designing for macOS](https://developer.apple.com/design/human-interface-guidelines/designing-for-macos),
  [Sidebars](https://developer.apple.com/design/human-interface-guidelines/sidebars),
  [Layout](https://developer.apple.com/design/human-interface-guidelines/layout), and
  [Toolbars](https://developer.apple.com/design/human-interface-guidelines/toolbars):
  precision input, keyboard operation, clear navigation/content separation and
  appropriate native controls. The documentation pages use client-side rendering;
  their text was available in search extracts, not fully in the text fetcher.

These are references for established patterns, not a claim that there is one
objectively "best" Mac design. Current Liquid Glass guidance is not backported
as a custom imitation: Maine supports macOS 14 and uses native system materials.

## Implementation

- A real resizable `HSplitView` without NavigationSplitView's unneeded navigation
  toolbar/insets. Set the initial sidebar to 260 points; retain 220–340 resizing.
- One desktop spacing vocabulary in `DesktopMetrics`: 10-point sidebar margins,
  44-point two-line host rows, 30-point session rows, 20-point child indentation,
  56-point connection header, 36-point footer, 12-point terminal breathing room.
  These are Maine's choices, not prescribed Apple measurements.
- Native `NSSearchField`, with its clear button and Command-F. Search results,
  an empty host list and a locked vault have distinct explanations and actions.
- Hover and selection share the exact same rounded row. Inactive-window selection
  is subdued. Close controls keep fixed hit areas; buttons no longer scale.
- Name and endpoint are the connection hierarchy. The full SSH command remains
  available in the endpoint's help tooltip; status uses a small icon and label,
  not a large bright capsule. Restored sessions reconnect on their own, so the
  header carries no Connect button; failures surface through the notice banner.
- Double-click rename edits in place: Return commits, Escape cancels, loss of
  focus commits. Empty names cancel. Delete closes the selected session; deleting
  a saved host remains an explicit menu/context action.
- Generated versus custom session names persist separately, so an ordinary
  single-session host does not unexpectedly expand after restarting the app.
- The terminal keeps consistent dark contrast in both appearances, independent
  of sidebar chrome. Terminal rendering and scrollback remain SwiftTerm-owned.
- Existing short, Reduce-Motion-aware transitions remain. No gradients, ornamental
  cards, enormous titles, added tab bars, decorative charts or unrelated features.

## Verification

Hosted-window tests cover light/dark appearances, 780×520, 1080×720 and 1440×900,
single/multiple sessions, fixed chrome heights, terminal insets, and bounded
header/terminal geometry. Optional `MAINE_UI_PREVIEW_DIR` captures only the synthetic
fixture window, with invented endpoints/output, for visual inspection. Tests also
retain the error-banner/disconnect, reconnect, encrypted persistence and lifecycle
regressions. Real SSH interaction still needs an application retest.
