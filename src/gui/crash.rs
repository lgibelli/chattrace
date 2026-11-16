// SPDX-License-Identifier: GPL-3.0-or-later
//! "ChatTrace closed unexpectedly" dialog: shows saved crash reports and sends
//! them only when the user clicks Send (see crate::crash).

use super::theme::{self, AMBER, GREEN, RED, ph};
use crate::{crash, update::Settings};
use eframe::egui::{self, RichText};
use std::sync::mpsc;

pub struct CrashUi {
    reports: Vec<crash::Pending>,
    sending: Option<mpsc::Receiver<Result<(), String>>>,
    sent: bool,
    error: Option<String>,
}

impl CrashUi {
    pub fn new() -> Self {
        let reports = if crash::enabled() { crash::pending() } else { Vec::new() };
        CrashUi { reports, sending: None, sent: false, error: None }
    }

    pub fn busy(&self) -> bool {
        self.sending.is_some()
    }

    /// Show the dialog while there is something to show.
    pub fn show(&mut self, ctx: &egui::Context, settings: &mut Settings) {
        if let Some(rx) = &self.sending
            && let Ok(r) = rx.try_recv()
        {
            self.sending = None;
            match r {
                Ok(()) => {
                    self.reports.clear();
                    self.sent = true;
                }
                Err(e) => self.error = Some(e),
            }
        }
        if self.reports.is_empty() && !self.sent {
            return;
        }
        let mut close = false;
        let mut send = false;
        let mut discard = false;
        let mut never = false;
        egui::Modal::new(egui::Id::unique("crash-report")).show(ctx, |ui| {
            ui.set_width(600.0);
            if self.sent {
                ui.horizontal(|ui| {
                    ui.label(theme::icon(ph::CHECK_CIRCLE, 24.0, GREEN));
                    ui.label(theme::title("Crash report sent. Thank you!", 18.0));
                });
                ui.add_space(8.0);
                close = theme::primary(ui, "Close").clicked();
                return;
            }
            ui.horizontal(|ui| {
                ui.label(theme::icon(ph::WARNING_CIRCLE, 24.0, AMBER));
                ui.label(theme::title("ChatTrace closed unexpectedly", 18.0));
            });
            ui.add_space(2.0);
            ui.add(
                egui::Label::new(
                    RichText::new(
                        "A crash report was saved on this computer. You can send it to the developer to help fix \
                         the problem. It contains only what is shown below: the version, your operating system and \
                         where in ChatTrace's code the crash happened. Quoted text, file paths, e-mail addresses and \
                         IDs were removed; there is nothing about your cases or chats, and no computer or user name.",
                    )
                    .size(12.5),
                )
                .wrap(),
            );
            ui.add_space(6.0);
            let mut text = self.reports.iter().map(|r| r.preview()).collect::<Vec<_>>().join("\n\n");
            egui::ScrollArea::vertical().max_height(260.0).show(ui, |ui| {
                ui.add(
                    egui::TextEdit::multiline(&mut text)
                        .font(egui::TextStyle::Monospace)
                        .desired_width(f32::INFINITY)
                        .interactive(true),
                );
            });
            if let Some(e) = &self.error {
                ui.add_space(4.0);
                theme::banner(RED).show(ui, |ui| {
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("{e}. The report is kept; try again later.")).size(12.5),
                        )
                        .wrap(),
                    );
                });
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if self.sending.is_some() {
                    ui.spinner();
                    ui.label("Sending…");
                    return;
                }
                let n = self.reports.len();
                let label = if n > 1 {
                    format!("{} Send {n} reports", ph::EXPORT)
                } else {
                    format!("{} Send report", ph::EXPORT)
                };
                send = theme::primary(ui, &label).clicked();
                discard = theme::secondary(ui, "Don't send").clicked();
                never = ui
                    .button("Never ask")
                    .on_hover_text("Delete this report and don't save crash reports again (can be changed in About)")
                    .clicked();
            });
            ui.add_space(2.0);
            ui.label(theme::muted("Sent to the developer's Sentry account (sentry.io) over HTTPS.").size(11.5));
        });
        if send {
            let reports = self.reports.clone();
            let (tx, rx) = mpsc::channel();
            let c = ctx.clone();
            self.error = None;
            std::thread::spawn(move || {
                let r = reports.iter().try_for_each(crash::send).map_err(|e| format!("{e:#}"));
                let _ = tx.send(r);
                c.request_repaint();
            });
            self.sending = Some(rx);
        }
        if discard || never {
            crash::discard(&self.reports);
            self.reports.clear();
        }
        if never {
            settings.crash_reports = false;
            settings.save();
        }
        if close {
            self.sent = false;
        }
    }
}
