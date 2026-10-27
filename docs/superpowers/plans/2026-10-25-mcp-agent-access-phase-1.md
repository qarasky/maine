# MCP Agent Access (Phase 1) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let MCP clients (Claude Code, Claude Desktop) drive real, visible Maine SSH sessions on hosts the user allowed: list hosts, open sessions, run commands with exit codes, read output and screen, send keys, close.

**Architecture:** The same executable runs as an MCP stdio bridge (`Maine --mcp`) that forwards tool calls over a 0600 Unix socket to the running app. The app authenticates the client token, checks device-local access rules, and drives `TerminalSession`s through an `AgentDriver` that types wrapped commands, taps PTY output, detects a nonce'd OSC completion marker and returns cleaned output. Pure logic (access, output cleaning, completion, framing, JSON-RPC) is isolated and tested headless; UI is a thin layer on top.

**Tech Stack:** Swift 6, SwiftUI/AppKit, SwiftTerm (only dependency), POSIX sockets + Dispatch, Security (Keychain), CryptoKit, Swift Testing.

**Spec:** `docs/mcp.md` (approved). Read it before starting any task.

## Global Constraints

- macOS 14+, Swift 6 strict concurrency, package at `apps/macos`; **no new package dependencies** (hand-roll MCP JSON-RPC).
- **No approval prompts and no permission modes.** Access (allowed groups/hosts) is the only control.
- Bridge flag: `--mcp` (checked in `MaineEntryPoint.main()` before `MaineApp.main()`); token env var: `MAINE_AGENT_TOKEN`.
- Socket: `~/Library/Application Support/Maine/agent.sock`, directory `0700`, socket `0600`, peer must pass `getpeereid` uid == `getuid()`.
- Client tokens: Keychain service `Maine Agents`, account = client UUID, `kSecAttrAccessibleWhenUnlockedThisDeviceOnly`; store only `SHA-256(token)` hex; compare in constant time.
- Access rules: per vault, in the encrypted local state file `sessions-<vault id>.maine` (field `agentAccess`); never in the vault, never synced, never in `UserDefaults`. The master switch (a non-sensitive bool) lives in `UserDefaults` key `agentAccessEnabled`, default `false`.
- Limits: ≤ 64 KiB output per result (middle-truncated with `…[N bytes omitted]…`, `truncated: true`); 2 MiB capture ring per session; `run` timeout default 60 s, max 600 s; `read_output` `wait_seconds` max 60; `open_session` settles within 30 s; quiet-period fallback 1.5 s; human-typing hold: agent input waits while the user typed within 2 s, at most 5 s.
- Completion marker: typed as `{ <command>\n}; printf '\033]7770;<nonce>;%d\007' "$?"\n`, detected as raw bytes `ESC ] 7770 ; <nonce> ; <digits> BEL`; nonce = 16 random bytes, lowercase hex.
- Exact error copy: `Maine is not running. Open Maine and unlock the vault.` · `This MCP client is not paired with Maine.` · `Maine is locked. Ask the user to unlock it.` · `a command is still running; use read_output, or send_keys ctrl-c` · `paused by the user` · `not found` (identical for missing and non-allowed ids).
- Agent sessions are ephemeral: never checkpointed into saved sessions, never restored; **Keep Session** converts one to a normal session. Vault lock pauses all agent control (processes keep running).
- Diagnostics: `DebugLog` gets tool name, session UUID, client UUID, outcome, durations, byte counts only — never commands, keys, output, screen text, host names, tokens or passwords.
- Tests: headless only. No test may create an `NSWindow`; such tests need `@Test(.uiTest)` and are **never** run locally. Verify with plain `swift test` from `apps/macos`.
- Commits: only if the owner has asked for them; otherwise each "Checkpoint" step is just a green `swift test`.

## Review Focus

1. **Session not connected** — `run`/`send_keys` on a session whose ssh exited or failed must return an error (`session is not connected`) immediately, not wait for the timeout. Test added to Task 7.
2. **Huge output** — `run "yes | head -n 200000"` must return ≤ 64 KiB with `truncated: true`, keep memory bounded by the 2 MiB ring, and still report the exit code. Test added to Task 7.
3. **UTF-8 split across PTY chunks** — a multibyte character (and an ESC sequence) cut between two `dataReceived` calls must come out intact, never as U+FFFD. Test added to Task 2.
4. **Stale socket after a crash** — a leftover `agent.sock` must not stop the server from starting (unlink it after confirming nothing is listening); the bridge with no server must answer `initialize` and return the not-running error per call. Tests added to Task 5.
5. **Concurrent calls** — a second `run` on a busy session gets the busy error; `run`s on two different sessions proceed in parallel; access revoked mid-`run` ends the call with `not found`. Tests added to Task 7.

