#!/usr/bin/env bash
# Builds the release binary and installs it for the current user:
#   ~/.local/bin/maine, a .desktop entry and the icon. No root needed.
# Usage: scripts/install.sh [--prefix DIR]   (default prefix: ~/.local)
set -euo pipefail

prefix="${HOME}/.local"
if [[ "${1:-}" == "--prefix" ]]; then prefix="${2:?--prefix needs a directory}"; fi
here="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

if ! pkg-config --exists Qt6Quick; then
    echo "Qt 6 development files are missing. On Ubuntu/Kubuntu:" >&2
    echo "  sudo apt install qt6-base-dev qt6-declarative-dev qml6-module-qtquick qml6-module-qtquick-controls qml6-module-qtquick-window qml6-module-qtquick-layouts libgl-dev pkg-config" >&2
    exit 1
fi

(cd "$here" && cargo build --release -p maine-app)

install -Dm755 "$here/target/release/maine" "$prefix/bin/maine"
install -Dm644 "$here/packaging/maine.svg" "$prefix/share/icons/hicolor/scalable/apps/maine.svg"
install -Dm644 "$here/packaging/maine.desktop" "$prefix/share/applications/maine.desktop"
command -v update-desktop-database >/dev/null && update-desktop-database "$prefix/share/applications" || true
command -v gtk-update-icon-cache >/dev/null && gtk-update-icon-cache -q -t "$prefix/share/icons/hicolor" || true
echo "Installed to $prefix. Run: maine   (make sure $prefix/bin is on PATH)"
