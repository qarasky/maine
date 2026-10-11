#!/bin/sh
# Vendors xterm.js + the fit addon next to terminal.html so the terminal
# works offline. Re-run to upgrade:
#   sh scripts/fetch-xterm.sh [version]
# Requires: npm (or edit the URLs below to download the files by hand).
set -eu
VERSION="${1:-5.5.0}"
HERE="$(cd "$(dirname "$0")/.." && pwd)"
LIB="$HERE/src/Maine.App/Terminal/lib"
mkdir -p "$LIB"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
cd "$TMP"
npm pack "@xterm/xterm@$VERSION" "@xterm/addon-fit@$VERSION" >/dev/null
tar -xzf xterm-*.tgz
tar -xzf xterm-addon-fit-*.tgz
cp package/lib/xterm.js package/lib/xterm.css "$LIB/"
cp package/lib/addon-fit.js "$LIB/xterm-addon-fit.js"
echo "Vendored xterm $VERSION into src/Maine.App/Terminal/lib/"
