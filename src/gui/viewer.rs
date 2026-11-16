// SPDX-License-Identifier: GPL-3.0-or-later
//! The viewer: top bar, conversation list, messages, details panel, status bar.

use super::theme::{self, ACCENT, AMBER, BAR, BG, BLUE, BORDER, GREEN, MUTED, RED, SURFACE, SURFACE2, TEXT, ph};
use super::{Action, App, Loaded, ViewState};
use crate::evidence;
use crate::teams::{Conversation, Message, Variant, local, utc};
use chrono::{DateTime, Local};
use eframe::egui::{self, Margin, RichText, Sense, Stroke, vec2};

fn dt(ms: Option<f64>) -> Option<DateTime<Local>> {
    ms.and_then(|m| DateTime::from_timestamp_millis(m as i64)).map(|d| d.with_timezone(&Local))
}

fn short_date(ms: Option<f64>) -> String {
    let Some(d) = dt(ms) else { return String::new() };
    let now = Local::now();
    if d.date_naive() == now.date_naive() {
        d.format("%H:%M").to_string()
    } else if d.format("%Y").to_string() == now.format("%Y").to_string() {
        d.format("%-d %b").to_string()
    } else {
        d.format("%-d %b %Y").to_string()
    }
}

fn kind_icon(kind: &str) -> &'static str {
    let k = kind.to_lowercase();
    if k.contains("meeting") {
        ph::CALENDAR_BLANK
    } else if k.contains("channel") || k.contains("team") {
        ph::HASH
    } else if k.contains("1:1") || k.contains("one") {
        ph::USER
    } else {
        ph::USERS
    }
}

pub fn show(app: &mut App, ui: &mut egui::Ui) {
    let live = app.data.as_ref().and_then(|d| d.source.as_ref()).is_some_and(|s| s.live_local);
    let running = live && app.teams_running();
    let loading = app.load.is_some();
    let App { data, view, actions, error, updater, .. } = app;
    let Some(d) = data.as_ref() else { return };

    top_bar(ui, d, actions, loading, updater);
    if running {
        egui::Panel::top("teams-running").frame(egui::Frame::new().fill(AMBER.gamma_multiply(0.16)).inner_margin(Margin::symmetric(14, 6))).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(theme::icon(ph::WARNING, 16.0, AMBER));
                ui.label(RichText::new(
                    "Microsoft Teams is running on this computer: it may still be writing to its cache. Quit Teams completely before exporting.",
                ).size(12.5));
            });
        });
    }
    if let Some(e) = error.as_ref() {
        let mut close = false;
        egui::Panel::top("reload-error")
            .frame(egui::Frame::new().fill(RED.gamma_multiply(0.16)).inner_margin(Margin::symmetric(14, 6)))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(theme::icon(ph::WARNING_CIRCLE, 16.0, RED));
                    ui.label(RichText::new(format!("Reload failed: {e}")).size(12.5));
                    close = ui.link("Dismiss").clicked();
                });
            });
        if close {
            *error = None;
        }
    }
    status_bar(ui, d);
    conversation_list(ui, d, view);
    if view.details {
        details_panel(ui, d, view);
    }
    messages(ui, d, view);
}

