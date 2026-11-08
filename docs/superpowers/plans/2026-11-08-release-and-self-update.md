# Versioning, Releases and Self-Update Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. **No commit steps:** AGENTS.md says commit only when asked.

**Goal:** Tag-driven versioning, a GitHub Actions workflow that publishes an unsigned macOS `.dmg`, and an in-app updater (2 h checks, popup, self-install).

**Architecture:** `AppVersion` is the single semver type. `build-app.sh` injects the version into the bundle's `Info.plist`. A pure `UpdateFeed` parses GitHub's latest-release JSON into an `UpdateOffer`; `UpdateInstaller` downloads, verifies SHA-256, stages the new `.app` beside the running one and hands off to a detached shell helper that swaps and relaunches; `UpdateCoordinator` schedules checks and shows the popup.

**Tech Stack:** Swift 6 / SwiftUI / CryptoKit / Foundation (no new dependency), bash, GitHub Actions (`macos-15`), `hdiutil`, `gh`.

**Spec:** `docs/superpowers/specs/2026-11-08-release-and-self-update-design.md`

All paths below are relative to the repo root; Swift commands run from `apps/macos`.

## Global Constraints

- No new Swift dependency; no Sparkle; no Ed25519 (declined). Integrity = HTTPS + SHA-256 from the same release.
- Ad-hoc signing only; no entitlements, no notarization (AGENTS.md invariant 7).
- No telemetry: one GET to `https://api.github.com/repos/qarasky/maine/releases/latest`, generic `User-Agent: Maine-Updater`, no identifiers. Logs carry outcome and version numbers only (invariant 8).
- Check cadence: at vault unlock, then every 2 hours (7200 s) while unlocked; no timer while locked.
- Popup buttons exactly: **Update**, **Later**, **Skip This Version**. Settings toggle "Check for updates automatically", default on; menu item "Check for Updates…".
- Asset names: `Maine-X.Y.Z.dmg` and `Maine-X.Y.Z.dmg.sha256`. Tags `vX.Y.Z`; a hyphen suffix means GitHub pre-release, ignored by the updater.
- Skipped version and the toggle live in `UserDefaults` (keys `app.maine.update.skippedVersion`, `app.maine.update.automatic`), never in the vault, never synced.
- Dev builds (`0.0.0-dev`, or not running from a `.app` bundle) never auto-check.
- Tests never touch the network or open windows (`.uiTest` stays off).

## Review Focus

- GitHub unreachable, rate-limited (403) or malformed JSON: automatic checks stay silent; a manual check says so.
- Release without the `.sha256` asset or the DMG: no offer (never install unverified).
- `v1.10.0` must beat `v1.9.0`; equal or older tag: no offer; draft/pre-release: no offer.
- Checksum mismatch: abort, delete the download, the installed app is untouched.
- App folder not writable, or app running from a mounted DMG: explain and open the release page instead of failing midway.
- Swap fails after the old bundle was moved aside: helper restores the old bundle.

---

### Task 1: Versioning (`AppVersion` + build script)

**Files:**
- Create: `apps/macos/Sources/Maine/Update/AppVersion.swift`
- Create: `apps/macos/Tests/MaineTests/AppVersionTests.swift`
- Modify: `apps/macos/scripts/build-app.sh`, `apps/macos/Resources/Info.plist` (placeholder `0.0.0-dev` / build `0`), `apps/macos/Sources/Maine/Views/HostListView.swift:738`, `apps/macos/Sources/Maine/App/MaineApp.swift:205`, `apps/macos/Sources/Maine/Agents/MCPBridge.swift:64`

**Interfaces:**
- Produces: `struct AppVersion: Comparable, Hashable, Sendable, CustomStringConvertible` with `init?(_ string: String)` (accepts a leading `v`; `X.Y.Z` and `X.Y.Z-pre.release`), `var isPrerelease: Bool`, `var description: String` (no `v`), `static var current: AppVersion` (bundle `CFBundleShortVersionString`, else `0.0.0-dev`), `var isDevelopment: Bool` (== `0.0.0-dev`).

