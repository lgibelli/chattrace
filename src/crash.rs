// SPDX-License-Identifier: GPL-3.0-or-later
//! Crash reports, sent only with the user's approval.
//!
//! * Only builds made with `CHATTRACE_SENTRY_DSN` set (the official releases) can
//!   report crashes; self-built copies never do.
//! * When ChatTrace panics, a report is saved on this computer: version, operating
//!   system, where in ChatTrace's code it happened (function names), and the panic
//!   message with quoted text, paths, e-mail addresses and IDs removed. No host or
//!   user name, no chat content, no case data.
//! * Nothing is sent automatically. On the next launch the app shows the exact
//!   report and asks (Send / Don't send / Never ask); on the command line,
//!   `chattrace-cli crash-report` shows it and `--send` sends it.
//! * Sent with the operating system's curl to Sentry's envelope endpoint.
//! * Disabled by "Never ask", the About window, or `CHATTRACE_NO_CRASH_REPORTS=1`.

use anyhow::{Context, Result};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};

/// Sentry DSN, compiled in by the release workflow (secret `SENTRY_DSN`).
const DSN: Option<&str> = option_env!("CHATTRACE_SENTRY_DSN");
/// Reports kept on disk at most (newest first).
const MAX_PENDING: usize = 3;

pub struct Dsn {
    key: String,
    endpoint: String,
    raw: String,
}

/// Parse `https://<key>@<host>[/<path>]/<project>`; anything else disables reporting.
pub fn parse_dsn(raw: &str) -> Option<Dsn> {
    let rest = raw.trim().strip_prefix("https://")?;
    let (key, hostpath) = rest.split_once('@')?;
    let key = key.split(':').next()?;
    let (prefix, project) = hostpath.trim_end_matches('/').rsplit_once('/')?;
    let ok = !key.is_empty()
        && key.chars().all(|c| c.is_ascii_alphanumeric())
        && !prefix.is_empty()
        && !prefix.contains(['@', ' ', '?', '#'])
        && !project.is_empty()
        && project.chars().all(|c| c.is_ascii_digit());
    ok.then(|| Dsn {
        key: key.into(),
        endpoint: format!("https://{prefix}/api/{project}/envelope/"),
        raw: raw.trim().into(),
    })
}

fn dsn() -> Option<Dsn> {
    parse_dsn(DSN?)
}

/// Is crash reporting part of this build?
pub fn available() -> bool {
    dsn().is_some()
}

/// Should crashes be recorded and offered for sending?
pub fn enabled() -> bool {
    available()
        && std::env::var_os("CHATTRACE_NO_CRASH_REPORTS").is_none()
        && crate::update::Settings::load().crash_reports
}

fn dir() -> Option<PathBuf> {
    crate::update::config_dir().map(|d| d.join("crash-reports"))
}

static IN_HOOK: AtomicBool = AtomicBool::new(false);

/// Install the panic hook (no-op when crash reporting is off). `interface` is
/// `"gui"` or `"cli"`.
pub fn install(interface: &'static str) {
    if !enabled() {
        return;
    }
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default(info);
        if IN_HOOK.swap(true, Ordering::SeqCst) {
            return;
        }
        let msg = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic".into());
        let location = info.location().map(|l| format!("{}:{}", clean_file(l.file()), l.line()));
        let bt = std::backtrace::Backtrace::force_capture().to_string();
        let thread = match std::thread::current().name() {
            Some("main") => "main",
            _ => "background",
        };
        let event = event_json(&scrub(&msg), location.as_deref(), parse_backtrace(&bt), interface, thread);
        if save(&event).is_ok() && interface == "cli" {
            eprintln!(
                "\nA crash report was saved on this computer; nothing has been sent.\n\
                 Review it with `chattrace-cli crash-report`, then send it with --send or delete it with --discard."
            );
        }
        IN_HOOK.store(false, Ordering::SeqCst);
    }));
}

// ------------------------------------------------------------------- report

fn new_id() -> String {
    use sha2::{Digest, Sha256};
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let local = 0u8;
    let h = Sha256::digest(format!("{}:{}:{:p}", std::process::id(), now.as_nanos(), &local));
    hex::encode(h)[..32].to_string()
}

fn os_name() -> &'static str {
    match std::env::consts::OS {
        "macos" => "macOS",
        "windows" => "Windows",
        "linux" => "Linux",
        o => o,
    }
}

