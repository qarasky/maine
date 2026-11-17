# Maine Linux Client — Phases 0 and 1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prove a Rust + Qt 6 terminal item is viable (phase 0), then build the Qt-free `maine-core` crate with a macOS-compatible vault, host model, `ssh` command builder and `~/.ssh/config` reader (phase 1).

**Architecture:** `apps/linux` is a Cargo workspace. `crates/maine-core` has no UI or Qt dependency and mirrors the macOS `Models/` and `Services/` layers. The spike lives in a throwaway crate and is deleted or promoted after the go/no-go decision. Phases 2–4 get their own plans after the spike result.

**Tech Stack:** Rust (cargo 1.100), `aes-gcm`, `pbkdf2`+`sha2`, `serde`/`serde_json`, `uuid`, `base64`, `glob`; spike: `cxx-qt` 0.10, Qt 6.11, `alacritty_terminal` 0.26, `portable-pty` 0.9.

**Spec:** `docs/superpowers/specs/2026-11-17-linux-client-design.md`

## Global Constraints

- Vault v2 envelope is byte-compatible with macOS: PBKDF2-SHA256, 600,000 iterations, 32-byte salt, 32-byte key; AES-256-GCM; sealed blob = `nonce(12) | ciphertext | tag(16)`; AAD = `maine-vault:{version}:{UPPERCASE-UUID}:{base64-salt}:{iterations}` (UTF-8); verification marker plaintext = `Maine password vault v1`; vault JSON keys `version,id,salt,iterations,verification,passwords,payload`, binary fields base64, UUIDs uppercase on write.
- Master password minimum 12 characters (counted as Unicode scalars/characters) on create; the password is hashed as its raw UTF-8 bytes, no normalization.
- Payload JSON (`Contents`): `groups` (optional), `hosts`, `passwords` (UUID-string → password), `sessions` (read-only legacy, never written). Dates in saved sessions are `f64` seconds since 2001-01-01 UTC.
- Wrong password, truncated or malformed file never modify the file on disk. Writes are atomic (temp file in the same directory, fsync, rename) with mode `0600`; the directory is `0700`.
- Host validation messages and rules match `Host.issues` in `apps/macos/Sources/Maine/Models/Host.swift` exactly. Schema version supported: 2 (1 readable); newer is refused and never overwritten.
- `maine-core` has no Qt, no GUI and no network dependency (HTTP arrives in phase 3). Nothing in it logs secrets.
- Commit only when asked (AGENTS.md). Plan "Commit" steps are replaced by leaving work in the tree until the user asks.
- UI tests never take over the screen: anything that creates a window runs with `QT_QPA_PLATFORM=offscreen`.

## Review Focus

- Non-ASCII master password (Cyrillic, NFC vs NFD forms of the same text): must derive from raw UTF-8 bytes, so a vault made on macOS opens. Pinned in Task 1.6.
- Truncated / garbage / wrong-version `passwords.maine`: must return `InvalidFile` and leave the file byte-identical. Pinned in Task 1.6.
- Vault written by macOS with lowercase UUIDs in payload (the golden file) and `\/`-escaped base64 slashes: must still unlock; password lookups are case-insensitive on UUID. Pinned in Task 1.7.
- Interrupted write (crash between temp write and rename): the previous vault must remain readable. Pinned in Task 1.6.
- `~/.ssh/config` with `Include` cycle, `Host *`, quoted values, `Host a b` multi-pattern, trailing comments: must not hang or panic. Pinned in Task 1.9.

---

## Phase 0 — Terminal spike (throwaway)

### Task 0.1: Qt development toolchain

**Files:** none in repo.

- [ ] **Step 1:** Install: `sudo apt-get install -y qt6-base-dev qt6-declarative-dev qml6-module-qtquick qml6-module-qtquick-controls qml6-module-qtquick-window qml6-module-qtquick-layouts libgl-dev pkg-config`
- [ ] **Step 2:** Verify: `pkg-config --modversion Qt6Core Qt6Quick` prints `6.11.x` for both; `qmake6 --version` runs.
- [ ] **Step 3:** If a package name is wrong, `apt-cache search` for it and fix; record the final package list in `docs/development.md` (Task 1.10).

### Task 0.2: cxx-qt hello-world

**Files:**
- Create: `apps/linux/Cargo.toml` (workspace, members `crates/*`), `apps/linux/crates/spike-terminal/{Cargo.toml,build.rs,src/main.rs,qml/main.qml}`

