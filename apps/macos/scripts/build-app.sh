#!/bin/bash
# Builds Maine.app from the Swift package.
#
# The package produces an executable; this script wraps it in the bundle
# structure macOS expects, so `open Maine.app` works the same as launching any
# other application.
#
# Usage:
#   scripts/build-app.sh [--release|--debug] [--run]
#
# Requires the Command Line Tools or a full Xcode installation:
#   xcode-select --install   # or: sudo xcode-select /Applications/Xcode.app

set -euo pipefail

cd "$(dirname "$0")/.."

configuration="release"
run=0

for argument in "$@"; do
    case "$argument" in
        --debug)   configuration="debug" ;;
        --release) configuration="release" ;;
        --run)     run=1 ;;
        *) echo "Unknown option: $argument" >&2; exit 2 ;;
    esac
done

app_name="Maine"
app_bundle="${app_name}.app"
build_root="build"
package_build_root=".build"
bundle_path="${build_root}/${app_bundle}"
executable_name="Maine"

echo "==> Compiling ($configuration)"
if [ "$configuration" = "release" ]; then
    swift build -c release
    product="${package_build_root}/release/${executable_name}"
else
    swift build
    product="${package_build_root}/debug/${executable_name}"
fi

if [ ! -x "$product" ]; then
    echo "error: expected the built executable at $product" >&2
    exit 1
fi

echo "==> Assembling ${bundle_path}"
rm -rf "$bundle_path"
mkdir -p "${bundle_path}/Contents/MacOS"
mkdir -p "${bundle_path}/Contents/Resources"

cp "$product" "${bundle_path}/Contents/MacOS/${executable_name}"
cp "Resources/Info.plist" "${bundle_path}/Contents/Info.plist"
cp "Resources/AppIcon.icns" "${bundle_path}/Contents/Resources/AppIcon.icns"

# Ad-hoc signature: no developer identity is needed to run a locally built app,
# and signing keeps the bundle structurally valid on newer macOS releases.
if command -v codesign >/dev/null 2>&1; then
    codesign --force --sign - "$bundle_path" >/dev/null
fi

echo
echo "Built ${bundle_path}"
echo "Run it with:  open ${bundle_path}"
echo "   or from a terminal:  ${bundle_path}/Contents/MacOS/${executable_name}"

if [ "$run" = "1" ]; then
    echo
    echo "==> Launching"
    open "$bundle_path"
fi