- [ ] **Step 1: Write failing tests** in `AppVersionTests`: `parsesWithAndWithoutV` (`"v1.2.3"` and `"1.2.3"` equal, description `"1.2.3"`), `rejectsGarbage` (`"1.2"`, `"1.2.x"`, `""`), `comparesNumerically` (`1.10.0 > 1.9.0`, `2.0.0 > 1.99.99`), `prereleaseSortsBeforeRelease` (`1.0.0-beta.1 < 1.0.0`, `1.0.0-beta.2 > 1.0.0-beta.1`, `1.0.0-beta.10 > 1.0.0-beta.2`), `devVersionIsDevelopment`.
- [ ] **Step 2:** `swift test --filter AppVersionTests` → compile failure / FAIL.
- [ ] **Step 3:** Implement `AppVersion` per the Interfaces block (semver 2.0 precedence; dot-separated prerelease identifiers compared numerically when both numeric).
- [ ] **Step 4:** Replace the three `Bundle.main…CFBundleShortVersionString` reads with `AppVersion.current.description`.
- [ ] **Step 5:** `build-app.sh`: add `--version X.Y.Z` (validated with `^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$`) and `--build N` (default `git rev-list --count HEAD`, `0` if git fails). After copying `Info.plist`, set both keys in the **bundle's copy** with `/usr/libexec/PlistBuddy`; default version `0.0.0-dev`. Update the usage header comment.
- [ ] **Step 6:** Verify: `swift test --filter AppVersionTests` passes; `scripts/build-app.sh --release --version 1.2.3 --build 7` then `/usr/libexec/PlistBuddy -c 'Print :CFBundleShortVersionString' build/Maine.app/Contents/Info.plist` prints `1.2.3` and `CFBundleVersion` prints `7`; a build without `--version` prints `0.0.0-dev`; `git diff apps/macos/Resources/Info.plist` shows only the placeholder change.

### Task 2: DMG packaging and release workflow

**Files:**
- Create: `apps/macos/scripts/make-dmg.sh`, `.github/workflows/release.yml`, `docs/releasing.md`

**Interfaces:**
- Consumes: `build-app.sh --version` (Task 1) producing `apps/macos/build/Maine.app`.
- Produces: `scripts/make-dmg.sh <version>` → `apps/macos/build/Maine-<version>.dmg` and `…dmg.sha256` (content: `<hex>  Maine-<version>.dmg`, i.e. `shasum -a 256` output run in `build/`).

- [ ] **Step 1:** `make-dmg.sh`: `set -euo pipefail`, require `build/Maine.app`, stage `Maine.app` + `ln -s /Applications Applications` in a temp dir, `hdiutil create -volname Maine -srcfolder … -ov -format UDZO`, write the `.sha256`.
- [ ] **Step 2:** Verify locally: after a Task 1 build, `scripts/make-dmg.sh 1.2.3` → `hdiutil verify build/Maine-1.2.3.dmg` succeeds; `hdiutil attach -nobrowse -readonly` shows `Maine.app` and `Applications`; `(cd build && shasum -a 256 -c Maine-1.2.3.dmg.sha256)` prints `OK`; detach.
- [ ] **Step 3:** `release.yml`: `on: push: tags: ['v*']`; `permissions: contents: write`; one job on `macos-15` that (a) validates the tag against `^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$` and fails otherwise, (b) selects newest Xcode exactly as `ci.yml` does, (c) runs `swift test` in `apps/macos`, (d) `scripts/build-app.sh --release --version "${GITHUB_REF_NAME#v}"`, (e) `scripts/make-dmg.sh`, (f) `gh release create "$GITHUB_REF_NAME" <dmg> <sha256> --generate-notes --title "Maine ${GITHUB_REF_NAME#v}"` adding `--prerelease` when the tag contains `-`.
- [ ] **Step 4:** Validate YAML locally (`python3 -c "import yaml,sys; yaml.safe_load(open('.github/workflows/release.yml'))"` or `actionlint` if installed). State plainly in the final report that the workflow itself can only be proven by pushing a tag.
- [ ] **Step 5:** `docs/releasing.md`: how to cut a release (`git tag vX.Y.Z && git push origin vX.Y.Z`), pre-release tags, the Gatekeeper first-launch note (right-click → Open, or `xattr -dr com.apple.quarantine /Applications/Maine.app`) and that updates installed by the app avoid it, what verification means (HTTPS + SHA-256, not authorship).