fn top_bar(
    ui: &mut egui::Ui,
    d: &Loaded,
    actions: &mut Vec<Action>,
    loading: bool,
    updater: &mut super::updates::Updater,
) {
    egui::Panel::top("top")
        .frame(egui::Frame::new().fill(BAR).inner_margin(Margin::symmetric(14, 9)).stroke(Stroke::new(1.0, BORDER)))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add(
                    egui::Image::new(egui::include_image!("../../assets/mark.svg")).fit_to_exact_size(vec2(28.0, 28.0)),
                );
                ui.label(theme::title("ChatTrace Forensic", 16.0));
                ui.add_space(10.0);
                if d.demo {
                    theme::chip(ui, format!("{} Demo data: fictional people, no real data", ph::PLAY_CIRCLE), BLUE);
                } else if let Some(s) = &d.source {
                    egui::Frame::new().fill(SURFACE).corner_radius(8).inner_margin(Margin::symmetric(8, 3)).show(
                        ui,
                        |ui| {
                            ui.horizontal(|ui| {
                                ui.label(theme::icon(
                                    if s.live_local { ph::DESKTOP } else { ph::HARD_DRIVES },
                                    15.0,
                                    ACCENT,
                                ));
                                ui.label(RichText::new(s.kind.label()).size(12.5));
                                let short: String = {
                                    let p = s.leveldb.display().to_string();
                                    let n = p.chars().count();
                                    if n > 56 {
                                        format!("…{}", p.chars().skip(n - 55).collect::<String>())
                                    } else {
                                        p
                                    }
                                };
                                ui.label(RichText::new(short).monospace().size(11.0).color(MUTED))
                                    .on_hover_text(s.leveldb.display().to_string());
                            });
                        },
                    );
                }
                theme::chip(ui, format!("{} read-only", ph::LOCK_SIMPLE), GREEN).on_hover_text(
                    "The source is never written to: files are opened read-only and decoded by ChatTrace's own parser.",
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(RichText::new(ph::INFO).size(16.0)).on_hover_text("About ChatTrace Forensic").clicked()
                    {
                        actions.push(Action::About);
                    }
                    if !d.demo
                        && ui
                            .add_enabled(!loading, egui::Button::new(format!("{} Reload", ph::ARROWS_CLOCKWISE)))
                            .on_hover_text("Read the cache again")
                            .clicked()
                    {
                        actions.push(Action::Reload);
                    }
                    if ui.button(format!("{} Change source", ph::FOLDER_OPEN)).clicked() {
                        actions.push(Action::Welcome);
                    }
                    if ui
                        .button(format!("{} Verify…", ph::SEAL_CHECK))
                        .on_hover_text("Check an evidence package against its checksums")
                        .clicked()
                    {
                        actions.push(Action::Verify);
                    }
                    let export = ui
                        .add_enabled_ui(!d.demo && !loading, |ui| {
                            theme::primary(ui, &format!("{} Export evidence…", ph::EXPORT))
                        })
                        .inner;
                    if export.clicked() {
                        actions.push(Action::Export);
                    }
                    updater.badge(ui);
                    if loading {
                        ui.spinner();
                    }
                });
            });
        });
}

fn status_bar(ui: &mut egui::Ui, d: &Loaded) {
    egui::Panel::bottom("status")
        .frame(egui::Frame::new().fill(BAR).inner_margin(Margin::symmetric(14, 5)).stroke(Stroke::new(1.0, BORDER)))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let ds = &d.ds;
                let latest = d.latest_count();
                let crc: usize = ds.catalog.files.iter().map(|f| f.crc_errors).sum();
                ui.label(theme::icon(ph::LOCK_SIMPLE, 13.0, GREEN));
                ui.label(theme::muted("Read-only").size(12.0));
                ui.label(theme::muted("·").size(12.0));
                ui.label(theme::muted(format!(
                    "{} accounts · {} conversations · {} messages · {} earlier versions · {} people · {} database files",
                    ds.accounts.len(),
                    ds.conversations.len(),
                    latest,
                    ds.messages.len() - latest,
                    ds.profiles.len(),
                    ds.catalog.files.len()
                )).size(12.0));
                if crc > 0 {
                    ui.label(RichText::new(format!("{crc} CRC errors")).size(12.0).color(RED));
                }
                if !ds.decode_errors.is_empty() {
                    ui.label(RichText::new(format!("{} undecodable records", ds.decode_errors.len())).size(12.0).color(AMBER));
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(theme::muted(format!("ChatTrace Forensic {}", crate::VERSION)).size(12.0));
                    if d.secs > 0.0 {
                        ui.label(theme::muted(format!("parsed in {:.1} s ·", d.secs)).size(12.0));
                    }
                });
            });
        });
}

