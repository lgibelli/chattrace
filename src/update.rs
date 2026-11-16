// SPDX-License-Identifier: GPL-3.0-or-later
//! Update check and one-click install from GitHub Releases.
//!
//! * The check is a single HTTPS GET of the latest-release metadata; nothing is
//!   sent except a generic User-Agent. It can be switched off (About window,
//!   `CHATTRACE_NO_UPDATE_CHECK=1`, or `--no-update-check`).
//! * Downloads use the operating system's `curl` (macOS, Windows 10+, Linux), so
//!   corporate proxies and certificate stores work and no TLS stack is bundled.
//! * An update is installed only if `SHA256SUMS` carries a valid minisign
//!   signature from the key embedded below and the downloaded zip matches it.
//!   On macOS the new app bundle must also pass `codesign --verify` and carry
//!   the same Team ID as the running app.

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// minisign public key that signs release checksums (private key: CI secret only).
pub const PUBLIC_KEY: &str = "RWQS/nTr3WZDCPSzTsnfOTk0BB3qWqMbUv0E3mk/3LZALUJug7z3DWaW";
const LATEST: &str = "https://api.github.com/repos/lgibelli/chattrace/releases/latest";

#[derive(Debug, Clone)]
pub struct Release {
    pub version: String,
    pub page: String,
    pub notes: String,
    assets: Vec<(String, String)>,
}

// ------------------------------------------------------------------ settings

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub update_check: bool,
    pub skip_version: Option<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { update_check: true, skip_version: None }
    }
}

fn config_dir() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support/ChatTrace Forensic"))
    } else if cfg!(windows) {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("ChatTrace Forensic"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
            .map(|d| d.join("chattrace"))
    }
}

