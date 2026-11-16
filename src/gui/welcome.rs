// SPDX-License-Identifier: GPL-3.0-or-later
//! Splash screen and the welcome / choose-a-source screen.

use super::theme::{self, ACCENT, AMBER, BAR, BG, BORDER, MUTED, TEXT, ph};
use super::{Action, App};
use crate::discover::{Found, human_bytes};
use eframe::egui::{self, Align2, Color32, FontId, Margin, RichText, Stroke, vec2};

const SPLASH_SECS: f64 = 2.4;

fn ease(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

fn mark() -> egui::Image<'static> {
    egui::Image::new(egui::include_image!("../../assets/mark.svg"))
}

/// Draws the splash while it is active; returns false once it is over.
pub fn splash(ui: &mut egui::Ui, t: f64) -> bool {
    // CHATTRACE_SPLASH_HOLD=<seconds> freezes the splash at that frame (screenshots).
    let hold = std::env::var("CHATTRACE_SPLASH_HOLD").ok().and_then(|s| s.parse::<f64>().ok());
    let t = hold.unwrap_or(t);
    let skip = ui.ctx().input(|i| i.pointer.any_click() || !i.keys_down.is_empty());
    let id = egui::Id::unique("splash-done");
    let done = ui.ctx().data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    if hold.is_none() && (done || t > SPLASH_SECS || (skip && t > 0.2)) {
        ui.ctx().data_mut(|d| d.insert_temp(id, true));
        return false;
    }
    ui.ctx().request_repaint();
    let t = t as f32;
    egui::CentralPanel::default().frame(egui::Frame::new().fill(BAR)).show(ui, |ui| {
        let rect = ui.max_rect();
        let p = ui.painter().clone();
        let c = rect.center() - vec2(0.0, 40.0);
        let fade_out = 1.0 - ((t - (SPLASH_SECS as f32 - 0.35)) / 0.35).clamp(0.0, 1.0);
        let a = ease(t / 0.6) * fade_out;
        theme::ridges(&p, c + vec2(0.0, 30.0), 420.0 * (0.9 + 0.1 * ease(t / 1.2)), ACCENT.gamma_multiply(0.05 * a));
        let s = 150.0 * (0.86 + 0.14 * ease(t / 0.7));
        let r = egui::Rect::from_center_size(c - vec2(0.0, 40.0), vec2(s, s));
        mark().tint(Color32::WHITE.gamma_multiply(a)).paint_at(ui, r);
        let ta = ease((t - 0.35) / 0.5) * fade_out;
        p.text(
            c + vec2(0.0, 70.0),
            Align2::CENTER_CENTER,
            "ChatTrace Forensic",
            FontId::new(34.0, theme::semibold()),
            TEXT.gamma_multiply(ta),
        );
        p.text(
            c + vec2(0.0, 108.0),
            Align2::CENTER_CENTER,
            "Read-only evidence capture from the Microsoft Teams desktop cache",
            FontId::proportional(15.0),
            MUTED.gamma_multiply(ta),
        );
        // A SHA-256 being "computed": hex digits typing out under a sweeping bar.
        let hex = "9f2c4e81a07d53b6e2f1c8a94d0b7e35";
        let k = (((t - 0.6) / 1.2).clamp(0.0, 1.0) * hex.len() as f32) as usize;
        let ha = ease((t - 0.6) / 0.3) * fade_out;
        p.text(
            c + vec2(0.0, 150.0),
            Align2::CENTER_CENTER,
            format!("sha256  {}{}", &hex[..k], "·".repeat(hex.len() - k)),
            FontId::monospace(13.0),
            ACCENT.gamma_multiply(0.85 * ha),
        );
        let w = 320.0;
        let bar = egui::Rect::from_min_size(c + vec2(-w / 2.0, 172.0), vec2(w, 2.0));
        p.rect_filled(bar, 1.0, BORDER.gamma_multiply(ha));
        let f = ((t - 0.6) / 1.4).clamp(0.0, 1.0);
        p.rect_filled(egui::Rect::from_min_size(bar.min, vec2(w * f, 2.0)), 1.0, ACCENT.gamma_multiply(ha));
        p.text(
            rect.center_bottom() - vec2(0.0, 28.0),
            Align2::CENTER_CENTER,
            format!("v{} · free software (GPL-3.0-or-later) · not affiliated with Microsoft", crate::VERSION),
            FontId::proportional(12.0),
            MUTED.gamma_multiply(0.8 * ta),
        );
    });
    true
}

fn feature(ui: &mut egui::Ui, glyph: &str, head: &str, body: &str) {
    ui.horizontal_top(|ui| {
        egui::Frame::new().fill(ACCENT.gamma_multiply(0.12)).corner_radius(10).inner_margin(Margin::same(8)).show(
            ui,
            |ui| {
                ui.label(theme::icon(glyph, 20.0, ACCENT));
            },
        );
        ui.vertical(|ui| {
            ui.label(theme::title(head, 15.0));
            ui.add(egui::Label::new(theme::muted(body)).wrap());
        });
    });
    ui.add_space(10.0);
}

