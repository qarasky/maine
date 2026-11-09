# Agent access over MCP — design

Status: **all three phases built**: core tools, socket, tokens, access,
Settings → Agents, sidebar badge and controls; `type_password`; OSC 133
completion; the encrypted activity log; "Hand to Agent".
Implementation notes: every session uses `CapturingTerminalView` (SwiftTerm's
`LocalProcessTerminalView` exposes no byte tap); marker support is probed once
per session after login settles or before the first command; results are JSON
objects (`list_hosts` → `{hosts: [...]}`), not bare arrays.

## Intent

Let an AI agent (Claude Code, Claude Desktop, or any MCP client) work on a
user's servers *through Maine*, in real sessions the user can watch and take
over, instead of firing one-off `ssh host cmd` calls nobody sees.

What makes this worth doing in Maine specifically:

* **Persistent sessions.** A session keeps its shell, working directory,
  environment, `sudo` timestamp and running jobs between agent calls.
* **The human sees everything, live.** An agent session is an ordinary Maine
  terminal in the sidebar. The user can read along, scroll, type into it, pause
  the agent or kill it.
* **Credentials stay in Maine.** Login uses the existing askpass path. When a
  remote prompt needs the saved password (typically `sudo`), the agent asks
  Maine to *type* it. The password goes into the terminal and never into the
  agent's context, the model provider's logs, or any tool result.
* **Scoped by the user.** Agents can only see and use the groups and hosts the
  user allowed, on this Mac.

## Goals

1. An MCP server exposing: list allowed hosts, open/close sessions, run a
   command and get its output and exit code, read output incrementally, read
   the visible screen, send keys, and type the saved password into a prompt.
2. Access is **off by default**, granted per group or per host, and stored on
   this device only.
3. A human-visible, interruptible experience: agent badge, activity indicator,
   Pause, Stop, Stop All.