fn os_version() -> Option<String> {
    let run = |cmd: &str, args: &[&str]| -> Option<String> {
        let mut c = std::process::Command::new(cmd);
        c.args(args);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
        }
        let o = c.output().ok()?;
        o.status.success().then(|| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    let v = if cfg!(target_os = "macos") {
        run("sw_vers", &["-productVersion"])
    } else if cfg!(windows) {
        // "Microsoft Windows [Version 10.0.22631.4460]"
        run("cmd", &["/C", "ver"]).and_then(|s| {
            let s = s.split("Version ").nth(1)?.trim_end_matches(']').to_string();
            Some(s)
        })
    } else {
        run("uname", &["-r"])
    }?;
    // Only a version number, never anything else that a command might print.
    (!v.is_empty() && v.len() < 40 && v.chars().all(|c| c.is_ascii_alphanumeric() || ".-_+".contains(c))).then_some(v)
}

fn event_json(msg: &str, location: Option<&str>, frames: Vec<Value>, interface: &str, thread: &str) -> Value {
    let value = match location {
        Some(l) => format!("{msg} (at {l})"),
        None => msg.to_string(),
    };
    let mut exception = json!({
        "type": "panic",
        "value": value,
        "mechanism": { "type": "panic", "handled": false },
    });
    if !frames.is_empty() {
        exception["stacktrace"] = json!({ "frames": frames });
    }
    json!({
        "event_id": new_id(),
        "timestamp": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        "platform": "native",
        "level": "fatal",
        "release": format!("chattrace@{}", crate::VERSION),
        "environment": if cfg!(debug_assertions) { "debug" } else { "release" },
        "tags": { "interface": interface, "thread": thread },
        "contexts": {
            "os": { "name": os_name(), "version": os_version() },
            "device": { "arch": std::env::consts::ARCH },
            "app": { "app_name": crate::APP_NAME, "app_version": crate::VERSION },
        },
        "exception": { "values": [exception] },
        // Never let Sentry infer the sender's IP address (and from it a location):
        // an explicit null address plus the SDK setting that disables inference.
        "user": { "ip_address": null },
        "sdk": {
            "name": "chattrace.crash-report",
            "version": crate::VERSION,
            "settings": { "infer_ip": "never" },
        },
    })
}

/// A source path reduced to its part inside a project: `src/gui/viewer.rs`,
/// `egui-0.37.0/src/…`, `rust/std/src/…`, or just the file name.
fn clean_file(p: &str) -> String {
    let p = p.replace('\\', "/");
    if let Some(i) = p.find("/registry/src/") {
        return p[i + 14..].split_once('/').map(|(_, r)| r.to_string()).unwrap_or_default();
    }
    if p.contains("rustc/")
        && let Some(i) = p.find("/library/")
    {
        return format!("rust/{}", &p[i + 9..]);
    }
    if p.starts_with("src/") {
        return p;
    }
    if let Some(i) = p.rfind("/src/") {
        return p[i + 1..].to_string();
    }
    p.rsplit('/').next().unwrap_or_default().to_string()
}

/// Function names (and, in debug builds, project-relative locations) from a
/// `std::backtrace` dump, without the panic machinery, outermost frame first.
fn parse_backtrace(bt: &str) -> Vec<Value> {
    let mut frames: Vec<(String, Option<(String, u64)>)> = Vec::new();
    for line in bt.lines() {
        let t = line.trim();
        if let Some(at) = t.strip_prefix("at ") {
            if let Some(last) = frames.last_mut()
                && last.1.is_none()
            {
                let mut it = at.rsplitn(3, ':');
                let _col = it.next();
                let line = it.next().and_then(|l| l.parse().ok());
                if let (Some(file), Some(line)) = (it.next(), line) {
                    last.1 = Some((clean_file(file), line));
                }
            }
        } else if let Some((n, f)) = t.split_once(": ")
            && !n.is_empty()
            && n.chars().all(|c| c.is_ascii_digit())
        {
            frames.push((f.chars().take(200).collect(), None));
        }
    }
    let machinery = |f: &str| {
        f.contains("panicking")
            || f.contains("begin_unwind")
            || f.contains("panic_fmt")
            || f.contains("short_backtrace")
    };
    let skip = match frames.iter().position(|(f, _)| f.contains("begin_unwind") || f.contains("rust_panic_with_hook")) {
        Some(mut i) => {
            while i < frames.len() && machinery(&frames[i].0) {
                i += 1;
            }
            i
        }
        None => 0,
    };
    let useful: Vec<_> = frames.into_iter().skip(skip).take(64).filter(|(f, _)| f != "<unknown>").collect();
    useful
        .into_iter()
        .rev()
        .map(|(function, loc)| {
            let mut fr = json!({ "function": function, "in_app": function.starts_with("chattrace") });
            if let Some((file, line)) = loc {
                fr["filename"] = json!(file);
                fr["lineno"] = json!(line);
            }
            fr
        })
        .collect()
}

/// Remove anything in a panic message that could identify a person, a case or
/// a file: quoted text, paths, e-mail addresses, IDs.
pub fn scrub(s: &str) -> String {
    let cs: Vec<char> = s.chars().collect();
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    while i < cs.len() {
        let c = cs[i];
        if c == '`' || c == '"' {
            match cs[i + 1..].iter().position(|&d| d == c) {
                Some(j) => {
                    out.extend([c, '…', c]);
                    i += j + 2;
                    continue;
                }
                None => {
                    out.extend([c, '…']);
                    break;
                }
            }
        }
        if c == '\''
            && let Some(j) = cs[i + 1..].iter().take(6).position(|&d| d == '\'')
            && (1..=4).contains(&j)
        {
            out.push_str("'…'");
            i += j + 2;
            continue;
        }
        out.push(c);
        i += 1;
    }
    let looks_like_id = |w: &str| {
        let hex_run = w
            .chars()
            .fold((0, 0), |(cur, max), c| if c.is_ascii_hexdigit() { (cur + 1, max.max(cur + 1)) } else { (0, max) })
            .1;
        w.contains("orgid")
            || hex_run >= 12
            || (w.len() >= 24 && w.chars().all(|c| c.is_ascii_alphanumeric() || "-_=+".contains(c)))
    };
    let words: Vec<&str> = out
        .split_whitespace()
        .map(|w| {
            if w.contains(['/', '\\']) || w.starts_with('~') {
                "<path>"
            } else if w.contains('@') {
                "<redacted>"
            } else if looks_like_id(w) {
                "<id>"
            } else {
                w
            }
        })
        .collect();
    let r = words.join(" ");
    if r.chars().count() > 300 { r.chars().take(300).collect::<String>() + "…" } else { r }
}

// ------------------------------------------------------------------ storage

fn save(event: &Value) -> Result<PathBuf> {
    let d = dir().context("no settings folder")?;
    let mut b = std::fs::DirBuilder::new();
    b.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    b.create(&d)?;
    let id = event["event_id"].as_str().unwrap_or("report");
    let p = d.join(format!("{id}.json"));
    let mut o = std::fs::OpenOptions::new();
    o.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    std::io::Write::write_all(&mut o.open(&p)?, serde_json::to_string_pretty(event)?.as_bytes())?;
    for old in pending().into_iter().skip(MAX_PENDING) {
        let _ = std::fs::remove_file(old.path);
    }
    Ok(p)
}

/// A crash report saved on this computer and not yet sent.
#[derive(Clone)]
pub struct Pending {
    pub path: PathBuf,
    pub event: Value,
}

impl Pending {
    /// Exactly what would be sent.
    pub fn preview(&self) -> String {
        serde_json::to_string_pretty(&self.event).unwrap_or_default()
    }
}

/// Saved reports, newest first.
pub fn pending() -> Vec<Pending> {
    let Some(d) = dir() else { return Vec::new() };
    let Ok(rd) = std::fs::read_dir(d) else { return Vec::new() };
    let mut v: Vec<(std::time::SystemTime, Pending)> = rd
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "json"))
        .filter_map(|path| {
            let event: Value = serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok()?;
            let t = std::fs::metadata(&path).and_then(|m| m.modified()).unwrap_or(std::time::UNIX_EPOCH);
            Some((t, Pending { path, event }))
        })
        .collect();
    v.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
    v.into_iter().map(|(_, p)| p).collect()
}

