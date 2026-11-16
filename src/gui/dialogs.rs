// SPDX-License-Identifier: GPL-3.0-or-later
//! Modal dialogs: export evidence, verify a package, about.

use super::Loaded;
use super::theme::{self, ACCENT, AMBER, GREEN, MUTED, RED, ph};
use crate::discover::Found;
use crate::evidence::{self, ExportOptions, ExportResult, VerifyReport};
use eframe::egui::{self, RichText, vec2};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};
use std::time::Instant;

#[derive(Default, Clone)]
struct Prog {
    stage: String,
    detail: String,
    done: u64,
    total: u64,
}

enum ExportState {
    Form,
    Running { started: Instant, prog: Arc<Mutex<Prog>>, rx: mpsc::Receiver<Result<ExportResult, String>> },
    Done(Result<ExportResult, String>),
}

pub struct ExportUi {
    source: Found,
    case: String,
    examiner: String,
    notes: String,
    output: String,
    all_stores: bool,
    keep_unzipped: bool,
    state: ExportState,
}

fn default_dir() -> PathBuf {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")).map(PathBuf::from);
    home.as_ref().map(|h| h.join("Desktop")).filter(|d| d.is_dir()).or(home).unwrap_or_else(|| PathBuf::from("."))
}

fn safe(s: &str) -> String {
    s.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '-' }).collect()
}

impl ExportUi {
    pub fn new(d: &Loaded) -> Self {
        let source = d.source.clone().expect("export needs a source");
        let name = format!("ChatTrace-{}.zip", chrono::Local::now().format("%Y%m%d-%H%M%S"));
        ExportUi {
            source,
            case: String::new(),
            examiner: String::new(),
            notes: String::new(),
            output: default_dir().join(name).display().to_string(),
            all_stores: true,
            keep_unzipped: false,
            state: ExportState::Form,
        }
    }

    pub fn running(&self) -> bool {
        matches!(self.state, ExportState::Running { .. })
    }

    fn start(&mut self, ctx: &egui::Context) {
        let opt = |s: &str| Some(s.trim().to_string()).filter(|s| !s.is_empty());
        let opts = ExportOptions {
            source: self.source.leveldb.clone(),
            output: PathBuf::from(self.output.trim()),
            case_reference: opt(&self.case),
            examiner: opt(&self.examiner),
            notes: opt(&self.notes),
            all_stores: self.all_stores,
            keep_staging: self.keep_unzipped,
        };
        let prog = Arc::new(Mutex::new(Prog::default()));
        let (tx, rx) = mpsc::channel();
        let (p, c) = (prog.clone(), ctx.clone());
        std::thread::spawn(move || {
            let r = evidence::export(&opts, &|x| {
                if let Ok(mut g) = p.lock() {
                    *g = Prog { stage: x.stage.into(), detail: x.detail.into(), done: x.done, total: x.total };
                }
                c.request_repaint();
            });
            let _ = tx.send(r.map_err(|e| format!("{e:#}")));
            c.request_repaint();
        });
        self.state = ExportState::Running { started: Instant::now(), prog, rx };
    }