4. **No permission modes and no approval prompts.** An allowed host is fully
   usable by the agent, including typing its saved password. Agents already
   have permission controls in their harnesses (Claude Code's permission
   modes, Desktop's tool approvals); Maine does not duplicate them. Maine's
   only control is *which hosts* an agent can reach.
5. No new network listener and no new third-party dependency.

## Non-goals (for this design)

* Agents reaching sessions the *human* opened on their own. Only the explicit
  **Hand to Agent** action (see *What the human sees*) gives an agent one.
* File transfer, port forwarding, SFTP.
* Remote (non-local) MCP clients. The server is reachable only from processes
  of the logged-in user on this Mac.
* Android support. Groups and access rules are macOS-only for now.

## Anti-goals — it fails if

* An agent can read a saved password, the vault key, or a sync token.
* An agent can learn that a non-allowed host exists.
* Agent activity happens somewhere the user can't see it.
* Access granted on one Mac silently applies on another (it must never sync).
* It adds a TCP port, an account, or a hosted component.
* It weakens an existing invariant without the change being written into
  `AGENTS.md` (see *Invariants*).

## Architecture

```
MCP client (Claude Code…)
   │  stdio, JSON-RPC (MCP)
   ▼
Maine --mcp            ← same executable, started by the client, no UI
   │  Unix socket  ~/Library/Application Support/Maine/agent.sock  (0600, dir 0700)
   │  length-prefixed JSON frames (4-byte big-endian length, max 8 MiB), token-authenticated
   ▼
Maine.app (running)    ← owns sessions, vault, access rules
   └─ AgentServer → AgentToolHandler → SessionManager / TerminalSession
```

* **The bridge (`Maine --mcp`).** `MaineEntryPoint.mode` looks for `--mcp`
  among the arguments after the executable path *before* `MaineApp.main()`,
  next to the branch that checks `MAINE_SSH_ASKPASS`. The bridge never creates
  an `NSApplication`. It implements the JSON-RPC surface over stdio (one
  message per line):
  `initialize` (a requested version among `2025-06-18`, `2025-03-26` and
  `2024-11-05` is echoed back; anything else gets `2025-06-18`), `ping`,
  `tools/list` and `tools/call`.
  Notifications get no reply. It forwards each tool call to the app over the
  socket and relays the result. It holds no state beyond the connection.
* **Why hand-rolled MCP.** The surface is four methods of JSON-RPC 2.0;
  hand-rolling keeps SwiftTerm as the package's only dependency. *Builder's
  call:* the official Swift MCP SDK is acceptable if it turns out to be
  materially simpler, but say so in the change.
* **If Maine isn't running**, the bridge answers `initialize` normally and
  every tool call returns the error "Maine is not running. Open Maine and
  unlock the vault." The bridge must **not** launch the app.
* **The app side** (`AgentServer`) listens on the socket only while the
  master switch (*Settings → Agents → Allow agent access*) is on. It checks
  the peer with `getpeereid` (same uid only) before reading anything. The
  bridge socket defaults to the path above and can be overridden with
  `MAINE_AGENT_SOCKET`.
* **Code layout.** Everything lives in `Sources/Maine/Agents/`:
  `MCPBridge.swift` (stdio side),
  `AgentServer.swift` (socket), `AgentSocket.swift` (POSIX helpers and the
  bridge's socket client), `AgentProtocol.swift` (frames and JSON values),
  `AgentToolCatalog.swift` (tool schemas), `AgentTools.swift` (the handler),
  `AgentDriver.swift` (drives one session), `CommandCompletion.swift`
  (wrapper, marker detector, OSC 133 detector, prompt heuristic),
  `AgentOutput.swift` (capture, ANSI stripping, masking), `AgentAccess.swift`
  (permission rules), `AgentClientStore.swift` (Keychain tokens),
  `AgentCoordinator.swift` (switchboard), `AgentAudit.swift` (activity
  entries); `Views/AgentSettingsView.swift`.

### Client tokens

Same-uid is the real security boundary; tokens make access *revocable and
attributable per client*.

* *Settings → Agents → Add Client…* asks for a name (defaults to "Claude
  Code"; an empty name becomes "MCP client"), generates a 256-bit token
  (`maine-agent-` followed by the 32 random bytes in Crockford base32,
  alphabet `0-9A-Z` without `I`, `L`, `O`, `U`), shows it **once** with a
  ready-to-copy config, e.g.

  ```sh
  claude mcp add maine --env MAINE_AGENT_TOKEN=maine-agent-… -- \
      "/Applications/Maine.app/Contents/MacOS/Maine" --mcp
  ```

  and the equivalent JSON for Claude Desktop (path = the running app's
  `Bundle.main.executableURL`):

  ```json
  {"mcpServers": {"maine": {"command": "/Applications/Maine.app/Contents/MacOS/Maine",
                            "args": ["--mcp"],
                            "env": {"MAINE_AGENT_TOKEN": "maine-agent-…"}}}}
  ```
* Tokens are stored in the Keychain (service `Maine Agents`, account = client
  UUID, accessible only while unlocked and never migrated to another Mac),
  like sync tokens. Only the SHA-256 of the token is stored; the app compares
  it in constant time. Revoking deletes the item; live connections of that
  client close immediately and agent control of its sessions ends, with the
  sessions left open for the human.
* The bridge reads `MAINE_AGENT_TOKEN` from its environment and sends it in
  the first frame. Missing/invalid token → the app answers `{ok: false}` and
  closes the socket; the bridge returns "This MCP client is not paired with
  Maine." for every call.
* Honest limit, stated in Settings: any program running as this user can read
  the client's config file and reuse the token. Maine cannot defend against
  malware already running as the user.

## Permissions

Stored **per vault, on this device**: in the encrypted local state file
(`sessions-<vault id>.maine`) as the `agentAccess` field of its version 2
`LocalState` payload, next to `agentAudit`. Never in the vault, never synced,
never in `UserDefaults`. (The master switch is an ordinary device preference,
key `agentAccessEnabled`.)

```swift
struct AgentAccess: Codable, Equatable {
    var groups: Set<UUID> = []   // every host in these groups is allowed
    var hosts: Set<UUID> = []    // individually allowed hosts (grouped or not)
}
```

* A host is **allowed** if its id is in `hosts` or its group is in `groups`.
  There is nothing else to configure: allowed means fully usable (commands,
  keys, saved-password typing). Ungrouped hosts are allowed individually. A
  host with no saved password opens with `~/.ssh` keys or ssh-agent as usual,
  and only `type_password` is unavailable for it.
* Everything else is invisible: `list_hosts` omits it, and any id that isn't
  allowed returns the same "not found" as an id that doesn't exist.
* Deleting a rule or a host, or moving a host out of an allowed group, takes
  effect on the next tool call (access is judged against the host's group as
  it is now); a `run` in progress stops. Agent control then ends and the
  session stays open for the human, its strip reading "Agent disconnected".
* Rules referencing groups/hosts that no longer exist are pruned whenever
  access is read and again before it is saved.

## Tools

All tools return MCP `content` with a single `text` item containing compact
JSON, plus `structuredContent` with the same object. Tool failures return
`isError: true` with a human-readable message in the `text` item. An unknown
tool name is a JSON-RPC error (`-32602`). Ids are UUID strings.

| Tool | Input | Result |
|---|---|---|
| `list_hosts` | none | `{hosts: [{id, name, group, hostname, port, username, has_saved_password}]}`. Allowed hosts in sidebar order; `group` is the group name or null. Never passwords. |
| `list_sessions` | none | `{sessions: [{session_id, host_id, name, state, busy}]}`. Sessions this client opened or was handed, whose host is still allowed. |
| `open_session` | `host_id`, `name?` | `{session_id, state, ready, output, hint?}`. Opens a new session (never reuses one) and waits up to 30 s for the remote shell to be ready (a shell prompt, or 1.5 s quiet at no question), failure, or ssh stopping at a password/host-key prompt. `ready: false` means `run` is refused (`ssh is still connecting…`) until the shell is up; `read_screen`, `type_password` and `send_keys` still work to answer the prompt. `list_sessions` shows such a session as `Connecting…`. |
| `run` | `session_id`, `command`, `timeout_seconds?` (1–600, default 60) | `{completed, exit_code, output, cursor, truncated, mode}`. See *Command completion*. If not completed in time: `completed: false`, partial output; the command keeps running. |
| `read_output` | `session_id`, `since?` (cursor, default 0 = everything retained), `wait_seconds?` (0–60, default 0), `max_bytes?` (1–65536, default 65536) | `{output, cursor, busy, truncated}`. New output since `since`; with `wait_seconds`, waits for new output or completion. |
| `read_screen` | `session_id`, `wait_seconds?` (0–60, default 0) | `{rows: [String], cursor_row, cursor_col, alternate_screen, changed, busy}` — the visible screen as text, for TUIs, prompts and multi-line progress that redraws in place. With `wait_seconds`, waits until the screen differs from what this agent's previous `read_screen` returned, the command ends, or time runs out; `changed` says whether it differs (always true on the first read). |
| `send_keys` | `session_id`, `keys: [String]` (at least one) | `{sent}`, the number of keys sent. Each item is a named key (`enter`, `tab`, `esc`, `backspace`, `up`, `down`, `left`, `right`, `ctrl-a`…`ctrl-z`) or `text:<literal>`. |
| `type_password` | `session_id` | `{typed: true}` or an error explaining why not. See *Saved password*. |
| `close_session` | `session_id` | `{closed: true}` — same as the user's explicit close. |

Rules shared by all tools:

* **Vault locked → every tool fails** with "Maine is locked. Ask the user to
  unlock it." Agents cannot unlock it. Locking also pauses all agent sessions
  (see *Lifecycle*).
* **Not found:** a session id that is unknown, belongs to another client, or
  sits on a host that is no longer allowed answers "not found", exactly like
  one that never existed.
* **Busy:** `run` refuses while a `run` in that session is still incomplete
  ("a command is still running; use read_output, or send_keys ctrl-c").
  `send_keys`, `type_password` and the read tools are allowed while busy —
  answering a prompt from the running command (typically `sudo`) is the point.
* **Paused by the user:** every call on that session returns "paused by the
  user" until Resume.
* **Output size:** any single result carries at most 64 KiB of output; longer
  output is cut in the middle with a `…[N bytes omitted]…` marker and
  `truncated: true`. The per-session capture buffer keeps the last 2 MiB.

## Command completion

The hard problem: knowing a command finished and what its exit code was.

**Default: a per-command marker.** `run` types:

```
{ <command>
}; printf '\033]7770;<nonce>;%d\007' "$?"
```

* `<nonce>` is 16 fresh random bytes as lowercase hex (32 characters) per
  command. The completion detector scans the **raw PTY byte stream** for the
  OSC sequence `ESC ] 7770 ; <nonce> ; <n> BEL`. The echoed command line
  contains the characters `\033`, not an ESC byte, so the echo never matches;
  a remote program can't guess the nonce, so it can't fake completion. The
  marker may be split across chunks.
* The `{ … }` group plus a newline before `}` keeps `#` comments, trailing
  `&&`, and multi-line commands working. Unbalanced quotes leave the shell
  waiting; that surfaces as a timeout, and the agent can `send_keys ctrl-c`.
* SwiftTerm ignores the unknown OSC, so the human sees only the echoed
  wrapper line, not the marker output. That echo is visible noise in the
  terminal; it's the accepted cost.
* `output` returned to the agent is the cleaned stream between the command
  being typed and the marker, with the echoed wrapper lines removed.
* Works in bash, zsh, sh/dash, ksh. **fish** and other non-POSIX shells: the
  session probes once, after login settles or before the first command, by
  typing a wrapped `true` and waiting up to 2 s for its marker. No marker
  means *quiet-period mode*: `completed` after 1.5 s of no output with the
  cursor on a line that looks like a prompt; `exit_code` is null. An
  interrupted run (`ctrl-c`/`ctrl-d`) finishes after 0.5 s of quiet. The
  `run` result's `mode` says which mode was used (`marker`, `quiet` or
  `osc133`).
* **OSC 133 (built):** if the remote shell itself emits `ESC ] 133 ; A` prompt
  marks (iTerm2/VS Code/Ghostty shell integration), the session runs in
  `osc133` mode: `run` types the bare command (no wrapper, no echo noise),
  `C` marks where the output starts and `D;<exit>` ends the command. The
  marks carry no nonce, so a program's output could fake one; the worst that
  does is end a `run` early. Plain shells keep the nonce marker above.

## Output capture

* Every session runs on `CapturingTerminalView`, which owns its own
  `LocalProcess`: each chunk of PTY output goes to the session (and so to the
  agent's `AgentOutput` ring) before it is fed to the terminal renderer. Bytes
  reach the terminal unchanged; the human view is unaffected. This is also
  what lets a session the user opened be handed to an agent without replacing
  its terminal view.
* Cleaning for the agent: strip CSI/OSC/DCS (and APC/PM/SOS) sequences, apply
  `\r`-overwrite and backspace on the current line, keep tabs, drop NULs,
  normalize to `\n`. The **cursor** is a position in the cleaned stream that
  only grows, and grows on every change: for plain output it is the byte
  count, and a line redrawn in place behind `\r` (progress bars, spinners)
  advances it on each redraw that changes the text, so `read_output` with
  `wait_seconds` wakes for it and a narrower redraw never moves it backwards.
  A `since` inside a line returns that whole line again. The current, unfinished line is readable, so prompts are visible. A line is
  capped at 64 KiB. Saved-password masks are applied when a line is rendered,
  so a password never appears in the buffer.
* `read_screen` reads SwiftTerm's visible buffer instead, for full-screen
  programs (`alternate_screen: true` when the terminal is in the alternate
  screen, tracked from `CSI ?1049/1047/47 h/l`). Masks apply to these rows too.
  It is also the right read for multi-line progress that moves the cursor up
  (`docker compose pull`, `apt`): the cleaned stream ignores cursor movement,
  so those redraws arrive there as repeated lines. A waiting `read_screen`
  re-renders the rows only when PTY output arrived, and compares them (plus
  cursor and alternate-screen state) with what it last returned. It holds
  only the visible rows; history is `read_output`.

## Saved password (`type_password`)

Types the host's saved password plus Enter into the session. It never appears
in any result, log, or notification.

Preconditions, all required:

1. The host is allowed (as for every tool) and has a saved password. Without
   one the call fails with "no saved password for this host".
2. The **last non-empty screen line** looks like a password prompt:
   `SSHAskPass.isPasswordPrompt`, i.e. the line contains `password:` or
   contains `password for …` and ends with `:`, and it is not a passphrase,
   one-time or verification prompt. The driver waits up to 500 ms for the
   prompt to reach the screen. Otherwise the call fails with "no password
   prompt is showing; the password was not typed".

No confirmation: allowing a host includes letting agents type its saved
password into a detected prompt. Prompt detection is a correctness guard (so
the password is never typed into a shell line or an editor), not a
permission check.

After typing:

* **Masking:** every later output result for that session replaces each
  occurrence of the password with `••••••` before it reaches the agent.
  Masking also applies to `read_screen`.
* State plainly in Settings and here: this keeps the password out of the
  agent's context; it does **not** stop an agent with a shell (and sudo) from
  capturing input it causes to be typed. Grant it only to agents you'd trust
  with sudo.

## What the human sees

* **Hand to Agent:** a session's context menu lists *Hand to Agent ▸ client*
  when agent access is on, the vault is unlocked, the host is allowed, the
  session has no agent, and at least one client is paired. The agent then
  sees that session in `list_sessions` and drives it like one it opened, with
  the same strip, Pause and Stop. It is refused unless the session is live and
  the last screen line looks like a shell prompt (the first `run` types into
  the shell). The agent sees only output from that moment on. The session
  stays the user's: it keeps its terminal, is still saved and restored, has no
  "agent disconnected" leftover when control ends, and the hand-off is written
  to the activity log. OSC 52 clipboard, link clicks and the bell are off
  while an agent drives it, as in agent-opened sessions.

* **Sidebar:** agent sessions sit under their host like any session, with a
  small agent badge that spins while a command runs or the agent is typing; a
  paused agent shows a pause glyph. After control ends, an agent-opened
  session keeps a muted badge with the tooltip "Agent disconnected".

* **Strip above the terminal:** while driven, "Driven by *Claude Code*";
  while paused, "Paused" followed by the client name. Both show **Pause** /
  **Resume**, **Stop** and **Keep Session**. Stop ends agent control and keeps
  the session open. After control ends the strip reads "Agent disconnected
  (*Claude Code*)" with **Keep Session** and **Close**.

* **Human input wins:** the user can type at any time. While the user has
  typed within the last 2 s, agent `run`, `send_keys` and `type_password` wait
  (up to 5 s, then proceed).

* **Menu bar item:** "Agents: N active" and **Stop All Agents**, shown only
  while at least one agent session exists. Stop All ends all agent control;
  sessions stay open.

* **Settings → Agents:**
  * **Allow agent access** (off by default), then the consent note: *Terminal
    output of agent sessions and the names and addresses of allowed hosts are
    sent to the agent and its model provider. Allowed hosts are fully usable
    by agents, including typing their saved passwords into password prompts.
    Any program running as you can read a client's configuration and reuse
    its token.*
  * **Clients:** add (name, then the one-time token with the Claude Code
    command and Claude Desktop JSON, each with a Copy button) and revoke.
  * **Access:** a checkbox per group and per host. A host inside an allowed
    group is shown checked and disabled, because the group already covers it.
  * **Activity:** the latest 100 entries from the encrypted record, with a
    Clear button (see *Logging and privacy*).

## Lifecycle

* **Agent-opened sessions are ephemeral (approved).** They are not
  checkpointed into saved sessions, not restored after relaunch, and not
  reconnected automatically. After control ends they stay open, marked "Agent
  disconnected", until the user keeps or closes them. A **Keep Session**
  action converts one into an ordinary saved session (normal rules from then
  on). Reason: restoring and auto-reconnecting a session an agent was driving,
  with no agent attached, surprises the user; ephemeral avoids touching
  invariant 3.
* **Handed-off sessions are not ephemeral.** A session the user opened and
  handed to an agent keeps its saved record and its terminal while the agent
  drives it, and simply returns to the user when control ends.
* **Vault lock (approved):** pauses every agent session (the strip shows
  Paused) and fails all tools until unlock. The SSH processes keep running
  (invariant: lock keeps live sessions connected); only agent control is
  suspended. The visible activity list is empty while locked.
* **Quit:** agent-opened sessions close with everything else and are not
  persisted; a handed-off session is checkpointed like any other.
* **Workspace switch / pulled vault:** as for any session. Hosts that vanish
  close their sessions; access rules are per vault, so the other workspace's
  rules apply after a switch.
* **Master switch off:** the socket closes and agent control of every session
  ends. **A client revoked:** its connections close and agent control of its
  sessions ends; other clients keep working. In both cases the sessions stay
  open for the human.

## Logging and privacy

* `DebugLog` (opt-in diagnostics) records only: tool name, session UUID,
  client id, outcome, durations, byte counts. **Never** commands, keys,
  output, screen contents, host names, tokens or passwords — same rule as
  today, and debug regions stay geometry-only.
* What leaves the Mac, only after opt-in and only for allowed hosts: host
  name, group name, hostname, port, username, and the terminal output of
  agent sessions. Nothing about non-allowed hosts.
* **Activity log (built):** an encrypted, device-local record of agent actions
  (session opened, command with its exit code, keys sent, "typed the saved
  password", session closed, session handed off), newest 1000 kept per vault,
  shown in Settings → Agents → Activity (latest 100 shown) with a Clear
  button. It lives in the local state file's `agentAudit` field (never in the
  vault, never synced), is empty while locked, and never holds passwords,
  output, or the literal text of `send_keys` (a `text:` item is recorded as
  its length). A recorded command is cut at 2048 bytes. It is not written to
  diagnostics.

## Threat model (short)

| Threat | Mitigation |
|---|---|
| Other users on the Mac | Socket dir 0700, socket 0600, `getpeereid` uid check |
| Unpaired local program | Token required; revocable per client |
| Same-user malware with the token | Out of scope (stated) |
| Prompt injection from server output | Host scoping; the agent harness's own permission prompts; visible activity, Pause/Stop |
| Agent exfiltrating the typed password | Only into a detected prompt, masked in all output; limit stated — allow only hosts you'd trust the agent with sudo on |
| Faked command completion | Random per-command nonce in an OSC the echo can't produce (OSC 133 marks can only end a run early) |
| Runaway or hung commands | Timeouts, `send_keys ctrl-c`, Stop |
| Access leaking across devices | Rules and tokens are device-local, never synced |
| Agent reading the user's clipboard (OSC 52) | Clipboard read/write is disabled in agent sessions' terminal |
| A slow or stuck client stalling others | Socket writes run on per-connection queues, never under shared locks |
| A command run twice after a dropped connection | The bridge retries only calls that were never sent |

## Invariants (`AGENTS.md`)

Invariant 12 now reads:

> **12. Agent access is opt-in, scoped and local.** The MCP server is off by
> default and reachable only through `Maine --mcp` over a 0600 Unix socket
> with a revocable per-client token. Agents see only allowed groups/hosts
> (device-local rules in the encrypted local state file, never synced) and
> only sessions they opened or the user explicitly handed to them. There are
> no approval modes: an allowed host is fully usable; agents' own harnesses
> gate commands. Saved passwords are typed into detected prompts, never
> returned. Agent-created sessions are visible, pausable and ephemeral until
> the user keeps them; a session the user hands to an agent stays an ordinary
> saved session. Locking the vault pauses agent control.

Invariant 8 now says diagnostics stay metadata-only *including* agent tool
calls, and describes the separate user-facing activity log. Invariant 11 says
collapsed groups, agent access rules, the activity log and client tokens never
sync.

## Testing

Everything below runs headless under plain `swift test` — **no test may open
a window** (new window tests need `@Test(.uiTest)`; don't run them locally).
The suite is `Tests/MaineTests/`:

* MCP bridge (`MCPBridgeTests`): JSON-RPC framing,
  `initialize`/`tools/list`/`tools/call`, "not running" and "not paired" paths
  (bridge against an in-test socket).
* Socket server (`AgentServerTests`, `AgentProtocolTests`): socket
  permissions, token accept/reject/revoke, frame limits, stale socket files,
  concurrent traffic.
* Access rules (`AgentAccessTests`): resolution (group, host override,
  ungrouped), pruning, not-found for disallowed ids, storage in the local
  state file (vault bytes unchanged; absent on a copied vault).
* Completion detector (`CommandCompletionTests`, `OSC133Tests`): marker split
  across chunks, wrong nonce ignored, echoed wrapper never matches, exit codes,
  quiet-period fallback, OSC 133 marks.
* Output cleaning (`AgentOutputTests`): ANSI stripping, `\r`/backspace
  handling, 64 KiB middle truncation, cursors (monotonic across in-place
  redraws and ring trimming), masking across chunks; `AgentDriverTests`
  checks a waiting `read_output` wakes for a redraw, and a waiting
  `read_screen` wakes for a cursor-up redraw and otherwise reports no change.
* Password (`AgentDriverTests`, `AgentToolsTests`, `CommandCompletionTests`):
  prompt detection (sudo/plain/passphrase/MFA cases), masking, refusal paths.
* Activity log (`AgentAuditTests`): entry kinds, command truncation, the 1000
  entry limit, storage in the local state file.
* End to end with a real PTY and no SSH (`AgentDriverTests`,
  `AgentToolsTests`): a `TerminalSession` whose executable is `/bin/sh` (as
  `TerminalSessionTests` already do), driven through the tool handlers:
  `run "echo hi; false"` → output `hi`, exit 1.

## Phases (all built)

1. **Core:** bridge, socket, tokens, master switch, access rules, `list_hosts`,
   `list_sessions`, `open_session`, `run`, `read_output`, `read_screen`,
   `send_keys`, `close_session`, agent badge, Pause/Stop/Stop All,
   Settings → Agents, docs + invariant 12.
2. **Passwords:** `type_password` with prompt detection and masking.
3. **Polish:** encrypted local audit log, OSC 133 support, "Hand to Agent" for
   human-opened sessions.

## Decisions (owner, approved)

1. Agent-opened sessions are **ephemeral**: not restored, not auto-reconnected;
   **Keep Session** converts one into a normal saved session. A session the
   user hands to an agent keeps its saved record.
2. Vault **lock pauses agents**; SSH processes keep running.
3. **No permission modes or approval prompts.** Access is the only control:
   an allowed host is fully usable, including saved-password typing. Agent
   harnesses own command approval.
