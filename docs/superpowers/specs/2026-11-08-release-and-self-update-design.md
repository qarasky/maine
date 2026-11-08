# Versioning, macOS releases and self-update: design

Date: 2026-11-08. Status: approved in chat by Skyler (verification: SHA-256 only, no Ed25519).

## Intent
Maine ships to real users. Today the version is a hardcoded `1.0.0` in `Info.plist`, there is no release process, and updating means rebuilding. Add proper versioning, an automated unsigned macOS release, and an in-app updater.

## Goals
1. One source of truth for the version: a git tag `vX.Y.Z`.
2. Pushing a tag builds and publishes a GitHub Release with `Maine-X.Y.Z.dmg` (containing `Maine.app`, ad-hoc signed only) and `Maine-X.Y.Z.dmg.sha256`.
3. The app checks for updates at launch and every 2 hours (while the vault is unlocked), offers a popup, and can install the update itself and relaunch.

## Non-goals / anti-goals
- No Apple Developer ID, notarization or provisioned signing (AGENTS.md invariant 7). No new entitlements.
- No Sparkle or other dependency; no Ed25519 update signatures (declined). Integrity is HTTPS plus SHA-256 from the same release; this does not prove authorship, and the docs say so.
- No telemetry: the check is a plain GET to the GitHub Releases API, no identifiers, no custom headers beyond a generic User-Agent.
- Android and the sync server are untouched.

## Design (calls marked [guess] are the builder's defaults, not agreed)
**Versioning.** `scripts/build-app.sh --version X.Y.Z [--build N]` writes `CFBundleShortVersionString` and `CFBundleVersion` into the bundle's copy of `Info.plist`. Without `--version`, local builds show `0.0.0-dev` [guess]; the checked-in plist keeps a placeholder. CI passes the tag (without `v`) and `github.run_number`-style monotonic build [guess: commit count]. Tags with a hyphen suffix (`-beta.1`) are GitHub pre-releases and are ignored by the updater [guess]. A single `AppVersion` type parses and compares semver and is used by the footer, MCP bridge and updater.

**Release workflow** `.github/workflows/release.yml`: trigger on `v*` tags; `macos-15`; select newest Xcode; `swift test`; `scripts/build-app.sh --release --version`; new `scripts/make-dmg.sh` (hdiutil, app + /Applications symlink); sha256 file; `gh release create` with generated notes and both assets. Permissions `contents: write` for this workflow only.

**Updater** (`Sources/Maine/Update/`):
- `UpdateChecker`: fetches `https://api.github.com/repos/qarasky/maine/releases/latest` (stable only), compares to the running version, finds the DMG and `.sha256` assets. Runs at unlock, then every 2 h while unlocked [guess]; no timer while locked. Failures are silent (debug log metadata only).
- Popup: version, release notes, **Update**, **Later**, **Skip This Version**. Skipped version stored in UserDefaults (not the vault, never synced). "Later" re-offers at the next check [guess].
- Install: download DMG to a temp dir, verify SHA-256, mount read-only, copy `Maine.app` next to the running bundle, then a detached helper script waits for the process to exit, swaps the bundle, and relaunches. Needs the app's folder to be writable; if not, show a message and open the release page [guess]. Never starts while an agent run is busy or a sync push is in flight; defers instead [guess]. Quit flushes sessions as usual (invariant 4: quit keeps records).
- Settings: a toggle "Check for updates automatically" (default on) and a "Check for Updates…" menu item.
- Diagnostics rule (invariant 8): log only outcome and version numbers.

## Docs
README, `docs/architecture.md`, a new `docs/releasing.md` (how to cut a release, the Gatekeeper first-launch caveat), and the AGENTS.md invariants (updater is the app's only automatic non-sync network call).

## Testing
Unit tests: semver parse/compare, release JSON parsing (stable vs pre-release, missing assets), skip-version logic, SHA-256 verification, helper-script generation. No window tests. The DMG build and an end-to-end update are verified by running the scripts locally; the workflow itself can only be verified on GitHub.