fn conversation_list(ui: &mut egui::Ui, d: &Loaded, view: &mut ViewState) {
    egui::Panel::left("conversations")
        .resizable(true)
        .default_size(330.0)
        .min_size(250.0)
        .frame(egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(10, 10)).stroke(Stroke::new(1.0, BORDER)))
        .show(ui, |ui| {
            let ds = &d.ds;
            if ds.accounts.len() > 1 {
                let label = |k: &Option<String>| match k {
                    None => "All accounts".to_string(),
                    Some(k) => ds.account(k).map(|a| a.label()).unwrap_or_else(|| k.clone()),
                };
                egui::ComboBox::from_id_salt("account")
                    .width(ui.available_width())
                    .selected_text(format!("{} {}", ph::USER_CIRCLE, label(&view.account)))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut view.account, None, "All accounts");
                        for a in &ds.accounts {
                            ui.selectable_value(&mut view.account, Some(a.key.clone()), a.label());
                        }
                    });
                ui.add_space(4.0);
            } else if let Some(a) = ds.accounts.first() {
                ui.horizontal(|ui| {
                    ui.label(theme::icon(ph::USER_CIRCLE, 16.0, MUTED));
                    ui.add(egui::Label::new(theme::muted(a.label()).size(12.5)).truncate());
                });
            }
            ui.add(
                egui::TextEdit::singleline(&mut view.conv_filter)
                    .hint_text(format!("{}  Filter conversations", ph::MAGNIFYING_GLASS))
                    .desired_width(f32::INFINITY),
            );
            ui.add_space(4.0);
            let f = view.conv_filter.to_lowercase();
            let list: Vec<&Conversation> = ds
                .conversations
                .iter()
                .filter(|c| view.account.as_ref().is_none_or(|a| &c.account == a))
                .filter(|c| {
                    f.is_empty()
                        || c.title.to_lowercase().contains(&f)
                        || c.id.to_lowercase().contains(&f)
                        || c.members.iter().any(|m| m.name.to_lowercase().contains(&f))
                })
                .collect();
            ui.label(theme::muted(format!("{} conversations", list.len())).size(11.5));
            let row_h = 54.0;
            egui::ScrollArea::vertical().id_salt("convlist").auto_shrink([false, false]).show_rows(
                ui,
                row_h,
                list.len(),
                |ui, range| {
                    for c in &list[range] {
                        let key = (c.account.clone(), c.id.clone());
                        let selected = view.conv.as_ref() == Some(&key);
                        let (rect, resp) =
                            ui.allocate_exact_size(vec2(ui.available_width(), row_h - 2.0), Sense::click());
                        let p = ui.painter();
                        if selected {
                            p.rect_filled(rect, 10.0, SURFACE2);
                            p.rect_filled(
                                egui::Rect::from_min_size(rect.min + vec2(0.0, 10.0), vec2(3.0, rect.height() - 20.0)),
                                2.0,
                                ACCENT,
                            );
                        } else if resp.hovered() {
                            p.rect_filled(rect, 10.0, SURFACE);
                        }
                        let ac = rect.left_center() + vec2(26.0, 0.0);
                        p.circle_filled(ac, 17.0, theme::avatar_color(&c.id));
                        p.text(
                            ac,
                            egui::Align2::CENTER_CENTER,
                            kind_icon(&c.kind),
                            egui::FontId::proportional(16.0),
                            egui::Color32::WHITE,
                        );
                        let x = rect.left() + 52.0;
                        let date = short_date(c.last_activity);
                        let date_w = p
                            .text(
                                rect.right_top() + vec2(-8.0, 10.0),
                                egui::Align2::RIGHT_TOP,
                                &date,
                                egui::FontId::proportional(11.5),
                                MUTED,
                            )
                            .width();
                        let title_w = rect.right() - 16.0 - date_w - x;
                        let galley = ui.fonts_mut(|fo| {
                            let mut job = egui::text::LayoutJob::simple_singleline(
                                c.title.clone(),
                                egui::FontId::new(13.5, theme::semibold()),
                                TEXT,
                            );
                            job.wrap = egui::text::TextWrapping::truncate_at_width(title_w.max(40.0));
                            fo.layout_job(job)
                        });
                        p.galley(egui::pos2(x, rect.top() + 8.0), galley, TEXT);
                        let mut sub = format!("{} · {} messages", c.kind, c.message_count);
                        if !c.cache_purges.is_empty() {
                            sub = format!("{} {sub}", ph::WARNING);
                        }
                        p.text(
                            egui::pos2(x, rect.top() + 29.0),
                            egui::Align2::LEFT_TOP,
                            sub,
                            egui::FontId::proportional(11.5),
                            if c.cache_purges.is_empty() { MUTED } else { AMBER },
                        );
                        if resp.clicked() {
                            view.conv = Some(key);
                            view.msg = None;
                            view.raw_cache = None;
                            view.details_tab_conv = false;
                            view.scroll_top = true;
                        }
                    }
                },
            );
        });
}

