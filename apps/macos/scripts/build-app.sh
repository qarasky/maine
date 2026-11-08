#!/bin/bash
# Builds Maine.app from the Swift package.
#
# The package produces an executable; this script wraps it in the bundle
# structure macOS expects, so `open Maine.app` works the same as launching any
# other application.
#
# Usage:
#   scripts/build-app.sh [--release|--debug] [--version X.Y.Z] [--build N] [--run]
#
# --version is written to CFBundleShortVersionString in the bundle's copy of
# Info.plist (default 0.0.0-dev, which the app treats as a development build).
# --build is written to CFBundleVersion and defaults to the commit count.
#
# Requires the Command Line Tools or a full Xcode installation:
#   xcode-select --install   # or: sudo xcode-select /Applications/Xcode.app

set -euo pipefail

cd "$(dirname "$0")/.."

configuration="release"
run=0
version="0.0.0-dev"
build_number=""

while [ $# -gt 0 ]; do
    case "$1" in
        --debug)   configuration="debug"; shift ;;
        --release) configuration="release"; shift ;;
        --run)     run=1; shift ;;
        --version)
            if [ $# -lt 2 ]; then
                echo "error: --version needs a value" >&2
                exit 2
            fi
            version="$2"
            shift 2
            ;;
        --build)
            if [ $# -lt 2 ]; then
                echo "error: --build needs a value" >&2
                exit 2
            fi
            build_number="$2"
            shift 2
            ;;
        *) echo "Unknown option: $1" >&2; exit 2 ;;
    esac
done

if ! [[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]]; then
    echo "error: invalid version '$version' (expected X.Y.Z or X.Y.Z-prerelease)" >&2
    exit 2
fi

if [ -z "$build_number" ]; then
    build_number=$(git rev-list --count HEAD 2>/dev/null || true)
    build_number="${build_number:-0}"
fi

if ! [[ "$build_number" =~ ^[0-9]+$ ]]; then
    echo "error: invalid build number '$build_number' (expected digits)" >&2
    exit 2
fi

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

echo "==> Stamping version ${version} (build ${build_number})"
/usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString ${version}" "${bundle_path}/Contents/Info.plist"
/usr/libexec/PlistBuddy -c "Set :CFBundleVersion ${build_number}" "${bundle_path}/Contents/Info.plist"

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