impl Settings {
    pub fn load() -> Self {
        config_dir()
            .and_then(|d| std::fs::read_to_string(d.join("settings.json")).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }
    pub fn save(&self) {
        if let Some(d) = config_dir() {
            let _ = std::fs::create_dir_all(&d);
            let _ = std::fs::write(d.join("settings.json"), serde_json::to_string_pretty(self).unwrap_or_default());
        }
    }
}

/// Should the app check for updates at launch?
pub fn auto_check_enabled() -> bool {
    std::env::var_os("CHATTRACE_NO_UPDATE_CHECK").is_none()
        && !std::env::args().any(|a| a == "--no-update-check")
        && Settings::load().update_check
}

// --------------------------------------------------------------------- http

fn curl(args: &[&str]) -> Result<Vec<u8>> {
    let bin = if cfg!(windows) { "curl.exe" } else { "curl" };
    let ua = format!("ChatTrace-Forensic/{}", crate::VERSION);
    let mut c = std::process::Command::new(bin);
    c.args(["-fsSL", "--proto", "=https", "--proto-redir", "=https", "--connect-timeout", "15", "-A", &ua]);
    c.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let o = c.output().with_context(|| format!("running {bin}"))?;
    if !o.status.success() {
        bail!("download failed: {}", String::from_utf8_lossy(&o.stderr).trim());
    }
    Ok(o.stdout)
}

fn version_tuple(v: &str) -> Option<(u64, u64, u64)> {
    let v = v.trim().trim_start_matches('v');
    let core = v.split(['-', '+']).next()?;
    let mut it = core.split('.').map(|x| x.parse::<u64>().ok());
    Some((it.next()??, it.next().flatten().unwrap_or(0), it.next().flatten().unwrap_or(0)))
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (version_tuple(candidate), version_tuple(current)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// Latest published release, if newer than this build.
pub fn check() -> Result<Option<Release>> {
    let body = curl(&["--max-time", "20", "-H", "Accept: application/vnd.github+json", LATEST])?;
    let j: serde_json::Value = serde_json::from_slice(&body).context("unexpected reply from GitHub")?;
    let tag = j["tag_name"].as_str().context("no release found")?;
    let version = tag.trim_start_matches('v').to_string();
    if !is_newer(&version, crate::VERSION) {
        return Ok(None);
    }
    let assets = j["assets"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|x| {
                    Some((x["name"].as_str()?.to_string(), x["browser_download_url"].as_str()?.to_string()))
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(Some(Release {
        version,
        page: j["html_url"].as_str().unwrap_or(crate::REPOSITORY).to_string(),
        notes: j["body"].as_str().unwrap_or("").to_string(),
        assets,
    }))
}

/// Name of the update payload for this platform in a release.
pub fn asset_name(version: &str) -> String {
    let platform = if cfg!(target_os = "macos") {
        "macos-universal"
    } else if cfg!(windows) {
        "windows-x64"
    } else {
        "linux-x86_64"
    };
    format!("ChatTrace-Forensic-{version}-{platform}.zip")
}

impl Release {
    fn url(&self, name: &str) -> Result<&str> {
        self.assets
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, u)| u.as_str())
            .with_context(|| format!("the release has no {name}"))
    }
}

// ------------------------------------------------------------------ install

/// Check that `sums` is signed by [`PUBLIC_KEY`]; return the SHA-256 listed for `name`.
pub fn verified_hash(sums: &[u8], minisig: &str, name: &str) -> Result<String> {
    let pk = minisign_verify::PublicKey::from_base64(PUBLIC_KEY).context("bad embedded public key")?;
    let sig = minisign_verify::Signature::decode(minisig).context("unreadable signature")?;
    pk.verify(sums, &sig, false).context("the release checksums are NOT signed by the ChatTrace key")?;
    let text = std::str::from_utf8(sums)?;
    text.lines()
        .find_map(|l| {
            let (h, n) = l.split_once(char::is_whitespace)?;
            (n.trim().trim_start_matches('*') == name).then(|| h.to_lowercase())
        })
        .with_context(|| format!("{name} is not listed in SHA256SUMS"))
}

#[cfg(target_os = "macos")]
fn run_ok(cmd: &str, args: &[&str]) -> Result<String> {
    let o = std::process::Command::new(cmd).args(args).output()?;
    let all = format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr));
    if !o.status.success() {
        bail!("{cmd} failed: {}", all.trim());
    }
    Ok(all)
}

#[cfg(target_os = "macos")]
fn team_id(app: &Path) -> Option<String> {
    let out = run_ok("codesign", &["-dv", &app.to_string_lossy()]).ok()?;
    out.lines().find_map(|l| l.strip_prefix("TeamIdentifier=")).map(|s| s.trim().to_string()).filter(|s| s != "not set")
}

/// A new, private (0700 on Unix) working folder with an unguessable name. Creation
/// is exclusive: it fails rather than reuse anything that already exists.
fn private_work_dir() -> Result<PathBuf> {
    use sha2::{Digest, Sha256};
    let base = std::env::temp_dir();
    for attempt in 0u32..16 {
        let mut h = Sha256::new();
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        let local = 0u8;
        h.update(format!("{}:{}:{attempt}:{:p}", std::process::id(), now.as_nanos(), &local));
        let dir = base.join(format!("chattrace-update-{}", &hex::encode(h.finalize())[..24]));
        let mut b = std::fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
        match b.create(&dir) {
            Ok(()) => return Ok(dir),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e).context("creating a private folder for the update"),
        }
    }
    bail!("could not create a private folder for the update")
}

/// Refuse to write through a symbolic link that an earlier zip entry created.
fn ensure_no_symlink(dest: &Path, out: &Path) -> Result<()> {
    let mut p = out.parent();
    while let Some(d) = p {
        if d == dest || !d.starts_with(dest) {
            break;
        }
        if std::fs::symlink_metadata(d).is_ok_and(|m| m.file_type().is_symlink()) {
            bail!("unsafe path in update zip");
        }
        p = d.parent();
    }
    Ok(())
}

fn extract(zip_bytes: &[u8], dest: &Path) -> Result<()> {
    let mut za = zip::ZipArchive::new(std::io::Cursor::new(zip_bytes))?;
    for i in 0..za.len() {
        let mut f = za.by_index(i)?;
        let Some(rel) = f.enclosed_name() else { bail!("unsafe path in update zip") };
        let out = dest.join(rel);
        ensure_no_symlink(dest, &out)?;
        if f.is_dir() {
            std::fs::create_dir_all(&out)?;
            continue;
        }
        if let Some(p) = out.parent() {
            std::fs::create_dir_all(p)?;
        }
        ensure_no_symlink(dest, &out)?;
        #[cfg(unix)]
        let is_link = f.unix_mode().is_some_and(|m| m & 0o170000 == 0o120000);
        #[cfg(not(unix))]
        let is_link = false;
        if is_link {
            #[cfg(unix)]
            {
                let mut target = String::new();
                std::io::Read::read_to_string(&mut f, &mut target)?;
                std::os::unix::fs::symlink(target, &out)?;
            }
            continue;
        }
        // create_new: never follow or overwrite something already at that path
        let mut w = std::fs::OpenOptions::new().write(true).create_new(true).open(&out)?;
        std::io::copy(&mut f, &mut w)?;
        #[cfg(unix)]
        if let Some(m) = f.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&out, std::fs::Permissions::from_mode(m & 0o777))?;
        }
    }
    Ok(())
}