fn visible<'a>(d: &'a Loaded, view: &ViewState) -> (Option<&'a Conversation>, Vec<usize>, usize) {
    let Some((acc, id)) = &view.conv else { return (None, vec![], 0) };
    let conv = d.ds.conversation(acc, id);
    let all = d.by_conv.get(&(acc.clone(), id.clone())).cloned().unwrap_or_default();
    let total = all.iter().filter(|&&i| d.ds.messages[i].variant == Variant::Latest).count();
    let q = view.search.to_lowercase();
    let v = all
        .into_iter()
        .filter(|&i| {
            let m = &d.ds.messages[i];
            (view.show_system || !m.is_system())
                && (m.variant == Variant::Latest || (view.show_earlier && (m.substantive || view.show_meta_only)))
                && (q.is_empty()
                    || m.text.to_lowercase().contains(&q)
                    || m.sender_name.to_lowercase().contains(&q)
                    || m.id.contains(&q))
        })
        .collect();
    (conv, v, total)
}

fn messages(ui: &mut egui::Ui, d: &Loaded, view: &mut ViewState) {
    egui::CentralPanel::default().frame(egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(18, 12))).show(ui, |ui| {
        let (conv, list, total) = visible(d, view);
        let Some(c) = conv else {
            ui.centered_and_justified(|ui| ui.label(theme::muted("Choose a conversation on the left.")));
            return;
        };
        // Header
        ui.horizontal(|ui| {
            ui.label(theme::icon(kind_icon(&c.kind), 22.0, ACCENT));
            ui.add(egui::Label::new(theme::title(&c.title, 19.0)).truncate());
        });
        let acc = d.ds.account(&c.account).map(|a| a.label()).unwrap_or_default();
        ui.add(egui::Label::new(theme::muted(format!("{} · {} members · {} messages · {}", c.kind, c.members.len(), c.message_count, acc)).size(12.0)).truncate());
        if !c.cache_purges.is_empty() {
            ui.add_space(4.0);
            theme::banner(AMBER).show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(theme::icon(ph::WARNING, 15.0, AMBER));
                    ui.label(RichText::new(format!(
                        "Teams recorded {} cache purge(s) for this conversation: older messages may be missing from this computer. See Conversation info.",
                        c.cache_purges.len()
                    )).size(12.5));
                });
            });
        }
        ui.add_space(6.0);
        // Toolbar
        ui.horizontal_wrapped(|ui| {
            ui.add(egui::TextEdit::singleline(&mut view.search).hint_text(format!("{}  Search in this conversation", ph::MAGNIFYING_GLASS)).desired_width(220.0));
            ui.toggle_value(&mut view.show_system, "System events");
            ui.toggle_value(&mut view.show_earlier, "Earlier versions")
                .on_hover_text("Older states of edited or deleted messages recovered from the database");
            if view.show_earlier {
                ui.toggle_value(&mut view.show_meta_only, "Metadata-only")
                    .on_hover_text("Also show earlier versions that differ only in internal metadata");
            }
            ui.separator();
            if ui.button(format!("{} Copy transcript", ph::COPY)).on_hover_text("Plain-text transcript of this conversation").clicked() {
                ui.ctx().copy_text(evidence::transcript(&d.ds, c));
            }
            if ui.button(ph::INFO).on_hover_text("Conversation info: members, cache purges, source record").clicked() {
                view.details = true;
                view.details_tab_conv = true;
            }
            if ui.selectable_label(view.details, ph::SIDEBAR_SIMPLE).on_hover_text("Show or hide the details panel").clicked() {
                view.details = !view.details;
            }
        });
        ui.label(theme::muted(format!("{} of {} messages shown", list.iter().filter(|&&i| d.ds.messages[i].variant == Variant::Latest).count(), total)).size(11.5));
        ui.add_space(2.0);
        let mut sa = egui::ScrollArea::vertical().id_salt(("msgs", &c.id)).auto_shrink([false, false]);
        if view.scroll_top {
            sa = sa.vertical_scroll_offset(0.0);
            view.scroll_top = false;
        }
        sa.show(ui, |ui| {
            if list.is_empty() {
                ui.add_space(30.0);
                ui.vertical_centered(|ui| ui.label(theme::muted("No messages match.")));
            }
            let mut last_day = String::new();
            for &i in &list {
                let m = &d.ds.messages[i];
                let day = dt(m.arrival).map(|x| x.format("%A, %-d %B %Y").to_string()).unwrap_or_else(|| "Unknown date".into());
                if day != last_day && m.variant == Variant::Latest {
                    ui.add_space(8.0);
                    ui.vertical_centered(|ui| {
                        egui::Frame::new().fill(SURFACE2).corner_radius(99).inner_margin(Margin::symmetric(12, 3)).show(ui, |ui| {
                            ui.label(RichText::new(&day).size(11.5).color(MUTED).family(theme::semibold()));
                        });
                    });
                    ui.add_space(4.0);
                    last_day = day;
                }
                if message_card(ui, m, view.msg == Some(i)) {
                    view.msg = Some(i);
                    view.details_tab_conv = false;
                    view.details = true;
                }
            }
            ui.add_space(20.0);
        });
    });
}

