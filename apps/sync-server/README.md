# Maine sync server

A small, self-hosted, zero-knowledge sync server for the Maine SSH client. It
stores one opaque blob per slot — the encrypted `passwords.maine` vault, byte
for byte — and never sees a hostname, a credential, a master password or a
token. The macOS client implements the protocol (Maine → Settings → Sync);
Windows and Android are not there yet.

- One token = one slot. The token is both the address and the credential,
  like a wallet seed: whoever holds it can read and replace the slot's bytes.
- The server stores only `SHA-256(token)`, so a stolen database does not leak
  access.
- Whole-file sync with optimistic concurrency: `If-Match` on every write,
  `409` on a stale one. Clients keep a conflict copy; the server never merges.
- The last 32 versions of each slot are kept for recovery and pruned on write.

The wire contract is [`docs/sync-protocol.md`](../../docs/sync-protocol.md);
the client-side checklist is [`docs/sync-client.md`](../../docs/sync-client.md).

## Quick start

```sh
cargo run --release
# listening on 127.0.0.1:8484, database in ./data/sync.sqlite3
```

The first device creates a slot (the token is returned exactly once):

```sh
curl -sS -X POST http://127.0.0.1:8484/v1/vaults \
  --data-binary @passwords.maine
# {"token":"maine-2F7K-…","version":1,"etag":"\"v1\"","size":1234,"sha256":"…"}
```

Every other device pulls with the token:

```sh
curl -sS -H "Authorization: Bearer maine-2F7K-…" \
  http://127.0.0.1:8484/v1/vault -o passwords.maine
```

Then unlock the vault with the master password it already had. There is no
account to create, no email, no dashboard.

## Configuration

| Flag | Environment | Default | Meaning |
| ---- | ----------- | ------- | ------- |
| `--bind` | `MAINE_SYNC_BIND` | `127.0.0.1:8484` | Listen address. Loopback by default, so exposing the server is a decision, not an accident. |
| `--data-dir` | `MAINE_SYNC_DATA_DIR` | `./data` | Directory for `sync.sqlite3`. |
| `--keep-versions` | `MAINE_SYNC_KEEP_VERSIONS` | `32` | Stored versions per slot; older ones are pruned on write. |
| `--max-blob` | `MAINE_SYNC_MAX_BLOB` | `64MiB` | Largest accepted vault. Accepts `B`, `KiB`, `MiB`, `GiB`. |
| `--create-secret` | `MAINE_SYNC_CREATE_SECRET` | unset | When set, creating slots requires the `X-Maine-Create-Secret` header. Existing slots are unaffected. |

`RUST_LOG` controls logging (`info` by default). Request lines are logged at
`debug`: method, path, status, duration — never headers, bodies or tokens.

## Running it for real

Put it behind a reverse proxy for TLS; the binary speaks plain HTTP on
purpose. On a tailnet or LAN, plain HTTP is a reasonable choice — the blobs
are already encrypted — but on the open internet, use HTTPS.

Caddy, in one file:

```
sync.example.com {
    reverse_proxy 127.0.0.1:8484
}
```

systemd:

```ini
[Unit]
Description=Maine sync server
After=network.target

[Service]
ExecStart=/usr/local/bin/maine-sync-server --bind 127.0.0.1:8484 --data-dir /var/lib/maine-sync
User=maine
EnvironmentFile=/etc/maine-sync.env   # MAINE_SYNC_CREATE_SECRET=…
Restart=on-failure

[Install]
WantedBy=multi-user.target
```

Docker:

```sh
docker build -t maine-sync-server .
docker run -d -p 127.0.0.1:8484:8484 -v maine-sync:/data maine-sync-server
```

## What the server can and cannot see

| | |
| --- | --- |
| Stored | Encrypted vault bytes, `SHA-256` of each token, byte sizes, version numbers, timestamps |
| Never stored | Tokens, master passwords, plaintext, hostnames, session contents |
| Never logged | Tokens, request bodies, authorization headers |
| Visible to an operator | Which slots exist, when they change and how big they are |

The operator can read the encrypted blobs out of SQLite but cannot decrypt
them without a master password. Losing a token means losing the slot — there
is no reset and no recovery service. The operator can still rescue bytes for
a user who lost a token by reading the database directly:

```sh
sqlite3 data/sync.sqlite3 \
  "SELECT version, length(blob) FROM versions ORDER BY version DESC"
```

## Development

```sh
cargo test          # unit + HTTP integration tests against a real server
scripts/smoke.sh    # the protocol end to end with curl
cargo clippy --all-targets
cargo fmt
```

The integration suite starts a real server on an ephemeral loopback port and
exercises create, conditional pull, conflict, rotation, deletion, retention,
oversized bodies, the create secret and a two-writer race.
