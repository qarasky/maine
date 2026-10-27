# Agent access over MCP — design

Status: **phase 1 built** (core tools, socket, tokens, access, Settings → Agents,
sidebar badge and controls). Phase 2 (`type_password`) and phase 3 are not built.
Implementation notes: agent sessions use `CapturingTerminalView` (SwiftTerm's
`LocalProcessTerminalView` exposes no byte tap); marker support is probed once
per session when it opens; results are JSON objects (`list_hosts` →
`{hosts: [...]}`).

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

* Agents controlling sessions the *human* opened. (Possible later as an
  explicit "Hand to agent" action; see *Later*.)
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
  `AGENTS.md` (see *Invariant changes*).

## Architecture

```
MCP client (Claude Code…)
   │  stdio, JSON-RPC (MCP)
   ▼
Maine --mcp            ← same executable, started by the client, no UI
   │  Unix socket  ~/Library/Application Support/Maine/agent.sock  (0600, dir 0700)
   │  length-prefixed JSON frames, token-authenticated
   ▼
Maine.app (running)    ← owns sessions, vault, access rules
   └─ AgentServer → AgentToolHandlers → SessionManager / TerminalSession
```

* **The bridge (`Maine --mcp`).** `MaineEntryPoint.main()` checks
  `CommandLine.arguments` for `--mcp` *before* `MaineApp.main()`, exactly like
  the askpass branch checks `MAINE_SSH_ASKPASS`. The bridge never creates an
  `NSApplication`. It implements the minimal MCP server surface over stdio:
  `initialize`, `notifications/initialized`, `ping`, `tools/list`,
  `tools/call`. It forwards each tool call to the app over the socket and
  relays the result. It holds no state beyond the connection.
* **Why hand-rolled MCP.** The surface is five methods of JSON-RPC 2.0;
  hand-rolling keeps SwiftTerm as the package's only dependency. *Builder's
  call:* the official Swift MCP SDK is acceptable if it turns out to be
  materially simpler, but say so in the change.
* **If Maine isn't running**, the bridge answers `initialize` normally and
  every tool call returns the error "Maine is not running. Open Maine and
  unlock the vault." The bridge must **not** launch the app.
* **The app side** (`AgentServer`) listens on the socket only while the
  master switch (*Settings → Agents → Allow agent access*) is on. It checks
  the peer with `getpeereid` (same uid only) before reading anything.
* **Code layout** (builder may adjust): `Sources/Maine/Agents/` —
  `MCPBridge.swift` (stdio side), `AgentServer.swift` (socket),
  `AgentProtocol.swift` (frames shared by both), `AgentTools.swift`,
  `AgentAccess.swift` (permission rules), `CommandCompletion.swift`,
  `AgentOutput.swift` (capture, ANSI stripping, masking);
  `Views/AgentSettingsView.swift`.

### Client tokens

Same-uid is the real security boundary; tokens make access *revocable and
attributable per client*.

* *Settings → Agents → Add Client…* asks for a name ("Claude Code"), generates
  a 256-bit token (reuse the sync token generator/format), shows it **once**
  with a ready-to-copy config, e.g.

  ```sh
  claude mcp add maine --env MAINE_AGENT_TOKEN=maine-… -- \
      /Applications/Maine.app/Contents/MacOS/Maine --mcp
  ```

  and the equivalent JSON for Claude Desktop (path = the running app's
  `Bundle.main.executableURL`).
* Tokens are stored in the Keychain (service `Maine Agents`, account = client
  id), like sync tokens. The app compares SHA-256 of the presented token in
  constant time. Revoking deletes the item; live connections of that client
  close immediately.
* The bridge reads `MAINE_AGENT_TOKEN` from its environment and sends it in
  the first frame. Missing/invalid token → the app closes the socket; the
  bridge returns "This MCP client is not paired with Maine" for every call.
* Honest limit, stated in Settings: any program running as this user can read
  the client's config file and reuse the token. Maine cannot defend against
  malware already running as the user.

## Permissions

Stored **per vault, on this device**: in the encrypted local state file
(`sessions-<vault id>.maine`, add an `agentAccess` field to `LocalState`;
bump the local file version). Never in the vault, never synced, never in
`UserDefaults`.

