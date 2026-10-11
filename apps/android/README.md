# Maine for Android

Second Maine client: Kotlin + Jetpack Compose, same local-first SSH model as
the macOS app. Status: functional against real hosts — real VT/xterm terminal
as the input surface, live sshj transport, portable vault.

## Layout

```
apps/android/
├── settings.gradle.kts / build.gradle.kts / gradle.properties
├── gradle/libs.versions.toml        version catalog (AGP, Kotlin, Compose BOM, sshj)
├── app/                             com.android.application module (app.maine.android)
│   └── src/main/java/app/maine/android/
│       ├── model/    Host, HostConfiguration, SavedSession (schema parity)
│       ├── ssh/      SshTarget (display args + dial params), SshConfigFile reader
│       ├── vault/    VaultStore (v2 vault, byte-compatible with macOS)
│       ├── store/    HostStore (owns the host list)
│       ├── session/  SshConnection (transport seam), SshTerminal (emulator bridge),
│       │             TerminalSession, SessionManager
│       └── ui/       Compose screens: unlock, host list, editor, session;
│                     TerminalSurface + MaineTerminalPalette
├── terminal/                        vendored Termux terminal-emulator + terminal-view
│                                    (Apache-2.0; see THIRD_PARTY_LICENSES.md)
├── app/src/test/                    JVM unit tests (no emulator needed)
└── tools/make-golden-vault.swift    Swift generator for the vault interop fixture
```

## Build and test

```sh
cd apps/android
./gradlew :app:testDebugUnitTest   # JVM unit tests (~50 tests, no emulator)
./gradlew :app:assembleDebug       # APK; install on the emulator when ready
./gradlew :terminal:assembleDebug  # vendored terminal module alone
```

Requires: JDK 17+, Android SDK with platform 36 + build tools 36
(`~/Library/Android/sdk` works out of the box; licenses already accepted).
First run downloads Gradle 8.13 and dependencies (~500 MB).

## Key decisions

