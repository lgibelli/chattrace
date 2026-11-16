// SPDX-License-Identifier: GPL-3.0-or-later
//! Evidence package: byte-for-byte copy of the Teams IndexedDB, SHA-256 of
//! every file, readable dumps (CSV, HTML, JSONL, text) derived *from the copy*,
//! a manifest, and a verified zip.

use crate::discover;
use crate::idb;
use crate::report;
use crate::teams::{self, Dataset, Message, Variant, local, utc};
use anyhow::{Context, Result, bail};
use chrono::{DateTime, Local, Utc};
use serde::Serialize;
use serde_json::{Value as Json, json};
use sha2::{Digest, Sha256};
use std::fs::File;
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

pub const APP_NAME: &str = "ChatTrace Forensic";

pub struct ExportOptions {
    /// The `…indexeddb.leveldb` directory (anywhere: local profile, external disk, copy).
    pub source: PathBuf,
    /// Zip file to create (must not exist).
    pub output: PathBuf,
    pub case_reference: Option<String>,
    pub examiner: Option<String>,
    pub notes: Option<String>,
    /// Also decode every other IndexedDB store into all_records.jsonl.
    pub all_stores: bool,
    pub keep_staging: bool,
}

pub struct ExportResult {
    pub zip: PathBuf,
    pub zip_sha256: String,
    pub files: usize,
    pub messages: usize,
    pub warnings: Vec<String>,
}

/// Progress callback payload.
pub struct Progress<'a> {
    pub stage: &'a str,
    pub done: u64,
    pub total: u64,
    pub detail: &'a str,
}

#[derive(Serialize, Clone)]
struct FileEntry {
    path: String,
    size: u64,
    sha256: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    modified_utc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    created_utc: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    source_path: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    changed_during_copy: bool,
}

fn sys_utc(t: std::io::Result<SystemTime>) -> Option<String> {
    t.ok().map(|t| DateTime::<Utc>::from(t).to_rfc3339_opts(chrono::SecondsFormat::Micros, true))
}

fn hash_reader(mut r: impl Read, mut also: impl FnMut(&[u8]) -> Result<()>) -> Result<(String, u64)> {
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut n = 0u64;
    loop {
        let k = r.read(&mut buf)?;
        if k == 0 {
            break;
        }
        h.update(&buf[..k]);
        also(&buf[..k])?;
        n += k as u64;
    }
    Ok((hex::encode(h.finalize()), n))
}

pub fn sha256_file(p: &Path) -> Result<(String, u64)> {
    hash_reader(File::open(p)?, |_| Ok(()))
}

/// Copy `src` to `dst`, reading the source read-only and hashing what was read.
fn copy_hashed(src: &Path, dst: &Path, on_bytes: &mut dyn FnMut(u64)) -> Result<(String, u64)> {
    let input = File::open(src).with_context(|| format!("opening {}", src.display()))?;
    let mut out = File::create(dst).with_context(|| format!("creating {}", dst.display()))?;
    let r = hash_reader(input, |b| {
        out.write_all(b)?;
        on_bytes(b.len() as u64);
        Ok(())
    })?;
    out.flush()?;
    if let Ok(m) = std::fs::metadata(src).and_then(|m| m.modified()) {
        let _ = out.set_modified(m);
    }
    Ok(r)
}

fn run(cmd: &str, args: &[&str]) -> Option<String> {
    let mut c = std::process::Command::new(cmd);
    c.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let o = c.output().ok()?;
    let s = String::from_utf8_lossy(&o.stdout).trim().to_string();
    (o.status.success() && !s.is_empty()).then_some(s)
}

/// Is the Teams client running on this computer?
pub fn teams_running() -> bool {
    if cfg!(windows) {
        run("tasklist", &["/FI", "IMAGENAME eq ms-teams.exe", "/NH"]).is_some_and(|s| s.contains("ms-teams.exe"))
    } else {
        run("pgrep", &["-x", "MSTeams"]).is_some()
    }
}

pub fn host_info() -> Json {
    let host = std::env::var("COMPUTERNAME").ok().or_else(|| run("hostname", &[]));
    let user = std::env::var("USER").or_else(|_| std::env::var("USERNAME")).ok();
    let (os_version, teams_version) = if cfg!(target_os = "macos") {
        (
            run("sw_vers", &["-productVersion"]),
            run(
                "plutil",
                &[
                    "-extract",
                    "CFBundleShortVersionString",
                    "raw",
                    "-o",
                    "-",
                    "/Applications/Microsoft Teams.app/Contents/Info.plist",
                ],
            ),
        )
    } else if cfg!(windows) {
        (
            run("cmd", &["/C", "ver"]),
            run("powershell", &["-NoProfile", "-Command", "(Get-AppxPackage -Name MSTeams).Version"]),
        )
    } else {
        (None, None)
    };
    json!({
        "hostname": host,
        "os": std::env::consts::OS,
        "os_version": os_version,
        "os_user": user,
        "teams_app_version": teams_version,
    })
}