```swift
struct AgentAccess: Codable, Equatable {
    var groups: Set<UUID> = []   // every host in these groups is allowed
    var hosts: Set<UUID> = []    // individually allowed hosts (grouped or not)
}
```

* A host is **allowed** if its id is in `hosts` or its group is in `groups`.
  There is nothing else to configure: allowed means fully usable (commands,
  keys, saved-password typing). Ungrouped hosts are allowed individually.
* Everything else is invisible: `list_hosts` omits it, and any id that isn't
  allowed returns the same "not found" as an id that doesn't exist.
* Moving a host out of an allowed group, deleting a rule, or deleting the
  host takes effect immediately: agent control of its sessions ends (the
  session itself stays open for the human, marked "agent disconnected").
* Rules referencing groups/hosts that no longer exist are pruned on load.

## Tools

All tools return MCP `content` with a single `text` item containing JSON (and
`structuredContent` with the same object). Errors use `isError: true` with a
human-readable message. Ids are UUID strings.

| Tool | Input | Result |
|---|---|---|
| `list_hosts` | — | `[{id, name, group, hostname, port, username, has_saved_password}]` — allowed hosts only; `group` is the group name or null. Never passwords. |
| `list_sessions` | — | `[{session_id, host_id, name, state, busy}]` — only sessions this client opened. |
| `open_session` | `host_id`, `name?` | `{session_id, state, output}` — opens a new session (never reuses one), waits up to 30 s for login to settle (first quiet period after output, or failure), returns the login output. |
| `run` | `session_id`, `command`, `timeout_seconds?` (default 60, max 600) | `{completed, exit_code, output, cursor, truncated}`. See *Command completion*. If not completed in time: `completed: false`, partial output; the command keeps running. |
| `read_output` | `session_id`, `since?` (cursor), `wait_seconds?` (max 60), `max_bytes?` | `{output, cursor, busy}` — new output since `since`; with `wait_seconds`, waits for new output or completion. |
| `read_screen` | `session_id` | `{rows: [String], cursor_row, cursor_col, alternate_screen}` — the visible screen as text, for TUIs and prompts. |
| `send_keys` | `session_id`, `keys: [String]` | `{sent}` — each item is a named key (`enter`, `tab`, `esc`, `backspace`, `up`, `down`, `left`, `right`, `ctrl-a`…`ctrl-z`) or `text:<literal>`. |
| `type_password` | `session_id` | `{typed: true}` or an error explaining why not. See *Saved password*. |
| `close_session` | `session_id` | `{closed: true}` — same as the user's explicit close. |

Rules shared by all tools:

* **Vault locked → every tool fails** with "Maine is locked. Ask the user to
  unlock it." Agents cannot unlock it. Locking also pauses all agent sessions
  (see *Lifecycle*).
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

* `<nonce>` is fresh random hex per command. The completion detector scans
  the **raw PTY byte stream** for the OSC sequence `ESC ] 7770 ; <nonce> ; <n> BEL`.
  The echoed command line contains the characters `\033`, not an ESC byte, so
  the echo never matches; a remote program can't guess the nonce, so it can't
  fake completion.
* The `{ … }` group plus a newline before `}` keeps `#` comments, trailing
  `&&`, and multi-line commands working. Unbalanced quotes leave the shell
  waiting; that surfaces as a timeout, and the agent can `send_keys ctrl-c`.
* SwiftTerm ignores the unknown OSC, so the human sees only the echoed
  wrapper line, not the marker output. That echo is visible noise in the
  terminal; it's the accepted cost. (*Builder's call:* a short shell function
  defined once per session to shrink the wrapper is fine.)
* `output` returned to the agent is the cleaned stream between the command
  being typed and the marker, with the echoed wrapper lines removed.
* Works in bash, zsh, sh/dash, ksh. **fish** and other non-POSIX shells: the
  first `run` detects failure to complete (no marker, prompt returns) and the
  session falls back to *quiet-period mode*: `completed` after 1.5 s of no
  output with the cursor on a line that looks like a prompt; `exit_code` is
  null. The result says which mode was used.