fn found_row(ui: &mut egui::Ui, f: &Found, actions: &mut Vec<Action>, busy: bool) {
    egui::Frame::new()
        .fill(theme::SURFACE2)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(10)
        .inner_margin(Margin::symmetric(12, 10))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let glyph = if f.live_local { ph::DESKTOP } else { ph::HARD_DRIVES };
                ui.label(theme::icon(glyph, 22.0, ACCENT));
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(theme::title(f.kind.label(), 14.0));
                        if let Some(u) = &f.os_user {
                            theme::chip(ui, format!("user {u}"), MUTED);
                        }
                        if let Some(l) = f.layout {
                            theme::chip(ui, l, MUTED);
                        }
                    });
                    let newest = f
                        .newest
                        .map(|t| chrono::DateTime::<chrono::Local>::from(t).format("%-d %b %Y %H:%M").to_string())
                        .unwrap_or_default();
                    ui.label(
                        theme::muted(format!("{} · {} files · modified {newest}", human_bytes(f.bytes), f.files))
                            .size(12.0),
                    );
                    ui.add(
                        egui::Label::new(
                            RichText::new(f.leveldb.display().to_string()).monospace().size(11.0).color(MUTED),
                        )
                        .truncate(),
                    )
                    .on_hover_text(f.leveldb.display().to_string());
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if theme::primary_enabled(ui, !busy, "Open").clicked() {
                        actions.push(Action::Open(f.clone()));
                    }
                });
            });
        });
    ui.add_space(6.0);
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let running = !app.local.is_empty() && app.teams_running();
    let busy = app.load.is_some();
    egui::CentralPanel::default().frame(egui::Frame::new().fill(BG).inner_margin(Margin::same(0))).show(ui, |ui| {
        let full = ui.max_rect();
        theme::ridges(ui.painter(), full.right_top() + vec2(-120.0, 120.0), 520.0, ACCENT.gamma_multiply(0.035));
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            let w = ui.available_width().min(1160.0);
            let pad = ((ui.available_width() - w) / 2.0).max(24.0);
            ui.add_space(36.0);
            ui.horizontal_top(|ui| {
                ui.add_space(pad);
                ui.vertical(|ui| {
                    ui.set_max_width(w);
                    ui.horizontal(|ui| {
                        ui.add(mark().fit_to_exact_size(vec2(64.0, 64.0)));
                        ui.add_space(6.0);
                        ui.vertical(|ui| {
                            ui.label(theme::title("ChatTrace Forensic", 30.0));
                            ui.label(
                                theme::muted("Read-only evidence capture from the Microsoft Teams desktop cache")
                                    .size(15.0),
                            );
                        });
                    });
                    ui.add_space(22.0);
                    ui.columns(2, |cols| {
                        let top = egui::Layout::top_down(egui::Align::Min);
                        cols[0].with_layout(top, left_column);
                        cols[1].with_layout(top, |ui| right_column(app, ui, running, busy));
                    });
                    ui.add_space(18.0);
                    ui.separator();
                    ui.horizontal(|ui| {
                        ui.label(theme::muted(format!(
                        "Version {} · Free software under the GNU GPL v3 or later · Not affiliated with Microsoft",
                        crate::VERSION
                    )).size(12.0));
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.link("About").clicked() {
                                app.actions.push(Action::About);
                            }
                            app.updater.badge(ui);
                            ui.hyperlink_to("Source code", crate::REPOSITORY);
                        });
                    });
                    ui.add_space(24.0);
                });
            });
        });
    });
}

fn left_column(ui: &mut egui::Ui) {
    ui.add_space(4.0);
    feature(
        ui,
        ph::CHATS_CIRCLE,
        "View what Teams cached",
        "Chats, meetings and channels the Teams desktop client stored on this computer, including earlier versions \
         of edited and deleted messages that are still in its database, with every timestamp and identifier.",
    );
    feature(
        ui,
        ph::SEAL_CHECK,
        "Preserve it as evidence",
        "One zip with a byte-for-byte copy of the original database files, the SHA-256 of every file, a manifest, \
         and readable CSV, HTML, JSON and text with all metadata, generated from the copy and verified after writing.",
    );
    feature(
        ui,
        ph::LOCK_SIMPLE,
        "Never modify the source",
        "Files are only opened for reading and decoded by ChatTrace's own parser; no database engine ever opens them. \
         Works on this computer, on an external disk, on a mounted disk image or on a copied folder.",
    );
    ui.add_space(6.0);
    theme::banner(ACCENT).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(theme::icon(ph::SCALES, 22.0, ACCENT));
            ui.label(theme::title("Use Microsoft Purview whenever possible", 15.0));
        });
        ui.add(
            egui::Label::new(RichText::new(
                "If your organization can run Microsoft Purview eDiscovery or Content search, use it: it exports the \
                 server-side copy of every conversation, complete and with Microsoft's own chain of custody.\n\n\
                 ChatTrace is for when that is not available, for example when you are not an administrator of the \
                 Microsoft 365 tenant, when the tenant can no longer be accessed, or when you examine a computer or a \
                 disk image. It only contains what one Teams client had cached.",
            ).color(TEXT).size(13.0))
            .wrap(),
        );
    });
}

