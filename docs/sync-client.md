# Sync client implementation guide

The server and its wire contract ([`sync-protocol.md`](sync-protocol.md)) are
done, and **the macOS client implements this guide** (`Sources/Maine/Sync/`).
Android has not started. This file remains the specification:
what to store, when to talk to the server, exactly how to resolve conflicts,
and how "workspaces" (several tokens on one device) should behave.

Read the protocol document first; this one assumes it.

## Status on macOS

`Sources/Maine/Sync/` implements the protocol; Settings → Sync and the sidebar
footer expose it:

- create, join (including the empty-slot push), pull, push, rotate, disable and
  delete;
- one active profile, with the other profiles parked as workspace files;
- conflict copies and the "another device changed the vault" banner;
- pull on unlock and on app foreground, a 2 s push debounce, a 30 s `HEAD`
  poll, a best-effort push on lock and a 2 s push on quit;
- `0600` vault, copy and workspace files written with `Data.write(.atomic)`;
- tokens in the Keychain (`Maine Sync`), profiles in preferences.

Not implemented on macOS:

- the version-history UI (`GET /v1/vault/versions`); the client never lists old
  versions, so a pruned version has no recovery flow in the app;
- several profiles syncing at once (deliberately, see Workspaces).

## Ground rules

1. **The synced object is the vault file, byte for byte.** Clients never
   decrypt, re-encrypt, filter or re-pack before upload. The blob on the
   server must be identical to `passwords.maine` on disk.
2. **The token is a secret on the level of the master password.** Never log
   it, never show it after pairing, never put it in the vault (the vault is
   replaced by sync; the token must survive that), never send it anywhere but
   the configured server.
3. **The master password never leaves the device** and is never sent to the
   server. Two devices can only sync if they open the same vault, which in
   v1 means the same master password.
4. **Nothing about sync may break the local-first behavior.** A client with
   no profile behaves exactly as today. Sync failures surface as status, not
   as blocked editing or blocked unlocking.
5. **Sync runs only while the vault is unlocked.** All triggers below assume
   an unlocked vault with a session key available. The best-effort lock push is
   started just before the vault locks and only reads the file, so it never
   needs the key.
6. **Diagnostics stay metadata-only.** Extend the existing rule: server URL
   and slot version may be logged; token, blob contents, conflict-copy
   contents may not.

## What a client stores

A **profile** is the client-side record of one sync pairing:

| Field | Secret | Storage |
| ----- | ------ | ------- |
| `id` | no | Preferences; a client-generated UUID |
| `name` | no | Preferences; user-editable label ("Homelab") |
| `serverURL` | no | Preferences |
| `token` | **yes** | OS secure storage, keyed by profile `id` |
| `lastVersion` | no | Preferences; the `ETag`/version the local vault came from (0 = never synced) |
| `lastSHA256` | no | Preferences; hex SHA-256 at `lastVersion`, used to skip no-op pushes |
| `lastSyncedAt` | no | Preferences; for UI only |

The token lives outside the vault deliberately. It must survive a remote vault
replacing the local file, and it must be available to the sync loop even if the
user never unlocks that particular vault file. (In v1 sync still only runs
while unlocked — see the ground rules — but the token's home is secure storage,
not the vault.)

Per-platform secure storage:

| Platform | Store |
| -------- | ----- |
| macOS | Keychain generic password, service `Maine Sync`, account = profile id |
| Android | Keystore-encrypted preferences (the vault key itself stays memory-only; the token must persist across launches, so it needs Keystore-backed storage) |

> macOS note: the Keychain now holds this Mac's vault key
> (`app.maine.vault-key`), the sync tokens (`Maine Sync`, account = profile id)
> and the MCP client token hashes (`Maine Agents`). That extension of the
> invariant is recorded in `AGENTS.md`; it is not a pending question.

## Workspaces: several tokens on one device

The server has nothing to add here: every token is already an independent
slot, and nothing on the server knows about a client's profile list. The
design decision is entirely client-side.

**Recommended model: a profile list with exactly one active profile.**

- Settings shows all profiles, marks the active one, and switches to another
  when its row is clicked; the user can pair as many as they want.