---

### Task 1: Device-local agent access rules

**Files:**
- Create: `Sources/Maine/Agents/AgentAccess.swift`
- Modify: `Sources/Maine/Services/PasswordStore.swift` (the `LocalState`/`SessionsFile` section)
- Test: `Tests/MaineTests/AgentAccessTests.swift`

**Interfaces:**
- Consumes: `HostLayout`, `HostGroup`, `Host` (existing); `PasswordStore` local-state helpers (existing, private).
- Produces:
  - `public struct AgentAccess: Codable, Equatable, Sendable { public var groups: Set<UUID>; public var hosts: Set<UUID>; public init(groups: Set<UUID> = [], hosts: Set<UUID> = []) }`
  - `extension AgentAccess { public func allows(_ host: Host) -> Bool; public func allowedHosts(in layout: HostLayout) -> [Host]; public func pruned(to layout: HostLayout) -> AgentAccess }`
  - `PasswordStore.agentAccess() throws -> AgentAccess` and `PasswordStore.setAgentAccess(_: AgentAccess) throws`

- [ ] **Step 1: Write the failing tests**

```swift
@Test func aHostIsAllowedByItsGroupOrByItself()
// layout: groups [prod, lab]; hosts web(prod), db(lab), loose(nil)
// AgentAccess(groups: [prod.id]).allows(web) == true, .allows(db) == false, .allows(loose) == false
// AgentAccess(hosts: [loose.id]).allows(loose) == true
// allowedHosts(in: layout) keeps layout order: groups [prod] + hosts [loose] -> ["loose", "web"] ordered as in layout.hosts

@Test func pruningDropsRulesForGoneGroupsAndHosts()
// AgentAccess(groups: [prod.id, UUID()], hosts: [web.id, UUID()]).pruned(to: layout) == AgentAccess(groups: [prod.id], hosts: [web.id])

@Test func rulesLiveInTheLocalFileNotTheVault()
// unlock store, save host+layout, record vault bytes
// setAgentAccess(AgentAccess(hosts: [web.id])) -> vault bytes unchanged; agentAccess() == that value
// sessions saved before survive; collapsedGroupIDs survive
// local file text contains neither web.id.uuidString nor "agentAccess" in plaintext
// copy vault to another dir + unlock -> agentAccess() == AgentAccess()
// lock -> agentAccess() throws VaultError.locked
```

- [ ] **Step 2: Run `swift test --filter AgentAccessTests`** — expected: compile failure (`AgentAccess` undefined).
- [ ] **Step 3: Implement `AgentAccess` and the store methods.** Add `var agentAccess = AgentAccess()` to `LocalState` with a custom `init(from:)` using `decodeIfPresent` so existing version-2 local files still decode; keep `SessionsFile.version = 2` (the field is additive inside the encrypted payload). `setAgentAccess` goes through `updateLocalState` (no vault write, no `.maineVaultDidWrite`).
- [ ] **Step 4: Run `swift test --filter AgentAccessTests`** — expected: PASS; then `swift test` — all green.
- [ ] **Step 5: Checkpoint** (commit only if asked): `feat(agents): device-local agent access rules`.

---

### Task 2: Output capture, cleaning and masking

**Files:**
- Create: `Sources/Maine/Agents/AgentOutput.swift`
- Test: `Tests/MaineTests/AgentOutputTests.swift`

**Interfaces:**
- Produces:
  - `struct AgentOutput: Sendable` (value type, owned by `AgentDriver`):
    - `init(capacity: Int = 2 * 1024 * 1024)`
    - `mutating func append(_ bytes: ArraySlice<UInt8>)` — streaming; buffers incomplete UTF-8 and incomplete escape sequences across calls
    - `var cursor: Int { get }` — total cleaned bytes ever produced (monotonic)
    - `func text(since: Int, maxBytes: Int = 65_536) -> (text: String, cursor: Int, truncated: Bool)` — `since` older than the ring start returns from the ring start with `truncated: true`
    - `var isAlternateScreen: Bool { get }` — tracks `CSI ? 1049 h/l`, `? 1047 h/l`, `? 47 h/l`
    - `var masks: [String]` — any occurrence is replaced by `••••••` in `text(since:)` output (used in phase 2; implement now)
  - `static func middleTruncated(_ text: String, maxBytes: Int) -> (String, Bool)` — marker `…[N bytes omitted]…`

- [ ] **Step 1: Write the failing tests**

