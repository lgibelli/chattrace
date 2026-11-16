#!/bin/bash
# Set the GitHub Actions secrets/variables used by .github/workflows/release.yml.
# Run it yourself (it handles your signing credentials):  scripts/set-ci-secrets.sh
# Reads paths and ids from scripts/release.env. Needs: gh (logged in), openssl.
# SPDX-License-Identifier: GPL-3.0-or-later
set -euo pipefail
cd "$(dirname "$0")/.."
source scripts/release.env
REPO=${REPO:-lgibelli/chattrace}
tmp=$(mktemp -d); trap 'rm -rf "$tmp"' EXIT

# Leaf-only .p12 with legacy algorithms (what the macOS keychain imports reliably).
if openssl x509 -inform DER -in "$MACOS_CERT_CER" -out "$tmp/cert.pem" 2>/dev/null; then :; else cp "$MACOS_CERT_CER" "$tmp/cert.pem"; fi
PW=$(openssl rand -hex 24)
openssl pkcs12 -export -inkey "$MACOS_CERT_KEY" -in "$tmp/cert.pem" -out "$tmp/cert.p12" \
  -keypbe PBE-SHA1-3DES -certpbe PBE-SHA1-3DES -macalg sha1 -passout "pass:$PW"

base64 < "$tmp/cert.p12" | tr -d '\n' | gh secret set MACOS_CERT_P12_BASE64 -R "$REPO"
printf '%s' "$PW" | gh secret set MACOS_CERT_PASSWORD -R "$REPO"
base64 < "$APPLE_API_KEY_PATH" | tr -d '\n' | gh secret set APPLE_API_KEY_B64 -R "$REPO"
printf '%s' "$APPLE_API_KEY_ID" | gh secret set APPLE_API_KEY_ID -R "$REPO"
printf '%s' "$APPLE_API_ISSUER" | gh secret set APPLE_API_ISSUER -R "$REPO"
gh secret set MINISIGN_KEY -R "$REPO" < "$MINISIGN_KEY_PATH"
gh variable set MACOS_SIGNING_IDENTITY -R "$REPO" --body "$MACOS_SIGNING_IDENTITY"
echo "done: secrets and variables set on $REPO"