**Interfaces:** Produces: a running QML window launched from Rust with a Rust `QObject` property visible in QML.

- [ ] **Step 1:** Create workspace and `spike-terminal` using `cxx-qt` / `cxx-qt-build` 0.10 and a `Greeter` QObject with a `message: QString` property, shown in a `Text`.
- [ ] **Step 2:** `QT_QPA_PLATFORM=offscreen cargo run -p spike-terminal -- --selftest` builds and exits 0 after the QML engine loads (selftest flag quits after first frame). Expected: exit code 0, no QML warnings.
- [ ] **Step 3:** If cxx-qt 0.10 fails to build against Qt 6.11, record the exact error and try the next cxx-qt release; if none builds, stop and report (toolkit decision reopens).

### Task 0.3: Terminal item

**Files:**
- Create: `apps/linux/crates/spike-terminal/cpp/terminal_item.{h,cpp}`, `src/session.rs`

**Interfaces:** Produces: QML type `TerminalItem` with `start(program: QString, args: QStringList)`, a `cols/rows` readout, keyboard input, and painting of an `alacritty_terminal` grid.

- [ ] **Step 1:** `session.rs`: spawn `portable-pty` child (bash), reader thread feeding `alacritty_terminal::Term` through its `vte` processor; a `snapshot()` returns rows of `(char, fg, bg, flags)` cells plus cursor; resize sends `SIGWINCH` via the pty.
- [ ] **Step 2:** `terminal_item.cpp`: a `QQuickPaintedItem` that paints the snapshot with a monospace `QFont`, handles `keyPressEvent` → bytes (modifiers, arrows, function keys, Ctrl-letter), `inputMethodEvent` for compose/Cyrillic, mouse selection, wheel scrollback, and calls Rust through a cxx bridge (`write_input`, `resize`, `snapshot`).
- [ ] **Step 3:** Offscreen smoke test: start `printf 'héllo\n'` and assert the snapshot's first row text is `héllo` (`cargo test -p spike-terminal --test snapshot`).
- [ ] **Step 4:** Manual check on the real desktop (the user's session; ask nothing, run it briefly with `gui-run`): vim, htop, tmux render; `seq 1 2000000` stays responsive; Cyrillic via `wl-copy` + Ctrl+Shift+V; wide chars and emoji keep columns aligned; resize reflows.

### Task 0.4: Go/no-go note

**Files:** Create `docs/superpowers/notes/2026-11-17-linux-spike-result.md`

- [ ] **Step 1:** Record each criterion from the spec (*Phases → 0*) as pass/fail with evidence. If any fail, state whether it is fixable inside the custom item or reopens the toolkit decision.
- [ ] **Step 2:** Go → promote the item into `crates/maine-app` in phase 2. No-go → stop and ask the user.

---

## Phase 1 — `maine-core`

Rust edition 2021, `#![forbid(unsafe_code)]` in `maine-core`. All paths below are under `apps/linux/crates/maine-core/`.

### Task 1.1: Crate skeleton and error type

**Files:** Create `Cargo.toml`, `src/lib.rs`, `src/error.rs`.

**Interfaces:** Produces `pub enum VaultError { Locked, InvalidPassword, InvalidFile, WeakPassword, UnsupportedVersion(u32), Io(String) }` implementing `std::error::Error` + `Display` with the macOS user-facing wording for the first four (`apps/macos/.../PasswordStore.swift` `errorDescription`); `pub struct StoreError(pub String)` for layout/validation failures.

- [ ] **Step 1:** Test `error_messages_match_macos` asserting `InvalidPassword.to_string() == "The master password is incorrect, or the vault has been damaged."` and `WeakPassword.to_string() == "Use a master password of at least 12 characters."`.
- [ ] **Step 2:** Run `cargo test -p maine-core` → FAIL. Implement; run → PASS.

### Task 1.2: Host model and validation

**Files:** Create `src/model/mod.rs`, `src/model/host.rs`. Test: in-file `#[cfg(test)]` plus `tests/host_validation.rs`.

**Interfaces:** Produces:
```rust
pub struct Host { pub id: Uuid, pub name: String, pub hostname: String, pub port: u16 /*stored as i64 on decode*/, pub username: String, pub group_id: Option<Uuid> }
impl Host { pub const DEFAULT_PORT: i64 = 22; pub fn issues(&self) -> Vec<String>; pub fn is_valid(&self) -> bool; pub fn display_name(&self) -> String; pub fn target(&self) -> String; pub fn subtitle(&self) -> String }
```
Port is stored as `i64` internally so `0` and `70000` from a file decode and are reported by `issues()` rather than failing decode.

- [ ] **Step 1:** Tests: blank name → `"A name is required."`; hostname with space → `"The hostname cannot contain spaces."`; `-oProxyCommand=x` → the `"-"` message verbatim from macOS; 254-char hostname; port 0 and 65536 → `"The port must be between 1 and 65535."`; username with whitespace / 129 chars; serde roundtrip emits keys `id,name,hostname,port,username` and `group` only when set; decoding a host with missing/`null` fields defaults them (name `""`, port 22, username `""`).
- [ ] **Step 2:** Run → FAIL. Implement `Host` with a lenient `Deserialize` (all fields optional, UUID default `Uuid::new_v4()`), `Serialize` skipping `group` when `None`. Run → PASS.

### Task 1.3: Groups, layout, host configuration

**Files:** Create `src/model/group.rs`, `src/model/layout.rs`, `src/model/configuration.rs`. Test: `tests/schema_examples.rs`.

**Interfaces:**
```rust
pub struct HostGroup { pub id: Uuid, pub name: String } // MAX_NAME_LENGTH = 128
pub struct HostLayout { /* private groups, hosts */ }
impl HostLayout { pub fn new(groups: Vec<HostGroup>, hosts: Vec<Host>) -> Self; pub fn groups(&self)->&[HostGroup]; pub fn hosts(&self)->&[Host]; pub fn ungrouped(&self)->Vec<&Host>; pub fn hosts_in(&self, g: Uuid)->Vec<&Host>; pub fn add_group(&mut self, base:&str)->HostGroup; pub fn rename_group(&mut self, id:Uuid, name:&str)->Result<(),StoreError>; pub fn delete_group(&mut self, id:Uuid); pub fn move_group(&mut self, id:Uuid, before:Option<Uuid>); pub fn move_host(&mut self, id:Uuid, to_group:Option<Uuid>, before:Option<Uuid>) }
pub struct HostConfiguration { pub schema_version: u32, pub groups: Vec<HostGroup>, pub hosts: Vec<Host> }
impl HostConfiguration { pub const CURRENT_SCHEMA_VERSION: u32 = 2; pub fn from_slice(&[u8]) -> Result<Self, ConfigError>; pub fn to_vec(&self) -> Vec<u8> }
pub enum ConfigError { UnsupportedVersion(u32), Malformed(String) }
```
Behavior copied from `HostLayout.swift` (dangling group → ungrouped; `add_group` suffixes `" 2"`, `" 3"`; delete group ungroups its hosts keeping order; `move_host` lands before target only if target is in the destination group, else after the last host of that section).

- [ ] **Step 1:** Port every assertion from `apps/macos/Tests/MaineTests/HostLayoutTests.swift` and `HostConfigurationTests.swift` as Rust tests (read those files; one Rust test per Swift test, same names in snake_case).
- [ ] **Step 2:** `schema_examples.rs`: decode `packages/schema/examples/{hosts.json,hosts.groups.json,hosts.empty.json}` (path via `env!("CARGO_MANIFEST_DIR")/../../../../packages/schema/examples`); `hosts.invalid-version.json` → `ConfigError::UnsupportedVersion`.
- [ ] **Step 3:** Run → FAIL; implement; run → PASS.

### Task 1.4: Credential text

**Files:** Create `src/model/credential_text.rs`.

**Interfaces:** `pub fn credential_text(host:&Host, password:Option<&str>) -> String` (format in `CredentialText.swift`).

- [ ] **Step 1:** Port `CredentialTextTests.swift`: default port omits `port:` line; blank username omits `username:`; empty password omitted. Run → FAIL; implement; run → PASS.

### Task 1.5: Vault crypto envelope

**Files:** Create `src/vault/mod.rs`, `src/vault/envelope.rs`.

**Interfaces:**
```rust
pub const ITERATIONS: u32 = 600_000;
pub const MARKER: &[u8] = b"Maine password vault v1";
pub struct VaultKey([u8;32]); // zeroize on drop, no Debug/Clone leaking contents
pub fn derive_key(password:&str, salt:&[u8], iterations:u32) -> VaultKey;
pub fn context(version:u32, id:Uuid, salt:&[u8], iterations:u32) -> Vec<u8>; // "maine-vault:{v}:{UPPER-UUID}:{b64 salt}:{iters}"
pub fn seal(plain:&[u8], key:&VaultKey, aad:&[u8]) -> Vec<u8>;           // random 12-byte nonce | ct | tag
pub fn open(sealed:&[u8], key:&VaultKey, aad:&[u8]) -> Result<Vec<u8>, VaultError>;
```

- [ ] **Step 1:** Tests: `context_format_matches_macos` (fixed id `6AA6118A-7CB3-4F6E-BBED-00F0E2972F2F`, salt `nD0Ue790c9S04xu2nWg/Tsfb20ejbbpFVY2Bo7nZ7x0=` → exact string); `seal_open_roundtrip`; `open_with_wrong_aad_fails`; `open_truncated_fails` (len < 28); `derive_key_is_deterministic_and_utf8_raw` (NFC "é" vs NFD "é" derive different keys — raw bytes).
- [ ] **Step 2:** Run → FAIL; implement with `pbkdf2::pbkdf2_hmac::<Sha256>` and `aes_gcm::Aes256Gcm`. Run → PASS.

### Task 1.6: Vault file operations

**Files:** Create `src/vault/file.rs`, `src/vault/contents.rs`. Test: `tests/vault_file.rs`.

**Interfaces:**
```rust
pub struct Contents { pub groups: Option<Vec<HostGroup>>, pub hosts: Vec<Host>, pub passwords: BTreeMap<Uuid,String>, pub sessions: Option<Vec<SavedSession>> }
pub struct SavedSession { pub id:Uuid, pub host_id:Uuid, pub name:String, pub history:String, pub saved_at:f64, pub uses_default_name:Option<bool>, pub number:Option<i64> }
pub struct VaultFile { /* path */ }
pub struct Unlocked { pub id: Uuid, key: VaultKey }
impl VaultFile {
  pub fn new(path: impl Into<PathBuf>) -> Self; pub fn exists(&self)->bool;
  pub fn default_path() -> PathBuf; // $XDG_DATA_HOME/maine/passwords.maine, else ~/.local/share/maine/
  pub fn create(&self, master:&str) -> Result<Unlocked, VaultError>;     // WeakPassword < 12 chars; refuses to overwrite an existing file (InvalidFile)
  pub fn unlock(&self, master:&str) -> Result<Unlocked, VaultError>;     // v1 → UnsupportedVersion(1); wrong pw → InvalidPassword
  pub fn contents(&self, u:&Unlocked) -> Result<Contents, VaultError>;
  pub fn update(&self, u:&Unlocked, f: impl FnOnce(&mut Contents)) -> Result<(), VaultError>; // clears `sessions`, reseals payload, atomic write
  pub fn validate_blob(data:&[u8], master:&str) -> Result<(), VaultError>;  // for sync adoption later; touches nothing
  pub fn key_opens(&self, data:&[u8], u:&Unlocked) -> bool;
}
```
Structural validation as `decodeVault`: version ∈ {1,2}; salt 32 bytes; iterations == 600000; v2 requires payload and empty `passwords`; password keys are UUIDs.

- [ ] **Step 1:** Tests in `tests/vault_file.rs` (temp dir via `tempfile`): `create_then_unlock_roundtrip`; `create_rejects_11_char_password`; `create_refuses_existing_file`; `wrong_password_leaves_file_byte_identical`; `truncated_file_is_invalid_and_untouched`; `garbage_json_is_invalid_and_untouched`; `unlock_with_cyrillic_password_roundtrips`; `file_mode_is_0600_and_dir_0700`; `update_persists_hosts_groups_and_passwords`; `update_clears_legacy_sessions`; `blank_password_removes_entry` (via `update`); `interrupted_write_keeps_previous_vault` (create a stray `passwords.maine.tmp` garbage file beside the vault, assert `unlock` still works and a following `update` replaces the temp cleanly); `validate_blob_does_not_touch_files`.
- [ ] **Step 2:** Run → FAIL; implement. JSON write: `serde_json::to_vec_pretty`; atomic write via temp file `.<name>.tmp-<random>` in the same directory, `File::sync_all`, `rename`, `chmod 0600`, directory fsync. Run → PASS.

### Task 1.7: Golden-file interoperability

**Files:** Create `tests/fixtures/vault-golden-v2.json` (copy of `apps/android/app/src/test/resources/vault-golden-v2.json`), `tests/golden.rs`.

- [ ] **Step 1:** Test `reads_macos_golden_vault`: copy the fixture to a temp file, `unlock("golden-horse-battery-42")`; assert 2 hosts; host `Golden web` has hostname `203.0.113.10`, port 2222, username `deploy`, id `0F8FAD5B-D9CB-469F-A165-70867728950E`; password for it is `golden-password-1`; password for `6ba7b810-9dad-11d1-80b4-00c04fd430c8` is `golden-password-2`; the legacy `sessions` array holds one session named `Session 1` with history `"golden scrollback\n$ "` and `saved_at == 804692800.5`.
- [ ] **Step 2:** `golden_roundtrip_stays_readable`: `update` (no-op write) then reopen with a fresh `VaultFile` and re-unlock; hosts and passwords equal.
- [ ] **Step 3:** `golden_wrong_password_fails` → `InvalidPassword`.
- [ ] **Step 4:** Run → FAIL (before Task 1.6 impl is complete this is the gate) → PASS.

### Task 1.8: SSH target and terminal environment

**Files:** Create `src/ssh/mod.rs`, `src/ssh/target.rs`.

**Interfaces:**
```rust
pub const SSH_PATH: &str = "/usr/bin/ssh";
pub struct SshTarget { pub executable: String, pub arguments: Vec<String> }
impl SshTarget { pub fn make(host:&Host)->Self; pub fn command_line(&self)->String; pub fn resolved(&self, config:&SshConfigFile)->Resolution }
pub struct Resolution { pub hostname:String, pub username:String, pub port:i64, pub config_pattern:Option<String> }
pub fn terminal_environment() -> Vec<(String,String)>; // inherited env + TERM=xterm-256color, COLORTERM=truecolor, LANG default en_US.UTF-8 when unset/empty
```

- [ ] **Step 1:** Port `SSHTargetTests.swift`: default port & no user → `["example.com"]`; port 2222 → `["-p","2222","example.com"]`; user → `["-l","deploy",...]`; `command_line` joins with spaces; `resolved` takes config `Port`/`User`/`HostName`, explicit flags win, local user from `$USER`; `terminal_environment` keeps `SSH_AUTH_SOCK` and sets `TERM`.
- [ ] **Step 2:** Run → FAIL; implement; run → PASS.

### Task 1.9: `~/.ssh/config` reader

**Files:** Create `src/ssh/config_file.rs`. Test: `tests/ssh_config.rs` with fixtures written to a temp dir.

**Interfaces:**
```rust
pub struct SshConfigEntry { pub pattern:String, pub hostname:Option<String>, pub user:Option<String>, pub port:Option<i64> }
impl SshConfigEntry { pub fn is_connectable(pattern:&str)->bool; pub fn to_host(&self)->Option<Host> }
pub struct SshConfigFile { pub entries: Vec<SshConfigEntry> }
impl SshConfigFile { pub fn default_path()->PathBuf; pub fn load(path:&Path)->Self; pub fn entry(&self, pattern:&str)->Option<&SshConfigEntry> }
```
Rules from `SSHConfigFile.swift`: first value wins; `Host a b` creates both entries; `Include` (tilde, relative to including file, glob with braces expanded manually, depth < 8, visited set); `=` and tab separators; quotes stripped; `#` comments only at line start or after whitespace; case-insensitive pattern lookup; missing file → empty.

- [ ] **Step 1:** Port `SSHConfigFileTests.swift` cases; add `include_cycle_terminates`, `include_glob_and_braces`, `wildcard_host_not_connectable`, `multi_pattern_host_line`, `hash_inside_value_survives`.
- [ ] **Step 2:** Run → FAIL; implement; run → PASS.

### Task 1.10: Docs and wiring

**Files:** Modify `AGENTS.md` (commands + architecture row for `apps/linux`), `docs/development.md` (Linux status row, Qt dev packages from Task 0.1, `cargo test` command), `docs/architecture.md` (replace "`apps/linux` is not created yet"), `README.md` status line.

- [ ] **Step 1:** Update the four docs to say the Linux client exists, phase 1 (core) is done and later phases are planned; list `cd apps/linux && cargo test`.
- [ ] **Step 2:** Full verification: `cd apps/linux && cargo fmt --check && cargo clippy --all-targets -- -D warnings && cargo test` all green; `(cd packages/schema && python3 tools/validate.py)` still passes.

---

## Later phases (separate plans, written after the spike)

- Phase 2 App MVP · Phase 3 Sync client · Phase 4 MCP agent access — as in the spec.