/// Download, verify and install `rel`. Returns what to relaunch.
pub fn install(rel: &Release, progress: &dyn Fn(&str)) -> Result<PathBuf> {
    let name = asset_name(&rel.version);
    let tmp = private_work_dir()?;

    progress("Downloading the signed checksums…");
    let sums = curl(&["--max-time", "60", rel.url("SHA256SUMS")?])?;
    let sig = curl(&["--max-time", "60", rel.url("SHA256SUMS.minisig")?])?;
    let want = verified_hash(&sums, &String::from_utf8_lossy(&sig), &name)?;

    progress(&format!("Downloading {name}…"));
    // Kept in memory: the bytes that are verified are the bytes that are unpacked.
    let zip_bytes = curl(&["--max-time", "900", rel.url(&name)?])?;
    progress("Verifying the download…");
    let got = {
        use sha2::{Digest, Sha256};
        hex::encode(Sha256::digest(&zip_bytes))
    };
    if got != want {
        bail!("the downloaded file does not match its signed checksum; nothing was changed");
    }
    progress("Unpacking…");
    let x = tmp.join("x");
    extract(&zip_bytes, &x)?;
    drop(zip_bytes);

    let exe = std::env::current_exe()?.canonicalize()?;
    #[cfg(target_os = "macos")]
    {
        let bundle = exe
            .ancestors()
            .find(|p| p.extension().is_some_and(|e| e == "app"))
            .context("ChatTrace is not running from an app bundle; download the update manually")?
            .to_path_buf();
        let new = std::fs::read_dir(&x)?
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .find(|p| p.extension().is_some_and(|e| e == "app"))
            .context("no app in the update")?;
        progress("Checking the code signature…");
        run_ok("codesign", &["--verify", "--deep", "--strict", &new.to_string_lossy()])
            .context("the new app's code signature is invalid")?;
        if let Some(cur) = team_id(&bundle)
            && team_id(&new).as_deref() != Some(cur.as_str())
        {
            bail!("the new app is signed by a different developer; nothing was changed");
        }
        progress("Installing…");
        let backup = tmp.join("previous.app");
        std::fs::rename(&bundle, &backup).with_context(|| format!("cannot replace {}", bundle.display()))?;
        if let Err(e) = std::fs::rename(&new, &bundle).or_else(|_| {
            run_ok("ditto", &[&new.to_string_lossy(), &bundle.to_string_lossy()])
                .map(|_| ())
                .map_err(std::io::Error::other)
        }) {
            let _ = std::fs::rename(&backup, &bundle);
            bail!("installing failed ({e}); the previous version was restored");
        }
        Ok(bundle)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let dir = exe.parent().context("no install folder")?.to_path_buf();
        progress("Installing…");
        let mut gui = None;
        for e in std::fs::read_dir(&x)?.filter_map(|e| e.ok()) {
            let n = e.file_name().to_string_lossy().into_owned();
            let is_bin = n.starts_with("chattrace") && (cfg!(not(windows)) || n.ends_with(".exe"));
            if !is_bin || !e.path().is_file() {
                continue;
            }
            let target = dir.join(&n);
            let old = dir.join(format!("{n}.old"));
            let _ = std::fs::remove_file(&old);
            if target.exists() {
                std::fs::rename(&target, &old).with_context(|| format!("cannot replace {}", target.display()))?;
            }
            std::fs::copy(e.path(), &target)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o755))?;
            }
            if n == format!("chattrace{}", std::env::consts::EXE_SUFFIX) {
                gui = Some(target);
            }
        }
        gui.context("no ChatTrace executable in the update")
    }
}