### Task 3: Update feed (pure parsing) and skip store

**Files:**
- Create: `apps/macos/Sources/Maine/Update/UpdateFeed.swift`, `apps/macos/Tests/MaineTests/UpdateFeedTests.swift`

**Interfaces:**
- Consumes: `AppVersion` (Task 1).
- Consumes also: `struct UpdateOffer: Equatable, Sendable { version: AppVersion; notes: String; dmgURL: URL; sha256URL: URL; pageURL: URL }`, already present in `Sources/Maine/Update/UpdateOffer.swift` (created by the orchestrator after Task 1; do not redefine it).
- Produces: `enum UpdateFeed { static let latestReleaseURL: URL; static func offer(from json: Data, current: AppVersion) throws -> UpdateOffer?; static func expectedSHA256(from checksumFile: String, fileName: String) -> String? }`; `struct SkippedVersionStore { init(defaults: UserDefaults = .standard); var skipped: AppVersion? { get nonmutating set } }`.

- [ ] **Step 1: Write failing tests** with fixture JSON strings: `offersANewerStableRelease`; `ignoresEqualOrOlder`; `ignoresDraftAndPrerelease`; `ignoresReleaseMissingDMGOrChecksum`; `ignoresNonGithubOrNonHTTPSAssetURLs` (host must be `github.com`, scheme `https`); `compares110Above19`; `malformedJSONThrows`; `parsesChecksumLine` (`"abc123  Maine-1.2.3.dmg\n"` → `"abc123"`; wrong file name → nil; non-hex → nil); `skippedVersionRoundTrips` (isolated `UserDefaults(suiteName:)`).
- [ ] **Step 2:** `swift test --filter UpdateFeedTests` → FAIL.
- [ ] **Step 3:** Implement against GitHub's JSON (`tag_name`, `body`, `html_url`, `draft`, `prerelease`, `assets[].name/browser_download_url`). Asset names must equal `Maine-<version>.dmg` / `.dmg.sha256` for the tag's own version. Checksum token must be 64 hex chars.
- [ ] **Step 4:** `swift test --filter UpdateFeedTests` → PASS.

### Task 4: Installer (download, verify, stage, swap helper)

**Files:**
- Create: `apps/macos/Sources/Maine/Update/UpdateInstaller.swift`, `apps/macos/Tests/MaineTests/UpdateInstallerTests.swift`

