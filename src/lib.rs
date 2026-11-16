// SPDX-License-Identifier: GPL-3.0-or-later
//! ChatTrace Forensic: read-only viewer and evidence exporter for the local
//! cache of the Microsoft Teams desktop client.
//!
//! Layering (each module only uses the ones above it):
//!   leveldb   raw LevelDB reader (.log + .ldb), every record version, CRC checked
//!   v8        V8/Blink structured-clone deserializer
//!   idb       Chromium IndexedDB on top of LevelDB (+ external blob files)
//!   teams     Teams data model: accounts, conversations, messages, versions
//!   discover  find Teams stores on this computer or under any folder / disk
//!   report    self-contained HTML report
//!   evidence  evidence package: copy + SHA-256 + CSV/HTML/JSON + manifest + zip
//!   cli, gui  front ends
//!
//! ChatTrace Forensic is not affiliated with Microsoft.

pub mod cli;
pub mod crash;
pub mod demo;
pub mod discover;
pub mod evidence;
pub mod gui;
pub mod idb;
pub mod leveldb;
pub mod report;
pub mod teams;
pub mod update;
pub mod v8;

pub const APP_NAME: &str = "ChatTrace Forensic";
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
pub const AUTHOR: &str = "Luca Gibelli · Salamacchine";
pub const WEBSITE: &str = "https://salamacchine.it/apps/chattrace/";
