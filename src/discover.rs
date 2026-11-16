// SPDX-License-Identifier: GPL-3.0-or-later
//! Find Teams IndexedDB stores on this computer or under any folder:
//! an external disk, a mounted image of another computer, or a copied folder.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum StoreKind {
    /// New Teams (2023+), work or school account profile.
    NewTeamsWork,
    /// New Teams, personal (consumer) account profile.
    NewTeamsPersonal,
    /// Classic Teams (Electron, retired 2024–2025). Raw export only.
    ClassicTeams,
    /// A LevelDB folder that doesn't sit in a recognised Teams layout.
    Unknown,
}

impl StoreKind {
    pub fn label(&self) -> &'static str {
        match self {
            StoreKind::NewTeamsWork => "New Teams · work/school",
            StoreKind::NewTeamsPersonal => "New Teams · personal",
            StoreKind::ClassicTeams => "Classic Teams (raw export only)",
            StoreKind::Unknown => "LevelDB folder",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Found {
    pub leveldb: PathBuf,
    pub blob: Option<PathBuf>,
    pub kind: StoreKind,
    /// "macOS" / "Windows" layout the store was found in, if recognisable.
    pub layout: Option<&'static str>,
    /// OS account name taken from the path (…/Users/<name>/…).
    pub os_user: Option<String>,
    pub bytes: u64,
    pub files: usize,
    #[serde(skip)]
    pub newest: Option<SystemTime>,
    /// The live profile of the Teams client on this computer.
    pub live_local: bool,
}

/// Teams' folders on this computer: (the app's own folder, its profile folder inside).
fn local_layouts() -> Vec<(PathBuf, PathBuf)> {
    let mut v = Vec::new();
    if cfg!(target_os = "macos") {
        if let Some(home) = std::env::var_os("HOME") {
            let h = PathBuf::from(home);
            let app = h.join("Library/Containers/com.microsoft.teams2");
            let root = app.join("Data/Library/Application Support/Microsoft/MSTeams/EBWebView");
            v.push((app, root));
            let app = h.join("Library/Application Support/Microsoft/Teams");
            let root = app.join("IndexedDB");
            v.push((app, root));
        }
    } else if cfg!(windows) {
        if let Some(la) = std::env::var_os("LOCALAPPDATA") {
            let app = PathBuf::from(la).join(r"Packages\MSTeams_8wekyb3d8bbwe");
            let root = app.join(r"LocalCache\Microsoft\MSTeams\EBWebView");
            v.push((app, root));
        }
        if let Some(ad) = std::env::var_os("APPDATA") {
            let app = PathBuf::from(ad).join(r"Microsoft\Teams");
            let root = app.join("IndexedDB");
            v.push((app, root));
        }
    }
    v
}

/// Teams' own profile folders on this computer.
pub fn local_roots() -> Vec<PathBuf> {
    local_layouts().into_iter().map(|(_, root)| root).collect()
}

fn denied(e: &std::io::Error) -> bool {
    e.kind() == std::io::ErrorKind::PermissionDenied
}

/// Does the operating system refuse to let us read `p`? On macOS this is the
/// protection of other apps' data and of external disks, lifted by allowing
/// access or by Full Disk Access.
pub fn access_denied(p: &Path) -> bool {
    match std::fs::metadata(p) {
        Err(e) => denied(&e),
        Ok(m) => m.is_dir() && std::fs::read_dir(p).err().is_some_and(|e| denied(&e)),
    }
}

/// Is something between `app` and `root` (inclusive) present but unreadable?
fn blocked_below(app: &Path, root: &Path) -> bool {
    for p in root.ancestors() {
        match std::fs::metadata(p) {
            Ok(m) => return m.is_dir() && std::fs::read_dir(p).err().is_some_and(|e| denied(&e)),
            Err(e) if denied(&e) => return true,
            Err(_) => {}
        }
        if p == app {
            break;
        }
    }
    false
}

/// Teams folders on this computer that exist but that the OS won't let us
/// read (macOS privacy protection). Returns the Teams app folders.
pub fn local_blocked() -> Vec<PathBuf> {
    local_layouts().into_iter().filter(|(app, root)| blocked_below(app, root)).map(|(app, _)| app).collect()
}

/// What to tell the user when the OS blocks access (`gui`: wording for the app).
pub fn access_help(gui: bool) -> &'static str {
    if cfg!(target_os = "macos") {
        if gui {
            "macOS keeps other apps out of Teams' data, and out of external disks, until you allow it. \
             ChatTrace only reads. Open System Settings › Privacy & Security › Full Disk Access, turn on \
             ChatTrace Forensic (add it with + if it isn't listed), then quit and reopen ChatTrace."
        } else {
            "macOS is blocking access (privacy protection of other apps' data and external disks). \
             Give the app you run this from (e.g. Terminal) Full Disk Access in System Settings › \
             Privacy & Security › Full Disk Access, restart it, and try again."
        }
    } else {
        "The operating system refused access. Check that your account can read this folder."
    }
}

/// Open the Full Disk Access pane of System Settings (macOS).
pub fn open_privacy_settings() {
    #[cfg(target_os = "macos")]
    let _ = std::process::Command::new("open")
        .arg("x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles")
        .spawn();
}

/// Stores of the Teams client installed on this computer.
pub fn local() -> Vec<Found> {
    let mut out = Vec::new();
    for r in local_roots() {
        if r.is_dir() {
            collect_walk(&r, 4, &AtomicBool::new(false), &mut |_| {}, &mut out);
        }
    }
    finish(out)
}

pub fn is_live_local(p: &Path) -> bool {
    let Ok(c) = p.canonicalize() else { return false };
    local_roots().iter().filter_map(|r| r.canonicalize().ok()).any(|r| c.starts_with(r))
}

fn lower(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/").to_lowercase()
}

fn kind_of(p: &Path) -> (StoreKind, Option<&'static str>) {
    let s = lower(p);
    let layout = if s.contains("/library/") {
        Some("macOS")
    } else if s.contains("/appdata/") {
        Some("Windows")
    } else {
        None
    };
    if s.contains("mstea") || s.contains("com.microsoft.teams2") {
        if s.contains("wv2profile_tfl") {
            (StoreKind::NewTeamsPersonal, layout)
        } else {
            (StoreKind::NewTeamsWork, layout)
        }
    } else if s.contains("microsoft/teams/indexeddb") {
        (StoreKind::ClassicTeams, layout)
    } else {
        (StoreKind::Unknown, layout)
    }
}

/// The user folder the store belongs to: the component after the innermost
/// `Users` / `home`, so a disk image mounted inside the examiner's own profile
/// (`C:\Users\examiner\case\image\Users\jdoe\…`) is attributed to `jdoe`.
fn os_user(p: &Path) -> Option<String> {
    let parts: Vec<String> = p.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
    parts
        .iter()
        .rposition(|c| {
            c.eq_ignore_ascii_case("Users")
                || c.eq_ignore_ascii_case("home")
                || c.eq_ignore_ascii_case("Documents and Settings")
        })
        .and_then(|i| parts.get(i + 1).cloned())
}

/// A directory that is itself a LevelDB database.
pub fn is_leveldb_dir(p: &Path) -> bool {
    if !p.join("CURRENT").is_file() {
        return false;
    }
    std::fs::read_dir(p)
        .map(|rd| rd.filter_map(|e| e.ok()).any(|e| e.file_name().to_string_lossy().starts_with("MANIFEST-")))
        .unwrap_or(false)
}

fn is_teams_store_name(name: &str) -> bool {
    let n = name.to_lowercase();
    n.ends_with(".indexeddb.leveldb") && n.starts_with("https_teams.")
}

/// The `.blob` directory that belongs to a `.leveldb` directory.
pub fn blob_dir_for(leveldb: &Path) -> Option<PathBuf> {
    let name = leveldb.file_name()?.to_str()?;
    let lname = name.to_lowercase();
    if let Some(stem) = lname.strip_suffix(".leveldb") {
        let parent = leveldb.parent()?;
        if let Ok(rd) = std::fs::read_dir(parent) {
            for e in rd.filter_map(|e| e.ok()) {
                let n = e.file_name().to_string_lossy().to_lowercase();
                if n == format!("{stem}.blob") && e.path().is_dir() {
                    return Some(e.path());
                }
            }
        }
    }
    None
}

fn found(p: &Path) -> Found {
    let (kind, layout) = kind_of(p);
    let blob = blob_dir_for(p);
    let (mut bytes, mut files, mut newest) = (0u64, 0usize, None::<SystemTime>);
    for d in std::iter::once(p.to_path_buf()).chain(blob.clone()) {
        for e in walkdir::WalkDir::new(d).into_iter().filter_map(|e| e.ok()) {
            if let Ok(m) = e.metadata()
                && m.is_file()
            {
                bytes += m.len();
                files += 1;
                if let Ok(t) = m.modified() {
                    newest = Some(newest.map_or(t, |n: SystemTime| n.max(t)));
                }
            }
        }
    }
    Found {
        leveldb: p.to_path_buf(),
        blob,
        kind,
        layout,
        os_user: os_user(p),
        bytes,
        files,
        newest,
        live_local: is_live_local(p),
    }
}

const SKIP: &[&str] = &[
    ".git",
    "node_modules",
    ".trash",
    ".trashes",
    "$recycle.bin",
    ".spotlight-v100",
    ".fseventsd",
    "system volume information",
    "windows",
    "program files",
    "program files (x86)",
    "programdata",
    "caches",
    "system",
    "private",
    "cache_data",
    "code cache",
    "gpucache",
    "service worker",
];

fn collect_walk(
    root: &Path,
    max_depth: usize,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(usize),
    out: &mut Vec<Found>,
) {
    let mut seen = 0usize;
    let mut it = walkdir::WalkDir::new(root).follow_links(false).max_depth(max_depth).into_iter();
    while let Some(entry) = it.next() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let Ok(e) = entry else { continue };
        if !e.file_type().is_dir() {
            continue;
        }
        seen += 1;
        if seen.is_multiple_of(500) {
            progress(seen);
        }
        let name = e.file_name().to_string_lossy();
        if e.depth() > 0 && SKIP.contains(&name.to_lowercase().as_str()) {
            it.skip_current_dir();
            continue;
        }
        if is_teams_store_name(&name) {
            out.push(found(e.path()));
            it.skip_current_dir();
        }
    }
    progress(seen);
}

