// SPDX-License-Identifier: GPL-3.0-or-later
use chattrace::{discover, evidence, report};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

fn tempdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("chattrace-test-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn html_escaping() {
    assert_eq!(report::esc("<img src=x onerror=\"a\">&'"), "&lt;img src=x onerror=&quot;a&quot;&gt;&amp;&#39;");
}

#[test]
fn demo_report_is_static_and_safe() {
    let out = tempdir("report");
    report::write(&chattrace::demo::dataset(), &chattrace::demo::capture(), &out).unwrap();
    let index = std::fs::read_to_string(out.join("index.html")).unwrap();
    assert!(index.contains("Content-Security-Policy"));
    assert!(index.contains("Use Microsoft Purview") || index.contains("Microsoft Purview"));
    let pages: Vec<_> = std::fs::read_dir(out.join("conversations")).unwrap().collect();
    assert_eq!(pages.len(), 3);
    for p in pages {
        let html = std::fs::read_to_string(p.unwrap().path()).unwrap();
        assert!(!html.contains("<script"), "reports must not contain scripts");
        assert!(html.contains("Raw record"));
    }
    std::fs::remove_dir_all(out).unwrap();
}

#[test]
fn store_kinds_from_paths() {
    let mac = Path::new(
        "/Volumes/X/Users/jdoe/Library/Containers/com.microsoft.teams2/Data/Library/Application Support/Microsoft/MSTeams/EBWebView/WV2Profile_tfw/IndexedDB/https_teams.microsoft.com_0.indexeddb.leveldb",
    );
    assert_eq!(discover::found_kind(mac), discover::StoreKind::NewTeamsWork);
    let win = Path::new(
        "E:/Users/jdoe/AppData/Local/Packages/MSTeams_8wekyb3d8bbwe/LocalCache/Microsoft/MSTeams/EBWebView/WV2Profile_tfl/IndexedDB/https_teams.live.com_0.indexeddb.leveldb",
    );
    assert_eq!(discover::found_kind(win), discover::StoreKind::NewTeamsPersonal);
    let classic = Path::new(
        "C:/Users/jdoe/AppData/Roaming/Microsoft/Teams/IndexedDB/https_teams.microsoft.com_0.indexeddb.leveldb",
    );
    assert_eq!(discover::found_kind(classic), discover::StoreKind::ClassicTeams);
}

#[test]
fn discover_finds_store_in_any_folder() {
    let root = tempdir("discover");
    assert!(discover::discover(&root, &AtomicBool::new(false), &mut |_| {}).is_empty());
    let store = root.join("copy/of/disk/Users/jdoe/AppData/Local/Packages/MSTeams_8wekyb3d8bbwe/LocalCache/Microsoft/MSTeams/EBWebView/WV2Profile_tfw/IndexedDB/https_teams.microsoft.com_0.indexeddb.leveldb");
    std::fs::create_dir_all(&store).unwrap();
    std::fs::write(store.join("CURRENT"), "MANIFEST-000001\n").unwrap();
    std::fs::write(store.join("MANIFEST-000001"), b"").unwrap();
    let found = discover::discover(&root, &AtomicBool::new(false), &mut |_| {});
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].kind, discover::StoreKind::NewTeamsWork);
    assert_eq!(found[0].os_user.as_deref(), Some("jdoe"));
    // Pointing straight at the LevelDB folder works too.
    assert_eq!(discover::discover(&store, &AtomicBool::new(false), &mut |_| {}).len(), 1);
    std::fs::remove_dir_all(root).unwrap();
}

fn make_package(path: &Path, content: &[u8], listed_hash: &str) {
    let mut zw = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
    let o = zip::write::SimpleFileOptions::default();
    zw.start_file("SHA256SUMS", o).unwrap();
    writeln!(zw, "{listed_hash}  original/a.ldb").unwrap();
    zw.start_file("original/a.ldb", o).unwrap();
    zw.write_all(content).unwrap();
    zw.finish().unwrap();
}

#[test]
fn verify_detects_changes() {
    use sha2::{Digest, Sha256};
    let d = tempdir("verify");
    let h = hex::encode(Sha256::digest(b"evidence"));
    let good = d.join("good.zip");
    make_package(&good, b"evidence", &h);
    let r = evidence::verify_zip(&good, &|_| {}).unwrap();
    assert!(r.ok() && r.checked == 1);

    let bad = d.join("bad.zip");
    make_package(&bad, b"evidencE", &h);
    let r = evidence::verify_zip(&bad, &|_| {}).unwrap();
    assert!(!r.ok());
    assert_eq!(r.mismatched, vec!["original/a.ldb".to_string()]);

    std::fs::write(d.join("good.zip.sha256"), "0000  good.zip\n").unwrap();
    assert!(!evidence::verify_zip(&good, &|_| {}).unwrap().ok(), "a wrong sidecar must fail");
    std::fs::remove_dir_all(d).unwrap();
}