    /// Returns false when the dialog should close.
    pub fn show(&mut self, ctx: &egui::Context, teams_running: bool) -> bool {
        if let ExportState::Running { rx, .. } = &self.state
            && let Ok(r) = rx.try_recv()
        {
            self.state = ExportState::Done(r);
        }
        let mut open = true;
        let mut start = false;
        egui::Modal::new(egui::Id::unique("export")).show(ctx, |ui| {
            ui.set_width(600.0);
            ui.horizontal(|ui| {
                ui.label(theme::icon(ph::SEAL_CHECK, 24.0, ACCENT));
                ui.label(theme::title("Export evidence package", 19.0));
            });
            ui.add_space(4.0);
            match &self.state {
                ExportState::Form => {
                    ui.add(egui::Label::new(theme::muted(
                        "Creates one zip: a byte-for-byte copy of the Teams database files, the SHA-256 of every file, a manifest, \
                         and readable CSV, HTML, JSON and text with all metadata, generated from the copy. The zip is verified after writing.",
                    ).size(12.5)).wrap());
                    ui.add_space(6.0);
                    ui.label(RichText::new(self.source.leveldb.display().to_string()).monospace().size(11.0).color(MUTED));
                    if teams_running {
                        ui.add_space(4.0);
                        theme::banner(AMBER).show(ui, |ui| {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(theme::icon(ph::WARNING, 16.0, AMBER));
                                ui.label(RichText::new("Microsoft Teams is running. Quit it completely first (macOS: Teams › Quit; Windows: tray icon › Quit), otherwise files may change while they are copied. The package records this.").size(12.5));
                            });
                        });
                    }
                    ui.add_space(8.0);
                    egui::Grid::new("export-form").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                        ui.label("Case / reference");
                        ui.add(egui::TextEdit::singleline(&mut self.case).hint_text("e.g. 2026-041").desired_width(440.0));
                        ui.end_row();
                        ui.label("Examiner");
                        ui.add(egui::TextEdit::singleline(&mut self.examiner).hint_text("Who is performing the capture").desired_width(440.0));
                        ui.end_row();
                        ui.label("Notes");
                        ui.add(egui::TextEdit::multiline(&mut self.notes).hint_text("Reason, scope, circumstances…").desired_rows(3).desired_width(440.0));
                        ui.end_row();
                        ui.label("Save to");
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut self.output).desired_width(350.0));
                            if ui.button("Choose…").clicked() {
                                let name = PathBuf::from(&self.output).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                                let name = if self.case.trim().is_empty() { name } else { format!("ChatTrace-{}-{}.zip", safe(self.case.trim()), chrono::Local::now().format("%Y%m%d-%H%M%S")) };
                                if let Some(p) = rfd::FileDialog::new().set_file_name(name).add_filter("Zip archive", &["zip"]).save_file() {
                                    self.output = p.display().to_string();
                                }
                            }
                        });
                        ui.end_row();
                    });
                    ui.add_space(4.0);
                    ui.checkbox(&mut self.all_stores, "Also decode every other IndexedDB store (all_records.jsonl)");
                    ui.checkbox(&mut self.keep_unzipped, "Keep an unzipped copy next to the zip (to browse the HTML report)");
                    let out = PathBuf::from(self.output.trim());
                    let exists = out.exists();
                    if exists {
                        ui.label(RichText::new("A file with this name already exists; choose another name.").color(RED).size(12.5));
                    }
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if theme::primary_enabled(ui, !exists && !self.output.trim().is_empty(), &format!("{} Create package", ph::FILE_ZIP)).clicked() {
                            start = true;
                        }
                        if theme::secondary(ui, "Cancel").clicked() {
                            open = false;
                        }
                    });
                }
                ExportState::Running { started, prog, .. } => {
                    let p = prog.lock().map(|g| g.clone()).unwrap_or_default();
                    ui.add_space(6.0);
                    ui.label(theme::title(if p.stage.is_empty() { "Starting…" } else { &p.stage }, 14.0));
                    let frac = if p.total > 0 { p.done as f32 / p.total as f32 } else { 0.0 };
                    let bar = egui::ProgressBar::new(frac).fill(ACCENT).desired_height(8.0);
                    ui.add(if p.total > 0 { bar } else { bar.animate(true) });
                    ui.add(egui::Label::new(RichText::new(&p.detail).monospace().size(11.0).color(MUTED)).truncate());
                    ui.label(theme::muted(format!("{:.0} s elapsed · the source is only read, never written", started.elapsed().as_secs_f32())).size(12.0));
                }
                ExportState::Done(Ok(r)) => {
                    ui.horizontal(|ui| {
                        ui.label(theme::icon(ph::CHECK_CIRCLE, 22.0, GREEN));
                        ui.label(theme::title("Evidence package created and verified", 15.0).color(GREEN));
                    });
                    ui.add_space(4.0);
                    theme::kv(ui, "export-done", &[
                        ("Package", r.zip.display().to_string()),
                        ("SHA-256", r.zip_sha256.clone()),
                        ("Contents", format!("{} files · {} messages", r.files, r.messages)),
                    ]);
                    ui.label(theme::muted("The SHA-256 is also written next to the zip (.sha256). Keep it separately, e.g. in your case notes or an e-mail to yourself, so the package can be shown unchanged later.").size(12.0));
                    if !r.warnings.is_empty() {
                        theme::banner(AMBER).show(ui, |ui| {
                            for w in &r.warnings {
                                ui.label(RichText::new(format!("{} {w}", ph::WARNING)).size(12.5));
                            }
                        });
                    }
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button(format!("{} Copy SHA-256", ph::COPY)).clicked() {
                            ui.ctx().copy_text(r.zip_sha256.clone());
                        }
                        let reveal = if cfg!(target_os = "macos") { "Show in Finder" } else if cfg!(windows) { "Show in Explorer" } else { "Show folder" };
                        if ui.button(format!("{} {reveal}", ph::FOLDER_OPEN)).clicked() {
                            evidence::reveal(&r.zip);
                        }
                        let report = r.zip.with_extension("staging").join("readable/report/index.html");
                        if report.is_file() && ui.button(format!("{} Open HTML report", ph::FILE_HTML)).clicked() {
                            evidence::open_path(&report);
                        }
                        if theme::primary(ui, "Done").clicked() {
                            open = false;
                        }
                    });
                }
                ExportState::Done(Err(e)) => {
                    ui.label(theme::title("The export failed", 15.0).color(RED));
                    ui.add(egui::Label::new(RichText::new(e).size(12.5)).wrap().selectable(true));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if theme::secondary(ui, "Back").clicked() {
                            self.state = ExportState::Form;
                        }
                        if theme::secondary(ui, "Close").clicked() {
                            open = false;
                        }
                    });
                }
            }
        });
        if start {
            self.start(ctx);
        }
        if !self.running() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            open = false;
        }
        open
    }
}

#[derive(Default)]
pub struct VerifyUi {
    path: Option<PathBuf>,
    rx: Option<mpsc::Receiver<Result<VerifyReport, String>>>,
    result: Option<Result<VerifyReport, String>>,
}

