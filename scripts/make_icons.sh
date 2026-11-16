#!/bin/bash
# Regenerate every icon file from the SVG sources (macOS: needs iconutil).
set -euo pipefail
cd "$(dirname "$0")/.."
python3 -I scripts/make_logo.py
tmp=$(mktemp -d)
cargo run --quiet --example make_icons -- "$tmp/ChatTrace.iconset"
iconutil -c icns -o assets/ChatTrace.icns "$tmp/ChatTrace.iconset"
rm -rf "$tmp"
ls -la assets
