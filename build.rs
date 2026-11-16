// SPDX-License-Identifier: GPL-3.0-or-later
//! Windows: embed the icon and version information into the executables.
//! Best effort: if no resource compiler is available (e.g. some cross builds),
//! the build continues without them.

fn main() {
    println!("cargo:rerun-if-changed=assets/chattrace.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let version = env!("CARGO_PKG_VERSION");
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/chattrace.ico")
        .set("ProductName", "ChatTrace Forensic")
        .set("FileDescription", "ChatTrace Forensic: read-only Teams cache viewer and evidence exporter")
        .set("CompanyName", "Luca Gibelli · Salamacchine")
        .set("LegalCopyright", "© 2026 Luca Gibelli · Salamacchine. GPL-3.0-or-later. Not affiliated with Microsoft.")
        .set("ProductVersion", version)
        .set("FileVersion", version);
    if let Err(e) = res.compile() {
        println!("cargo:warning=Windows resources not embedded: {e}");
    }
}