pub fn discard(reports: &[Pending]) {
    for r in reports {
        let _ = std::fs::remove_file(&r.path);
    }
}

fn envelope(d: &Dsn, event: &Value) -> Vec<u8> {
    let payload = serde_json::to_vec(event).unwrap_or_default();
    let header = json!({
        "event_id": event["event_id"],
        "dsn": d.raw,
        "sent_at": chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
    });
    let item = json!({ "type": "event", "length": payload.len(), "content_type": "application/json" });
    let mut out = serde_json::to_vec(&header).unwrap_or_default();
    out.push(b'\n');
    out.extend(serde_json::to_vec(&item).unwrap_or_default());
    out.push(b'\n');
    out.extend(payload);
    out.push(b'\n');
    out
}

/// Send one report (after the user approved it) and delete it on success.
pub fn send(report: &Pending) -> Result<()> {
    let d = dsn().context("this build of ChatTrace has no crash reporting")?;
    let auth = format!(
        "X-Sentry-Auth: Sentry sentry_version=7, sentry_client=chattrace/{}, sentry_key={}",
        crate::VERSION,
        d.key
    );
    crate::update::curl_with_input(
        &[
            "--max-time",
            "30",
            "-X",
            "POST",
            "-H",
            "Content-Type: application/x-sentry-envelope",
            "-H",
            &auth,
            "--data-binary",
            "@-",
            &d.endpoint,
        ],
        Some(&envelope(&d, &report.event)),
    )
    .context("the crash report could not be sent")?;
    let _ = std::fs::remove_file(&report.path);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dsn_parsing() {
        let d = parse_dsn("https://abc123@o42.ingest.de.sentry.io/4507").unwrap();
        assert_eq!(d.endpoint, "https://o42.ingest.de.sentry.io/api/4507/envelope/");
        assert_eq!(d.key, "abc123");
        let d = parse_dsn("https://k@sentry.example.org/prefix/7/").unwrap();
        assert_eq!(d.endpoint, "https://sentry.example.org/prefix/api/7/envelope/");
        for bad in ["", "http://k@h/1", "https://h/1", "https://k@h/abc", "https://k@/1", "https://k x@h/1"] {
            assert!(parse_dsn(bad).is_none(), "{bad:?}");
        }
    }

    #[test]
    fn scrub_removes_personal_data() {
        let cases = [
            ("byte index 3 is not a char boundary; it is inside 'é' (bytes 2..4) of `Ciao Mario, ci vediamo`", "Mario"),
            (r#"called `Result::unwrap()` on an `Err` value: Os { message: "No such file: /Users/jdoe/x" }"#, "jdoe"),
            ("cannot open /Volumes/Case-Rossi/Users/jdoe/AppData", "Rossi"),
            (r"cannot open C:\Users\jdoe\AppData\Local", "jdoe"),
            ("unknown sender jdoe@halden.example", "jdoe"),
            ("bad mri 8:orgid:00000000-0000-4000-8000-00000000b0b2", "b0b2"),
        ];
        for (input, secret) in cases {
            let s = scrub(input);
            assert!(!s.contains(secret), "{input:?} -> {s:?}");
        }
        assert_eq!(
            scrub("index out of bounds: the len is 3 but the index is 5"),
            "index out of bounds: the len is 3 but the index is 5"
        );
        assert!(scrub(&"x ".repeat(400)).chars().count() <= 301);
    }

    #[test]
    fn backtrace_keeps_only_code_locations() {
        let bt = "   0: std::backtrace::Backtrace::force_capture\n\
                     1: chattrace::crash::install::{{closure}}\n\
                     2: std::panicking::rust_panic_with_hook\n\
                     3: std::panicking::begin_panic_handler::{{closure}}\n\
                     4: std::sys::backtrace::__rust_end_short_backtrace\n\
                     5: __rustc::rust_begin_unwind\n\
                     6: core::panicking::panic_fmt\n\
                     7: chattrace::teams::load\n\
                               at /Users/runner/work/chattrace/chattrace/src/teams.rs:120:9\n\
                     8: chattrace::cli::run\n\
                     9: std::panicking::catch_unwind\n\
                    10: main\n";
        let f = parse_backtrace(bt);
        let names: Vec<&str> = f.iter().map(|v| v["function"].as_str().unwrap()).collect();
        assert_eq!(names, ["main", "std::panicking::catch_unwind", "chattrace::cli::run", "chattrace::teams::load"]);
        assert_eq!(f[3]["filename"], "src/teams.rs");
        assert_eq!(f[3]["lineno"], 120);
        assert_eq!(f[3]["in_app"], true);
    }

    #[test]
    fn source_paths_are_project_relative() {
        assert_eq!(clean_file("src/gui/viewer.rs"), "src/gui/viewer.rs");
        assert_eq!(
            clean_file("/Users/x/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/egui-0.37.0/src/ui.rs"),
            "egui-0.37.0/src/ui.rs"
        );
        assert_eq!(clean_file("/rustc/59807616e/library/core/src/panicking.rs"), "rust/core/src/panicking.rs");
        assert_eq!(clean_file(r"C:\Users\jdoe\chattrace\src\teams.rs"), "src/teams.rs");
    }

    #[test]
    fn report_has_no_host_or_user() {
        let ev = event_json("boom", Some("src/x.rs:1"), Vec::new(), "gui", "main");
        let s = ev.to_string();
        for key in ["server_name", "hostname"] {
            assert!(ev.get(key).is_none(), "{key}");
        }
        assert_eq!(ev["user"], json!({ "ip_address": null }), "user must carry nothing but a null IP");
        assert_eq!(ev["sdk"]["settings"]["infer_ip"], "never");
        if let Ok(u) = std::env::var("USER").or_else(|_| std::env::var("USERNAME")) {
            assert!(u.len() < 3 || !s.contains(&u), "user name in report");
        }
        let env = envelope(&parse_dsn("https://k@h.example/1").unwrap(), &ev);
        assert_eq!(env.iter().filter(|&&b| b == b'\n').count(), 3);
    }
}
