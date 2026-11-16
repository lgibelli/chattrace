#!/bin/bash
# Build the universal macOS app, sign it, notarize + staple it, and package
#   dist/ChatTrace-Forensic-<v>-macos-universal.dmg   (for people)
#   dist/ChatTrace-Forensic-<v>-macos-universal.zip   (for the in-app updater)
#
# Signing and notarization are used when these are set (env, or the git-ignored
# scripts/release.env; see scripts/release.env.example). Without them the app is
# ad-hoc signed, which is fine for local testing only.
#   MACOS_SIGNING_IDENTITY   "Developer ID Application: <name> (<team>)"
#   APPLE_API_KEY_PATH / APPLE_API_KEY_ID / APPLE_API_ISSUER   (App Store Connect API key for notarytool)
# SPDX-License-Identifier: GPL-3.0-or-later
set -euo pipefail
cd "$(dirname "$0")/.."
[ -f scripts/release.env ] && source scripts/release.env

VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
NAME="ChatTrace Forensic"
BASE="ChatTrace-Forensic-$VERSION-macos-universal"
export MACOSX_DEPLOYMENT_TARGET=11.0

for t in aarch64-apple-darwin x86_64-apple-darwin; do
  rustup target add "$t" >/dev/null 2>&1 || true
  cargo build --release --locked --target "$t" --bin chattrace --bin chattrace-cli
done

mkdir -p dist
APP="dist/$NAME.app"
rm -rf "$APP" "dist/$BASE.dmg" "dist/$BASE.zip"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
for b in chattrace chattrace-cli; do
  lipo -create -output "$APP/Contents/MacOS/$b" target/aarch64-apple-darwin/release/$b target/x86_64-apple-darwin/release/$b
done
cp assets/ChatTrace.icns "$APP/Contents/Resources/ChatTrace.icns"
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>$NAME</string>
  <key>CFBundleDisplayName</key><string>$NAME</string>
  <key>CFBundleIdentifier</key><string>it.salamacchine.chattrace</string>
  <key>CFBundleExecutable</key><string>chattrace</string>
  <key>CFBundleIconFile</key><string>ChatTrace</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>LSApplicationCategoryType</key><string>public.app-category.utilities</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSHumanReadableCopyright</key><string>© $(date +%Y) Luca Gibelli · Salamacchine. GPL-3.0-or-later. Not affiliated with Microsoft.</string>
  <key>NSRemovableVolumesUsageDescription</key><string>ChatTrace Forensic reads Teams caches on external disks you choose. It never writes to them.</string>
  <key>NSNetworkVolumesUsageDescription</key><string>ChatTrace Forensic reads Teams caches on network volumes you choose. It never writes to them.</string>
  <key>NSDesktopFolderUsageDescription</key><string>ChatTrace Forensic saves evidence packages where you choose, for example the Desktop.</string>
  <key>NSDocumentsFolderUsageDescription</key><string>ChatTrace Forensic saves evidence packages where you choose, for example Documents.</string>
  <key>NSDownloadsFolderUsageDescription</key><string>ChatTrace Forensic saves evidence packages where you choose, for example Downloads.</string>
</dict>
</plist>
PLIST

notarize() {  # $1 = file to submit
  xcrun notarytool submit "$1" --key "$APPLE_API_KEY_PATH" --key-id "$APPLE_API_KEY_ID" --issuer "$APPLE_API_ISSUER" --wait --timeout 30m
}

if [ -n "${MACOS_SIGNING_IDENTITY:-}" ]; then
  echo "signing with Developer ID"
  codesign --force --timestamp --options runtime --sign "$MACOS_SIGNING_IDENTITY" "$APP/Contents/MacOS/chattrace-cli"
  codesign --force --timestamp --options runtime --sign "$MACOS_SIGNING_IDENTITY" "$APP"
  codesign --verify --deep --strict --verbose=2 "$APP"
  if [ -n "${APPLE_API_KEY_PATH:-}" ]; then
    ditto -c -k --keepParent "$APP" "dist/notarize-app.zip"
    notarize "dist/notarize-app.zip"
    rm -f "dist/notarize-app.zip"
    xcrun stapler staple "$APP"
  fi
else
  echo "WARNING: MACOS_SIGNING_IDENTITY not set: ad-hoc signature (local testing only)"
  codesign --force --sign - "$APP/Contents/MacOS/chattrace-cli"
  codesign --force --sign - "$APP"
fi

# Update payload (the in-app updater installs this).
ditto -c -k --sequesterRsrc --keepParent "$APP" "dist/$BASE.zip"

# Disk image for people.
STAGE=$(mktemp -d)
cp -R "$APP" "$STAGE/"
ln -s /Applications "$STAGE/Applications"
cp README.md LICENSE "$STAGE/" 2>/dev/null || true
hdiutil create -volname "$NAME" -srcfolder "$STAGE" -ov -format UDZO -fs HFS+ "dist/$BASE.dmg" >/dev/null
rm -rf "$STAGE"
if [ -n "${MACOS_SIGNING_IDENTITY:-}" ]; then
  codesign --force --timestamp --sign "$MACOS_SIGNING_IDENTITY" "dist/$BASE.dmg"
  if [ -n "${APPLE_API_KEY_PATH:-}" ]; then
    notarize "dist/$BASE.dmg"
    xcrun stapler staple "dist/$BASE.dmg"
    spctl --assess --type open --context context:primary-signature --verbose=2 "dist/$BASE.dmg" || true
    spctl --assess --type execute --verbose=2 "$APP" || true
  fi
fi
ls -la dist
