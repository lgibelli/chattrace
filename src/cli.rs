// SPDX-License-Identifier: GPL-3.0-or-later
//! Command line, shared by `chattrace` (GUI by default) and `chattrace-cli`.

use crate::discover::{self, Found, human_bytes};
use crate::evidence;
use crate::gui;
use crate::idb;
use crate::teams;
use crate::{APP_NAME, VERSION};
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::AtomicBool;

const ABOUT: &str = "ChatTrace Forensic: read-only viewer and evidence exporter for the local cache of the \
Microsoft Teams desktop client.\n\nUse Microsoft Purview (eDiscovery / Content search) whenever you have access to it: it is \
the authoritative, complete record. ChatTrace is for when you don't: it preserves what one Teams client cached on one \
computer, without modifying it.";

const AFTER: &str = "Examples:\n  \
chattrace-cli discover                       Teams caches on this computer\n  \
chattrace-cli discover /Volumes/EvidenceDisk Teams caches on an external disk or mounted image\n  \
chattrace-cli summary\n  \
chattrace-cli export --case 2026-041 --examiner \"J. Doe\" -o evidence.zip\n  \
chattrace-cli export --source /Volumes/EvidenceDisk/Users/jdoe -o jdoe.zip\n  \
chattrace-cli verify evidence.zip\n\n\
Not affiliated with Microsoft. Free software under the GNU GPL v3 or later; see `chattrace-cli license`.";

#[derive(Parser)]
#[command(name = "chattrace-cli", version, about = ABOUT, after_help = AFTER)]
struct Cli {
    #[command(subcommand)]
    cmd: Option<Cmd>,
    /// Don't check GitHub for a newer version at startup (GUI). Also: CHATTRACE_NO_UPDATE_CHECK=1.
    #[arg(long, global = true)]
    no_update_check: bool,
}

#[derive(Subcommand)]
enum Cmd {
    /// Open the viewer window, optionally on a path (any folder, disk or copy).
    View {
        /// Folder to open: a Teams store, a profile or home folder, a whole disk, …
        path: Option<PathBuf>,
        /// Show a synthetic demo dataset (no real data), e.g. for screenshots or training.
        #[arg(long, hide = true)]
        demo: bool,
    },
    /// List Teams caches on this computer, or anywhere under PATH (external disk, mounted image, copied folder).
    Discover {
        /// Folder to search. Omit to list the Teams caches of this computer's user.
        path: Option<PathBuf>,
        /// Print JSON instead of a table.
        #[arg(long)]
        json: bool,
    },
    /// Summary of the accounts, conversations and messages in a Teams cache.
    Summary {
        /// Teams store, or any folder containing exactly one (default: this computer's Teams).
        #[arg(long, short)]
        source: Option<PathBuf>,
    },
    /// Create an evidence package (zip): original files, SHA-256, CSV/HTML/JSON/text, manifest.
    Export {
        /// Teams store, or any folder containing exactly one (default: this computer's Teams).
        #[arg(long, short)]
        source: Option<PathBuf>,
        /// Zip to create (default: ./chattrace-<UTC timestamp>.zip). Never overwritten.
        #[arg(long, short)]
        out: Option<PathBuf>,
        /// Case or matter reference, recorded in the manifest and the report.
        #[arg(long)]
        case: Option<String>,
        /// Person performing the capture, recorded in the manifest and the report.
        #[arg(long)]
        examiner: Option<String>,
        /// Free-text notes (reason, scope, …) recorded in the manifest and the report.
        #[arg(long)]
        notes: Option<String>,
        /// Leave out all_records.jsonl (every record of every IndexedDB store).
        #[arg(long)]
        chats_only: bool,
        /// Also keep the unzipped package next to the zip (to browse the HTML report).
        #[arg(long, alias = "keep-staging")]
        keep_unzipped: bool,
    },
    /// Verify an evidence zip: its .sha256 file and every file against SHA256SUMS.
    Verify {
        /// The evidence zip.
        zip: PathBuf,
    },
    /// Print the license and third-party notices.
    License,
    /// Check for a newer version and install it (signed releases from GitHub).
    Update {
        /// Only report whether an update is available.
        #[arg(long)]
        check: bool,
    },
    /// Show crash reports saved on this computer; send them only with --send.
    CrashReport {
        /// Send the reports shown (to the developer's Sentry), then delete them.
        #[arg(long, conflicts_with = "discard")]
        send: bool,
        /// Delete the reports without sending them.
        #[arg(long)]
        discard: bool,
    },
    /// Testing: crash on purpose to check crash reporting.
    #[command(hide = true)]
    CrashTest,
    /// Advanced: list IndexedDB databases and object stores with record counts.
    Stores {
        #[arg(long, short)]
        source: Option<PathBuf>,
    },
    /// Advanced: print sample records of one store (strings truncated).
    Sample {
        #[arg(long, short)]
        source: Option<PathBuf>,
        /// Substring of the database name.
        #[arg(long)]
        db: String,
        /// Object store name.
        #[arg(long)]
        store: String,
        /// Number of records.
        #[arg(short, default_value_t = 1)]
        n: usize,
        /// Truncate strings to this many characters.
        #[arg(long, default_value_t = 40)]
        trunc: usize,
        /// Only records whose JSON contains this substring.
        #[arg(long)]
        contains: Option<String>,
        /// Include superseded/deleted versions.
        #[arg(long)]
        all: bool,
    },
    /// Advanced: count the JSON field paths in a store.
    Fields {
        #[arg(long, short)]
        source: Option<PathBuf>,
        #[arg(long)]
        db: String,
        #[arg(long)]
        store: String,
        /// Print example values for paths containing this substring (truncated).
        #[arg(long)]
        show: Option<String>,
    },
    /// Write the HTML report of the synthetic demo dataset (for documentation).
    #[command(hide = true)]
    DemoReport {
        #[arg(long, short)]
        out: PathBuf,
    },
}