impl VerifyUi {
    pub fn show(&mut self, ctx: &egui::Context) -> bool {
        if let Some(rx) = &self.rx
            && let Ok(r) = rx.try_recv()
        {
            self.result = Some(r);
            self.rx = None;
        }
        let mut open = true;
        let mut pick = false;
        egui::Modal::new(egui::Id::unique("verify")).show(ctx, |ui| {
            ui.set_width(560.0);
            ui.horizontal(|ui| {
                ui.label(theme::icon(ph::SEAL_CHECK, 24.0, ACCENT));
                ui.label(theme::title("Verify an evidence package", 19.0));
            });
            ui.add(egui::Label::new(theme::muted("Recomputes the SHA-256 of the zip and of every file inside it, and compares them with the .sha256 file and SHA256SUMS.").size(12.5)).wrap());
            ui.add_space(6.0);
            if let Some(p) = &self.path {
                ui.label(RichText::new(p.display().to_string()).monospace().size(11.5));
            }
            if self.rx.is_some() {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Verifying…");
                });
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
            match &self.result {
                Some(Ok(r)) => {
                    ui.add_space(4.0);
                    if r.ok() {
                        ui.label(theme::title(format!("{} Intact: every file matches its checksum", ph::CHECK_CIRCLE), 15.0).color(GREEN));
                    } else {
                        ui.label(theme::title(format!("{} Does NOT match its checksums", ph::X_CIRCLE), 15.0).color(RED));
                    }
                    theme::kv(ui, "verify", &[
                        ("Zip SHA-256", r.zip_sha256.clone()),
                        ("Sidecar .sha256", match r.sidecar_ok { Some(true) => "matches".into(), Some(false) => "DOES NOT MATCH".into(), None => "not found next to the zip".into() }),
                        ("Package", r.manifest_summary.clone().unwrap_or_default()),
                        ("Files checked", r.checked.to_string()),
                        ("Modified", r.mismatched.join("\n")),
                        ("Missing", r.missing.join("\n")),
                        ("Not listed", r.unlisted.join("\n")),
                    ]);
                }
                Some(Err(e)) => {
                    ui.label(RichText::new(e).color(RED));
                }
                None => {}
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if theme::primary_enabled(ui, self.rx.is_none(), &format!("{} Choose zip…", ph::FILE_ZIP)).clicked() {
                    pick = true;
                }
                if theme::secondary(ui, "Close").clicked() {
                    open = false;
                }
            });
        });
        if pick && let Some(p) = rfd::FileDialog::new().add_filter("Zip archive", &["zip"]).pick_file() {
            let (tx, rx) = mpsc::channel();
            let (q, c) = (p.clone(), ctx.clone());
            std::thread::spawn(move || {
                let _ = tx.send(evidence::verify_zip(&q, &|_| {}).map_err(|e| format!("{e:#}")));
                c.request_repaint();
            });
            self.path = Some(p);
            self.result = None;
            self.rx = Some(rx);
        }
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            open = false;
        }
        open
    }
}

/// About window; returns false when closed.
pub fn about(ctx: &egui::Context, updater: &mut super::updates::Updater) -> bool {
    let mut open = true;
    egui::Modal::new(egui::Id::unique("about")).show(ctx, |ui| {
        ui.set_width(560.0);
        ui.horizontal(|ui| {
            ui.add(egui::Image::new(egui::include_image!("../../assets/mark.svg")).fit_to_exact_size(vec2(56.0, 56.0)));
            ui.vertical(|ui| {
                ui.label(theme::title("ChatTrace Forensic", 22.0));
                ui.label(
                    theme::muted(format!(
                        "Version {} · read-only viewer and evidence exporter for the Microsoft Teams desktop cache",
                        crate::VERSION
                    ))
                    .size(12.5),
                );
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("By {}", crate::AUTHOR)).size(13.0));
                    ui.hyperlink_to("salamacchine.it", crate::WEBSITE);
                });
            });
        });
        ui.add_space(8.0);
        theme::banner(ACCENT).show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(evidence::PURVIEW_NOTE).size(12.5)).wrap());
        });
        ui.add_space(8.0);
        egui::ScrollArea::vertical().max_height(220.0).show(ui, |ui| {
            ui.add(egui::Label::new(RichText::new(super::LICENSE_NOTICE).size(12.0)).wrap());
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.hyperlink_to(format!("{} Source code", ph::GITHUB_LOGO), crate::REPOSITORY);
            ui.hyperlink_to("GNU GPL v3", "https://www.gnu.org/licenses/gpl-3.0.html");
            ui.hyperlink_to("Third-party notices", format!("{}/blob/main/THIRD_PARTY_NOTICES.md", crate::REPOSITORY));
        });
        ui.label(
            theme::muted("Fonts: Inter (SIL OFL 1.1), Phosphor Icons (MIT), Hack (MIT), Noto Emoji (OFL).").size(11.5),
        );
        updater.about_section(ui);
        ui.add_space(8.0);
        if theme::primary(ui, "Close").clicked() {
            open = false;
        }
    });
    if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
        open = false;
    }
    open
}