/// Returns true when clicked.
fn message_card(ui: &mut egui::Ui, m: &Message, selected: bool) -> bool {
    let earlier = m.variant == Variant::Earlier;
    let system = m.is_system();
    let indent = if earlier { 44.0 } else { 0.0 };
    let mut clicked = false;
    ui.horizontal_top(|ui| {
        ui.add_space(indent);
        let stroke = if selected {
            Stroke::new(1.5, ACCENT)
        } else if earlier {
            Stroke::new(1.0, AMBER.gamma_multiply(0.5))
        } else {
            Stroke::new(1.0, BORDER)
        };
        let fill = if earlier {
            AMBER.gamma_multiply(0.06)
        } else if m.deleted.is_some() {
            RED.gamma_multiply(0.06)
        } else if system {
            BG
        } else {
            SURFACE
        };
        let r = egui::Frame::new()
            .fill(fill)
            .stroke(stroke)
            .corner_radius(10)
            .inner_margin(Margin::symmetric(12, 9))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal_top(|ui| {
                    theme::avatar(ui, &m.sender_name, &m.sender_mri, 30.0, system);
                    ui.vertical(|ui| {
                        ui.horizontal_wrapped(|ui| {
                            if earlier {
                                ui.label(
                                    RichText::new(format!("{} Earlier version", ph::CLOCK_COUNTER_CLOCKWISE))
                                        .color(AMBER)
                                        .family(theme::semibold())
                                        .size(13.0),
                                );
                                ui.label(theme::muted(format!("differs in {}", m.changes.join(", "))).size(12.0));
                            } else {
                                ui.label(theme::title(&m.sender_name, 13.5));
                                ui.label(
                                    RichText::new(
                                        dt(m.arrival).map(|x| x.format("%H:%M:%S").to_string()).unwrap_or_default(),
                                    )
                                    .monospace()
                                    .size(11.5)
                                    .color(MUTED),
                                )
                                .on_hover_text(format!(
                                    "{}\n{} UTC",
                                    local(m.arrival),
                                    utc(m.arrival)
                                ));
                            }
                            if m.from_me && !earlier {
                                theme::chip(ui, "account owner", ACCENT);
                            }
                            if m.edited.is_some() {
                                theme::chip(
                                    ui,
                                    format!(
                                        "{} edited {}",
                                        ph::PENCIL_SIMPLE,
                                        dt(m.edited).map(|x| x.format("%-d %b %H:%M").to_string()).unwrap_or_default()
                                    ),
                                    BLUE,
                                );
                            }
                            if m.deleted.is_some() {
                                theme::chip(
                                    ui,
                                    format!(
                                        "{} deleted {}",
                                        ph::TRASH,
                                        dt(m.deleted).map(|x| x.format("%-d %b %H:%M").to_string()).unwrap_or_default()
                                    ),
                                    RED,
                                );
                            }
                            if m.importance.eq_ignore_ascii_case("high") || m.importance.eq_ignore_ascii_case("urgent")
                            {
                                theme::chip(ui, format!("{} {}", ph::WARNING_DIAMOND, m.importance), AMBER);
                            }
                            if !m.prov.crc_ok {
                                theme::chip(ui, "CRC failed", RED);
                            }
                            if m.versions_known > 1 && !earlier {
                                theme::chip(ui, format!("{} versions", m.versions_known), MUTED);
                            }
                        });
                        let text = RichText::new(&m.text).size(14.0);
                        let text =
                            if system || m.deleted.is_some() && !earlier { text.italics().color(MUTED) } else { text };
                        ui.add(egui::Label::new(text).wrap().selectable(true));
                        if !m.reactions.is_empty() || !m.files.is_empty() {
                            ui.horizontal_wrapped(|ui| {
                                let mut grouped: Vec<(String, Vec<String>)> = Vec::new();
                                for r in &m.reactions {
                                    match grouped.iter_mut().find(|g| g.0 == r.emoji) {
                                        Some(g) => g.1.push(r.name.clone()),
                                        None => grouped.push((r.emoji.clone(), vec![r.name.clone()])),
                                    }
                                }
                                for (e, who) in grouped {
                                    theme::chip(ui, format!("{} {}", theme::reaction_emoji(&e), who.len()), TEXT)
                                        .on_hover_text(who.join("\n"));
                                }
                                for f in &m.files {
                                    theme::chip(ui, format!("{} {}", ph::PAPERCLIP, f.name), BLUE)
                                        .on_hover_text(&f.url);
                                }
                            });
                        }
                    });
                });
            });
        clicked = r.response.interact(Sense::click()).clicked();
    });
    ui.add_space(5.0);
    clicked
}

