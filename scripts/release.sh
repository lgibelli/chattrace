#!/bin/bash
# Local release build (the CI workflow does the same on GitHub's runners):
#   macOS universal DMG + updater zip (signed/notarized if configured),
#   Windows x64 zip (cargo-xwin), SHA256SUMS and its minisign signature.
# Nothing is uploaded. SPDX-License-Identifier: GPL-3.0-or-later
set -euo pipefail
cd "$(dirname "$0")/.."
[ -f scripts/release.env ] && source scripts/release.env
VERSION=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)

scripts/build-macos.sh

cargo xwin build --release --locked --target x86_64-pc-windows-msvc --bin chattrace --bin chattrace-cli
ROOT=$PWD
W=$(mktemp -d)
cp target/x86_64-pc-windows-msvc/release/chattrace.exe target/x86_64-pc-windows-msvc/release/chattrace-cli.exe "$W/"
cp README.md LICENSE THIRD_PARTY_NOTICES.md THIRD_PARTY_LICENSES.txt "$W/"
# Executables at the top level of the zip (the in-app updater expects that).
rm -f "dist/ChatTrace-Forensic-$VERSION-windows-x64.zip"
(cd "$W" && zip -qr -X "$ROOT/dist/ChatTrace-Forensic-$VERSION-windows-x64.zip" .)
rm -rf "$W"

cd dist
rm -f SHA256SUMS SHA256SUMS.minisig
shasum -a 256 ChatTrace-Forensic-"$VERSION"-* > SHA256SUMS
if [ -n "${MINISIGN_KEY_PATH:-}" ] && [ -f "$MINISIGN_KEY_PATH" ]; then
  minisign -S -s "$MINISIGN_KEY_PATH" -m SHA256SUMS -x SHA256SUMS.minisig -t "ChatTrace Forensic $VERSION"
fi
cat SHA256SUMS
ls -la