fn right_column(app: &mut App, ui: &mut egui::Ui, running: bool, busy: bool) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(theme::title("Open a Teams cache", 18.0));
        ui.add_space(4.0);

        if let Some(l) = &app.load {
            theme::banner(ACCENT).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.vertical(|ui| {
                        ui.label(theme::title("Reading the cache (read-only)…", 14.0));
                        ui.label(theme::muted(format!("{:.0} s · {}", l.started.elapsed().as_secs_f32(), l.found.leveldb.display())).size(11.5));
                    });
                });
            });
            ui.add_space(6.0);
        }
        if let Some(e) = &app.error {
            theme::banner(theme::RED).show(ui, |ui| {
                ui.label(theme::title("Could not read this cache", 14.0).color(theme::RED));
                ui.add(egui::Label::new(RichText::new(e).size(12.5)).wrap());
                ui.label(theme::muted(
                    "On macOS, allow access when asked, or give ChatTrace Forensic Full Disk Access in System Settings › \
                     Privacy & Security. On an external disk, check that it is mounted and readable.",
                ).size(12.0));
            });
            ui.add_space(6.0);
        }

        theme::section(ui, "On this computer");
        if !app.local_blocked.is_empty() {
            access_banner(ui, "macOS is blocking access to Teams' data", &mut app.actions);
            ui.add_space(4.0);
        }
        if app.local.is_empty() {
            if app.local_blocked.is_empty() {
                ui.label(theme::muted("No Teams cache found for this computer's user."));
            }
        } else {
            if running {
                theme::banner(AMBER).show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        ui.label(theme::icon(ph::WARNING, 16.0, AMBER));
                        ui.label(RichText::new(
                            "Microsoft Teams is running. Quit it completely before exporting, so the files don't change while they are copied.",
                        ).size(12.5));
                    });
                });
                ui.add_space(4.0);
            }
            let local = app.local.clone();
            for f in &local {
                found_row(ui, f, &mut app.actions, busy);
            }
        }

        ui.add_space(8.0);
        theme::section(ui, "On another disk or folder");
        ui.label(theme::muted("An external disk, a mounted disk image, another user's folder or a copy. ChatTrace searches it for Teams caches.").size(12.5));
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            if theme::secondary(ui, &format!("{}  Choose folder…", ph::FOLDER_OPEN)).clicked() {
                app.actions.push(Action::ChooseFolder);
            }
            let resp = ui.add(
                egui::TextEdit::singleline(&mut app.path_input)
                    .hint_text("…or type or paste a path, or drop a folder here")
                    .desired_width(ui.available_width() - 80.0),
            );
            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
            if (theme::secondary(ui, "Search").clicked() || enter) && !app.path_input.trim().is_empty() {
                app.actions.push(Action::Scan(app.path_input.trim().into(), false));
            }
        });
        if let Some(s) = &app.scan {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(theme::muted(format!(
                    "Searching {} · {} folders",
                    s.root.display(),
                    s.progress.load(std::sync::atomic::Ordering::Relaxed)
                )).size(12.5));
                if ui.link("Cancel").clicked() {
                    app.actions.push(Action::CancelScan);
                }
            });
        }
        if let Some((root, v)) = &app.scan_result {
            ui.add_space(6.0);
            if v.is_empty() && app.scan_denied {
                let title = format!("macOS did not allow ChatTrace to read {}", root.display());
                access_banner(ui, &title, &mut app.actions);
            } else if v.is_empty() {
                ui.label(theme::muted(format!("No Teams cache found under {}.", root.display())));
            } else {
                ui.label(theme::muted(format!("{} found under {}", v.len(), root.display())).size(12.5));
                let v = v.clone();
                for f in &v {
                    found_row(ui, f, &mut app.actions, busy);
                }
            }
        }

        ui.add_space(10.0);
        ui.separator();
        ui.horizontal(|ui| {
            if ui.link(format!("{} Verify an evidence package…", ph::SEAL_CHECK)).clicked() {
                app.actions.push(Action::Verify);
            }
            ui.label(theme::muted("·"));
            if ui.link(format!("{} Explore the demo data", ph::PLAY_CIRCLE)).clicked() {
                app.actions.push(Action::Demo);
            }
        });
    });
}

/// The OS refused access: say so plainly, and how to allow it.
fn access_banner(ui: &mut egui::Ui, title: &str, actions: &mut Vec<Action>) {
    theme::banner(AMBER).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(theme::icon(ph::LOCK_SIMPLE, 18.0, AMBER));
            ui.label(theme::title(title, 14.0).color(AMBER));
        });
        ui.add(egui::Label::new(RichText::new(crate::discover::access_help(true)).size(12.5)).wrap());
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if cfg!(target_os = "macos")
                && theme::primary(ui, &format!("{}  Open Full Disk Access settings", ph::GEAR_SIX)).clicked()
            {
                crate::discover::open_privacy_settings();
            }
            if theme::secondary(ui, &format!("{}  Check again", ph::ARROWS_CLOCKWISE)).clicked() {
                actions.push(Action::Welcome);
            }
        });
    });
}