fn details_panel(ui: &mut egui::Ui, d: &Loaded, view: &mut ViewState) {
    egui::Panel::right("details")
        .resizable(true)
        .default_size(400.0)
        .min_size(300.0)
        .frame(egui::Frame::new().fill(BG).inner_margin(Margin::symmetric(12, 10)).stroke(Stroke::new(1.0, BORDER)))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let has_msg = view.msg.is_some();
                if ui
                    .selectable_label(!view.details_tab_conv && has_msg, format!("{} Message", ph::CHAT_CIRCLE_TEXT))
                    .clicked()
                    && has_msg
                {
                    view.details_tab_conv = false;
                }
                if ui
                    .selectable_label(view.details_tab_conv || !has_msg, format!("{} Conversation", ph::CHATS_CIRCLE))
                    .clicked()
                {
                    view.details_tab_conv = true;
                }
            });
            ui.separator();
            egui::ScrollArea::vertical().id_salt("details").auto_shrink([false, false]).show(ui, |ui| match view.msg {
                Some(i) if !view.details_tab_conv => message_details(ui, d, i, view),
                _ => conversation_details(ui, d, view),
            });
        });
}

fn message_details(ui: &mut egui::Ui, d: &Loaded, i: usize, view: &mut ViewState) {
    let m = &d.ds.messages[i];
    theme::section(ui, "Message");
    theme::kv(
        ui,
        "m-msg",
        &[
            ("Message ID", m.id.clone()),
            ("Version", m.version.clone()),
            (
                "Version status",
                format!(
                    "{} of {} known",
                    if m.variant == Variant::Latest { "latest" } else { "earlier" },
                    m.versions_known
                ),
            ),
            ("Differs from latest in", m.changes.join(", ")),
            ("Message type", m.message_type.clone()),
            ("Content type", m.content_type.clone()),
            ("Importance", m.importance.clone()),
            ("Subject", m.subject.clone()),
            ("Reply chain ID", m.reply_chain_id.clone()),
            ("Client message ID", m.client_message_id.clone()),
            ("Sequence ID", m.sequence_id.map(|s| s.to_string()).unwrap_or_default()),
        ],
    );
    theme::section(ui, "Time");
    theme::kv(
        ui,
        "m-time",
        &[
            ("Arrival (UTC)", utc(m.arrival)),
            ("Arrival (local)", local(m.arrival)),
            ("Client arrival (UTC)", utc(m.client_arrival)),
            ("Compose time", m.compose_time.clone().unwrap_or_default()),
            ("Edited (UTC)", utc(m.edited)),
            ("Deleted (UTC)", utc(m.deleted)),
            ("Deleted by", m.deleted_by.clone().unwrap_or_default()),
        ],
    );
    theme::section(ui, "Sender");
    theme::kv(
        ui,
        "m-sender",
        &[
            ("Name", m.sender_name.clone()),
            ("MRI", m.sender_mri.clone()),
            ("Account owner", if m.from_me { "yes".into() } else { "no".into() }),
        ],
    );
    if !m.reactions.is_empty() || !m.mentions.is_empty() || !m.files.is_empty() {
        theme::section(ui, "Reactions, mentions, files");
        let rows: Vec<(String, String)> = m
            .reactions
            .iter()
            .map(|r| {
                (
                    format!("{} {}", theme::reaction_emoji(&r.emoji), r.emoji),
                    format!("{} · {} · {}", r.name, r.mri, utc(r.time)),
                )
            })
            .chain(m.mentions.iter().map(|x| ("Mention".to_string(), x.clone())))
            .chain(m.files.iter().map(|f| (format!("{} File", ph::PAPERCLIP), format!("{}\n{}", f.name, f.url))))
            .collect();
        let refs: Vec<(&str, String)> = rows.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
        theme::kv(ui, "m-extra", &refs);
    }
    theme::section(ui, "Source record");
    theme::kv(
        ui,
        "m-prov",
        &[
            ("Database", m.prov.database.clone()),
            ("Object store", m.prov.store.clone()),
            ("File", m.prov.file.clone()),
            ("LevelDB sequence", m.prov.seq.to_string()),
            ("Record status", m.prov.status.as_str().to_string()),
            ("CRC", if m.prov.crc_ok { "ok".into() } else { "FAILED".into() }),
            ("Seen in records", m.occurrences.to_string()),
            ("Fingerprint", m.fingerprint.clone()),
        ],
    );
    theme::section(ui, "Raw record");
    if view.raw_cache.as_ref().is_none_or(|(k, _)| *k != i) {
        view.raw_cache = Some((i, serde_json::to_string_pretty(&m.raw).unwrap_or_default()));
    }
    let raw = &view.raw_cache.as_ref().unwrap().1;
    if ui.button(format!("{} Copy JSON", ph::COPY)).clicked() {
        ui.ctx().copy_text(raw.clone());
    }
    let mut s = raw.as_str();
    ui.add(egui::TextEdit::multiline(&mut s).code_editor().desired_width(f32::INFINITY));
}