- Only the active profile's vault is the live `passwords.maine`. This keeps
  the "one active vault file" invariant intact and keeps every existing
  feature (hosts, sessions, keychain entry, helper protocol) untouched.
- Inactive workspaces keep their last synced file as
  `passwords.workspace-<profile-id>.maine` (0600) next to the active vault.
  Nothing about them is loaded into memory. Pairing a second profile copies
  the live vault into the outgoing profile's workspace file, so switching back
  is lossless.
- **Switching workspaces**: checkpoint every open session's snapshot into the
  vault being left, stop and drop the sessions (their hosts belong to that
  vault), push it with a 3 s timeout, move the active file to its workspace
  file (or to a pre-sync copy when no profile is outgoing), move the target
  workspace file into `passwords.maine` when it exists, lock, and ask for that
  vault's master password; normal startup then restores sessions and resumes
  syncing the new profile.
- **Not in v1**: several profiles syncing concurrently in the background.
  It multiplies conflict paths, gives the "which vault is live?" question no
  good answer, and buys almost nothing — a workspace that is not active is
  not being edited. If it is ever wanted, the server needs no change.

Alternative that was considered and rejected for v1: one local vault per
profile with a sidebar switcher showing several vaults at once. That turns
every feature (session manager, menus, keychain handling) into a
multi-vault feature and breaks the single-active-file invariant.

## Lifecycle and triggers

| Event | Action |
| ----- | ------ |
| App unlocks the vault | Pull |
| Join flow completes | Adopt the file already downloaded (or push, for an empty slot) |
| Every vault save (host edit, password change) | Push, debounced ~2 s |
| Vault lock | Best-effort push, fire and forget |
| Quit | Best-effort push with a 2 s timeout |
| App foreground | Pull (`HEAD` first) |
| While unlocked | Poll every 30 s (`HEAD`; `304` almost always) |
| Before switching workspaces | Push with a 3 s timeout |
| Network/server failure | Polling backs off 30 s → 2 min → 5 min, cap; never blocks the UI |

One pull or push at a time per profile. A trigger that arrives while a request
is in flight is dropped, not queued: the next vault save (for a push),
foreground or poll (for a pull) picks the work up again.

## Algorithms

### Create (this device becomes the origin)

1. User provides server URL (and an optional name) and asks to enable sync.
2. `POST /v1/vaults` with the current `passwords.maine` as the body (create
   secret if the server requires one). With no local vault the body is empty.
   The URL is normalized before sending: `http://` is added when no scheme is
   given, and trailing slashes are trimmed; a reverse-proxy path is kept.
3. If another profile was active, copy the live vault into that profile's
   workspace file before saving the new profile, so switching back is lossless.
4. Store the returned token in OS secure storage **before** showing success.
   The token is shown once, with a copy button and the sentence "this is the
   only time it can be shown."
5. Save the profile with `lastVersion` and `lastSHA256` from the response and
   make it the active profile. The local vault is now this slot's file.

### Join (a new device adopts an existing slot)

1. User pastes server URL + token and the master password that vault uses.
   Send the token as pasted (the server accepts any display form); only trim
   surrounding whitespace. The URL gets the same normalization as Create.
2. `GET /v1/vault`:
   - `204` — the slot is empty. Offer to push the local vault (this device
     becomes the origin), or cancel.
   - `200` — continue.
   - `401` — "token not recognized"; stop, change nothing.
3. Verify `X-Maine-SHA256` against the bytes just fetched.
4. Try to decrypt the downloaded bytes with the entered master password and
   the client's vault parser:
   - **Success**: if another profile is active, push it (3 s timeout) and
     move the live vault to that profile's workspace file. Otherwise, if a
     local vault exists, move it to
     `passwords.pre-sync-<YYYYMMDD-HHMMSS>.maine` (0600). Then atomically
     place the downloaded file as `passwords.maine`. Save the profile with
     the response's version and make it active.
   - **Decryption failure** — wrong master password or not a vault this
     client can read. Change nothing; keep the downloaded bytes as
     `passwords.remote-<YYYYMMDD-HHMMSS>.maine` (0600) and explain.
   - **Payload `schemaVersion` newer than this client supports** — same
     treatment as failure: do not adopt a vault you cannot understand;
     tell the user to update the app.