fn safe_name(s: &str, max: usize) -> String {
    let out: String = s.chars().map(|c| if c.is_alphanumeric() || " -_.,()".contains(c) { c } else { '_' }).collect();
    let out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    let out: String = out.chars().take(max).collect();
    let out = out.trim_matches(|c: char| c == '.' || c == ' ').to_string();
    if out.is_empty() { "untitled".into() } else { out }
}

fn short_hash(s: &str) -> String {
    hex::encode(&Sha256::digest(s.as_bytes())[..4])
}

/// Characters that make spreadsheet programs treat a cell as a formula (including
/// the full-width forms some locales accept).
const FORMULA_START: &[char] = &['=', '+', '-', '@', '\t', '\r', '\u{FF1D}', '\u{FF0B}', '\u{FF0D}', '\u{FF20}'];

/// Chat content is written by other people: a message such as `=HYPERLINK(…)` must
/// not become a live formula when the CSV is opened in Excel, LibreOffice or Numbers.
/// Such cells get a leading `'`; the exact values are in the .jsonl files.
pub fn csv_cell(v: &str) -> std::borrow::Cow<'_, str> {
    if v.starts_with(FORMULA_START) { format!("'{v}").into() } else { v.into() }
}

/// CSV writer that neutralizes formula-like cells (see [`csv_cell`]).
struct CsvOut(csv::Writer<BufWriter<File>>);

impl CsvOut {
    fn write_record<I, T>(&mut self, record: I) -> csv::Result<()>
    where
        I: IntoIterator<Item = T>,
        T: AsRef<str>,
    {
        self.0.write_record(record.into_iter().map(|v| csv_cell(v.as_ref()).into_owned()))
    }
    fn flush(&mut self) -> std::io::Result<()> {
        self.0.flush()
    }
}

fn csv_writer(p: &Path) -> Result<CsvOut> {
    let mut f = BufWriter::new(File::create(p)?);
    f.write_all(b"\xEF\xBB\xBF")?; // UTF-8 BOM so Excel detects the encoding
    Ok(CsvOut(csv::Writer::from_writer(f)))
}

fn reactions_str(m: &Message) -> String {
    m.reactions.iter().map(|r| format!("{}: {} ({})", r.emoji, r.name, utc(r.time))).collect::<Vec<_>>().join("; ")
}

fn files_str(m: &Message) -> String {
    m.files.iter().map(|f| format!("{} <{}>", f.name, f.url)).collect::<Vec<_>>().join("; ")
}

// ------------------------------------------------------------ readable files

fn write_messages_csv(ds: &Dataset, p: &Path) -> Result<()> {
    let mut w = csv_writer(p)?;
    w.write_record([
        "account",
        "conversation_id",
        "conversation_title",
        "message_id",
        "version",
        "variant",
        "versions_known",
        "changes_vs_latest",
        "substantive_change",
        "arrival_utc",
        "arrival_local",
        "client_arrival_utc",
        "compose_time",
        "sender_mri",
        "sender_name",
        "sent_by_account_owner",
        "message_type",
        "content_type",
        "text",
        "content_raw",
        "edited_utc",
        "deleted_utc",
        "deleted_by",
        "reactions",
        "mentions",
        "files",
        "importance",
        "subject",
        "reply_chain_id",
        "client_message_id",
        "sequence_id",
        "properties_json",
        "source_database",
        "source_store",
        "source_file",
        "leveldb_seq",
        "record_status",
        "crc_ok",
        "occurrences",
        "version_fingerprint",
    ])?;
    for m in &ds.messages {
        let title = ds.conversation(&m.account, &m.conversation_id).map(|c| c.title.as_str()).unwrap_or("");
        w.write_record([
            m.account.as_str(),
            &m.conversation_id,
            title,
            &m.id,
            &m.version,
            if m.variant == Variant::Latest { "latest" } else { "earlier" },
            &m.versions_known.to_string(),
            &m.changes.join(" "),
            if m.substantive { "yes" } else { "" },
            &utc(m.arrival),
            &local(m.arrival),
            &utc(m.client_arrival),
            m.compose_time.as_deref().unwrap_or(""),
            &m.sender_mri,
            &m.sender_name,
            if m.from_me { "yes" } else { "" },
            &m.message_type,
            &m.content_type,
            &m.text,
            &m.content,
            &utc(m.edited),
            &utc(m.deleted),
            m.deleted_by.as_deref().unwrap_or(""),
            &reactions_str(m),
            &m.mentions.join("; "),
            &files_str(m),
            &m.importance,
            &m.subject,
            &m.reply_chain_id,
            &m.client_message_id,
            &m.sequence_id.map(|s| s.to_string()).unwrap_or_default(),
            &m.raw.get("properties").map(|p| p.to_string()).unwrap_or_default(),
            &m.prov.database,
            &m.prov.store,
            &m.prov.file,
            &m.prov.seq.to_string(),
            m.prov.status.as_str(),
            if m.prov.crc_ok { "yes" } else { "NO" },
            &m.occurrences.to_string(),
            &m.fingerprint,
        ])?;
    }
    w.flush()?;
    Ok(())
}