* **No system `ssh`.** The macOS client shells out to `/usr/bin/ssh`; Android
  uses the [sshj](https://github.com/hierynomus/sshj) library. `SshTarget` keeps
  the same display rules (port 22 omits `-p`, empty user omits `-l`) and also
  carries the dial parameters.
* **Every sshj call is off the main thread.** Android raises
  `NetworkOnMainThreadException` for socket work on the UI thread, and sshj
  writes to the socket from whichever thread calls it — so a window-change or a
  disconnect issued from the UI thread aborts mid-packet and corrupts the SSH
  stream, which the server answers with a connection reset. Connecting and
  reading run on one worker thread; `resize` and `close` are handed to a
  single-threaded executor, which also coalesces the burst of resizes a keyboard
  animation produces. Resizes become SSH `window-change`, so `stty size` in the
  session matches the view.
* **BouncyCastle is swapped in.** Android registers a stripped-down
  BouncyCastle under the same provider name ("BC") that sshj looks up, and it
  cannot do `ssh-ed25519`, which most servers now offer.
  `installSshSecurityProvider()` replaces it with the full provider sshj
  depends on, from `MainActivity` before anything can connect.
* **Host keys are pinned on first use.** `PinnedHostKeyVerifier` remembers a
  host's key the first time it is seen and refuses any later key that does not
  match, reporting both fingerprints so the change can be checked. macOS defers
  to the user's own `ssh` and its `known_hosts`; this is the same policy kept in
  app-private storage (`files/known_hosts` — public fingerprints only, so it
  needs no encryption). It is deliberately *not* in the vault: trust is this
  device's view of a server, and keeping it out leaves `passwords.maine`
  portable. Entries are keyed by host UUID, so deleting a host forgets its key,
  which is the documented way to re-trust a server that was rebuilt.
  Fingerprints are computed over the SSH wire encoding, so they read exactly
  like `ssh-keygen -lf` (`SHA256:...`) and can be compared against the server;
  `HostKeyFingerprintTest` pins that against a vector produced by `ssh-keygen`.
* **Password auth only.** There is no `~/.ssh` on Android to read keys from;
  importing keys from the vault is a later pass.
* **Restored scrollback is a replay, not a snapshot.** A restored session is
  seeded by replaying its saved output stream, which keeps colors and needs no
  second serializer — but it also re-enters whatever terminal modes that output
  set, and answers the terminal queries inside it. Two guards follow from that:
  emulator replies are suppressed while seeding, otherwise a replayed
  `XTGETTCAP` lookup gets its answer typed into the live shell; and the emulator
  is returned to the main screen afterwards, so a transcript captured while a
  full-screen program was open — the app killed mid-`vim` — replays as the
  shell's history instead of leaving the new connection drawn over that
  program's screen. macOS sidesteps all of this by saving plain text only, and
  losing colors with it.
* **Vault file is portable.** `VaultStore` implements the macOS v2 envelope
  byte-for-byte (PBKDF2-SHA256 600k, AES-256-GCM, identical AAD), so a
  `passwords.maine` file unlocks on either client. It lives in app-private
  internal storage (`filesDir/passwords.maine`). The derived key is held in
  memory only — every launch starts locked, no "remember me". Version-1
  vaults are rejected (unlock once on macOS to migrate).
* **UUID casing.** macOS writes uppercase `UUID().uuidString` and looks
  passwords up with that exact casing. Android normalizes IDs to uppercase
  on read/write; comparisons are case-insensitive.
* **Real terminal emulation, vendored.** The terminal is Termux's
  `terminal-emulator` + `terminal-view`, copied verbatim into the `terminal/`
  module under Apache-2.0 (see `terminal/THIRD_PARTY_LICENSES.md`). Local
  patches: `TerminalSession` gains a public `Transport` interface so an SSH byte
  stream drives the emulator instead of Termux's native PTY fork — skipping
  every JNI call — and that interface carries the size, so resizes reach the
  remote shell. Verified on a real host: colored `ls`, a real PTY (`/dev/pts/0`,
  `TERM=xterm-256color`), `stty size` following the keyboard, and full-screen
  `vim` with the alternate screen restoring cleanly on exit.
* **The terminal is the input surface.** Tapping the terminal focuses it and
  raises the keyboard; there is no separate "send input" field. `TerminalView`
  handles the IME, hardware keys, pinch-to-zoom text and text selection, and its
  `onCopyTextToClipboard`/`onPasteTextFromClipboard` callbacks bridge OSC-52 and
  paste to the system clipboard.
* **The emulator lives in the session, not the view.** So navigating away and
  back rebuilds only the view and re-attaches to the same terminal state. It is
  created lazily on the main thread (the vendored session binds a `Handler`),
  which is also why JVM unit tests never construct one. Remote bytes arriving
  before a view exists are buffered (capped) and replayed, so a session restored
  at launch is never blank.
* **Terminal palette is Maine's.** `MaineTerminalPalette` rewrites Termux's
  default xterm palette at launch: pure-black background, the brand yellow as
  cursor and ANSI "yellow", desaturated ANSI colors so `ls` does not fight the
  monochrome UI. Terminals stay dark on both app themes, as on macOS.
* **Extra keys row.** ESC, CTRL, ALT, TAB and the four arrows sit between the
  terminal and the soft keyboard, and only while the keyboard is up, because a
  soft keyboard has no Tab, Escape or arrows — without them there is no tab
  completion, no `Ctrl+C` and no way out of `vim`'s insert mode. CTRL and ALT
  light up while armed and are one-shot, so an armed Ctrl cannot control-code
  the rest of a typed command. The rest are sent as real VT sequences through
  `KeyHandler`, so arrows honour the application-cursor mode that `vim` and
  `less` switch on rather than always sending the base sequence.
* **`~/.ssh/config` reader** is ported for host import and display
  resolution, but Android devices normally have no such file — it loads
  empty and stays read-only, like macOS.

## Interop fixture

`app/src/test/resources/vault-golden-v2.json` is generated by the Swift
implementation itself:

```sh
cd apps/android
swift tools/make-golden-vault.swift golden-horse-battery-42 \
  app/src/test/resources/vault-golden-v2.json
```

`VaultGoldenTest` unlocks it with the checked-in throwaway password. If that
test passes, Android reads macOS vaults. Regenerate only when the envelope
format changes; the file is a test vector, never a real vault.