5. On success, unlock with the password just verified, reload the host list,
   restore sessions and start polling. The user is not asked for the password
   a second time.

### Pull

1. `GET /v1/vault` (or `HEAD` while polling) with
   `If-None-Match: "v<lastVersion>"`.
2. `304` — done. `204` — empty slot; nothing to adopt (offer push only if the
   user just created the profile this way). `401` — mark the profile as
   "token rejected", stop polling, keep the local vault working. (macOS stops
   the 30 s loop; a later foreground or edit tries again and sees `401`.)
   `200`: continue.
3. Verify `X-Maine-SHA256`.
4. Decrypt-check with the session key (already available — the vault is
   unlocked). Failure means this is not the same vault; keep the bytes as
   `passwords.remote-…`, stop adopting, surface the problem.
5. If the local vault has unsaved/unpushed changes ("dirty" flag), save it
   as `passwords.conflict-<YYYYMMDD-HHMMSS>.maine` (0600) first. Otherwise
   replace silently.
6. Atomically replace `passwords.maine`, update `lastVersion`, reload the
   host list, and close sessions whose host UUID is gone from the adopted
   vault. Sessions whose hosts still exist keep running — never reparent or
   restart a terminal for a sync.
7. Session snapshots do not travel: they live in a device-local file next
   to the vault (`sessions-<vault id>.maine` on macOS, which also holds
   collapsed groups, agent access rules and the agent activity log), sealed
   with the vault's key and never pushed. Writing it must not mark the vault
   dirty or schedule a push. After adopting, drop local records whose host the
   adopted vault no longer has. A pulled vault written by an older client may
   still carry a `sessions` array in its payload: on the next unlock the
   macOS client copies those records into the local file (a local record with
   the same id wins) and rewrites the vault without the array. A new client
   never writes one.

### Push

1. If `lastVersion` is 0: `PUT` with `If-None-Match: *`.
   Otherwise: `PUT` with `If-Match: "v<lastVersion>"`.
2. `200` — update `lastVersion` from the response. If the local file changed
   again while the request was in flight, push once more.
3. `409` / `412` — run **Pull** (which saves a conflict copy if the local
   vault was dirty). Do not retry the push automatically: after adoption the
   local state equals the server's, and the user's version is preserved in
   the conflict copy. Surface a banner: "Another device changed the vault.
   Your version was saved as passwords.conflict-…".
4. `413` — "the vault is larger than this server accepts"; stop pushing
   until the vault file changes again or the operator raises `--max-blob`.
5. `401` as in Pull. `428` is a client bug: surface it as a failure and do
   not retry that write.
6. Optimisation worth doing: skip the push when the local file's SHA-256
   equals the hash recorded at `lastVersion` (no-op saves are common).

### Rotate the token

Only when the user asks (a leaked token, a device leaving the household):

1. `POST /v1/vault/rotate` with the old token.
2. Persist the new token to secure storage **before** showing success.
3. If the response is lost, the slot is unreachable — there is no recovery.
   Say so in the confirmation dialog before rotating.

### Disable sync / delete synced data

Two distinct actions:

- **Disable sync** — delete the profile and its token from this device. The
  slot and its history stay on the server; the local vault is untouched.
- **Delete synced data** — `DELETE /v1/vault`, then remove the profile. This
  wipes the slot and every stored version. Confirm explicitly.

## Error handling summary

| Condition | Client behavior |
| --------- | --------------- |
| Offline, DNS failure, timeout | Back off; status dot "offline"; retry on the schedule above |
| `401` | Stop syncing the profile; "token rejected" status; local vault unaffected |
| `403` on create | Ask for the create secret; it is a server setting, not a token |
| `409` / `412` | Pull, conflict copy, adopt, banner (see Push) |
| `413` | Stop pushing; tell the user the vault is too large for the server |
| `404` on a version fetch | The version was pruned; offer the next-oldest from the versions list (not implemented in the macOS app) |
| `5xx` | Treat like offline; back off and retry |
| Malformed JSON | Treat like `5xx`; never crash the app |