fn write_conversations_csv(ds: &Dataset, p: &Path) -> Result<()> {
    let mut w = csv_writer(p)?;
    w.write_record([
        "account",
        "conversation_id",
        "type",
        "title",
        "created_utc",
        "last_activity_utc",
        "messages_in_cache",
        "members",
        "cache_purges_recorded",
        "has_metadata",
        "source_database",
        "source_file",
        "record_status",
    ])?;
    for c in &ds.conversations {
        let members = c
            .members
            .iter()
            .map(|m| format!("{} [{}] {}", if m.name.is_empty() { "?" } else { &m.name }, m.role, m.mri))
            .collect::<Vec<_>>()
            .join("; ");
        w.write_record([
            c.account.as_str(),
            &c.id,
            &c.kind,
            &c.title,
            &utc(c.created),
            &utc(c.last_activity),
            &c.message_count.to_string(),
            &members,
            &serde_json::to_string(&c.cache_purges)?,
            if c.has_metadata { "yes" } else { "no" },
            c.prov.as_ref().map(|p| p.database.as_str()).unwrap_or(""),
            c.prov.as_ref().map(|p| p.file.as_str()).unwrap_or(""),
            c.prov.as_ref().map(|p| p.status.as_str()).unwrap_or(""),
        ])?;
    }
    w.flush()?;
    Ok(())
}

fn write_participants_csv(ds: &Dataset, p: &Path) -> Result<()> {
    let mut w = csv_writer(p)?;
    w.write_record([
        "mri",
        "display_name",
        "email",
        "user_principal_name",
        "job_title",
        "tenant_name",
        "source_database",
    ])?;
    for pr in ds.profiles.values() {
        w.write_record([
            pr.mri.as_str(),
            &pr.display_name,
            pr.email.as_deref().unwrap_or(""),
            pr.upn.as_deref().unwrap_or(""),
            pr.job_title.as_deref().unwrap_or(""),
            pr.tenant_name.as_deref().unwrap_or(""),
            &pr.prov.database,
        ])?;
    }
    w.flush()?;
    Ok(())
}

fn write_jsonl<T: Serialize>(items: impl Iterator<Item = T>, p: &Path) -> Result<()> {
    let mut f = BufWriter::new(File::create(p)?);
    for it in items {
        serde_json::to_writer(&mut f, &it)?;
        f.write_all(b"\n")?;
    }
    f.flush()?;
    Ok(())
}

pub fn transcript(ds: &Dataset, c: &teams::Conversation) -> String {
    use std::fmt::Write as _;
    let mut t = String::new();
    let acc = ds.account(&c.account).map(|a| a.label()).unwrap_or_else(|| c.account.clone());
    let tz = iana_time_zone::get_timezone().unwrap_or_default();
    let _ = writeln!(t, "Conversation: {}", c.title);
    let _ = writeln!(t, "Type: {}    ID: {}", c.kind, c.id);
    let _ = writeln!(t, "Account: {acc}");
    if !c.members.is_empty() {
        let m: Vec<String> = c.members.iter().map(|m| format!("{} ({})", m.name, m.role)).collect();
        let _ = writeln!(t, "Members: {}", m.join(", "));
    }
    let _ = writeln!(t, "Created: {}    Messages in local cache: {}", utc(c.created), c.message_count);
    for p in &c.cache_purges {
        let _ = writeln!(
            t,
            "Teams cache purge: at {} removed {} reply chains (sync window {} .. {})",
            utc(teams::as_ms(p.get("deleteTime"))),
            p.get("deletedRCsCount").map(|x| x.to_string()).unwrap_or_default(),
            utc(teams::as_ms(p.get("startOfSyncWindow"))),
            utc(teams::as_ms(p.get("endOfSyncWindow"))),
        );
    }
    let _ = writeln!(
        t,
        "Times are local ({tz}); UTC in messages.csv. Generated by {APP_NAME} from the local Teams cache only."
    );
    let _ = writeln!(t, "{}", "-".repeat(78));
    for m in ds.messages.iter().filter(|m| m.account == c.account && m.conversation_id == c.id) {
        if m.variant == Variant::Earlier && !m.substantive {
            continue;
        }
        let head = if m.variant == Variant::Earlier {
            format!("  ↳ EARLIER VERSION (differs in: {})", m.changes.join(", "))
        } else {
            format!("[{}] {}:", local(m.arrival), m.sender_name)
        };
        let _ = writeln!(t, "{head}");
        for line in m.text.lines() {
            let _ = writeln!(t, "    {line}");
        }
        let mut flags = Vec::new();
        if m.edited.is_some() {
            flags.push(format!("edited {}", local(m.edited)));
        }
        if m.deleted.is_some() {
            flags.push(format!(
                "deleted {}{}",
                local(m.deleted),
                m.deleted_by.as_ref().map(|d| format!(" by {d}")).unwrap_or_default()
            ));
        }
        if !m.reactions.is_empty() {
            flags.push(format!(
                "reactions: {}",
                m.reactions.iter().map(|r| format!("{} {}", r.emoji, r.name)).collect::<Vec<_>>().join(", ")
            ));
        }
        if !m.files.is_empty() {
            flags.push(format!("files: {}", files_str(m)));
        }
        if !flags.is_empty() {
            let _ = writeln!(t, "    ({})", flags.join("; "));
        }
    }
    t
}

