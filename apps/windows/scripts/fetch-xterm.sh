#!/bin/sh
# Vendors xterm.js + the fit addon next to terminal.html so the terminal
# works offline. The two packages version independently: pass both versions
# to upgrade:
#   sh scripts/fetch-xterm.sh [xterm-version] [addon-fit-version]
# Requires: npm (or edit the URLs below to download the files by hand).
set -eu
XTERM_VERSION="${1:-5.5.0}"
FIT_VERSION="${2:-0.11.0}"
HERE="$(cd "$(dirname "$0")/.." && pwd)"
LIB="$HERE/src/Maine.App/Terminal/lib"
mkdir -p "$LIB"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
cd "$TMP"
npm pack "@xterm/xterm@$XTERM_VERSION" "@xterm/addon-fit@$FIT_VERSION" >/dev/null
# Both archives unpack into package/; the second merges beside the first.
tar -xzf "xterm-xterm-$XTERM_VERSION.tgz"
tar -xzf "xterm-addon-fit-$FIT_VERSION.tgz"
cp package/lib/xterm.js package/css/xterm.css "$LIB/"
cp package/lib/addon-fit.js "$LIB/xterm-addon-fit.js"
echo "Vendored xterm $XTERM_VERSION + addon-fit $FIT_VERSION into src/Maine.App/Terminal/lib/"
