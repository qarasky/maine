<p align="center">
  <img src="docs/images/maine-icon.png" alt="Maine" width="96">
</p>

<h1 align="center">Maine</h1>

<p align="center"><b>A local-first SSH client for macOS. Save a host, click it, get a real terminal.</b></p>

<p align="center">
  <a href="https://github.com/qarasky/maine/releases">Download</a> ·
  <a href="docs/guide.md">User guide</a> ·
  <a href="docs/development.md">Development</a> ·
  <a href="docs/architecture.md">Architecture</a> ·
  <a href="docs/mcp.md">Agents (MCP)</a>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/macOS-14%2B-5263ff?style=flat-square" alt="macOS 14+">
  <img src="https://img.shields.io/badge/Swift-6-5263ff?style=flat-square" alt="Swift 6">
  <img src="https://img.shields.io/badge/SSH-system_OpenSSH-5263ff?style=flat-square" alt="System OpenSSH">
</p>

[![Maine on macOS with two sessions on one host](docs/images/maine-macos.png)](docs/images/maine-macos.png)

## What is Maine?

Maine is a small SSH client with a host list in the sidebar. It keeps your hosts and passwords in one encrypted file, runs the `ssh` already on your Mac, and brings your sessions back after you restart.

There are no accounts, no telemetry and no cloud of ours. If you want the same hosts on several devices, you run the sync server yourself, and it only ever sees encrypted bytes.

## What can Maine do?

- **Click to connect.** Pick a host and you are in a full terminal. No details page, no retyping addresses.
- **Use your own `ssh`.** `ProxyJump`, `IdentityFile`, `ForwardAgent`, certificates and hardware keys behave as they do in Terminal. The connection header shows the exact command line.
- **Keep passwords in one vault.** One master-password-encrypted file (AES-256-GCM, PBKDF2-SHA256). Maine types a saved password when the server asks, or does nothing if you use keys. It stores no private keys.
- **Run several sessions per host.** Name them, close them, and get them back with their scrollback after a restart.
- **Organise with groups.** Collapsible, drag-and-drop, synced with your hosts.
- **Sync on your own server.** An optional Rust server holds the encrypted vault. Conflicts keep a copy instead of overwriting.
- **Let AI agents work on your servers.** Off by default. Agents like Claude Code use real sessions you can watch, pause and stop, only on hosts you allow.
- **Stay out of the way.** Lives in the Dock and menu bar, honours Reduce Motion, and updates itself with a checksum check.

## Install

Download `Maine-X.Y.Z.dmg` from [GitHub Releases](https://github.com/qarasky/maine/releases), open it and drag **Maine** onto **Applications**.

Releases are ad-hoc signed, not notarized, so macOS may block the first launch. Right-click **Maine** in Finder and choose **Open**, or run:

```sh
xattr -dr com.apple.quarantine /Applications/Maine.app
```

To build from source instead (Xcode 16+ required):

```sh
cd apps/macos
scripts/build-app.sh --release --run
```

See [docs/development.md](docs/development.md) for details.

## Quick start

1. Open Maine and create a vault with a master password (12+ characters). Maine asks for it on every launch.
2. Press ⌘N, enter a name and a hostname or IP. Port, username and password are optional.
3. Click the host to connect.

That is the whole loop. Everything else is in the [user guide](docs/guide.md): the vault, groups, saved sessions, sync, agents and debugging.

| Shortcut | Action |
| --- | --- |
| ⌘N | New host |
| ⌘F | Search hosts |
| ⌘⏎ | Connect the selected host |
| ⇧⌘R | Reconnect |
| ⇧⌘W | Close a session |
| ⌘W | Hide the window; Maine keeps running in the menu bar |

## Why not just `ssh`?

Plain `ssh` is great if you live in a terminal. Maine adds what sits around it: saved hosts, automatic passwords, several named sessions per host and scrollback that survives restarts. The connection itself is still your `ssh`.

## Sync and agents in short

**Sync** is optional and self-hosted. Run the server from `apps/sync-server`, create a slot in **Settings → Sync**, and join from other devices with the same token. The server stores `passwords.maine` byte for byte and never sees a hostname or password. macOS has the client; Android does not yet. Details: [user guide](docs/guide.md#sync), [apps/sync-server/README.md](apps/sync-server/README.md), [docs/sync-protocol.md](docs/sync-protocol.md).

**Agents** are off until you switch them on in **Settings → Agents** and allow specific groups and hosts. Saved passwords are typed locally into prompts and never sent to the agent. Details: [user guide](docs/guide.md#agents-mcp), [docs/mcp.md](docs/mcp.md).

## Project layout

```
maine/
├── apps/
│   ├── macos/         Swift + SwiftUI app
│   ├── android/       Kotlin + Compose app (vault parity, real terminal; in progress)
│   └── sync-server/   Rust sync server
├── packages/schema/   Versioned JSON config format + validator
└── docs/              Guide, development, architecture, protocols, design, releasing
```

## Status

macOS is the first and main client. Sync, MCP agent access and DMG releases with self-update are done. Android has vault parity, host editing and a real terminal; Android sync and a Linux client are not started. Accounts, a hosted service and key management are not planned. The full table is in [docs/development.md](docs/development.md#status).

## License

Maine depends on [SwiftTerm](https://github.com/migueldeicaza/SwiftTerm) (MIT). No license has been chosen for Maine itself yet.