pub const PURVIEW_NOTE: &str = "Whenever the organization's Microsoft Purview (eDiscovery / Content search) is available, its \
export is the authoritative, complete record and should be preferred. This package is meant for situations where that \
access is not available, and only reflects what one Teams client had cached on one computer.";

pub fn limitations() -> Vec<&'static str> {
    vec![
        PURVIEW_NOTE,
        "Only data the Teams client had cached is included. History that was never loaded, or that Teams purged from its \
         cache (see cache_purges_recorded in conversations.csv), is absent.",
        "Earlier versions of messages exist only until LevelDB compaction discards them; their absence does not mean a \
         message was never edited.",
        "Attachments are references (SharePoint/OneDrive links); file contents and most inline images are not part of this store.",
        "Hashes show the files have not changed since capture; they do not by themselves prove the authenticity or \
         completeness of the original conversation.",
    ]
}

// ------------------------------------------------------------------- export

pub fn export(opts: &ExportOptions, progress: &dyn Fn(Progress)) -> Result<ExportResult> {
    let started = Utc::now();
    let mut warnings: Vec<String> = Vec::new();
    let say = |stage: &str, done: u64, total: u64, detail: &str| progress(Progress { stage, done, total, detail });

    if opts.output.exists() {
        bail!("{} already exists; refusing to overwrite", opts.output.display());
    }
    let out_parent = opts
        .output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    let src_canon = opts.source.canonicalize().with_context(|| format!("source {}", opts.source.display()))?;
    let src_root = src_canon.parent().unwrap_or(&src_canon).to_path_buf();
    if let Ok(oc) = out_parent.canonicalize()
        && oc.starts_with(&src_root)
    {
        bail!("the output must not be written inside the evidence folder ({})", src_root.display());
    }
    let staging = opts.output.with_extension("staging");
    if staging.exists() {
        bail!("staging directory {} already exists; remove it first", staging.display());
    }

    let live_local = discover::is_live_local(&opts.source);
    let running = live_local && teams_running();
    if running {
        warnings.push(
            "Microsoft Teams was running during the capture: files may have changed while being copied. \
             Quit Teams completely and export again for a consistent snapshot."
                .into(),
        );
    }
    if !live_local && discover::same_volume(&opts.source, &out_parent) {
        warnings.push("The package was written to the same volume as the source evidence.".into());
    }
    let read_only = discover::volume_read_only(&opts.source);
    let source_kind = discover::found_kind(&opts.source);

    // 1. Byte-for-byte copy with hashes.
    let blob_dir = discover::blob_dir_for(&opts.source);
    let mut sources = vec![opts.source.clone()];
    sources.extend(blob_dir.clone());
    let mut todo: Vec<(PathBuf, String, u64)> = Vec::new();
    for src in &sources {
        let dname = src.file_name().context("source has no name")?.to_string_lossy().into_owned();
        for e in walkdir::WalkDir::new(src).sort_by_file_name() {
            let e = e?;
            if !e.file_type().is_file() {
                continue;
            }
            let rel = e.path().strip_prefix(src)?;
            let rel_str = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
            todo.push((e.path().to_path_buf(), format!("original/{dname}/{rel_str}"), e.metadata()?.len()));
        }
    }
    let total_bytes: u64 = todo.iter().map(|t| t.2).sum();
    let mut copied = 0u64;
    let mut entries: Vec<FileEntry> = Vec::new();
    for (src, zpath, _) in &todo {
        say("Copying and hashing the original files", copied, total_bytes, zpath);
        let dst = staging.join(zpath);
        std::fs::create_dir_all(dst.parent().unwrap())?;
        let before = std::fs::metadata(src)?;
        let (sha, n) = copy_hashed(src, &dst, &mut |k| copied += k)?;
        let after = std::fs::metadata(src)?;
        let changed =
            n != before.len() || before.len() != after.len() || before.modified().ok() != after.modified().ok();
        if changed {
            warnings.push(format!("{zpath} changed while it was being copied"));
        }
        let (sha2, _) = sha256_file(&dst)?;
        if sha2 != sha {
            bail!("verification failed for {zpath}: the copy does not match what was read");
        }
        entries.push(FileEntry {
            path: zpath.clone(),
            size: n,
            sha256: sha,
            modified_utc: sys_utc(before.modified()),
            created_utc: sys_utc(before.created()),
            source_path: Some(src.display().to_string()),
            changed_during_copy: changed,
        });
    }

    // 2. Parse the COPY (never the live files).
    let s_leveldb = staging.join("original").join(opts.source.file_name().unwrap());
    let s_blob = blob_dir.as_ref().map(|b| staging.join("original").join(b.file_name().unwrap()));
    say("Parsing the copied database", 0, 0, "");
    let ds = teams::load(&s_leveldb, s_blob.as_deref())?;

    let crc: usize = ds.catalog.files.iter().map(|f| f.crc_errors).sum();
    if crc > 0 {
        warnings.push(format!("{crc} LevelDB blocks/records failed their CRC check (see leveldb_files.jsonl)"));
    }
    if !ds.decode_errors.is_empty() {
        warnings
            .push(format!("{} chat records could not be decoded (see decode_errors.jsonl)", ds.decode_errors.len()));
    }
    if ds.accounts.is_empty() {
        warnings.push(
            "No new-Teams chat data was recognised in this store; the original files and all_records.jsonl are still included."
                .into(),
        );
    }

    let capture = json!({
        "tool": { "name": APP_NAME, "version": env!("CARGO_PKG_VERSION") },
        "case_reference": opts.case_reference,
        "examiner": opts.examiner,
        "notes": opts.notes,
        "started_utc": started.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "started_local": started.with_timezone(&Local).to_rfc3339(),
        "timezone": iana_time_zone::get_timezone().ok(),
        "host": host_info(),
        "source_paths": sources.iter().map(|s| s.display().to_string()).collect::<Vec<_>>(),
        "source_kind": source_kind.label(),
        "source_is_live_local_profile": live_local,
        "source_volume_read_only": read_only,
        "teams_running_during_capture": running,
        "method": "Every file of the Teams IndexedDB LevelDB directory (and its .blob directory) was opened read-only and \
                   copied byte for byte; SHA-256 was computed over the bytes read and re-verified on the copy. Readable files \
                   were then generated from the copy by ChatTrace Forensic's own LevelDB/IndexedDB/V8 parser; no LevelDB \
                   library is used, so the database is never opened for writing, and superseded/deleted record versions \
                   still present in the files are included and labelled.",
        "warnings": warnings,
        "limitations": limitations(),
    });

    let rd = staging.join("readable");
    std::fs::create_dir_all(rd.join("transcripts"))?;
    say("Writing CSV, HTML and JSON", 0, 0, "");
    write_messages_csv(&ds, &rd.join("messages.csv"))?;
    write_conversations_csv(&ds, &rd.join("conversations.csv"))?;
    write_participants_csv(&ds, &rd.join("participants.csv"))?;
    write_jsonl(ds.accounts.iter(), &rd.join("accounts.jsonl"))?;
    write_jsonl(ds.messages.iter(), &rd.join("messages.jsonl"))?;
    write_jsonl(ds.conversations.iter(), &rd.join("conversations.jsonl"))?;
    write_jsonl(ds.decode_errors.iter(), &rd.join("decode_errors.jsonl"))?;
    write_jsonl(ds.catalog.files.iter(), &rd.join("leveldb_files.jsonl"))?;
    report::write(&ds, &capture, &rd.join("report"))?;
    for c in &ds.conversations {
        let acc = ds.account(&c.account).map(|a| a.label()).unwrap_or_else(|| c.account.clone());
        let dir = rd.join("transcripts").join(safe_name(&acc, 60));
        std::fs::create_dir_all(&dir)?;
        let name = format!("{} [{}].txt", safe_name(&c.title, 80), short_hash(&c.id));
        std::fs::write(dir.join(name), transcript(&ds, c))?;
    }

    // Every store, every version (maximum metadata).
    if opts.all_stores {
        say("Decoding every IndexedDB store", 0, 0, "");
        let mut counts: std::collections::BTreeMap<(u64, u64), [usize; 4]> = Default::default();
        let mut f = BufWriter::new(File::create(rd.join("all_records.jsonl"))?);
        let mut werr: Option<std::io::Error> = None;
        idb::scan_records(&s_leveldb, s_blob.as_deref(), &ds.catalog, &|_, _| true, &mut |r| {
            let c = counts.entry((r.db, r.store)).or_default();
            c[r.status as usize] += 1;
            if r.value.is_err() {
                c[3] += 1;
            }
            let row = json!({
                "database": ds.catalog.db_name(r.db),
                "store": ds.catalog.store_name(r.db, r.store),
                "key": r.key.to_string(),
                "status": r.status,
                "seq": r.seq,
                "file": ds.catalog.files.get(r.file).map(|f| f.name.as_str()),
                "origin": r.origin,
                "crc_ok": r.crc_ok,
                "value": r.value.as_ref().ok().map(|v| v.to_json()),
                "error": r.value.as_ref().err(),
            });
            if werr.is_none()
                && let Err(e) =
                    serde_json::to_writer(&mut f, &row).map_err(std::io::Error::from).and_then(|_| f.write_all(b"\n"))
            {
                werr = Some(e);
            }
        })?;
        if let Some(e) = werr {
            return Err(e.into());
        }
        f.flush()?;
        let mut w = csv_writer(&rd.join("stores.csv"))?;
        w.write_record([
            "database_id",
            "database",
            "store_id",
            "store",
            "live",
            "superseded",
            "deleted",
            "decode_errors",
        ])?;
        for d in ds.catalog.databases.values() {
            for (sid, sn) in &d.stores {
                let c = counts.get(&(d.id, *sid)).copied().unwrap_or_default();
                w.write_record([
                    d.id.to_string(),
                    d.name.clone(),
                    sid.to_string(),
                    sn.clone(),
                    c[0].to_string(),
                    c[1].to_string(),
                    c[2].to_string(),
                    c[3].to_string(),
                ])?;
            }
        }
        w.flush()?;
    }

    // Hash the readable files.
    for e in walkdir::WalkDir::new(&rd).sort_by_file_name() {
        let e = e?;
        if !e.file_type().is_file() {
            continue;
        }
        let rel = e.path().strip_prefix(&staging)?;
        let zpath = rel.components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/");
        let (sha, n) = sha256_file(e.path())?;
        entries.push(FileEntry {
            path: zpath,
            size: n,
            sha256: sha,
            modified_utc: None,
            created_utc: None,
            source_path: None,
            changed_during_copy: false,
        });
    }

    // 3. Manifest.
    let latest: Vec<&Message> = ds.messages.iter().filter(|m| m.variant == Variant::Latest).collect();
    let arrivals: Vec<f64> = latest.iter().filter_map(|m| m.arrival).collect();
    let per_account: Vec<Json> = ds
        .accounts
        .iter()
        .map(|a| {
            let ms: Vec<&&Message> = latest.iter().filter(|m| m.account == a.key).collect();
            let arr: Vec<f64> = ms.iter().filter_map(|m| m.arrival).collect();
            json!({
                "account": a,
                "label": a.label(),
                "conversations": ds.conversations.iter().filter(|c| c.account == a.key).count(),
                "messages": ms.len(),
                "earlier_versions": ds.messages.iter().filter(|m| m.account == a.key && m.variant == Variant::Earlier).count(),
                "first_message_utc": utc(arr.iter().copied().reduce(f64::min)),
                "last_message_utc": utc(arr.iter().copied().reduce(f64::max)),
            })
        })
        .collect();
    let mut capture_final = capture.clone();
    capture_final["finished_utc"] = json!(Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true));
    let manifest = json!({
        "tool": capture["tool"],
        "capture": capture_final,
        "coverage": {
            "accounts": per_account,
            "conversations": ds.conversations.len(),
            "messages": latest.len(),
            "earlier_versions": ds.messages.len() - latest.len(),
            "earlier_versions_substantive": ds.messages.iter().filter(|m| m.substantive).count(),
            "first_message_utc": utc(arrivals.iter().copied().reduce(f64::min)),
            "last_message_utc": utc(arrivals.iter().copied().reduce(f64::max)),
        },
        "parse": {
            "leveldb_files": ds.catalog.files.len(),
            "crc_errors": crc,
            "chat_decode_errors": ds.decode_errors.len(),
        },
        "files": entries,
    });
    std::fs::write(staging.join("manifest.json"), serde_json::to_string_pretty(&manifest)?)?;

    let readme = format!(
        "{APP_NAME} - evidence package\n\
         ==================================\n\
         Created {} by {APP_NAME} {} on host {}.\n\n\
         {PURVIEW_NOTE}\n\n\
         original/   Byte-for-byte copy of the Teams IndexedDB (LevelDB) directory and blob directory.\n\
         readable/   Derived from original/ by the same tool:\n\
         \x20 report/index.html  HTML report: capture details, accounts, conversations; every message with all metadata\n\
         \x20 messages.csv       one row per message version (latest + earlier versions recovered from the cache)\n\
         \x20 conversations.csv  chats/meetings/channels with members and recorded cache purges\n\
         \x20 participants.csv   people known to the cache (MRI, name, e-mail)\n\
         \x20 transcripts/       plain-text transcript per conversation\n\
         \x20 *.jsonl            full decoded records with provenance (database, store, file, LevelDB sequence, status)\n\
         \x20 all_records.jsonl  every record of every IndexedDB store, all versions (if selected)\n\
         \x20 In the CSV files, a cell that begins with = + - @ (or a tab or carriage return) is prefixed\n\
         \x20 with ' so spreadsheet programs show it as text instead of running it as a formula.\n\
         \x20 The exact values are in the .jsonl files.\n\
         manifest.json  capture details, coverage, warnings, limitations and SHA-256 of every file\n\
         SHA256SUMS     checksums of every file in this package (except itself)\n\n\
         Verify:  shasum -a 256 -c SHA256SUMS     (macOS/Linux, run inside the extracted folder)\n\
         \x20        sha256sum -c SHA256SUMS\n\
         The SHA-256 of the zip itself is in the .sha256 file written next to it.\n\
         Zip entry dates are local time; exact UTC modification times are in manifest.json.\n\n\
         {APP_NAME} is free software (GPL-3.0-or-later) and is not affiliated with Microsoft.\n",
        started.to_rfc3339(),
        env!("CARGO_PKG_VERSION"),
        capture["host"]["hostname"].as_str().unwrap_or("?"),
    );
    std::fs::write(staging.join("README.txt"), readme)?;

    // SHA256SUMS over everything except itself.
    let mut sums = String::new();
    for name in ["README.txt", "manifest.json"] {
        let (h, _) = sha256_file(&staging.join(name))?;
        sums.push_str(&format!("{h}  {name}\n"));
    }
    for e in &entries {
        sums.push_str(&format!("{}  {}\n", e.sha256, e.path));
    }
    std::fs::write(staging.join("SHA256SUMS"), &sums)?;

    // 4. Zip.
    let mut order: Vec<String> = vec!["README.txt".into(), "manifest.json".into(), "SHA256SUMS".into()];
    order.extend(entries.iter().map(|e| e.path.clone()));
    {
        let out = File::create(&opts.output)?;
        let mut zw = zip::ZipWriter::new(BufWriter::new(out));
        for (i, name) in order.iter().enumerate() {
            say("Writing the zip", i as u64, order.len() as u64, name);
            let p = staging.join(name);
            let md = std::fs::metadata(&p)?;
            let mtime = md.modified().unwrap_or(SystemTime::now());
            let lt: DateTime<Local> = mtime.into();
            let mut o = zip::write::FullFileOptions::default()
                .compression_method(zip::CompressionMethod::Deflated)
                .large_file(md.len() > u32::MAX as u64 / 2);
            {
                use chrono::{Datelike, Timelike};
                if let Ok(dt) = zip::DateTime::from_date_and_time(
                    lt.year() as u16,
                    lt.month() as u8,
                    lt.day() as u8,
                    lt.hour() as u8,
                    lt.minute() as u8,
                    lt.second() as u8,
                ) {
                    o = o.last_modified_time(dt);
                }
            }
            // Extended timestamp (0x5455): exact UTC mtime.
            let secs = mtime.duration_since(SystemTime::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let mut ext = vec![1u8];
            ext.extend_from_slice(&(secs.min(u32::MAX as u64) as u32).to_le_bytes());
            o.add_extra_field(0x5455, ext, false)?;
            zw.start_file(name.as_str(), o)?;
            let mut f = File::open(&p)?;
            std::io::copy(&mut f, &mut zw)?;
        }
        zw.finish()?.flush()?;
    }

    // 5. Verify the zip against SHA256SUMS, then hash it.
    {
        let mut za = zip::ZipArchive::new(File::open(&opts.output)?)?;
        let lines: Vec<&str> = sums.lines().collect();
        for (i, line) in lines.iter().enumerate() {
            let (h, name) = line.split_once("  ").context("bad SHA256SUMS line")?;
            say("Verifying the zip", i as u64, lines.len() as u64, name);
            let zf = za.by_name(name).with_context(|| format!("{name} missing from zip"))?;
            let (got, _) = hash_reader(zf, |_| Ok(()))?;
            if got != h {
                bail!("zip verification failed for {name}");
            }
        }
    }
    let (zip_sha, _) = sha256_file(&opts.output)?;
    let zname = opts.output.file_name().unwrap().to_string_lossy();
    let mut side = opts.output.as_os_str().to_owned();
    side.push(".sha256");
    std::fs::write(PathBuf::from(side), format!("{zip_sha}  {zname}\n"))?;

    if !opts.keep_staging {
        std::fs::remove_dir_all(&staging)?;
    }
    Ok(ExportResult {
        zip: opts.output.clone(),
        zip_sha256: zip_sha,
        files: order.len(),
        messages: latest.len(),
        warnings,
    })
}

