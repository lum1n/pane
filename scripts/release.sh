#!/usr/bin/env bash
# Production build: build/Pane-<version>.dmg with Pane.app signed with
# Developer ID, notarized and stapled (the DMG and the app in build/).
#
# Needs, once:
# - the "Developer ID Application: ... (DR7HQ24F77)" certificate in the keychain
# - a notarytool keychain profile (app-specific password from account.apple.com):
#     xcrun notarytool store-credentials pane-notary \
#       --apple-id <Apple ID> --team-id DR7HQ24F77
#   NOTARY_PROFILE=<name> uses another one (lapp-notary works too).
set -euo pipefail
cd "$(dirname "$0")/.."

TEAM=DR7HQ24F77
PROFILE="${NOTARY_PROFILE:-pane-notary}"
APP=build/Pane.app

if [[ "$(uname)" != "Darwin" ]]; then
    echo "release.sh needs macOS." >&2
    exit 1
fi

IDENTITY="$(security find-identity -v -p codesigning |
    sed -n "s/.*\"\(Developer ID Application: .*($TEAM)\)\".*/\1/p" | head -n 1)"
if [[ -z "$IDENTITY" ]]; then
    echo "No \"Developer ID Application: ... ($TEAM)\" identity in the keychain." >&2
    exit 1
fi
# Fail now rather than after the build if the profile is missing.
xcrun notarytool history --keychain-profile "$PROFILE" >/dev/null || {
    echo "notarytool profile $PROFILE missing or unusable; make it with:" >&2
    echo "  xcrun notarytool store-credentials $PROFILE --apple-id <Apple ID> --team-id $TEAM" >&2
    exit 1
}

CODESIGN_IDENTITY="$IDENTITY" scripts/build-app.sh

VERSION="$(/usr/libexec/PlistBuddy -c 'Print CFBundleShortVersionString' "$APP/Contents/Info.plist")"
DMG="build/Pane-$VERSION.dmg"

echo "==> DMG"
STAGING="$(mktemp -d)"
trap 'rm -rf "$STAGING"' EXIT
cp -R "$APP" "$STAGING/"
ln -s /Applications "$STAGING/Applications"
rm -f "$DMG"
hdiutil create -volname Pane -srcfolder "$STAGING" -ov -format UDZO "$DMG"
codesign --sign "$IDENTITY" --timestamp "$DMG"

echo "==> Notarize ($PROFILE)"
# One submission covers the DMG and the app inside it.
RESULT="$(xcrun notarytool submit "$DMG" --keychain-profile "$PROFILE" --wait --output-format json)"
echo "$RESULT"
# A rejected submission doesn't always make notarytool exit non-zero.
STATUS="$(plutil -extract status raw -o - - <<<"$RESULT")"
if [[ "$STATUS" != Accepted ]]; then
    ID="$(plutil -extract id raw -o - - <<<"$RESULT")"
    xcrun notarytool log "$ID" --keychain-profile "$PROFILE" >&2 || true
    echo "Notarization $STATUS." >&2
    exit 1
fi

# stapler asks Apple's servers, which now and then drop the connection.
retry() {
    for attempt in 1 2 3 4 5; do
        "$@" && return 0
        echo "$* failed (attempt $attempt), retrying" >&2
        sleep 15
    done
    return 1
}
retry xcrun stapler staple "$DMG"
retry xcrun stapler staple "$APP"
retry xcrun stapler validate "$DMG"
spctl --assess --type open --context context:primary-signature --verbose "$DMG"
spctl --assess --type execute --verbose "$APP"

(cd build && shasum -a 256 "Pane-$VERSION.dmg" > "Pane-$VERSION.dmg.sha256")
echo "==> Release ready: $DMG"