```swift
@Test func escapeSequencesAreStripped()
// append("\u{1B}[1;32mok\u{1B}[0m\r\n\u{1B}]0;title\u{07}done") -> text(since: 0).text == "ok\ndone"

@Test func carriageReturnAndBackspaceRewriteTheCurrentLine()
// "50%\r100%\n" -> "100%\n"
// "abc\u{08}\u{08}X\n" -> "aXc\n"   (backspace moves left one column; the next character overwrites)

@Test func utf8AndEscapesSplitAcrossChunksSurvive()   // Review Focus 3
// bytes of "é✓\u{1B}[31mred" split at every index i (two appends) -> text == "é✓red", never contains "\u{FFFD}"

@Test func cursorsAreMonotonicAndRingIsBounded()
// capacity 1_024; append 10_000 "x\n" bytes -> cursor == 10_000 (cleaned count); text(since: 0).truncated == true; text(since: 0).text.utf8.count <= 1_024

@Test func longResultsAreCutInTheMiddle()
// middleTruncated(String(repeating: "a", count: 100_000), maxBytes: 65_536) -> result.utf8.count <= 65_536, contains "…[", "bytes omitted]…", truncated == true

@Test func alternateScreenIsTracked()
// "\u{1B}[?1049h" -> isAlternateScreen; "\u{1B}[?1049l" -> !isAlternateScreen

@Test func masksAreReplacedIncludingAcrossChunks()
// masks = ["hunter2"]; append "pw: hun", then "ter2\n" -> text contains "••••••", not "hunter2"
```

- [ ] **Step 2: Run `swift test --filter AgentOutputTests`** — expected: compile failure.
- [ ] **Step 3: Implement `AgentOutput`.** A small state machine over bytes (ground / ESC / CSI / OSC / DCS-or-SOS-PM-APC / string-terminator), a pending-UTF-8 byte holder, and a current-line buffer with a column index for `\r`/`\b` handling; completed lines are appended to a byte ring (`Data` with trim-from-front when over capacity, tracking `ringStartCursor`). Masking is applied in `text(since:)` over the returned window plus a `max(mask.count)` lookback so split secrets still match.
- [ ] **Step 4: Run `swift test --filter AgentOutputTests`** — PASS; `swift test` — green.
- [ ] **Step 5: Checkpoint**: `feat(agents): PTY output cleaning ring buffer`.

---

### Task 3: Command completion and prompt detection

**Files:**
- Create: `Sources/Maine/Agents/CommandCompletion.swift`
- Modify: `Sources/Maine/Services/SSHAskPass.swift` (extend `isPasswordPrompt`, keep existing cases passing)
- Test: `Tests/MaineTests/CommandCompletionTests.swift`

**Interfaces:**
- Produces:
  - `struct CommandWrapper: Sendable { let nonce: String; let typedText: String; init(command: String, nonce: String = CommandWrapper.makeNonce()); static func makeNonce() -> String }` — `typedText` is exactly `"{ \(command)\n}; printf '\\033]7770;\(nonce);%d\\007' \"$?\"\n"`
  - `struct CompletionDetector: Sendable { init(nonce: String); mutating func scan(_ bytes: ArraySlice<UInt8>) -> Int32?` } — returns the exit code once, when the full marker for this nonce has been seen (marker may span calls)
  - `enum PromptHeuristics { static func looksLikeShellPrompt(_ line: String) -> Bool }` — trailing `$ `, `# `, `% `, `> ` (after trimming trailing spaces to one)
  - `SSHAskPass.isPasswordPrompt(_:)` additionally true for `[sudo] password for alice:` and `Password for alice@host:`; still false for passphrase / one-time / verification

- [ ] **Step 1: Write the failing tests**

```swift
@Test func wrapperTextIsExact()
// CommandWrapper(command: "ls -la", nonce: "ab12").typedText == "{ ls -la\n}; printf '\\033]7770;ab12;%d\\007' \"$?\"\n"

@Test func markerIsFoundAcrossChunkBoundaries()
// marker bytes "\u{1B}]7770;ab12;3\u{07}" split at every index -> scan returns 3 exactly once

@Test func echoedWrapperAndWrongNoncesNeverMatch()
// scan(typedText bytes) -> nil;  scan("\u{1B}]7770;ffff;0\u{07}") with nonce ab12 -> nil

@Test func nonceIsRandomHex()
// makeNonce().count == 32, all hex, two calls differ

@Test func passwordPromptsIncludeSudo()
// true: "[sudo] password for alice: ", "Password:", "Password for alice@web: "
// false: "Enter passphrase for key '/x':", "One-time password:", "Verification code:"

@Test func shellPromptHeuristic()
// true: "alice@web:~$ ", "root@web:/# ", "web% "; false: "Downloading 40%", ""
```