/// Open the system file manager with `p` selected.
pub fn reveal(p: &Path) {
    if cfg!(target_os = "macos") {
        let _ = std::process::Command::new("open").arg("-R").arg(p).spawn();
    } else if cfg!(windows) {
        let _ = std::process::Command::new("explorer").arg(format!("/select,{}", p.display())).spawn();
    } else if let Some(d) = p.parent() {
        let _ = std::process::Command::new("xdg-open").arg(d).spawn();
    }
}

/// Open a file with the default application (e.g. the HTML report).
pub fn open_path(p: &Path) {
    if cfg!(target_os = "macos") {
        let _ = std::process::Command::new("open").arg(p).spawn();
    } else if cfg!(windows) {
        let _ = std::process::Command::new("cmd").args(["/C", "start", ""]).arg(p).spawn();
    } else {
        let _ = std::process::Command::new("xdg-open").arg(p).spawn();
    }
}

// ------------------------------------------------------------------- verify

pub struct VerifyReport {
    pub zip_sha256: String,
    /// The `<zip>.sha256` file, if present.
    pub sidecar: Option<String>,
    pub sidecar_ok: Option<bool>,
    pub checked: usize,
    pub mismatched: Vec<String>,
    pub missing: Vec<String>,
    /// Files in the zip that SHA256SUMS does not list.
    pub unlisted: Vec<String>,
    pub manifest_summary: Option<String>,
}