pub fn main(gui_by_default: bool) -> ExitCode {
    // Finder used to pass -psn_… to apps; ignore it.
    let args: Vec<String> = std::env::args().filter(|a| !a.starts_with("-psn_")).collect();
    let cli = match Cli::try_parse_from(&args) {
        Ok(c) => c,
        Err(e) => {
            let _ = e.print();
            return if e.use_stderr() { ExitCode::from(2) } else { ExitCode::SUCCESS };
        }
    };
    let cmd = match cli.cmd {
        Some(c) => c,
        None if gui_by_default => Cmd::View { path: None, demo: false },
        None => {
            use clap::CommandFactory;
            let _ = Cli::command().print_help();
            return ExitCode::SUCCESS;
        }
    };
    crate::crash::install(if matches!(cmd, Cmd::View { .. }) { "gui" } else { "cli" });
    match run(cmd) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e:#}");
            ExitCode::FAILURE
        }
    }
}

/// Resolve `--source` (any path) or pick this computer's Teams store.
fn source_or_local(source: Option<PathBuf>) -> Result<Found> {
    if let Some(p) = source {
        return discover::resolve(&p);
    }
    let mut v = discover::local();
    if v.is_empty() && !discover::local_blocked().is_empty() {
        anyhow::bail!("Microsoft Teams is installed but its data cannot be read: {}", discover::access_help(false));
    }
    if v.is_empty() {
        anyhow::bail!(
            "no Teams cache found for this computer's user; pass --source <path> (any folder, disk or copy), \
             or run `chattrace-cli discover <path>`"
        );
    }
    if v.len() > 1 {
        eprintln!("note: {} Teams stores on this computer, using the first; choose with --source:", v.len());
        for f in &v {
            eprintln!("  {}  ({})", f.leveldb.display(), f.kind.label());
        }
    }
    Ok(v.remove(0))
}