- [ ] **Step 2: Run `swift test --filter CommandCompletionTests`** — compile failure.
- [ ] **Step 3: Implement.** `CompletionDetector` keeps a rolling byte suffix of length `marker prefix + nonce + 12`, matches `ESC ] 7770 ; <nonce> ; digits BEL`, parses digits as `Int32`, and latches after the first match.
- [ ] **Step 4: Run tests** — PASS; `swift test` (incl. existing `PasswordStoreTests` askpass cases) — green.
- [ ] **Step 5: Checkpoint**: `feat(agents): completion marker and prompt detection`.

---

### Task 4: Wire protocol and client tokens

**Files:**
- Create: `Sources/Maine/Agents/AgentProtocol.swift`, `Sources/Maine/Agents/AgentClientStore.swift`
- Test: `Tests/MaineTests/AgentProtocolTests.swift`

**Interfaces:**
- Produces:
  - Frames: 4-byte big-endian length + UTF-8 JSON; max frame 8 MiB. `enum AgentFrame { static func encode<T: Encodable>(_: T) throws -> Data; struct Reader { mutating func feed(_ data: Data) throws -> [Data] } }` — `Reader` throws `AgentProtocolError.frameTooLarge` above 8 MiB.
  - Messages: `struct AgentHello: Codable { var token: String; var clientName: String? }`, `struct AgentHelloReply: Codable { var ok: Bool; var error: String? }`, `struct AgentCall: Codable { var id: Int; var tool: String; var arguments: JSONValue }`, `struct AgentReply: Codable { var id: Int; var result: JSONValue?; var error: String? }`
  - `enum JSONValue: Codable, Equatable, Sendable` (null/bool/number(Double)/string/array/object) with accessors `string(_ key:)`, `int(_ key:)`, `array(_ key:)`.
  - `struct AgentClient: Identifiable, Equatable, Sendable { let id: UUID; var name: String; var createdAt: Date }`
  - `struct AgentClientStore: Sendable { init(service: String = "Maine Agents"); func clients() throws -> [AgentClient]; func add(name: String) throws -> (client: AgentClient, token: String); func revoke(_ id: UUID) throws; func client(forToken: String) throws -> AgentClient? }` — tokens `"maine-agent-"` + 52 base32 chars (Crockford alphabet, 32 random bytes); Keychain value = SHA-256 hex of the token; name in `kSecAttrLabel`; `createdAt` in `kSecAttrCreationDate`; lookup compares hashes in constant time.

- [ ] **Step 1: Write the failing tests**

```swift
@Test func framesRoundTripAndSplitAnywhere()
// encode AgentCall(id: 7, tool: "run", arguments: ["command": "ls"]) ; feed byte-by-byte -> one frame decoding to the same value

@Test func oversizedFramesAreRefused()
// feed length header 0x00800001 -> throws AgentProtocolError.frameTooLarge

@Test func tokensAreShownOnceAndStoredHashed()
// store with service "app.maine.tests.\(UUID())"; (client, token) = add(name: "Claude Code")
// token.hasPrefix("maine-agent-"), token.count == 64
// client(forToken: token)?.id == client.id ; client(forToken: token + "x") == nil
// raw Keychain value for client.id != Data(token.utf8)   (read via SecItemCopyMatching in the test)
// clients().map(\.name) == ["Claude Code"]; revoke(client.id) -> client(forToken: token) == nil
// defer: revoke all test items
```

- [ ] **Step 2: Run `swift test --filter AgentProtocolTests`** — compile failure.
- [ ] **Step 3: Implement** both files (Keychain code mirrors `SyncTokenStore`; `HostStoreError` for Keychain failures).
- [ ] **Step 4: Run tests** — PASS; `swift test` — green.
- [ ] **Step 5: Checkpoint**: `feat(agents): socket framing and client tokens`.

---

### Task 5: Socket server and MCP stdio bridge

**Files:**
- Create: `Sources/Maine/Agents/AgentServer.swift`, `Sources/Maine/Agents/MCPBridge.swift`, `Sources/Maine/Agents/AgentToolCatalog.swift`
- Modify: `Sources/Maine/App/MaineApp.swift` (`MaineEntryPoint.main`: `if CommandLine.arguments.contains("--mcp") { exit(MCPBridge.runStdio()) }` before the askpass check)
- Test: `Tests/MaineTests/AgentServerTests.swift`, `Tests/MaineTests/MCPBridgeTests.swift`