impl VerifyReport {
    pub fn ok(&self) -> bool {
        self.sidecar_ok != Some(false)
            && self.mismatched.is_empty()
            && self.missing.is_empty()
            && self.unlisted.is_empty()
            && self.checked > 0
    }
}

/// Check an evidence zip: its sidecar `.sha256` and every entry against SHA256SUMS.
pub fn verify_zip(zip_path: &Path, progress: &dyn Fn(Progress)) -> Result<VerifyReport> {
    let (zip_sha256, _) = sha256_file(zip_path).with_context(|| format!("reading {}", zip_path.display()))?;
    let mut side = zip_path.as_os_str().to_owned();
    side.push(".sha256");
    let side = PathBuf::from(side);
    let (sidecar, sidecar_ok) = match std::fs::read_to_string(&side) {
        Ok(s) => {
            let want = s.split_whitespace().next().unwrap_or("").to_lowercase();
            (Some(side.display().to_string()), Some(want == zip_sha256))
        }
        Err(_) => (None, None),
    };
    let mut za = zip::ZipArchive::new(File::open(zip_path)?).context("not a readable zip file")?;
    let mut sums = String::new();
    za.by_name("SHA256SUMS")
        .context("SHA256SUMS missing: not a ChatTrace evidence package")?
        .read_to_string(&mut sums)?;
    let manifest_summary = za.by_name("manifest.json").ok().and_then(|mut f| {
        let mut s = String::new();
        f.read_to_string(&mut s).ok()?;
        let m: Json = serde_json::from_str(&s).ok()?;
        let c = &m["capture"];
        let mut parts = vec![format!(
            "captured {} with {} {}",
            c["started_utc"].as_str().unwrap_or("?"),
            m["tool"]["name"].as_str().unwrap_or("?"),
            m["tool"]["version"].as_str().unwrap_or("?")
        )];
        if let Some(r) = c["case_reference"].as_str() {
            parts.push(format!("case {r}"));
        }
        if let Some(e) = c["examiner"].as_str() {
            parts.push(format!("examiner {e}"));
        }
        Some(parts.join(", "))
    });
    let mut listed = std::collections::BTreeSet::new();
    let (mut checked, mut mismatched, mut missing) = (0, Vec::new(), Vec::new());
    let lines: Vec<&str> = sums.lines().filter(|l| !l.trim().is_empty()).collect();
    for (i, line) in lines.iter().enumerate() {
        let Some((h, name)) = line.split_once("  ") else { continue };
        listed.insert(name.to_string());
        progress(Progress { stage: "Verifying", done: i as u64, total: lines.len() as u64, detail: name });
        match za.by_name(name) {
            Ok(zf) => {
                checked += 1;
                match hash_reader(zf, |_| Ok(())) {
                    Ok((got, _)) if got == h => {}
                    Ok(_) => mismatched.push(name.to_string()),
                    Err(e) => mismatched.push(format!("{name} (unreadable: {e})")),
                }
            }
            Err(_) => missing.push(name.to_string()),
        }
    }
    let mut unlisted = Vec::new();
    for i in 0..za.len() {
        let name = za.by_index_raw(i)?.name()?.to_string();
        if name != "SHA256SUMS" && !name.ends_with('/') && !listed.contains(&name) {
            unlisted.push(name);
        }
    }
    Ok(VerifyReport { zip_sha256, sidecar, sidecar_ok, checked, mismatched, missing, unlisted, manifest_summary })
}
