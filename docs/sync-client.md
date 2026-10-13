# Sync client implementation guide

The server and its wire contract ([`sync-protocol.md`](sync-protocol.md)) are
done. No client implements sync yet. This file is the checklist for the client
phase: what to store, when to talk to the server, exactly how to resolve
conflicts, and how "workspaces" (several tokens on one device) should behave.

Read the protocol document first; this one assumes it.

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
   an unlocked vault with a session key available.
6. **Diagnostics stay metadata-only.** Extend the existing rule: server URL
   and slot version may be logged; token, blob contents, conflict-copy
   contents may not.

## What a client stores

A **profile** is the client-side record of one sync pairing:

| Field | Secret | Storage |
| ----- | ------ | ------- |
| `id` | no | Preferences; a client-generated UUID |
| `name` | no | Preferences; user-editable label ("Homelab") |
| `serverUrl` | no | Preferences |
| `token` | **yes** | OS secure storage, keyed by profile `id` |
| `lastVersion` | no | Preferences; the `ETag`/version the local vault came from (0 = never synced) |
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
| Windows | DPAPI-protected file next to the existing vault key file |
| Android | Keystore-encrypted preferences (the vault key itself stays memory-only; the token must persist across launches, so it needs Keystore-backed storage) |

> macOS note: the current invariant says the Keychain holds only the session
> helper key. Storing the sync token there is a deliberate, documented
> extension of that invariant, and should be made explicitly when the client
> work starts — not smuggled in.

## Workspaces: several tokens on one device

The server has nothing to add here: every token is already an independent
slot, and nothing on the server knows about a client's profile list. The
design decision is entirely client-side.

**Recommended model: a profile list with exactly one active profile.**

- Settings shows all profiles; the user can pair as many as they want.
- Only the active profile's vault is the live `passwords.maine`. This keeps
  the "one active vault file" invariant intact and keeps every existing
  feature (hosts, sessions, keychain entry, helper protocol) untouched.
- Inactive workspaces keep their last synced file as
  `passwords.workspace-<profile-id>.maine` (0600) next to the active vault.
  Nothing about them is loaded into memory.
- **Switching workspaces**: flush and push the current vault, close all open
  sessions (their hosts belong to the vault being left), move the active file
  to its workspace file, move the target workspace file into
  `passwords.maine`, prompt for that vault's master password, then resume
  normal startup (restore sessions, start syncing the new profile).
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
| Join flow completes | Pull (or push, for an empty slot) |
| Every vault save (host edit, password change, session checkpoint) | Push, debounced ~2 s |
| Vault lock or quit, after the final session checkpoint | Best-effort push with a short timeout |
| App foreground | Pull |
| While unlocked | Poll every 30 s (`HEAD`; `304` almost always) |
| Before switching workspaces | Flush + push |
| Network/server failure | Back off 30 s → 2 min → 5 min, cap; never block the UI |

One pull or push at a time per profile; a trigger while one is in flight sets
a "dirty" flag rather than starting a second request.

## Algorithms

### Create (this device becomes the origin)

1. User provides server URL (and an optional name) and asks to enable sync.
2. `POST /v1/vaults` with the current `passwords.maine` as the body (create
   secret if the server requires one).
3. Store the returned token in OS secure storage **before** showing success.
   The token is shown once, with a copy button and the sentence "this is the
   only time it can be shown."
4. Save the profile with `lastVersion` from the response.

### Join (a new device adopts an existing slot)

1. User pastes server URL + token. Normalize nothing by hand — the server
   accepts any display form; just send it.
2. `GET /v1/vault`:
   - `204` — the slot is empty. Offer to push the local vault (this device
     becomes the origin), or cancel.
   - `200` — continue.
   - `401` — "token not recognized"; stop, change nothing.
3. If the slot has a blob: show what is about to happen ("This token points
   to a vault last updated <date>, <size>") and ask for the master password
   that vault uses.
4. Verify `X-Maine-SHA256` against the bytes fetched in step 2.
5. Try to decrypt the downloaded bytes with the entered master password and
   the client's vault parser:
   - **Success** — if a local vault exists, move it to
     `passwords.pre-sync-<YYYYMMDD-HHMMSS>.maine` (0600); then atomically
     place the downloaded file as `passwords.maine`. Save the profile with
     the response's version.
   - **Decryption failure** — wrong master password or not a vault this
     client can read. Change nothing; keep the downloaded bytes as
     `passwords.remote-<YYYYMMDD-HHMMSS>.maine` (0600) and explain.
   - **Payload `schemaVersion` newer than this client supports** — same
     treatment as failure: do not adopt a vault you cannot understand;
     tell the user to update the app.
6. On success, continue normal startup: unlock, restore sessions, start
   polling.

### Pull

1. `GET /v1/vault` (or `HEAD` while polling) with
   `If-None-Match: "v<lastVersion>"`.
2. `304` — done. `204` — empty slot; nothing to adopt (offer push only if the
   user just created the profile this way). `401` — mark the profile as
   "token rejected", stop syncing it, keep the local vault working. `200` —
   continue.
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
7. Note the accepted consequence: session snapshots travel with the vault,
   so sessions saved on another device are restored and reconnected here,
   exactly as a local restore would do. History is fed to the renderer as
   scrollback and is never replayed as commands (existing behavior).

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
   until the file shrinks or the operator raises `--max-blob`.
5. `401` — as in Pull. `428` — a client bug; log it and stop pushing.
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
| `404` on a version fetch | The version was pruned; offer the next-oldest from the versions list |
| `5xx` | Treat like offline; back off and retry |
| Malformed JSON | Treat like `5xx`; never crash the app |

## Per-platform mapping

| Concern | macOS | Windows | Android |
| ------- | ----- | ------- | ------- |
| HTTP | `URLSession` | `HttpClient` | `OkHttp` |
| Secure token storage | Keychain | DPAPI file | Keystore-backed prefs |
| Atomic vault replace | `Data.write(options: .atomic)` | write temp + `File.Replace` | temp file + `renameTo`, then fsync |
| Scheduling | `Timer` + `scenePhase` | `DispatcherTimer` + window events | coroutine + lifecycle observer |
| File permissions | `0600` (existing helper) | user-only ACL (existing pattern) | app-private storage (existing) |
| Conflict copies | Same directory as the vault, same permission helper | Same | Same |

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

## Open decisions for the client phase

Small, deliberate choices that are better made with UI in front of you:

- Conflict-copy naming and where the "a conflict was saved" banner lives.
- Whether the sidebar shows a per-profile sync status dot, and its vocabulary
  ("synced", "syncing", "offline", "token rejected", "conflict").
- Profile naming UI and the exact switching flow (including what happens to
  unsaved editor state when switching).
- Whether `lastSyncedAt`/version metadata lives in preferences or a small
  sidecar JSON; preferences are fine for v1.
- The wording of the token-loss warning in the rotate dialog.