**Interfaces:**
- Consumes: Task 4 frames/messages, `AgentClientStore`.
- Produces:
  - `protocol AgentCallHandling: Sendable { func handle(_ call: AgentCall, client: AgentClient) async -> AgentReply }`
  - `final class AgentServer: @unchecked Sendable { init(socketURL: URL, clients: AgentClientStore, handler: AgentCallHandling); func start() throws; func stop(); func disconnect(clientID: UUID) }` — `static var defaultSocketURL: URL` (Application Support/Maine/agent.sock). `start()` creates the dir 0700, removes a stale socket file only after a `connect()` probe fails, binds, `chmod 0600`, accepts on a background queue, rejects peers whose `getpeereid` uid ≠ `getuid()`, requires an `AgentHello` with a valid token as the first frame (replies `AgentHelloReply(ok:false, error: "This MCP client is not paired with Maine.")` and closes otherwise), then serves `AgentCall`s concurrently per connection.
  - `struct AgentToolDefinition: Sendable { let name: String; let description: String; let inputSchema: JSONValue }` and `enum AgentToolCatalog { static let tools: [AgentToolDefinition] }` for exactly: `list_hosts`, `list_sessions`, `open_session`, `run`, `read_output`, `read_screen`, `send_keys`, `close_session` (phase 1; `type_password` is phase 2). Input schemas use the argument names, defaults and limits from `docs/mcp.md` § Tools; descriptions say what each tool returns in one sentence.
  - `enum MCPBridge { static func runStdio() -> Int32; static func handle(line: String, session: inout BridgeSession) async -> String? }` — `BridgeSession` holds the socket connection (lazy, reconnecting) and `initialized` flag. Supports `initialize` (reply `protocolVersion`: the client's if in `["2025-06-18","2025-03-26","2024-11-05"]` else `"2025-06-18"`; `serverInfo {name:"maine", version: CFBundleShortVersionString or "development"}`; `capabilities {tools:{}}`), `notifications/initialized` (no reply), `ping`, `tools/list` (from `AgentToolCatalog.tools`), `tools/call` (forward, map `AgentReply` to `{content:[{type:"text",text:<json>}], structuredContent:<json>, isError:<bool>}`). Unknown method → JSON-RPC error `-32601`. One JSON message per line on stdin/stdout.

- [ ] **Step 1: Write the failing tests**

```swift
// AgentServerTests (socket in a short temp dir, e.g. /tmp/m-<8 hex>/agent.sock to stay under sun_path's 104 bytes)
@Test func pairedClientsAreServedAndOthersRefused()
// fake handler echoes tool name; valid token -> reply.result == ["tool": "list_hosts"]
// bad token -> hello reply ok == false, error == "This MCP client is not paired with Maine."
@Test func socketFileIsUserOnly()            // posixPermissions == 0o600, dir 0o700
@Test func aStaleSocketFileDoesNotBlockStart() // Review Focus 4: create a regular file at the path; start() succeeds
@Test func revokingDisconnectsLiveConnections() // disconnect(clientID:) -> the client's next read hits EOF

// MCPBridgeTests (no app; drive handle(line:session:) directly)
@Test func initializeNegotiatesTheProtocolVersion()
// {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26",...}} -> result.protocolVersion == "2025-03-26", serverInfo.name == "maine"
@Test func toolsListMatchesTheCatalog()            // names == AgentToolCatalog.tools.map(\.name)
@Test func callsWithoutAServerReturnTheNotRunningError() // Review Focus 4
// tools/call -> result.isError == true, text == "Maine is not running. Open Maine and unlock the vault."
@Test func unknownMethodsAreJSONRPCErrors()         // error.code == -32601
@Test func notificationsGetNoReply()                // handle(notifications/initialized) == nil
```

- [ ] **Step 2: Run `swift test --filter "AgentServerTests|MCPBridgeTests"`** — compile failure.
- [ ] **Step 3: Implement** (POSIX `socket(AF_UNIX, SOCK_STREAM)`, `DispatchSource` reads; the bridge's socket path comes from `AgentServer.defaultSocketURL`, overridable by env `MAINE_AGENT_SOCKET` for tests only). The bridge must not create `NSApplication` and must not launch the app.
- [ ] **Step 4: Run tests** — PASS; `swift test` — green.
- [ ] **Step 5: Checkpoint**: `feat(agents): unix socket server and MCP stdio bridge`.

---

### Task 6: Agent-drivable sessions

**Files:**
- Create: `Sources/Maine/Agents/AgentDriver.swift`
- Modify: `Sources/Maine/Sessions/TerminalSession.swift`, `Sources/Maine/Sessions/SessionManager.swift`
- Test: `Tests/MaineTests/AgentDriverTests.swift`

**Interfaces:**
- Consumes: `AgentOutput`, `CommandWrapper`, `CompletionDetector`, `PromptHeuristics` (Tasks 2–3).
- Produces:
  - `TerminalSession.init(host:executablePath:processArguments: [String]? = nil)` — non-nil replaces `SSHTarget` arguments (tests run `/bin/sh` with `[]`).
  - `TerminalSession.terminalView` becomes `MaineTerminalView: LocalProcessTerminalView` overriding `dataReceived(slice:)` (call `super`, then `outputTap?(slice)`) and `send(source:data:)` (call `super`, then record `lastUserInputAt = Date()`); `TerminalSession.sendInput(_ bytes: [UInt8])` writes to the process without touching `lastUserInputAt`; `var lastUserInputAt: Date?`.
  - `TerminalSession.agent: AgentDriver?` (`@Published`), `var isAgentSession: Bool { agent != nil }`.
  - `@MainActor final class AgentDriver: ObservableObject` — `let clientID: UUID; let clientName: String; @Published var isPaused: Bool; @Published private(set) var isBusy: Bool; @Published private(set) var isActive: Bool` (typing or running) ; `func run(_ command: String, timeout: Duration) async throws -> RunResult` ; `func readOutput(since: Int?, wait: Duration?, maxBytes: Int) async -> ReadResult` ; `func screen() -> ScreenResult` ; `func send(keys: [AgentKey]) async throws` ; `func settleAfterLogin(timeout: Duration) async -> ReadResult`. Result structs: `RunResult { completed, exitCode: Int32?, output, cursor, truncated, mode: "marker" | "quiet" }`, `ReadResult { output, cursor, busy, truncated }`, `ScreenResult { rows: [String], cursorRow, cursorCol, alternateScreen }`.
  - `enum AgentKey: Equatable { case named(String), text(String); init?(_ raw: String) }` — names: `enter`→`\r`, `tab`→`\t`, `esc`→`\u{1B}`, `backspace`→`\u{7F}`, `up/down/right/left`→`ESC [ A/B/C/D`, `ctrl-a`…`ctrl-z`→`0x01…0x1A`; `text:<literal>` → literal.
  - `SessionManager.openAgentSession(_ host: Host, client: AgentClient, name: String?) -> TerminalSession` (creates session, attaches `AgentDriver`, does **not** select it or steal focus, does not checkpoint); `checkpointSessions()` and `restoreSavedSessions` skip agent sessions; `keepSession(_ id: UUID)` detaches the driver so normal checkpointing applies; `endAgentControl(_ id: UUID)` detaches the driver and keeps the session; `endAgentControl(clientID: UUID)`; `var agentSessions: [TerminalSession]`.
  - `AgentDriverError: Error, Equatable` with cases `.busy`, `.paused`, `.notConnected` (messages from Global Constraints; `.notConnected` = `session is not connected`).

- [ ] **Step 1: Write the failing tests** (real PTY with `/bin/sh`, no window)

```swift
@Test func runReturnsOutputAndExitCode()
// driver on TerminalSession(host:, executablePath: "/bin/sh", processArguments: [])
// r = run("echo hi; false", timeout: .seconds(10)) -> r.completed, r.exitCode == 1, r.mode == "marker", r.output.trimmed == "hi" (wrapper echo lines removed)

@Test func timeoutsReturnPartialOutputAndLeaveTheCommandRunning()
// run("echo start; sleep 3; echo end", timeout: .seconds(1)) -> !completed, output contains "start", isBusy
// readOutput(since: r.cursor, wait: .seconds(5)) -> output contains "end", busy == false afterwards

@Test func aBusySessionRefusesAnotherRun()     // run while busy -> throws .busy
@Test func pausedDriversRefuseInput()          // isPaused = true; run -> throws .paused; send(keys:) -> throws .paused
@Test func ctrlCInterruptsARunningCommand()    // run("sleep 30", 1s) then send([.named("ctrl-c")]) -> later readOutput shows not busy
@Test func keysMapToBytes()                    // AgentKey("ctrl-c") == .named("ctrl-c") -> bytes [0x03]; AgentKey("text:ls") -> "ls"; AgentKey("bogus") == nil
@Test func screenReflectsTheVisibleTerminal()  // run("printf 'row-a\\n'") -> screen().rows.contains { $0.contains("row-a") }
@Test func agentSessionsAreNeverSavedOrRestored()
// vault-backed HostStore; manager.openAgentSession(...) ; checkpointSessions() -> store.savedSessions().isEmpty
// keepSession(id); checkpointSessions() -> savedSessions().count == 1
```

- [ ] **Step 2: Run `swift test --filter AgentDriverTests`** — compile failure.
- [ ] **Step 3: Implement.** `run`: refuse if paused/busy/not live; wait while `lastUserInputAt` is within 2 s (max 5 s); record cursor; `sendInput(wrapper.typedText)`; feed tapped bytes to both `AgentOutput` and the `CompletionDetector`; complete on marker or timeout; output = cleaned text since cursor with the two echoed wrapper lines dropped, then `middleTruncated(64 KiB)`. Quiet-period fallback: if no marker arrives but output has been quiet 1.5 s and the last screen line `looksLikeShellPrompt` after the wrapper was echoed, mark the session `mode = "quiet"` for future runs (no wrapper, no exit code). `settleAfterLogin`: wait for first output then 1.5 s of quiet, capped at the timeout; fail fast if the session state becomes `.failed`/`.closed`.
- [ ] **Step 4: Run tests** — PASS; `swift test` — green (existing `WorkspaceLifecycleTests` are `.uiTest` and skipped; `TerminalSessionTests` still pass).
- [ ] **Step 5: Checkpoint**: `feat(agents): agent-driven terminal sessions`.

---

### Task 7: Tool handlers

**Files:**
- Create: `Sources/Maine/Agents/AgentTools.swift`
- Test: `Tests/MaineTests/AgentToolsTests.swift`

**Interfaces:**
- Consumes: `AgentCallHandling`, `AgentToolCatalog`, `AgentCall/AgentReply/JSONValue` (Tasks 4–5), `AgentAccess` (Task 1), `SessionManager` agent APIs and `AgentDriver` (Task 6), `HostStore.layout`, `AppState.isVaultUnlocked`.
- Produces:
  - `@MainActor final class AgentToolHandler: AgentCallHandling` — `init(store: HostStore, sessions: SessionManager, access: @escaping @MainActor () -> AgentAccess, isUnlocked: @escaping @MainActor () -> Bool)`. Results are the JSON objects in the spec table (snake_case keys); `list_hosts` returns `has_saved_password` via `PasswordStore.password(for:) != nil` and never the password.

- [ ] **Step 1: Write the failing tests** (handler built on a vault-backed `HostStore` in a temp dir and a `SessionManager` whose `makeSession` returns `/bin/sh` sessions; no window)

```swift
@Test func listHostsShowsOnlyAllowedHosts()
// hosts web(prod), db(lab), loose; access groups [prod] -> names == ["Web"]; keys ⊆ {id,name,group,hostname,port,username,has_saved_password}; JSON text never contains "secret"
@Test func disallowedAndMissingIdsLookTheSame()
// open_session(db.id) error == "not found" ; open_session(UUID()) error == "not found"
@Test func lockedVaultRefusesEveryTool()
// isUnlocked false -> each tool error == "Maine is locked. Ask the user to unlock it."
@Test func sessionsAreScopedToTheirClient()
// client A opens; list_sessions for client B == []; run with A's session id as B -> "not found"
@Test func runOverTheToolSurface()
// open_session(web) -> session_id; run {command: "echo hi; false"} -> completed == true, exit_code == 1, output == "hi"
@Test func runOnADeadSessionFailsFast()              // Review Focus 1: session process exited -> error "session is not connected" in < 1 s
@Test func hugeOutputIsTruncated()                    // Review Focus 2: run "yes | head -n 200000" -> truncated true, output ≤ 65_536 bytes, exit_code 0
@Test func concurrentRunsFollowTheBusyRule()          // Review Focus 5: second run same session -> busy error; two sessions in parallel both complete
@Test func revokingAccessMidRunEndsTheCall()          // Review Focus 5: run "sleep 5"; change access to [] -> run returns "not found"; session still open, agent == nil
@Test func closeSessionClosesIt()                     // close_session -> sessions.openSessions doesn't contain it
@Test func argumentLimitsAreEnforced()                // run timeout_seconds 900 -> clamped to 600 (reported or behaves); read_output wait_seconds 120 -> clamped to 60
```

- [ ] **Step 2: Run `swift test --filter AgentToolsTests`** — compile failure.
- [ ] **Step 3: Implement** the handler: one `switch call.tool`; every session-bearing tool resolves `session_id` → session whose `agent?.clientID == client.id` and whose host is still allowed, else `not found`; access re-checked when a `run` returns (revocation → `endAgentControl` + `not found`). `DebugLog.shared.record("agent.tool", fields: ["tool", "session", "client", "outcome", "ms", "bytes"])` only.
- [ ] **Step 4: Run tests** — PASS; `swift test` — green.
- [ ] **Step 5: Checkpoint**: `feat(agents): MCP tool handlers`.

---

### Task 8: App integration, UI and docs

**Files:**
- Create: `Sources/Maine/Agents/AgentCoordinator.swift`, `Sources/Maine/Views/AgentSettingsView.swift`
- Modify: `Sources/Maine/App/MaineApp.swift` (Settings scene → `TabView { SyncSettingsView() Sync tab; AgentSettingsView() Agents tab }`; start coordinator after launch), `Sources/Maine/App/AppState.swift` (call coordinator on lock/unlock), `Sources/Maine/App/MenuBarController.swift` ("Agents: N active" + "Stop All Agents" when N > 0), `Sources/Maine/Views/HostListView.swift` (agent badge `cpu` SF Symbol + activity `ProgressView().controlSize(.mini)` on agent session rows; agent sessions always shown as child rows), `Sources/Maine/Views/SessionView.swift` (header strip "Driven by <client name>" with Pause/Resume, Stop, Keep Session)
- Modify docs: `AGENTS.md` (invariant 12 verbatim from `docs/mcp.md` § Invariant changes; amend 8 and 11; architecture-map row "Agents"), `README.md` (new "Agents (MCP)" section with setup + consent note), `docs/architecture.md` (agent access section), `docs/macos-design.md` (badge, header strip), `docs/mcp.md` (status: phase 1 built)
- Test: `Tests/MaineTests/AgentCoordinatorTests.swift`

**Interfaces:**
- Consumes: everything above.
- Produces: `@MainActor final class AgentCoordinator: ObservableObject` — `static let shared`; `@Published var isEnabled: Bool` (persists `UserDefaults` `agentAccessEnabled`; on → `server.start()`, off → `server.stop()` + `endAgentControl` for all); `@Published private(set) var clients: [AgentClient]`; `func addClient(name:) throws -> (AgentClient, token: String, claudeCodeCommand: String, desktopJSON: String)`; `func revoke(_ id: UUID)` (Keychain delete + `server.disconnect` + `endAgentControl(clientID:)`); `func access() -> AgentAccess` / `func setAccess(_:)` (via `PasswordStore`, pruned to `store.layout`); `func vaultDidLock()` (pause every driver, remember which were already paused), `func vaultDidUnlock()` (unpause those it paused); `func stopAll()`; `var activeCount: Int`. `claudeCodeCommand` = `claude mcp add maine --env MAINE_AGENT_TOKEN=<token> -- "<Bundle.main.executableURL.path>" --mcp`.

- [ ] **Step 1: Write the failing tests** (no window; inject a temp socket URL, test Keychain service, vault-backed store)

```swift
@Test func enablingStartsAndDisablingStopsTheServer()   // isEnabled true -> socket file exists; false -> gone, agent sessions have agent == nil
@Test func lockPausesAndUnlockResumesOnlyWhatLockPaused()
// two agent sessions, one paused by user; vaultDidLock -> both paused; vaultDidUnlock -> user-paused stays paused, other resumes
@Test func revokingAClientEndsItsControl()             // revoke -> its sessions agent == nil, sessions still open; client(forToken:) == nil
@Test func setupCommandUsesTheRunningExecutable()      // claudeCodeCommand contains "--mcp" and "MAINE_AGENT_TOKEN=maine-agent-"
@Test func accessIsPrunedToTheCurrentLayout()          // setAccess with a stale group id -> access() lacks it
```

- [ ] **Step 2: Run `swift test --filter AgentCoordinatorTests`** — compile failure.
- [ ] **Step 3: Implement the coordinator**, then the UI: Settings → Agents (master toggle; consent text from `docs/mcp.md` § What the human sees; clients list with Add Client… sheet showing token + both snippets once with Copy buttons, and Revoke; Access list: one checkbox per group and per host in sidebar order); sidebar badge/activity; session header strip ("Driven by <client name>" while controlled; "Agent disconnected" with a Close button after `endAgentControl`/revocation/master-switch-off, until the user closes or keeps the session); menu bar items. All motion respects Reduce Motion; hover/selection rules unchanged (invariants 5, 6, 9).
- [ ] **Step 4: Run `swift test`** — all green (UI tests skipped). Build the app: `scripts/build-app.sh --release`. Manually verify with a real client (owner): `claude mcp add …` from Settings → run `list_hosts` → `open_session` → `run "uname -a"` shows in the sidebar with the badge; Pause blocks the next call; lock pauses; Stop All ends control.
- [ ] **Step 5: Update the docs listed above**, run `(cd ../../packages/schema && python3 tools/validate.py)` (unchanged, still green) and `swift test` once more.
- [ ] **Step 6: Checkpoint**: `feat(agents): agent coordinator, settings and session UI`.