* *Later:* if the remote shell already emits OSC 133 shell-integration marks
  (`D;<exit>`), use those and skip the wrapper.

## Output capture

* Tap the bytes the PTY delivers to the terminal (subclass
  `LocalProcessTerminalView` and override its data-received path, or an
  equivalent SwiftTerm hook) and feed an `AgentOutput` ring buffer per agent
  session. Bytes still reach the terminal unchanged; the human view is
  unaffected.
* Cleaning for the agent: strip CSI/OSC/DCS sequences, apply `\r`-overwrite
  and backspace on the current line, normalize to `\n`, drop NULs. The
  **cursor** is an offset into the cleaned stream.
* `read_screen` reads SwiftTerm's visible buffer instead, for full-screen
  programs (`alternate_screen: true` tells the agent it's in one).

## Saved password (`type_password`)

Types the host's saved password plus Enter into the session. It never appears
in any result, log, or notification.

Preconditions, all required:

1. The host is allowed (as for every tool) and has a saved password.
2. The **last non-empty screen line** looks like a password prompt: reuse
   and extend `SSHAskPass.isPasswordPrompt` (`password:`, `[sudo] password
   for …:`, `Password for …:`; still excluding passphrase / one-time /
   verification prompts).

No confirmation: allowing a host includes letting agents type its saved
password into a detected prompt. Prompt detection is a correctness guard (so
the password is never typed into a shell line or an editor), not a
permission check.

After typing:

* **Masking:** every later output result for that session replaces any
  occurrence of the password (and of its common echoes, e.g. split across
  chunks) with `••••••` before it reaches the agent. Masking also applies to
  `read_screen`.
* State plainly in Settings and here: this keeps the password out of the
  agent's context; it does **not** stop an agent with a shell (and sudo) from
  capturing input it causes to be typed. Grant it only to agents you'd trust
  with sudo.

## What the human sees

* **Sidebar:** agent sessions sit under their host like any session, with a
  small agent badge and an activity indicator while a command runs or the
  agent is typing. Hover/selection rules unchanged.
* **Session header:** "Driven by *Claude Code*" with **Pause** / **Resume**
  and **Stop** (Stop ends agent control and keeps the session open for the
  human).
* **Human input wins:** the user can type at any time. While the user has
  typed within the last 2 s, agent `run`/`send_keys` wait (up to 5 s, then
  proceed). *Builder's call* on the exact debounce.
* **Menu bar item:** "Agents: N active" and **Stop All Agents** (ends all agent
  control; sessions stay open).
* **Settings → Agents:** master switch (off), clients (add / revoke, config
  snippet), access (a checkbox per group and per host; nothing else), and a plain-language consent note: *terminal output of agent
  sessions and the names/addresses of allowed hosts are sent to the agent and
  its model provider. Allowed hosts are fully usable by agents, including
  typing their saved passwords into password prompts.*

## Lifecycle

* **Agent sessions are ephemeral (approved).** They are not
  checkpointed into saved sessions, not restored after relaunch, and not
  reconnected automatically. A **Keep Session** action converts one into an
  ordinary saved session (normal rules from then on). Reason: restoring and
  auto-reconnecting a session an agent was driving, with no agent attached,
  surprises the user; ephemeral avoids touching invariant 3.
* **Vault lock (approved):** pauses every agent session and fails all
  tools until unlock. The SSH processes keep running (invariant: lock keeps
  live sessions connected); only agent control is suspended. Reason: a locked
  Maine means the user stepped away.
* **Quit:** agent sessions close with everything else; nothing is persisted.
* **Workspace switch / pulled vault:** as for any session. Hosts that vanish
  close their sessions; access rules are per vault, so the other workspace's
  rules apply after a switch.
* **Master switch off / client revoked:** sockets close, agent control of all
  affected sessions ends, sessions stay for the human.

## Logging and privacy

* `DebugLog` (opt-in diagnostics) records only: tool name, session UUID,
  client id, outcome, durations, byte counts. **Never** commands, keys,
  output, screen contents, host names, tokens or passwords — same rule as
  today, and debug regions stay geometry-only.
* What leaves the Mac, only after opt-in and only for allowed hosts: host
  name, group name, hostname, port, username, and the terminal output of
  agent sessions. Nothing about non-allowed hosts.
* *Later (phase 3):* an encrypted, device-local audit of agent actions per
  session (commands, typed-password events, exit codes), viewable in the app.

## Threat model (short)

| Threat | Mitigation |
|---|---|
| Other users on the Mac | Socket dir 0700, socket 0600, `getpeereid` uid check |
| Unpaired local program | Token required; revocable per client |
| Same-user malware with the token | Out of scope (stated) |
| Prompt injection from server output | Host scoping; the agent harness's own permission prompts; visible activity, Pause/Stop |
| Agent exfiltrating the typed password | Only into a detected prompt, masked in all output; limit stated — allow only hosts you'd trust the agent with sudo on |
| Faked command completion | Random per-command nonce in an OSC the echo can't produce |
| Runaway or hung commands | Timeouts, `send_keys ctrl-c`, Stop |
| Access leaking across devices | Rules and tokens are device-local, never synced |
| Agent reading the user's clipboard (OSC 52) | Clipboard read/write is disabled in agent sessions' terminal |
| A slow or stuck client stalling others | Socket writes run on per-connection queues, never under shared locks |
| A command run twice after a dropped connection | The bridge retries only calls that were never sent |

## Invariant changes (`AGENTS.md`)

Add, in the same change that ships phase 1:

> **12. Agent access is opt-in, scoped and local.** The MCP server is off by
> default and reachable only through `Maine --mcp` over a 0600 Unix socket
> with a revocable per-client token. Agents see only allowed groups/hosts
> (device-local rules in the encrypted local state file, never synced) and
> only sessions they opened. There are no approval modes: an allowed host is
> fully usable; agents' own harnesses gate commands. Saved passwords are typed
> into detected prompts, never returned; agent sessions are visible, pausable
> and ephemeral.

And amend **8** to say diagnostics stay metadata-only *including* agent tool
calls, and **11** to say agent access rules and client tokens never sync.

## Testing

Everything below runs headless under plain `swift test` — **no test may open
a window** (new window tests need `@Test(.uiTest)`; don't run them locally).

* MCP bridge: JSON-RPC framing, `initialize`/`tools/list`/`tools/call`,
  "not running" and "not paired" paths (bridge against an in-test socket).
* Socket server: uid check, token accept/reject/revoke, frame limits.
* Access rules: resolution (group, host override, ungrouped), pruning,
  not-found for disallowed ids, storage in the local state file (vault bytes
  unchanged; absent on a copied vault).
* Completion detector: marker split across chunks, wrong nonce ignored, echoed
  wrapper never matches, exit codes, quiet-period fallback.
* Output cleaning: ANSI stripping, `\r`/backspace handling, 64 KiB middle
  truncation, cursors.
* Password: prompt detection (sudo/plain/passphrase/MFA cases), masking across
  chunk boundaries, refusal paths.
* End to end with a real PTY and no SSH: a `TerminalSession` whose executable
  is `/bin/sh` (as `TerminalSessionTests` already do), driven through the tool
  handlers: `run "echo hi; false"` → output `hi`, exit 1.

## Phases

1. **Core:** bridge, socket, tokens, master switch, access rules, `list_hosts`,
   `list_sessions`, `open_session`, `run`, `read_output`, `read_screen`,
   `send_keys`, `close_session`, agent badge, Pause/Stop/Stop All,
   Settings → Agents, docs + invariant 12.
2. **Passwords:** `type_password` with prompt detection and masking.
3. **Polish:** encrypted local audit log, OSC 133 support, "Hand to agent" for
   human-opened sessions.

## Decisions (owner, approved)

1. Agent sessions are **ephemeral**: not restored, not auto-reconnected;
   **Keep Session** converts one into a normal saved session.
2. Vault **lock pauses agents**; SSH processes keep running.
3. **No permission modes or approval prompts.** Access is the only control:
   an allowed host is fully usable, including saved-password typing. Agent
   harnesses own command approval.
