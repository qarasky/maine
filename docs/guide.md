# User guide

How to use Maine on macOS. For the short version, see the [README](../README.md).

- [Getting started](#getting-started)
- [Hosts and sessions](#hosts-and-sessions)
- [Groups](#groups)
- [The vault](#the-vault)
- [Saved sessions](#saved-sessions)
- [Sync](#sync)
- [Agents (MCP)](#agents-mcp)
- [Updates](#updates)
- [Debugging](#debugging)

## Getting started

1. **Install the app** ([from Releases](https://github.com/qarasky/maine/releases) or [from source](development.md#build)). On first launch, create the password vault with a master password of at least 12 characters. Maine asks for it on every launch; canceling leaves passwords locked.
2. **Add a host.** Click **New Host** (⌘N) and fill in a name and a hostname or IP.
   - Leave the port blank for `22`.
   - Leave the username blank to let OpenSSH decide from `~/.ssh/config`, your key or the local user.
   - Leave the password empty for key login through `~/.ssh` and `ssh-agent`. If you save one, Maine types it when the server asks; clearing the field removes it.
   - Passwords cannot be edited or saved while the vault is locked.
3. **Connect.** Click the host in the sidebar.
4. **Work.** Close a session with the sidebar **×** (⇧⌘W). The saved host stays in the list. Double-click a session name to rename it (Return saves, Escape cancels).
5. **Come back.** Quit and reopen Maine, then unlock. Sessions return with their names and history and reconnect automatically.

Closing the window does not quit. Maine stays in the Dock and the menu bar. Click the Dock icon or left-click the menu bar item to reopen the window; right-click the menu bar item for Open Maine, Lock or Unlock, Quit, and Stop All Agents while agents are active.

Shortcuts: ⌘N new host, ⌘F search hosts, ⌘⏎ connect the selected host, ⇧⌘R reconnect, ⇧⌘W close a session, ⌘W hide the window.

## Hosts and sessions

Click a host to get a full terminal (keyboard input, resizing, `Ctrl`/`Esc` sequences) rendered with [SwiftTerm](https://github.com/migueldeicaza/SwiftTerm). Navigation is sidebar-only: there is no tab bar.

- **More sessions.** Right-click a host with a live session and choose **New Session**. Sessions appear as indented rows under their host, each with its own close control.
- **Copy credentials.** Hover a host and click its copy button, right-click it, or use **Host → Copy Credentials**. You get a plain-text block with `IP:`, `username:`, the saved `password:` (when the vault is unlocked) and `port:` (when not 22). It stays on the clipboard until something replaces it.
- **No hidden commands.** The connection header shows the endpoint, and its tooltip shows the exact `ssh` command line.

Because the connection is your system `ssh`, `ProxyJump`, `Match`, `IdentityFile`, `ForwardAgent`, certificates and hardware keys work exactly as in a terminal.

## Groups

Organise hosts into collapsible groups in the sidebar. Create one with **Host → New Group** (⇧⌘N), the folder button in the sidebar footer, or **Move to Group → New Group…** on a host. Click a header to collapse or expand; right-click it to rename or delete (deleting keeps the hosts, ungrouped).

Drag to organise:

- a host onto a group header: end of that group
- a host onto another host: just above it, in its group
- a host onto the strip at the bottom: out of its group
- a group header onto another header: reorder groups

**Move to Group** in the host menu and the **Group** picker in the host editor do the same without dragging. Ungrouped hosts stay at the top, and groups with a match are expanded while searching.

Groups and their order live in the vault, so they sync. Which groups are collapsed is remembered per Mac, encrypted, and never synced. Android does not show groups yet, and saving a host there drops them.

## The vault

**`~/Library/Application Support/Maine/passwords.maine` is the single active vault**, written with `0600` permissions. It encrypts host names, addresses, ports, usernames, IDs and passwords together with AES-256-GCM. The key comes from your master password through PBKDF2-SHA256 (600,000 iterations, random salt).

- **Unlock key.** It lives in Keychain only for the current app session, for the SSH helper. It is removed on quit and reset on launch, so quitting always requires the master password again. Closing the window does not. **Host → Lock Password Vault** and the menu bar's right-click menu remove it on demand. Locking does not disconnect open sessions.
- **No password saved?** Maine runs plain `ssh`, so keys, `ssh-agent` and `~/.ssh/config` authenticate, and any prompt appears in the terminal. Maine stores no private keys.

**Moving the vault by hand.** Choose **Host → Show Encrypted Vault** and copy only `passwords.maine`. Quit Maine on the receiving Mac, back up its existing files, place the copy in its Maine Application Support directory, then choose **Host → Unlock Password Vault**. Send the master password separately. Host IDs must stay unchanged, and sharing the vault shares every password in it. This is file copying, not sync or merging. A forgotten master password cannot be recovered.

**Migration.** On the first successful unlock, old `hosts.json` and version-1 passwords migrate into the vault. Malformed input and wrong passwords never overwrite it. The originals (`hosts.json`, `passwords.maine.v1-backup`) are kept and no longer used. **`hosts.json` is still plaintext:** after checking your data, remove or securely archive it yourself.

## Saved sessions

Sessions save their name and the terminal's retained plain-text buffer (up to 1 MiB), periodically and before a normal quit or vault lock. This is a snapshot, not an activity log: colors, cursor state, remote processes and shell state are not saved.

Output can contain secrets, so snapshots are encrypted with the vault key. They live in `sessions-<vault id>.maine` next to `passwords.maine`, stay on this Mac, and never sync or trigger a sync push. Another device sees your hosts but not your sessions. Vaults from older builds, which kept snapshots inside `passwords.maine`, move them to the local file on the next unlock.

- Renaming updates the saved name immediately.
- A normal quit keeps session records. Only **Close Session**, **×**, **Close Every Session** or deleting the host removes them.
- While the vault is locked, autosave pauses and closing a saved session requires unlocking. Quitting while locked keeps the last checkpoint.

## Sync

Optional and self-hosted. Maine never talks to a server of ours.

Run the sync server and pair each device with one token:

```sh
cd apps/sync-server
cargo run --release        # listening on 127.0.0.1:8484, data in ./data
```

In **Maine → Settings… → Sync**, the first device creates a slot and receives a token like `maine-2F7K-9QX2-…`. Every other device enters the same token and the vault's master password.

- The server stores the encrypted `passwords.maine` byte for byte. It sees only `SHA-256(token)`, sizes and timestamps.
- The vault pulls on unlock, pushes a couple of seconds after an edit, polls while open, and stops syncing when locked.
- The token lives in the login Keychain, not in the vault, so it survives the vault being replaced by the server's copy.
- Writes are conditional. A device that is behind gets a conflict copy (`passwords.conflict-….maine` next to the vault) instead of overwriting the other side. The last 32 versions of each slot are kept.
- Android does not speak the protocol yet.

Deployment (reverse-proxy TLS, systemd, Docker, `--create-secret` for servers reachable by strangers): [apps/sync-server/README.md](../apps/sync-server/README.md). Wire contract: [sync-protocol.md](sync-protocol.md). Client checklist: [sync-client.md](sync-client.md).

## Agents (MCP)

Maine can let AI agents such as Claude Code or Claude Desktop work on your servers through real sessions you can watch. It is off until you enable it in **Settings → Agents**:

1. Switch on **Allow agent access**.
2. **Add Client…**, name it, and copy the setup it shows once. For Claude Code that is a command like `claude mcp add maine --env MAINE_AGENT_TOKEN=… -- "/Applications/Maine.app/Contents/MacOS/Maine" --mcp`; for Claude Desktop it is JSON for `claude_desktop_config.json`.
3. Tick the groups and hosts agents may use. Everything else is invisible to them.

Agents can list allowed hosts and their own sessions, open sessions, run commands (getting output and exit code), read output and the screen, send keys, ask Maine to type a saved password into a prompt, and close their sessions. `type_password` fires only when the last screen line is a password prompt (sudo, ssh, su). Maine types the password plus Return, never returns it, and masks it in later output.

**What you see and control**

- Agent sessions appear under their host with an agent badge. The header shows who drives and offers **Pause**, **Stop** and **Keep Session**. The menu bar has **Stop All Agents**.
- Agent sessions are not saved or restored unless you keep them. Locking the vault pauses every agent.
- **Hand to Agent → client** (right-click your own session) lets an agent drive it. It needs agent access on, the vault unlocked, the host allowed, a paired client and an idle shell prompt, because the first command is typed into it. The session stays yours, keeps its scrollback, is still saved, and returns to you when the agent stops.

**What leaves your Mac**, only for allowed hosts: host names, groups, addresses, ports, usernames and the terminal output of agent sessions, sent to the agent and its model provider. Saved passwords are never sent. There are no approval prompts in Maine: allowed means fully usable, because agents ask permission in their own harnesses. Access rules, client tokens and the activity log stay on this Mac and never sync. Revoke a client in Settings to cut it off immediately.

**Activity log.** **Settings → Agents → Activity** lists what agents did here: sessions opened, handed over and closed, commands with exit codes, keys sent, and each saved-password type. It keeps the newest 1,000 entries (shows the latest 100) in the encrypted device-local file. It never records passwords, terminal output or the literal text of `send_keys` (only its length), is empty while locked, and has a **Clear…** button.

Design and tool reference: [mcp.md](mcp.md).

## Updates

Maine checks for a newer release once at unlock and every two hours while unlocked. **Maine → Check for Updates…** checks on demand. A newer release shows a popup with **Update**, **Later** and **Skip This Version**. **Update** downloads the disk image, verifies its published SHA-256, replaces the app and relaunches.

The check is one request to GitHub's public Releases API with no identifiers. **Settings → Updates** turns it off. Integrity means HTTPS plus the SHA-256 from the same release, not proof of authorship. How releases are cut: [releasing.md](releasing.md).

## Debugging

For a disappearing window or terminal:

1. Quit Maine and reopen `apps/macos/build/Maine.app` after rebuilding. The app must have a **Debug** menu; older running copies lack these probes.
2. Choose **Debug → Enable Debug Logging**, then reproduce the problem.
3. Leave the window as it is and choose **Debug → Capture Diagnostic Snapshot** (⌥⇧⌘L).
4. Choose **Debug → Show Debug Logs**. Logs are local JSON-lines files at `~/Library/Logs/Maine/debug.jsonl`, plus one rotated `debug.previous.jsonl`. Keep both when reporting, and disable logging afterwards.

Disconnect captures the window/view tree before termination and again after 100 ms, 500 ms and 2 s. Logs do **not** capture passwords, hostnames, usernames, commands, environment variables, terminal contents, keystrokes or screenshots. They do include host and session UUIDs and the app executable path. Files are `0600`, about 5 MB each for current and previous. Nothing is uploaded. Logging also works in release builds; run `apps/macos/build/Maine.app/Contents/MacOS/Maine --debug` to see native runtime warnings too.