/// Start the freshly installed version and exit this one.
pub fn relaunch(target: &Path) -> ! {
    if cfg!(target_os = "macos") {
        let _ = std::process::Command::new("open").arg("-n").arg(target).spawn();
    } else {
        let _ = std::process::Command::new(target).spawn();
    }
    std::process::exit(0)
}

/// Remove `*.old` executables left by a previous update (Windows/Linux).
pub fn cleanup() {
    if cfg!(target_os = "macos") {
        return;
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.parent().map(|p| p.to_path_buf()))
        && let Ok(rd) = std::fs::read_dir(dir)
    {
        for e in rd.filter_map(|e| e.ok()) {
            let n = e.file_name().to_string_lossy().into_owned();
            if n.starts_with("chattrace") && n.ends_with(".old") {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions() {
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("v1.0.0", "0.9.9"));
        assert!(!is_newer("0.1.0", "0.1.0"));
        assert!(!is_newer("0.1.0-rc.1", "0.1.0"));
        assert!(!is_newer("garbage", "0.1.0"));
    }

    #[test]
    fn rejects_unsigned_checksums() {
        let sums = b"0000  ChatTrace-Forensic-9.9.9-macos-universal.zip\n";
        let bogus = "untrusted comment: x\nRUQS/nTr3WZDCPSzTsnfOTk0BB3qWqMbUv0E3mk/3LZALUJug7z3DWaWAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==\ntrusted comment: x\nAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA==\n";
        assert!(verified_hash(sums, bogus, "ChatTrace-Forensic-9.9.9-macos-universal.zip").is_err());
    }

    fn zip_of(build: impl FnOnce(&mut zip::ZipWriter<std::io::Cursor<Vec<u8>>>)) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        build(&mut w);
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn private_work_dirs_are_new_and_unique() {
        let a = private_work_dir().unwrap();
        let b = private_work_dir().unwrap();
        assert_ne!(a, b);
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&a).unwrap().permissions().mode() & 0o777, 0o700);
        }
        let _ = std::fs::remove_dir_all(a);
        let _ = std::fs::remove_dir_all(b);
    }

    #[test]
    fn extracts_files() {
        use std::io::Write;
        let opts = zip::write::SimpleFileOptions::default();
        let z = zip_of(|w| {
            w.add_directory("App.app/Contents/", opts).unwrap();
            w.start_file("App.app/Contents/Info.plist", opts).unwrap();
            w.write_all(b"plist").unwrap();
        });
        let d = private_work_dir().unwrap();
        extract(&z, &d.join("x")).unwrap();
        assert_eq!(std::fs::read(d.join("x/App.app/Contents/Info.plist")).unwrap(), b"plist");
        let _ = std::fs::remove_dir_all(d);
    }

    #[cfg(unix)]
    #[test]
    fn refuses_to_write_through_symlinks() {
        use std::io::Write;
        let opts = zip::write::SimpleFileOptions::default();
        let outside = private_work_dir().unwrap();
        let z = zip_of(|w| {
            w.add_symlink("link", outside.to_string_lossy(), opts).unwrap();
            w.start_file("link/planted", opts).unwrap();
            w.write_all(b"x").unwrap();
        });
        let d = private_work_dir().unwrap();
        assert!(extract(&z, &d.join("x")).is_err());
        assert!(!outside.join("planted").exists());
        let _ = std::fs::remove_dir_all(d);
        let _ = std::fs::remove_dir_all(outside);
    }
}

#[cfg(test)]
mod signed_fixture {
    // tests/fixtures/update/SHA256SUMS is signed with the release key; it lists
    // a fake hash for an asset name no release uses.
    const SUMS: &[u8] = include_bytes!("../tests/fixtures/update/SHA256SUMS");
    const SIG: &str = include_str!("../tests/fixtures/update/SHA256SUMS.minisig");
    const NAME: &str = "chattrace-test-fixture-not-a-release.zip";

    #[test]
    fn accepts_signed_checksums() {
        let h = super::verified_hash(SUMS, SIG, NAME).unwrap();
        assert_eq!(h, "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef");
    }

    #[test]
    fn rejects_tampered_checksums() {
        let mut t = SUMS.to_vec();
        t[0] = b'f';
        assert!(super::verified_hash(&t, SIG, NAME).is_err());
    }
}
