#!/usr/bin/env bash
# Build build/Pane.app: Rust (release, arm64) + Swift shell, bundled and signed.
#
#   CODESIGN_IDENTITY  signing identity (default "Pane Dev", see make-dev-cert.sh;
#                      "-" for ad-hoc, which makes macOS re-ask for Accessibility
#                      after every rebuild)
set -euo pipefail
cd "$(dirname "$0")/.."

TARGET=aarch64-apple-darwin
IDENTITY="${CODESIGN_IDENTITY:-Pane Dev}"
APP=build/Pane.app

if [[ "$(uname)" != "Darwin" ]]; then
    echo "build-app.sh needs macOS (the Swift shell uses AppKit)." >&2
    exit 1
fi

echo "==> Rust ($TARGET, release)"
cargo build --release --target "$TARGET" -p pane-ffi -p pane-cli -p pane-agent
cp crates/pane-ffi/include/pane.h shell/Sources/CPane/include/pane.h

echo "==> Swift shell"
swift build -c release --package-path shell --arch arm64
BIN="$(swift build -c release --package-path shell --arch arm64 --show-bin-path)"

echo "==> Bundle"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Helpers" "$APP/Contents/Resources"
cp shell/Resources/Info.plist "$APP/Contents/Info.plist"
cp "$BIN/Pane" "$APP/Contents/MacOS/Pane"
# Helpers live outside MacOS/: "pane" and "Pane" collide on case-insensitive APFS.
cp "target/$TARGET/release/pane" "$APP/Contents/Helpers/pane"
cp "target/$TARGET/release/pane-agentd" "$APP/Contents/Helpers/pane-agentd"
if [[ -f shell/Resources/AppIcon.icns ]]; then
    cp shell/Resources/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"
fi

echo "==> Sign ($IDENTITY)"
if [[ "$IDENTITY" != "-" ]]; then
    # Resolve the name to the SHA-1 of a *valid* identity: stale or untrusted
    # certs with the same name make `codesign --sign <name>` fail as ambiguous.
    HASH="$(security find-identity -v -p codesigning \
        | awk -v name="\"$IDENTITY\"" 'index($0, name) { print $2; exit }')"
    if [[ -n "$HASH" ]]; then
        IDENTITY="$HASH"
    else
        echo "warning: identity \"$IDENTITY\" not found; signing ad-hoc." >&2
        echo "         Run scripts/make-dev-cert.sh once so Accessibility trust survives rebuilds." >&2
        IDENTITY="-"
    fi
fi
for helper in "$APP/Contents/Helpers/"*; do
    codesign --force --options runtime --timestamp=none --sign "$IDENTITY" "$helper"
done
codesign --force --options runtime --timestamp=none --sign "$IDENTITY" --identifier com.vegard.pane "$APP"
codesign --verify --strict --verbose=1 "$APP"
echo "==> $APP"