## Per-platform mapping

| Concern | macOS | Android |
| ------- | ----- | ------- |
| HTTP | `URLSession` | `OkHttp` |
| Secure token storage | Keychain | Keystore-backed prefs |
| Atomic vault replace | `Data.write(options: .atomic)` | temp file + `renameTo`, then fsync |
| Scheduling | 30 s async loop + `NSApplication.didBecomeActiveNotification` | coroutine + lifecycle observer |
| File permissions | `0600` (existing helper) | app-private storage (existing) |
| Conflict copies | Same directory as the vault, same permission helper | Same |

Conflict, pre-sync and workspace files all use the same naming scheme and
permissions as the vault. They are ordinary vault files: openable by pointing
the app at them manually, and never deleted by sync itself.

## Testing checklist

Every client should pass this list before sync is considered done. It maps
one-to-one onto the server's integration tests, which are the executable
specification:

1. Create → token shown once → second pull is `304`.
2. Edit locally → push → version increments; second device pulls the exact
   bytes.
3. Two writers from the same version → one `200`, one `409`; the loser keeps
   a conflict copy and adopts the winner's blob.
4. Join with the wrong master password → local vault untouched, remote bytes
   preserved, clear error.
5. Join into an empty slot → offers to push, works with `If-None-Match: *`.
6. Rotate → old token rejected, new token works, nothing else changes.
7. Delete → slot gone; local vault still opens.
8. Oversized vault → `413` surfaced, nothing stored, editing still works.
9. Token never appears in any log file or crash report.
10. Killing the app mid-push leaves the vault file intact (atomic replace).

For a two-device test without a second machine, `curl` is device B:
[`apps/sync-server/scripts/smoke.sh`](../apps/sync-server/scripts/smoke.sh)
does exactly that.

On macOS, `SyncEngineTests` (9 tests) covers create, the hash-skip, a push
conflict, a wrong master password, the empty slot, the pre-sync copy, the
conflict copy, a foreign vault, and rotate/delete. `SyncServerIntegrationTests`
(1 test) runs create, join, push, pull, conflict, rotate and delete against the
real server (`MAINE_SYNC_SERVER_BIN`). Items 9 and 10 are properties of the
implementation rather than separate tests: the sync code has no logging calls,
and every vault write goes through `Data.write(.atomic)` plus `0600`.

## Decisions the macOS client settled

These were open while the client was only a spec. The macOS client answers them
as follows; Android may choose differently, because the wire contract does not
care.

- **Conflict copies and banner.** `passwords.conflict-<YYYYMMDD-HHMMSS>.maine`
  next to the vault. The banner is the standard alert titled "Another device
  changed the vault", naming the copy; the status shows "Conflict saved as
  <name>".
- **Status dot and vocabulary.** A 6-point dot appears in Settings → Sync and in
  the sidebar footer while a profile is active. Labels: "Synced", "Syncing…",
  the offline message, "Token rejected", "Conflict saved as <name>", or the
  failure message. Green when synced, accent while syncing, orange when
  offline, red for conflicts, failures and rejected tokens.
- **Profile naming and switching.** Profiles are named on create and join, can
  be renamed from their Settings row, and switch when another row is clicked.
  Switching checkpoints and closes sessions, pushes the outgoing vault, swaps
  the files, then asks for the target vault's master password. The host editor
  draft lives in `AppState`, not in the view, so a switch does not move it into
  the outgoing vault; the editor's password field locks with the vault.
- **Metadata storage.** All of `lastVersion`, `lastSHA256` and `lastSyncedAt`
  live on the profile in preferences (`MaineSyncProfiles`, plus
  `MaineSyncActiveProfile` for the active id), not in a sidecar file.
- **Rotate wording.** The confirmation warns that the old token stops working
  immediately, and that if the response is lost the slot becomes unreachable
  with no recovery.
- **Token-once wording.** "Store it in a password manager now. The server keeps
  only its hash, so it cannot be shown again."

Still open: the version-history recovery flow for a pruned version; no client
has it yet.
