# Changelog

## 0.1.0 (unreleased)

First public version.

- Read-only viewer for the new Microsoft Teams desktop cache (macOS, Windows, Linux), with its own
  LevelDB / IndexedDB / V8 parser; recovers earlier versions of edited and deleted messages.
- Works on this computer or on any folder or disk: external drives, mounted images, copies.
- Evidence package: byte-for-byte copy, SHA-256 of every file, manifest, CSV, HTML report with all
  metadata, JSON, transcripts; verified zip and `.sha256` file.
- `verify` for evidence packages; `discover` to find Teams caches anywhere.
- Signed, notarized macOS app; Windows and Linux builds; signed in-app updates.
- Crash reports, sent only after the user has seen and approved each one (official builds only).
