#!/bin/bash
# Ships Maine to the user's Applications folder so it behaves like an installed app.
#
# Usage: scripts/install.sh [--launch]
set -euo pipefail

cd "$(dirname "$0")/.."

launch=0
for argument in "$@"; do
    case "$argument" in
        --launch) launch=1 ;;
        *) echo "Unknown option: $argument" >&2; exit 2 ;;
    esac
done

scripts/build-app.sh --release

destination="/Applications/${PWD##*/}.app"
if [ -d "/Applications/Maine.app" ]; then
    rm -rf "/Applications/Maine.app"
fi
cp -R "build/Maine.app" "/Applications/Maine.app"

echo "Installed /Applications/Maine.app"

if [ "$launch" = "1" ]; then
    open "/Applications/Maine.app"
fi