/// Known Teams locations relative to a home folder (both OS layouts).
const HOME_LAYOUTS: &[&str] = &[
    "Library/Containers/com.microsoft.teams2/Data/Library/Application Support/Microsoft/MSTeams/EBWebView",
    "Library/Application Support/Microsoft/Teams/IndexedDB",
    "AppData/Local/Packages/MSTeams_8wekyb3d8bbwe/LocalCache/Microsoft/MSTeams/EBWebView",
    "AppData/Roaming/Microsoft/Teams/IndexedDB",
];

/// Find Teams stores under `root`.
///
/// 1. `root` is itself a LevelDB folder → that folder.
/// 2. Known layouts under `root`, `root/Users/*` and `root/home/*` (a disk or a home folder).
/// 3. Otherwise a bounded recursive search (depth 24, skipping system folders).
pub fn discover(root: &Path, cancel: &AtomicBool, progress: &mut dyn FnMut(usize)) -> Vec<Found> {
    let mut out = Vec::new();
    if is_leveldb_dir(root) {
        out.push(found(root));
        return finish(out);
    }
    let mut homes = vec![root.to_path_buf()];
    for users in ["Users", "home", "Documents and Settings"] {
        if let Ok(rd) = std::fs::read_dir(root.join(users)) {
            homes.extend(rd.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.is_dir()));
        }
    }
    for h in &homes {
        for rel in HOME_LAYOUTS {
            let p = h.join(rel);
            if p.is_dir() {
                collect_walk(&p, 4, cancel, progress, &mut out);
            }
        }
    }
    if out.is_empty() {
        collect_walk(root, 24, cancel, progress, &mut out);
    }
    finish(out)
}

