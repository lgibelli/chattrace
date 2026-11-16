# Releasing (maintainers)

Releases are built by [GitHub Actions](../.github/workflows/release.yml) on GitHub's runners.

1. Update `version` in `Cargo.toml` (then `cargo build` to update `Cargo.lock`) and add an entry
   to `CHANGELOG.md`. Commit and push to `main`.
2. Push a branch named `release/<version>`, e.g. `git push origin HEAD:release/0.2.0`, or run the
   *Release* workflow manually.
3. The workflow creates a **draft** release `v<version>` and attaches:
   - `ChatTrace-Forensic-<v>-macos-universal.dmg` (Developer ID signed, notarized, stapled) and
     `…-macos-universal.zip` (the same app, for the in-app updater);
   - `ChatTrace-Forensic-<v>-windows-x64.zip` and `ChatTrace-Forensic-<v>-linux-x86_64.zip`;
   - `SHA256SUMS` and `SHA256SUMS.minisig`.
4. Check the draft, download and try a build, then **publish** it. Only published releases are
   seen by the in-app updater and the website.

## Secrets

The workflow signs only when these exist. Set them with `scripts/set-ci-secrets.sh`, which reads
`scripts/release.env` (git-ignored; template: `scripts/release.env.example`):

| Name | Kind | Use |
|---|---|---|
| `MACOS_CERT_P12_BASE64`, `MACOS_CERT_PASSWORD` | secret | Developer ID Application certificate (leaf-only .p12) |
| `APPLE_API_KEY_B64`, `APPLE_API_KEY_ID`, `APPLE_API_ISSUER` | secret | App Store Connect API key for `notarytool` |
| `MINISIGN_KEY` | secret | minisign secret key that signs `SHA256SUMS`; its public key is `assets/updater.pub` and is embedded in the app (`src/update.rs`) |
| `MACOS_SIGNING_IDENTITY` | variable | `Developer ID Application: <name> (<team>)` |

Without `MINISIGN_KEY` the release still builds, but the in-app updater refuses it. Keep the
minisign secret key backed up offline: losing it means existing installs cannot verify future
updates.

## Local builds

`scripts/release.sh` builds the same files locally into `dist/`: the macOS app via
`scripts/build-macos.sh` (signed and notarized if `scripts/release.env` is filled in), and
Windows via `cargo xwin`. It uploads nothing. Icons are regenerated with `scripts/make_icons.sh`,
and the third-party notices with `python3 scripts/gen_notices.py`.