fn print_found(v: &[Found]) {
    if v.is_empty() {
        println!("No Teams cache found.");
        return;
    }
    for (i, f) in v.iter().enumerate() {
        let newest = f
            .newest
            .map(|t| chrono::DateTime::<chrono::Local>::from(t).format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_default();
        println!("{:>2}. {}", i + 1, f.kind.label());
        println!("    {}", f.leveldb.display());
        let mut info = vec![format!("{} in {} files", human_bytes(f.bytes), f.files)];
        if !newest.is_empty() {
            info.push(format!("last modified {newest}"));
        }
        if let Some(l) = f.layout {
            info.push(format!("{l} layout"));
        }
        if let Some(u) = &f.os_user {
            info.push(format!("user folder \"{u}\""));
        }
        if f.live_local {
            info.push("this computer's live Teams profile".into());
        }
        println!("    {}", info.join(" · "));
    }
}

fn run(cmd: Cmd) -> Result<ExitCode> {
    match cmd {
        Cmd::View { path, demo } => {
            let start = if demo {
                gui::Start::Demo
            } else if let Some(p) = path {
                gui::Start::Path(p)
            } else {
                gui::Start::Welcome
            };
            gui::run(start)?;
        }
        Cmd::Discover { path, json } => {
            let v = match &path {
                Some(p) => {
                    if !p.exists() {
                        anyhow::bail!("{} does not exist", p.display());
                    }
                    let mut last = 0;
                    let v = discover::discover(p, &AtomicBool::new(false), &mut |n| {
                        if !json && n >= last + 5000 {
                            last = n;
                            eprint!("\r  searched {n} folders…");
                        }
                    });
                    if !json && last > 0 {
                        eprintln!();
                    }
                    v
                }
                None => {
                    let v = discover::local();
                    let blocked = discover::local_blocked();
                    if v.is_empty() && !blocked.is_empty() && !json {
                        for b in &blocked {
                            eprintln!("Microsoft Teams is installed ({}) but its data cannot be read.", b.display());
                        }
                        eprintln!("{}", discover::access_help(false));
                        return Ok(ExitCode::FAILURE);
                    }
                    v
                }
            };
            if json {
                println!("{}", serde_json::to_string_pretty(&v)?);
            } else {
                print_found(&v);
            }
        }
        Cmd::Export { source, out, case, examiner, notes, chats_only, keep_unzipped } => {
            let found = source_or_local(source)?;
            let out = out.unwrap_or_else(|| {
                PathBuf::from(format!("chattrace-{}.zip", chrono::Utc::now().format("%Y%m%dT%H%M%SZ")))
            });
            if found.live_local && evidence::teams_running() {
                eprintln!(
                    "WARNING: Microsoft Teams is running. Quit it completely (macOS: Teams > Quit; Windows: tray icon > Quit) \
                     for a consistent snapshot."
                );
            }
            eprintln!("{APP_NAME} {VERSION}");
            eprintln!("source: {}", found.leveldb.display());
            let opts = evidence::ExportOptions {
                source: found.leveldb.clone(),
                output: out,
                case_reference: case,
                examiner,
                notes,
                all_stores: !chats_only,
                keep_staging: keep_unzipped,
            };
            let last_stage = std::cell::RefCell::new((String::new(), 0u64));
            let r = evidence::export(&opts, &|p| {
                let mut l = last_stage.borrow_mut();
                if l.0 != p.stage {
                    if l.1 > 0 {
                        eprintln!();
                    }
                    eprint!("  {}", p.stage);
                    *l = (p.stage.to_string(), 0);
                }
                if p.total > 0 {
                    let pct = p.done * 100 / p.total.max(1);
                    if pct >= l.1 + 10 {
                        eprint!(" {pct}%");
                        l.1 = pct;
                    }
                }
                let _ = std::io::stderr().flush();
            })?;
            eprintln!();
            for w in &r.warnings {
                eprintln!("WARNING: {w}");
            }
            println!("wrote {} ({} files, {} messages)", r.zip.display(), r.files, r.messages);
            println!("SHA-256 {}", r.zip_sha256);
            if keep_unzipped {
                println!("unzipped copy: {}", opts.output.with_extension("staging").display());
            }
        }
        Cmd::Verify { zip } => {
            let r = evidence::verify_zip(&zip, &|_| {})?;
            println!("{}", zip.display());
            println!("  SHA-256 {}", r.zip_sha256);
            match r.sidecar_ok {
                Some(true) => println!("  matches {}", r.sidecar.as_deref().unwrap_or("")),
                Some(false) => println!("  DOES NOT MATCH {}", r.sidecar.as_deref().unwrap_or("")),
                None => println!("  (no .sha256 file next to the zip)"),
            }
            if let Some(m) = &r.manifest_summary {
                println!("  {m}");
            }
            println!("  {} files checked against SHA256SUMS", r.checked);
            for m in &r.mismatched {
                println!("  MODIFIED  {m}");
            }
            for m in &r.missing {
                println!("  MISSING   {m}");
            }
            for m in &r.unlisted {
                println!("  UNLISTED  {m}");
            }
            if r.ok() {
                println!("OK: the package is intact.");
            } else {
                println!("FAILED: the package does not match its checksums.");
                return Ok(ExitCode::from(3));
            }
        }
        Cmd::Update { check } => {
            eprintln!("checking github.com/lgibelli/chattrace/releases …");
            match crate::update::check()? {
                None => println!("{APP_NAME} {VERSION} is up to date."),
                Some(rel) => {
                    println!("version {} is available (you have {VERSION}): {}", rel.version, rel.page);
                    if !check {
                        let target = crate::update::install(&rel, &|m| eprintln!("  {m}"))?;
                        println!("installed: {}", target.display());
                    }
                }
            }
        }
        Cmd::CrashReport { send, discard } => {
            if !crate::crash::available() {
                println!("This build of {APP_NAME} has no crash reporting (only official releases do).");
                return Ok(ExitCode::SUCCESS);
            }
            let reports = crate::crash::pending();
            if reports.is_empty() {
                println!("No crash reports are saved on this computer.");
                return Ok(ExitCode::SUCCESS);
            }
            if discard {
                crate::crash::discard(&reports);
                println!("Deleted {} crash report(s); nothing was sent.", reports.len());
                return Ok(ExitCode::SUCCESS);
            }
            for r in &reports {
                println!("{}\n", r.preview());
            }
            if send {
                for r in &reports {
                    crate::crash::send(r)?;
                }
                println!("Sent {} crash report(s). Thank you.", reports.len());
            } else {
                println!(
                    "{} crash report(s) above are saved on this computer and have NOT been sent.\n\
                     This is everything a report contains. To send them: chattrace-cli crash-report --send\n\
                     To delete them: chattrace-cli crash-report --discard",
                    reports.len()
                );
            }
        }
        Cmd::CrashTest => {
            // The quoted text and the path must not appear in the saved report.
            panic!("crash test: \"Ciao Mario\" in /Users/jdoe/Case-Rossi");
        }
        Cmd::License => {
            println!("{}\nAuthor: {} ({})", crate::gui::LICENSE_NOTICE, crate::AUTHOR, crate::WEBSITE);
            println!("\nFull license text: {}/blob/main/LICENSE", crate::REPOSITORY);
            println!("Third-party notices: {}/blob/main/THIRD_PARTY_NOTICES.md", crate::REPOSITORY);
        }
        Cmd::Summary { source } => {
            let found = source_or_local(source)?;
            let t = std::time::Instant::now();
            let ds = teams::load(&found.leveldb, found.blob.as_deref())?;
            println!("source: {}  ({})", found.leveldb.display(), found.kind.label());
            println!("parsed in {:.1}s", t.elapsed().as_secs_f32());
            for a in &ds.accounts {
                let convs = ds.conversations.iter().filter(|c| c.account == a.key).count();
                let msgs: Vec<_> = ds.messages.iter().filter(|m| m.account == a.key).collect();
                let latest: Vec<_> = msgs.iter().filter(|m| m.variant == teams::Variant::Latest).collect();
                let deleted = latest.iter().filter(|m| m.deleted.is_some()).count();
                let edited = latest.iter().filter(|m| m.edited.is_some()).count();
                let first = msgs.iter().filter_map(|m| m.arrival).reduce(f64::min);
                let last = msgs.iter().filter_map(|m| m.arrival).reduce(f64::max);
                println!(
                    "account {} [{}]: {convs} conversations, {} messages ({} earlier versions, {edited} edited, {deleted} deleted), {} .. {}",
                    a.label(),
                    a.locales.join(","),
                    latest.len(),
                    msgs.len() - latest.len(),
                    teams::utc(first),
                    teams::utc(last),
                );
            }
            let mut diff: BTreeMap<String, usize> = BTreeMap::new();
            for e in ds.messages.iter().filter(|m| m.variant == teams::Variant::Earlier) {
                *diff
                    .entry(format!("{} {}", if e.substantive { "*" } else { " " }, e.changes.join("+")))
                    .or_default() += 1;
            }
            if !diff.is_empty() {
                println!("earlier versions by difference from the latest (* = visible to a reader):");
                for (k, n) in diff {
                    println!("  {n:5} {k}");
                }
            }
            println!("profiles: {}  decode errors: {}", ds.profiles.len(), ds.decode_errors.len());
            let crc: usize = ds.catalog.files.iter().map(|f| f.crc_errors).sum();
            println!("LevelDB files: {}  CRC errors: {crc}", ds.catalog.files.len());
        }
        Cmd::Stores { source } => {
            let found = source_or_local(source)?;
            let cat = idb::catalog(&found.leveldb)?;
            let mut counts: BTreeMap<(u64, u64), [usize; 4]> = BTreeMap::new();
            let mut errors = Vec::new();
            idb::scan_records(&found.leveldb, found.blob.as_deref(), &cat, &|_, _| true, &mut |r| {
                let c = counts.entry((r.db, r.store)).or_default();
                c[r.status as usize] += 1;
                if let Err(e) = &r.value {
                    c[3] += 1;
                    errors.push(format!("db={} store={} key={}: {e}", r.db, r.store, r.key));
                }
            })?;
            for d in cat.databases.values() {
                println!("db {} {:?} origin={}", d.id, d.name, d.origin);
                for (sid, sn) in &d.stores {
                    let c = counts.get(&(d.id, *sid)).copied().unwrap_or_default();
                    println!(
                        "    store {sid} {sn:?}: live={} superseded={} deleted={} decode_errors={}",
                        c[0], c[1], c[2], c[3]
                    );
                }
            }
            let crc: usize = cat.files.iter().map(|f| f.crc_errors).sum();
            let with_err: Vec<_> = cat.files.iter().filter(|f| !f.errors.is_empty()).collect();
            println!("files={} crc_errors={crc} files_with_errors={}", cat.files.len(), with_err.len());
            for f in with_err {
                println!("  {}: {:?}", f.name, &f.errors[..f.errors.len().min(3)]);
            }
            for e in errors.iter().take(5) {
                println!("decode error {e}");
            }
        }
        Cmd::Sample { source, db, store, n, trunc, contains, all } => {
            let found = source_or_local(source)?;
            let mut shown = 0;
            scan_store(&found, &db, &store, &mut |r| {
                if shown >= n || (!all && r.status != idb::Status::Live) {
                    return;
                }
                let j = match &r.value {
                    Ok(v) => v.to_json(),
                    Err(e) => serde_json::Value::String(format!("ERR {e}")),
                };
                if let Some(c) = &contains
                    && !j.to_string().contains(c.as_str())
                {
                    return;
                }
                shown += 1;
                println!(
                    "key={} seq={} status={}\n{}",
                    r.key,
                    r.seq,
                    r.status.as_str(),
                    serde_json::to_string_pretty(&truncate_json(&j, trunc)).unwrap_or_default()
                );
            })?;
        }
        Cmd::Fields { source, db, store, show } => {
            let found = source_or_local(source)?;
            let mut out = FieldStats::new();
            scan_store(&found, &db, &store, &mut |r| {
                if let Ok(v) = &r.value {
                    walk(&v.to_json(), String::new(), 0, &mut out);
                }
            })?;
            for (p, (n, ex)) in &out {
                match &show {
                    Some(f) if p.contains(f.as_str()) => println!("{n:7} {p}  {ex:?}"),
                    _ => println!("{n:7} {p}"),
                }
            }
        }
        Cmd::DemoReport { out } => {
            let ds = crate::demo::dataset();
            let capture = crate::demo::capture();
            crate::report::write(&ds, &capture, &out).with_context(|| format!("writing {}", out.display()))?;
            println!("wrote {}", out.join("index.html").display());
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn truncate_json(v: &serde_json::Value, n: usize) -> serde_json::Value {
    use serde_json::Value as J;
    match v {
        J::String(s) if s.chars().count() > n => J::String(s.chars().take(n).collect::<String>() + "…"),
        J::Array(a) => J::Array(a.iter().take(3).map(|x| truncate_json(x, n)).collect()),
        J::Object(m) => J::Object(m.iter().map(|(k, x)| (k.clone(), truncate_json(x, n))).collect()),
        other => other.clone(),
    }
}

type FieldStats = BTreeMap<String, (usize, BTreeSet<String>)>;

fn walk(v: &serde_json::Value, path: String, depth: usize, out: &mut FieldStats) {
    use serde_json::Value as J;
    let e = out.entry(path.clone()).or_default();
    e.0 += 1;
    match v {
        J::Object(m) if depth < 6 => {
            let collapse = path.ends_with("messageMap")
                || (m.len() > 30 && m.keys().all(|k| k.contains(':') || k.chars().all(|c| c.is_ascii_digit())));
            for (k, x) in m {
                let seg = if collapse { "*".to_string() } else { k.clone() };
                walk(x, format!("{path}.{seg}"), depth + 1, out);
            }
        }
        J::Array(a) if depth < 6 => {
            for x in a.iter().take(50) {
                walk(x, format!("{path}[]"), depth + 1, out);
            }
        }
        J::String(s) => {
            if e.1.len() < 6 {
                e.1.insert(s.chars().take(30).collect());
            }
        }
        J::Null => {}
        other => {
            if e.1.len() < 6 {
                e.1.insert(other.to_string());
            }
        }
    }
}

fn scan_store(found: &Found, db: &str, store: &str, sink: &mut dyn FnMut(idb::Record)) -> Result<()> {
    let cat = idb::catalog(&found.leveldb)?;
    idb::scan_records(&found.leveldb, found.blob.as_deref(), &cat, &|d, s| d.name.contains(db) && s == store, sink)
}
