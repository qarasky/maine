#!/bin/bash
# Packages build/Maine.app into a distributable disk image.
#
# Usage:
#   scripts/build-app.sh --release --version 1.2.3
#   scripts/make-dmg.sh 1.2.3
#
# Produces build/Maine-<version>.dmg and build/Maine-<version>.dmg.sha256.
# The checksum file is plain `shasum -a 256` output generated from inside
# build/, so it verifies with `(cd build && shasum -a 256 -c <file>)`.

set -euo pipefail

cd "$(dirname "$0")/.."

app_name="Maine"
app_bundle="${app_name}.app"
build_root="build"

usage() {
    echo "Usage: scripts/make-dmg.sh <version>" >&2
    echo "Example: scripts/make-dmg.sh 1.2.3" >&2
    exit 2
}

[ "$#" -eq 1 ] || usage

version="$1"
if ! printf '%s' "$version" | grep -Eq '^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$'; then
    echo "error: version must look like 1.2.3 or 1.2.3-rc.1, got '$version'" >&2
    exit 2
fi

if [ ! -d "${build_root}/${app_bundle}" ]; then
    echo "error: ${build_root}/${app_bundle} is missing; run scripts/build-app.sh --release --version ${version} first" >&2
    exit 1
fi

dmg_name="${app_name}-${version}.dmg"
dmg_path="${build_root}/${dmg_name}"
checksum_path="${build_root}/${dmg_name}.sha256"

staging="$(mktemp -d "${TMPDIR:-/tmp}/maine-dmg.XXXXXX")"
trap 'rm -rf "$staging"' EXIT

echo "==> Staging ${app_bundle}"
ditto "${build_root}/${app_bundle}" "${staging}/${app_bundle}"
ln -s /Applications "${staging}/Applications"

echo "==> Creating ${dmg_path}"
hdiutil create -volname "${app_name}" -srcfolder "$staging" -ov -format UDZO "$dmg_path" >/dev/null

echo "==> Writing ${checksum_path}"
(cd "$build_root" && shasum -a 256 "$dmg_name" > "${dmg_name}.sha256")

echo
echo "Created ${dmg_path}"
echo "Created ${checksum_path}"
