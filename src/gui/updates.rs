// SPDX-License-Identifier: GPL-3.0-or-later
//! Update banner, dialog and settings (see crate::update).

use super::theme::{self, ACCENT, GREEN, MUTED, RED, ph};
use crate::update::{self, Release, Settings};
use eframe::egui::{self, RichText};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, mpsc};

/// A running install: result channel + current status line.
type Installing = (mpsc::Receiver<Result<PathBuf, String>>, Arc<Mutex<String>>);

pub struct Updater {
    pub settings: Settings,
    checking: Option<mpsc::Receiver<Result<Option<Release>, String>>>,
    manual: bool,
    pub status: Option<String>,
    pub available: Option<Release>,
    pub dialog: bool,
    installing: Option<Installing>,
    installed: Option<PathBuf>,
    error: Option<String>,
}

impl Updater {
    pub fn new(ctx: &egui::Context) -> Self {
        update::cleanup();
        let mut u = Updater {
            settings: Settings::load(),
            checking: None,
            manual: false,
            status: None,
            available: None,
            dialog: false,
            installing: None,
            installed: None,
            error: None,
        };
        if update::auto_check_enabled() {
            u.check(ctx, false);
        }
        u
    }

    pub fn check(&mut self, ctx: &egui::Context, manual: bool) {
        let (tx, rx) = mpsc::channel();
        let c = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(update::check().map_err(|e| format!("{e:#}")));
            c.request_repaint();
        });
        self.manual = manual;
        self.status = Some("Checking for updates…".into());
        self.checking = Some(rx);
    }

    pub fn busy(&self) -> bool {
        self.checking.is_some() || self.installing.is_some()
    }

    pub fn poll(&mut self) {
        if let Some(rx) = &self.checking
            && let Ok(r) = rx.try_recv()
        {
            self.checking = None;
            match r {
                Ok(Some(rel)) => {
                    self.status = Some(format!("Version {} is available.", rel.version));
                    let skipped = !self.manual && self.settings.skip_version.as_deref() == Some(rel.version.as_str());
                    if !skipped {
                        if self.manual {
                            self.dialog = true;
                        }
                        self.available = Some(rel);
                    }
                }
                Ok(None) => self.status = Some(format!("ChatTrace Forensic {} is up to date.", crate::VERSION)),
                Err(e) => {
                    self.status = Some(if self.manual {
                        format!("Update check failed: {e}")
                    } else {
                        "Could not check for updates.".into()
                    })
                }
            }
        }
        if let Some((rx, _)) = &self.installing
            && let Ok(r) = rx.try_recv()
        {
            self.installing = None;
            match r {
                Ok(p) => self.installed = Some(p),
                Err(e) => self.error = Some(e),
            }
        }
    }

    /// Small "update available" button for the top bar / welcome screen.
    pub fn badge(&mut self, ui: &mut egui::Ui) {
        if let Some(rel) = &self.available {
            let b = egui::Button::new(
                RichText::new(format!("{} Update to {}", ph::ARROW_CIRCLE_UP, rel.version)).color(ACCENT),
            )
            .fill(ACCENT.gamma_multiply(0.14))
            .stroke(egui::Stroke::new(1.0, ACCENT.gamma_multiply(0.6)))
            .corner_radius(8);
            if ui.add(b).on_hover_text("A new version of ChatTrace Forensic is available").clicked() {
                self.dialog = true;
            }
        }
    }

    pub fn show_dialog(&mut self, ctx: &egui::Context) {
        if !self.dialog {
            return;
        }
        let Some(rel) = self.available.clone() else {
            self.dialog = false;
            return;
        };
        let mut close = false;
        let mut start = false;
        egui::Modal::new(egui::Id::unique("update")).show(ctx, |ui| {
            ui.set_width(520.0);
            ui.horizontal(|ui| {
                ui.label(theme::icon(ph::ARROW_CIRCLE_UP, 24.0, ACCENT));
                ui.label(theme::title(format!("ChatTrace Forensic {} is available", rel.version), 18.0));
            });
            ui.label(theme::muted(format!("You have version {}.", crate::VERSION)).size(12.5));
            if !rel.notes.trim().is_empty() {
                ui.add_space(4.0);
                egui::ScrollArea::vertical().max_height(180.0).show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(rel.notes.trim()).size(12.5)).wrap());
                });
            }
            ui.add_space(6.0);
            if let Some((_, status)) = &self.installing {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(status.lock().map(|s| s.clone()).unwrap_or_default());
                });
                ctx.request_repaint_after(std::time::Duration::from_millis(150));
            } else if let Some(p) = &self.installed {
                ui.label(theme::title(format!("{} Installed. Restart to use the new version.", ph::CHECK_CIRCLE), 14.0).color(GREEN));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if theme::primary(ui, "Restart now").clicked() {
                        update::relaunch(p);
                    }
                    if theme::secondary(ui, "Later").clicked() {
                        close = true;
                    }
                });
            } else {
                if let Some(e) = &self.error {
                    theme::banner(RED).show(ui, |ui| {
                        ui.add(egui::Label::new(RichText::new(format!("The update could not be installed: {e}")).size(12.5)).wrap());
                    });
                    ui.add_space(4.0);
                }
                ui.label(theme::muted(
                    "The download is installed only if its checksum is signed with the ChatTrace release key and, on macOS, \
                     the app carries the same Apple Developer ID. Open evidence is not affected.",
                ).size(12.0));
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if theme::primary(ui, &format!("{} Update now", ph::DOWNLOAD_SIMPLE)).clicked() {
                        start = true;
                    }
                    if ui.button("Release page").clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(&rel.page));
                    }
                    if ui.button("Skip this version").clicked() {
                        self.settings.skip_version = Some(rel.version.clone());
                        self.settings.save();
                        self.available = None;
                        close = true;
                    }
                    if ui.button("Later").clicked() {
                        close = true;
                    }
                });
            }
        });
        if start {
            let status = Arc::new(Mutex::new(String::from("Starting…")));
            let (tx, rx) = mpsc::channel();
            let (s, c, r) = (status.clone(), ctx.clone(), rel.clone());
            std::thread::spawn(move || {
                let res = update::install(&r, &|m| {
                    if let Ok(mut g) = s.lock() {
                        *g = m.to_string();
                    }
                    c.request_repaint();
                });
                let _ = tx.send(res.map_err(|e| format!("{e:#}")));
                c.request_repaint();
            });
            self.error = None;
            self.installing = Some((rx, status));
        }
        if close && self.installing.is_none() {
            self.dialog = false;
        }
    }

    /// Settings block for the About window.
    pub fn about_section(&mut self, ui: &mut egui::Ui) {
        theme::section(ui, "Updates");
        let mut on = self.settings.update_check;
        if ui.checkbox(&mut on, "Check for updates when ChatTrace starts").changed() {
            self.settings.update_check = on;
            self.settings.save();
        }
        ui.label(
            theme::muted(
                "One request to api.github.com for the latest release; no data about you or your cases is sent.",
            )
            .size(11.5),
        );
        ui.horizontal(|ui| {
            if ui.add_enabled(!self.busy(), egui::Button::new(format!("{} Check now", ph::ARROWS_CLOCKWISE))).clicked()
            {
                let ctx = ui.ctx().clone();
                self.check(&ctx, true);
            }
            if self.checking.is_some() {
                ui.spinner();
            }
            if let Some(s) = &self.status {
                ui.label(RichText::new(s).size(12.0).color(MUTED));
            }
        });
        if crate::crash::available() {
            ui.add_space(6.0);
            theme::section(ui, "Crash reports");
            let mut on = self.settings.crash_reports;
            if ui.checkbox(&mut on, "After a crash, offer to send a crash report").changed() {
                self.settings.crash_reports = on;
                self.settings.save();
            }
            ui.label(
                theme::muted(
                    "Nothing is sent without your click: the report is shown first. It has no chat content,                      paths, names or IDs.",
                )
                .size(11.5),
            );
        }
    }
}