fn conversation_details(ui: &mut egui::Ui, d: &Loaded, view: &ViewState) {
    let Some((acc, id)) = &view.conv else {
        ui.label(theme::muted("No conversation selected."));
        return;
    };
    let Some(c) = d.ds.conversation(acc, id) else { return };
    theme::section(ui, "Conversation");
    theme::kv(
        ui,
        "c-main",
        &[
            ("Title", c.title.clone()),
            ("Type", c.kind.clone()),
            ("Conversation ID", c.id.clone()),
            ("Account", d.ds.account(&c.account).map(|a| a.label()).unwrap_or_default()),
            ("Created (UTC)", utc(c.created)),
            ("Last activity (UTC)", utc(c.last_activity)),
            ("Messages in cache", c.message_count.to_string()),
            ("Metadata record", if c.has_metadata { "yes".into() } else { "no (known only from its messages)".into() }),
        ],
    );
    if !c.members.is_empty() {
        theme::section(ui, &format!("Members ({})", c.members.len()));
        for mem in &c.members {
            ui.horizontal(|ui| {
                theme::avatar(ui, &mem.name, &mem.mri, 22.0, false);
                ui.vertical(|ui| {
                    ui.label(RichText::new(if mem.name.is_empty() { "(unknown name)" } else { &mem.name }).size(13.0));
                    ui.add(
                        egui::Label::new(
                            RichText::new(format!("{} · {}", mem.role, mem.mri)).monospace().size(11.0).color(MUTED),
                        )
                        .wrap()
                        .selectable(true),
                    );
                });
            });
        }
    }
    if !c.cache_purges.is_empty() {
        theme::section(ui, "Cache purges recorded by Teams");
        for p in &c.cache_purges {
            theme::banner(AMBER).show(ui, |ui| {
                theme::kv(
                    ui,
                    &format!("purge-{}", p),
                    &[
                        ("Purged at (UTC)", utc(crate::teams::as_ms(p.get("deleteTime")))),
                        ("Reply chains removed", p.get("deletedRCsCount").map(|x| x.to_string()).unwrap_or_default()),
                        ("Sync window start", utc(crate::teams::as_ms(p.get("startOfSyncWindow")))),
                        ("Sync window end", utc(crate::teams::as_ms(p.get("endOfSyncWindow")))),
                    ],
                );
            });
            ui.add_space(4.0);
        }
    }
    if let Some(p) = &c.prov {
        theme::section(ui, "Source record");
        theme::kv(
            ui,
            "c-prov",
            &[
                ("Database", p.database.clone()),
                ("Object store", p.store.clone()),
                ("File", p.file.clone()),
                ("LevelDB sequence", p.seq.to_string()),
                ("Record status", p.status.as_str().to_string()),
            ],
        );
    }
    theme::section(ui, "Raw record");
    let raw = serde_json::to_string_pretty(&c.raw).unwrap_or_default();
    if ui.button(format!("{} Copy JSON", ph::COPY)).clicked() {
        ui.ctx().copy_text(raw.clone());
    }
    let mut s = raw.as_str();
    ui.add(egui::TextEdit::multiline(&mut s).code_editor().desired_width(f32::INFINITY));
}