fn finish(mut v: Vec<Found>) -> Vec<Found> {
    let key = |f: &Found| f.leveldb.canonicalize().unwrap_or_else(|_| f.leveldb.clone());
    v.sort_by_key(key);
    v.dedup_by_key(|f| key(f));
    // Recognised Teams stores first, biggest first.
    v.sort_by_key(|f| (f.kind == StoreKind::Unknown, f.kind == StoreKind::ClassicTeams, std::cmp::Reverse(f.bytes)));
    v
}

/// Resolve a user-supplied path to exactly one store (for the CLI).
pub fn resolve(p: &Path) -> anyhow::Result<Found> {
    if !p.exists() {
        anyhow::bail!("{} does not exist", p.display());
    }
    let mut v = discover(p, &AtomicBool::new(false), &mut |_| {});
    match v.len() {
        0 if access_denied(p) => anyhow::bail!("cannot read {}: {}", p.display(), access_help(false)),
        0 => anyhow::bail!("no Teams IndexedDB found under {}", p.display()),
        1 => Ok(v.remove(0)),
        _ => {
            let list: Vec<String> =
                v.iter().map(|f| format!("  {}  ({})", f.leveldb.display(), f.kind.label())).collect();
            anyhow::bail!(
                "{} Teams stores found under {}; pass one of them with --source:\n{}",
                v.len(),
                p.display(),
                list.join("\n")
            )
        }
    }
}

