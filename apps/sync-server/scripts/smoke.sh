#!/usr/bin/env bash
# End-to-end smoke test for the sync server: create, push, pull, conflict,
# rotate, delete — the whole protocol in one run. Needs curl and python3.
#
#   scripts/smoke.sh
#
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="${BIN:-$ROOT/target/debug/maine-sync-server}"

if [ ! -x "$BIN" ]; then
    echo "building $BIN ..."
    (cd "$ROOT" && cargo build --quiet)
fi

DATA_DIR="$(mktemp -d)"
PORT="$(python3 - <<'PY'
import socket
s = socket.socket()
s.bind(("127.0.0.1", 0))
print(s.getsockname()[1])
s.close()
PY
)"
BASE="http://127.0.0.1:$PORT"

"$BIN" --bind "127.0.0.1:$PORT" --data-dir "$DATA_DIR" --max-blob 1MiB \
    >"$DATA_DIR/server.log" 2>&1 &
SERVER_PID=$!
cleanup() {
    kill "$SERVER_PID" 2>/dev/null || true
    rm -rf "$DATA_DIR"
}
trap cleanup EXIT

for _ in $(seq 1 50); do
    if curl -fsS "$BASE/v1/health" >/dev/null 2>&1; then
        break
    fi
    sleep 0.1
done

json_field() {
    python3 -c 'import json,sys;print(json.load(sys.stdin)[sys.argv[1]])' "$1"
}
pass() {
    printf 'ok   %s\n' "$1"
}
status_of() {
    curl -sS -o /dev/null -w '%{http_code}' "$@"
}

# 1. Create a slot with the first version.
head -c 4096 /dev/urandom >"$DATA_DIR/vault.bin"
TOKEN="$(curl -fsS -X POST "$BASE/v1/vaults" \
    --data-binary @"$DATA_DIR/vault.bin" | json_field token)"
pass "created a slot holding 4096 opaque bytes"

# 2. Pull it back, byte for byte.
curl -fsS -H "Authorization: Bearer $TOKEN" "$BASE/v1/vault" -o "$DATA_DIR/pulled.bin"
cmp "$DATA_DIR/vault.bin" "$DATA_DIR/pulled.bin"
pass "pulled bytes match the pushed bytes"

# 3. A conditional GET is a 304.
[ "$(status_of -H "Authorization: Bearer $TOKEN" -H 'If-None-Match: "v1"' "$BASE/v1/vault")" = "304" ]
pass "conditional GET returned 304"

# 4. A stale write is refused with 409.
[ "$(status_of -X PUT -H "Authorization: Bearer $TOKEN" -H 'If-Match: "v0"' \
    --data-binary @"$DATA_DIR/vault.bin" "$BASE/v1/vault")" = "409" ]
pass "stale write returned 409"

# 5. A matching write advances the version.
head -c 4096 /dev/urandom >"$DATA_DIR/vault2.bin"
[ "$(status_of -X PUT -H "Authorization: Bearer $TOKEN" -H 'If-Match: "v1"' \
    --data-binary @"$DATA_DIR/vault2.bin" "$BASE/v1/vault")" = "200" ]
pass "matching write stored version 2"

# 6. Rotation kills the old token immediately.
NEW_TOKEN="$(curl -fsS -X POST -H "Authorization: Bearer $TOKEN" \
    "$BASE/v1/vault/rotate" | json_field token)"
[ "$(status_of -H "Authorization: Bearer $TOKEN" "$BASE/v1/vault")" = "401" ]
pass "the old token is rejected after rotation"

# 7. Delete wipes the slot.
[ "$(status_of -X DELETE -H "Authorization: Bearer $NEW_TOKEN" "$BASE/v1/vault")" = "204" ]
pass "slot deleted"

echo "smoke test passed"