**Interfaces:**
- Consumes: `UpdateOffer` (in `Update/UpdateOffer.swift`). Independent of Task 3: the caller passes the expected hash in; this task never parses a checksum file.
- Produces: `enum UpdateInstallError: LocalizedError` (cases `notWritable`, `checksumMismatch`, `badDownload`, `mountFailed`, `wrongApp`); `enum UpdateInstaller { static func sha256(ofFileAt: URL) throws -> String; static func canInstall(over app: URL) -> Bool` (bundle path ends `.app`, parent writable, not under `/Volumes/` or `AppTranslocation`); `static func swapScript(parentPID: Int32, staged: URL, target: URL, relaunch: String = "/usr/bin/open") -> String; static func stage(_ offer: UpdateOffer, expectedSHA256: String, over target: URL) async throws -> URL` (download the DMG, verify against `expectedSHA256`, `hdiutil attach -nobrowse -readonly -mountpoint`, `ditto` app into `<target parent>/.Maine-update-<uuid>/Maine.app`, check its `CFBundleIdentifier == "app.maine.Maine"` and version == `offer.version`, always detach, delete temp files); `static func launchSwap(parentPID: Int32, staged: URL, target: URL) throws` (writes the script to a temp file, runs it detached with `/bin/bash`, output to `/dev/null`) }`.

- [ ] **Step 1: Write failing tests:** `sha256MatchesKnownVector` (file containing `"abc"` → `ba7816bf…15ad`); `swapReplacesTheBundleAfterTheParentExits` (temp dirs holding fake `Maine.app` folders with a marker file; PID of an already-exited child; `relaunch: "/usr/bin/true"`; run the generated script with `/bin/bash`; assert target now has the new marker, no backup or staging dir left); `swapRestoresTheOldBundleWhenStagingIsMissing` (staged path absent → target unchanged); `canInstallRefusesVolumesAndReadOnlyParents` (paths under `/Volumes/…`, a `chmod 555` temp dir, a path not ending `.app`).
- [ ] **Step 2:** `swift test --filter UpdateInstallerTests` → FAIL.
- [ ] **Step 3:** Implement. Script semantics: wait `while kill -0 PID` (cap 60 s), `mv target target.old`, `mv staged target || { mv target.old target; exit 1 }`, `rm -rf target.old staged-parent`, then `relaunch target`. Quote every path; paths come in as arguments, never interpolated unquoted. `stage` throws `checksumMismatch` and deletes the download before anything is mounted.
- [ ] **Step 4:** `swift test --filter UpdateInstallerTests` → PASS. `stage` itself (network + hdiutil) is verified manually in Task 6, not in unit tests.

### Task 5: Coordinator, popup, settings, menu

**Files:**
- Create: `apps/macos/Sources/Maine/Update/UpdateCoordinator.swift`, `apps/macos/Sources/Maine/Views/UpdateSettingsView.swift`, `apps/macos/Tests/MaineTests/UpdateCoordinatorTests.swift`
- Modify: `apps/macos/Sources/Maine/App/MaineApp.swift` (Settings tab "Updates"; `CommandGroup(after: .appInfo)` with "Check for Updates…"), `apps/macos/Sources/Maine/Views/RootView.swift` (configure the coordinator next to `AgentCoordinator.shared.configure…`, and call `vaultDidUnlock()`/`vaultDidLock()` in the existing `onChange(of: appState.isVaultUnlocked)`)

**Interfaces:**
- Consumes: `UpdateFeed`, `SkippedVersionStore`, `UpdateInstaller`, `AppVersion.current`.
- Produces: `@MainActor final class UpdateCoordinator: ObservableObject` with `static let shared`, `init(current: AppVersion = .current, fetch: @escaping () async throws -> Data = UpdateCoordinator.fetchLatest, defaults: UserDefaults = .standard, isIdle: @escaping @MainActor () -> Bool = { true })`, `@Published var automaticChecks: Bool` (persisted, default true), `@Published private(set) var lastChecked: Date?`, `@Published private(set) var phase: Phase` (`idle, checking, downloading, waitingForIdle, failed(String)`), `func configure(appState: AppState, sessions: SessionManager)` (builds `isIdle` = no agent driver busy and `SyncCoordinator.shared.status != .syncing`), `func vaultDidUnlock()` (checks now, then every 7200 s), `func vaultDidLock()` (cancels the timer), `func check(manual: Bool) async -> UpdateOffer?` (automatic + disabled toggle or development build → no network, returns nil; skipped version filtered for automatic checks only; automatic failures silent, manual failures set `phase = .failed`), `func skip(_ offer: UpdateOffer)`, `func install(_ offer: UpdateOffer) async` (fetches `offer.sha256URL` and reads the hash with `UpdateFeed.expectedSHA256`, failing if absent; refuses via `UpdateInstaller.canInstall` → alert + `NSWorkspace.open(offer.pageURL)`; waits for `isIdle`; stages; `launchSwap`; quits via `NSApp.terminate`). The popup is an `NSAlert` ("Maine X.Y.Z is available", "You have A.B.C.", release notes in a scroll-view accessory capped at ~4000 characters) with buttons **Update**, **Later**, **Skip This Version**.

- [ ] **Step 1: Write failing tests** (injected fake `fetch` and an isolated `UserDefaults` suite; `current` set explicitly): `automaticCheckReturnsAnOfferForANewerRelease`; `skippedVersionIsHiddenFromAutomaticChecksButNotManualOnes`; `disabledToggleNeverFetches` (fetch counter stays 0); `developmentBuildNeverAutoChecks`; `automaticFailureIsSilent` (throwing fetch → nil, phase stays idle); `manualFailureSetsFailedPhase`; `lockStopsTheTimer` (after `vaultDidLock()` no further fetch is scheduled; expose a `var isScheduling: Bool` for the assertion).
- [ ] **Step 2:** `swift test --filter UpdateCoordinatorTests` → FAIL.
- [ ] **Step 3:** Implement the coordinator; `fetchLatest` is a `URLSession` GET with `Accept: application/vnd.github+json`, `User-Agent: Maine-Updater`, 15 s timeout, non-200 → throw. Present the popup from `check` results in a small `presentIfNeeded(_ offer:)` on the main actor; do not show a second popup while one is open.
- [ ] **Step 4:** `UpdateSettingsView`: version label, the toggle, **Check Now** button with status text (up to date / error / last checked time). Wire the Settings tab, menu command and RootView hooks listed above.
- [ ] **Step 5:** `swift test` (full suite) → all pass; `scripts/build-app.sh --release --version 9.9.9` compiles.

### Task 6: End-to-end check, docs, invariants

**Files:**
- Modify: `AGENTS.md` (new invariant on the updater's network use; commands list gets `scripts/make-dmg.sh`), `README.md`, `docs/architecture.md`, `apps/macos/README.md`, `docs/macos-design.md` (popup + settings tab)

- [ ] **Step 1:** Local update rehearsal without GitHub: build `--version 1.0.0` into `/tmp/maine-e2e/Maine.app` (copy), build `--version 1.0.1` and `make-dmg.sh 1.0.1`; serve the DMG, checksum and a fake `latest` JSON over `python3 -m http.server` on localhost; run the staging + swap path through a throwaway test harness script (or a temporary `MAINE_UPDATE_FEED_URL` debug override, removed before finishing) and confirm the copy at `/tmp/maine-e2e/Maine.app` reports `1.0.1` afterwards and relaunches. Do **not** touch `/Applications/Maine.app`.
- [ ] **Step 2:** Docs: AGENTS.md invariant ("The updater's GET to the GitHub Releases API is the app's only automatic network call besides sync; no identifiers, opt-out toggle, dev builds exempt"); README and architecture sections for versioning, releases and the updater; link `docs/releasing.md`.
- [ ] **Step 3:** Final verification: `swift test` passes; `scripts/build-app.sh --release --version 1.0.0` and `scripts/make-dmg.sh 1.0.0` succeed; report honestly that `release.yml` and the real GitHub round-trip are unproven until a tag is pushed (suggest `v1.0.0-rc.1` as the first tag, which also exercises the pre-release path).

## Self-review

- Spec coverage: versioning (T1), DMG + workflow + docs (T2, T6), feed/skip/verify (T3), installer incl. writable check and restore (T4), schedule/popup/settings/menu/idle deferral/dev-build exemption (T5), invariants and docs (T6). Declined Ed25519 appears only in Global Constraints.
- Names match across tasks: `AppVersion`, `UpdateOffer`, `UpdateFeed.offer/expectedSHA256`, `SkippedVersionStore`, `UpdateInstaller.stage/swapScript/launchSwap/canInstall`, `UpdateCoordinator.check/install/skip`.
