# Maine for Windows

A WPF application in a .NET 8 solution. The terminal UI is
[xterm.js](https://xtermjs.org/) hosted in WebView2; everything else is C#
plus the system OpenSSH client — Maine never ships its own SSH.

The solution splits along build lines so the portable logic is verifiable
anywhere:

```
apps/windows/
├── Maine.sln
├── global.json                  pins .NET 8
├── scripts/fetch-xterm.sh       vendors xterm.js for offline use
├── src/Maine.Core/              net8.0, no UI: hosts, vault crypto, ssh command,
│                                ssh-config reader, session snapshots
├── src/Maine.App/               net8.0-windows (WPF): sidebar, ConPTY sessions,
│                                WebView2 terminals, askpass helper, DPAPI key store
└── tests/Maine.Core.Tests/      net8.0 xUnit suite (runs on any OS)
```

`Maine.Core` builds and tests on macOS/Linux too, so vault and SSH behavior
can be checked without Windows. `Maine.App` only builds on Windows.

## Requirements

* Windows 10 version 1809 or later (ConPTY), Windows 11 recommended
* [.NET 8 SDK](https://dotnet.microsoft.com/download) (`dotnet --info` should
  report 8.0.x; `global.json` pins the feature band)
* WebView2 runtime (preinstalled on Windows 11 and with current Edge)
* System OpenSSH client (`C:\Windows\System32\OpenSSH\ssh.exe`, on PATH as
  `ssh.exe`) — install via Settings → Optional features if missing

## Build

```powershell
cd apps\windows
dotnet build Maine.sln
dotnet run --project src\Maine.App\Maine.App.csproj
```

First vendor the terminal libraries (they are not committed):

```sh
sh scripts/fetch-xterm.sh   # needs npm; downloads xterm.js into Terminal/lib/
```

Publish a single-folder build:

```powershell
dotnet publish src\Maine.App\Maine.App.csproj -c Release -o publish
```

## Test

```powershell
cd apps\windows
dotnet test Maine.sln
```

The suite mirrors the macOS suites: host validation, the versioned JSON
format, the exact `ssh` argument lists, the ssh-config reader, and the
encrypted vault. It also decrypts `Fixtures/swift-vault-v2.json` — a vault
produced by CryptoKit on macOS — which proves a vault written on one OS opens
on the other with the same master password.

## Parity with macOS

| Behavior | Status |
| -------- | ------ |
| Save, edit, delete, duplicate hosts | done (Core + UI) |
| Versioned JSON host format | done |
| Interactive SSH session over ConPTY + xterm.js | done |
| Multiple sessions, sidebar switching/closing, reconnect | done |
| Error reporting for failed connections | done |
| Unified encrypted vault (`passwords.maine`, portable with macOS) | done |
| Saved passwords via askpass helper, master-password unlock | done |
| Session snapshots + auto-reconnect after unlock | done |
| Debug diagnostics snapshot | not started |

Deliberate platform mappings:

* Vault file: `%APPDATA%\Maine\passwords.maine` (user-only ACL, the Windows
  equivalent of `0600`). Copy only this file to another machine and unlock it
  there — same as macOS.
* Remembered unlock key: DPAPI-protected file under
  `%LOCALAPPDATA%\Maine\vault-keys\`, deleted on lock and quit — the same
  lifetime as the macOS Keychain entry. It is never copied between machines.
* `ssh.exe` discovery: `PATH` first, then
  `C:\Windows\System32\OpenSSH\ssh.exe`. `-p`/`-l` rules are identical:
  port 22 and blank username are omitted so the user's ssh config decides.
* Askpass: `Maine.exe` re-runs itself as `SSH_ASKPASS` with
  `SSH_ASKPASS_REQUIRE=force`; only the host ID travels in the environment.
  Saved passwords are never reused for passphrases, MFA, or host trust.
* Terminals stay mounted: one WebView2 per session in a stable grid, only the
  selected one visible — switching never reparents a terminal view.

## Where things are stored

| What | Where |
| ---- | ----- |
| Saved hosts, passwords, session snapshots | `%APPDATA%\Maine\passwords.maine` (encrypted) |
| Remembered local vault key (DPAPI, current session only) | `%LOCALAPPDATA%\Maine\vault-keys\` |
| Keys, known hosts, ssh options | `%USERPROFILE%\.ssh`, untouched by Maine |

## Notes for contributors

* **Platform-specific code stays here.** The shared contract is the file
  format in `packages/schema` plus the byte shapes documented in
  `Maine.Core` (`VaultFormat`); there is no shared code with other clients.
* **Do not add a second way to reach a host.** Everything that talks to a
  server goes through the system `ssh.exe`.
* **Keep the terminal views alive.** Terminals are owned by their session,
  not by the view that displays them, so switching sessions keeps scrollback.
* **Diagnostics are metadata-only and opt-in**, same as macOS: never log
  passwords, keystrokes, hostnames, usernames, session names, or terminal
  contents.