pub fn human_bytes(n: u64) -> String {
    let units = ["B", "KB", "MB", "GB", "TB"];
    let mut v = n as f64;
    let mut i = 0;
    while v >= 1024.0 && i + 1 < units.len() {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 { format!("{n} B") } else { format!("{v:.1} {}", units[i]) }
}

/// Is the volume holding `p` mounted read-only? (None when unknown.)
pub fn volume_read_only(p: &Path) -> Option<bool> {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        let c = std::ffi::CString::new(p.as_os_str().as_bytes()).ok()?;
        let mut s: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(c.as_ptr(), &mut s) } != 0 {
            return None;
        }
        Some(s.f_flag & libc::ST_RDONLY != 0)
    }
    #[cfg(not(unix))]
    {
        let _ = p;
        None
    }
}

/// Are two paths on the same volume?
pub fn same_volume(a: &Path, b: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        match (std::fs::metadata(a), std::fs::metadata(b)) {
            (Ok(x), Ok(y)) => x.dev() == y.dev(),
            _ => false,
        }
    }
    #[cfg(not(unix))]
    {
        let root = |p: &Path| {
            p.canonicalize()
                .ok()
                .and_then(|c| c.components().next().map(|x| x.as_os_str().to_string_lossy().to_lowercase()))
        };
        root(a).is_some() && root(a) == root(b)
    }
}

/// What kind of store a path is (from its location).
pub fn found_kind(p: &Path) -> StoreKind {
    kind_of(p).0
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn blocked_is_not_the_same_as_missing() {
        let base = std::env::temp_dir().join(format!("chattrace-blocked-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let app = base.join("teams-app");
        let root = app.join("Data/EBWebView");
        // Teams not installed: not blocked.
        std::fs::create_dir_all(&base).unwrap();
        assert!(!blocked_below(&app, &root));
        // Installed and readable: not blocked.
        std::fs::create_dir_all(&root).unwrap();
        assert!(!blocked_below(&app, &root));
        assert!(!access_denied(&root));
        // Installed but unreadable (what macOS does without permission): blocked.
        let data = app.join("Data");
        std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o000)).unwrap();
        let readable_anyway = std::fs::read_dir(&data).is_ok(); // running as root
        if !readable_anyway {
            assert!(blocked_below(&app, &root));
            assert!(access_denied(&data));
        }
        std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::remove_dir_all(&base).unwrap();
    }
}
