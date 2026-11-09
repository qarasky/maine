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

- The window uses a hidden title bar, so the detail pane ignores the top safe area: the
  56-point connection header starts at the window's top edge (the traffic lights sit
  over the sidebar) instead of below an empty title-bar inset.
- A real resizable `HSplitView` without NavigationSplitView's unneeded navigation
  toolbar/insets. Set the initial sidebar to 260 points; retain 220–340 resizing.
- One desktop spacing vocabulary in `DesktopMetrics`: 10-point sidebar margins,
  44-point two-line host rows, 30-point session rows, 26-point group headers,
  20-point child indentation,
  56-point connection header, 36-point footer, 12-point terminal breathing room.
  These are Maine's choices, not prescribed Apple measurements.
- Native `NSSearchField`, with its clear button and Command-F. It matches name,
  address and username; while searching, groups with no match disappear and
  matching groups stay expanded. Search results, an empty host list and a locked
  vault have distinct explanations and actions.
- Hover and selection share the exact same rounded row. Inactive-window selection
  is subdued. Close controls keep fixed hit areas; buttons no longer scale.
- Hovering a host row reveals a copy button (24-point hit area, same style as
  close) just left of its close control, or at the row's trailing edge when the
  host row has no close control (no session, or several). It copies the address
  and, when present, the port, username and saved password, then shows a
  checkmark for about a second; it never shifts the row's highlight bounds.
- A host row shows child session rows only when it needs them: more than one
  session, or a session that is renamed, being renamed, created by an agent, or
  left behind by a disconnected agent.
- Agent-created sessions show as child rows under their host with a small `cpu`
  badge (a pause glyph when paused, a mini spinner while the agent types or a
  command runs); a session the user hands to an agent keeps its normal row and
  stays a saved session. Above the terminal, a tinted strip reads "Driven by *client*"
  with Pause/Resume, Stop and Keep Session, or "Agent disconnected" with Keep
  Session and Close; it is fixed-size like the notice banner and never grows
  the split view. A live session's context menu offers **Hand to Agent** for each
  paired client when agent access is enabled and the host is allowed. After an
  agent-created session's control ends, its row keeps a tertiary `cpu` glyph
  until the session is kept or closed. Settings has Sync, Agents and Updates
  tabs; the Agents tab holds the master switch, client pairing, per-group and
  per-host access, and the device-local activity list.
- Groups are quiet section headers, in the style of Finder's sidebar: a rotating
  chevron, a secondary semibold name, and a host count when collapsed and
  non-empty. The whole header toggles and shares the row hover highlight. Hosts
  inside a group are not indented, so the two-line host rhythm is unchanged.
  Drop targets show an accent insertion line above a host, or an accent outline
  around a group header, an empty group's "Drag hosts here" row or the bottom
  "Remove from group" strip. Rename Group… edits the header in place; Delete
  Group leaves its hosts ungrouped and in order. Collapsed groups are remembered
  per vault in the device-local state file, so they survive relaunch and never
  sync. Collapse and drop animations respect Reduce Motion.
- The sidebar footer keeps the labeled New Host button, a New Group folder
  button, the app version with a gear button that opens Settings, and, while a
  sync profile is active, the sync status dot with the profile name in its
  tooltip. The search field sits just below the traffic lights; the hidden title
  bar's safe-area inset provides that clearance, so it gets almost no padding.
- Name and endpoint are the connection hierarchy. The full SSH command remains
  available in the endpoint's help tooltip; status uses a small icon and label,
  not a large bright capsule. Restored sessions reconnect on their own, so the
  header carries no Connect button; failures surface through the notice banner.
- Double-click renames a session in place: on a host row that has exactly one
  session, or on a child session row. Return commits, Escape cancels, loss of
  focus commits. Empty names cancel. Delete closes the selected session; deleting
  a saved host remains an explicit menu/context action.
- Adding and editing a host use the same 480-point sheet: a grouped form with
  Name, Address, Port (blank means 22), optional Username, Group and an optional
  password. The password prompt reads "Optional, leave empty for key login", so
  key-based hosts save without one. While the vault is locked the password field
  is disabled and the form offers Unlock Password Vault… (or Create Password
  Vault… on first use). Validation issues are listed inside the form, and Add
  Host / Save Changes stays disabled until the host is valid.
- Generated versus custom session names persist separately, so an ordinary
  single-session host does not unexpectedly expand after restarting the app.
- The terminal keeps consistent dark contrast in both appearances, independent
  of sidebar chrome. Terminal rendering and scrollback remain SwiftTerm-owned.
- Existing short, Reduce-Motion-aware transitions remain. No gradients, ornamental
  cards, enormous titles, added tab bars, decorative charts or unrelated features.
- Maine is a menu bar resident: the status item carries the M mark, left-click
  restores the window, and right-click keeps Lock, Unlock and Quit one gesture
  away. While agents are running, that menu also shows "Agents: N active" and
  Stop All Agents. Command-W hides the window; Shift-Command-W still closes the
  selected session. The Dock icon restores the window the same way; no background
  item is added. The single-window, sidebar-only layout is unchanged when the
  window returns.
- An available update is a standard `NSAlert`: "Maine X.Y.Z is available",
  "You have A.B.C.", release notes in a scrollable accessory capped at about
  4,000 characters, and exactly **Update**, **Later** and **Skip This Version**.
  **Later** offers it again at the next check; **Skip This Version** remembers
  that version locally, so automatic checks ignore it while a manual check still
  offers it. Only one update popup is open at a time.
- **Settings → Updates** shows the running version, "Check for updates
  automatically" (default on) and **Check Now**, with a quiet status line: up to
  date, last checked, downloading, or the error. **Maine → Check for Updates…**
  does the same from the app menu and reports a failure instead of staying
  silent. A successful **Update** quits Maine and relaunches it after the swap.

## Verification

Hosted-window tests cover light/dark appearances, 780×520, 1080×720 and 1440×900,
single/multiple sessions, fixed chrome heights, terminal insets, and bounded
header/terminal geometry. Optional `MAINE_UI_PREVIEW_DIR` captures only the synthetic
fixture window, with invented endpoints/output, for visual inspection. Tests also
retain the error-banner/disconnect, reconnect, encrypted persistence and lifecycle
regressions. Real SSH interaction still needs an application retest.
