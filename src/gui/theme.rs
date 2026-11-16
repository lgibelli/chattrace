// SPDX-License-Identifier: GPL-3.0-or-later
//! Colours, fonts and small reusable widgets.

use eframe::egui::{self, Color32, CornerRadius, FontData, FontFamily, FontId, Margin, RichText, Stroke, TextStyle};
use std::sync::Arc;

pub(crate) use super::icons as ph;

pub const BAR: Color32 = Color32::from_rgb(0x0a, 0x12, 0x20);
pub const BG: Color32 = Color32::from_rgb(0x0e, 0x17, 0x26);
pub const SURFACE: Color32 = Color32::from_rgb(0x14, 0x20, 0x34);
pub const SURFACE2: Color32 = Color32::from_rgb(0x1b, 0x29, 0x41);
pub const BORDER: Color32 = Color32::from_rgb(0x26, 0x36, 0x52);
pub const TEXT: Color32 = Color32::from_rgb(0xe6, 0xed, 0xf6);
pub const MUTED: Color32 = Color32::from_rgb(0x8f, 0xa1, 0xb9);
pub const ACCENT: Color32 = Color32::from_rgb(0x2c, 0xcf, 0xc0);
pub const ACCENT_DIM: Color32 = Color32::from_rgb(0x14, 0x5e, 0x5e);
pub const AMBER: Color32 = Color32::from_rgb(0xf2, 0xa3, 0x3a);
pub const RED: Color32 = Color32::from_rgb(0xf0, 0x6a, 0x5d);
pub const GREEN: Color32 = Color32::from_rgb(0x3c, 0xcb, 0x7f);
pub const BLUE: Color32 = Color32::from_rgb(0x6e, 0xa8, 0xfe);

pub fn semibold() -> FontFamily {
    FontFamily::Name("semibold".into())
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        "inter".into(),
        Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/Inter-Regular.ttf"))),
    );
    fonts.font_data.insert(
        "inter-semibold".into(),
        Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/Inter-SemiBold.ttf"))),
    );
    if let Some(p) = fonts.families.get_mut(&FontFamily::Proportional) {
        p.insert(0, "inter".into());
    }
    // Phosphor icons right after Inter (whose Private Use Area mappings were removed).
    fonts
        .font_data
        .insert("phosphor".into(), Arc::new(FontData::from_static(include_bytes!("../../assets/fonts/Phosphor.ttf"))));
    if let Some(p) = fonts.families.get_mut(&FontFamily::Proportional) {
        p.insert(1, "phosphor".into());
    }
    let mut sb = vec!["inter-semibold".to_string()];
    sb.extend(fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default());
    fonts.families.insert(semibold(), sb);
    ctx.set_fonts(fonts);

    let mut v = egui::Visuals::dark();
    v.override_text_color = Some(TEXT);
    v.panel_fill = BG;
    v.window_fill = SURFACE;
    v.window_stroke = Stroke::new(1.0, BORDER);
    v.window_corner_radius = CornerRadius::same(14);
    v.menu_corner_radius = CornerRadius::same(10);
    v.extreme_bg_color = Color32::from_rgb(0x0b, 0x13, 0x21);
    v.faint_bg_color = Color32::from_rgb(0x12, 0x1d, 0x2f);
    v.code_bg_color = Color32::from_rgb(0x0b, 0x13, 0x21);
    v.hyperlink_color = ACCENT;
    v.warn_fg_color = AMBER;
    v.error_fg_color = RED;
    v.selection.bg_fill = ACCENT_DIM;
    v.selection.stroke = Stroke::new(1.0, ACCENT);
    let r = CornerRadius::same(8);
    v.widgets.noninteractive.bg_fill = SURFACE;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.noninteractive.corner_radius = r;
    v.widgets.inactive.bg_fill = SURFACE2;
    v.widgets.inactive.weak_bg_fill = SURFACE2;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0, BORDER);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, TEXT);
    v.widgets.inactive.corner_radius = r;
    v.widgets.hovered.bg_fill = Color32::from_rgb(0x22, 0x34, 0x52);
    v.widgets.hovered.weak_bg_fill = Color32::from_rgb(0x22, 0x34, 0x52);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, Color32::from_rgb(0x3a, 0x52, 0x78));
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, TEXT);
    v.widgets.hovered.corner_radius = r;
    v.widgets.active.bg_fill = ACCENT_DIM;
    v.widgets.active.weak_bg_fill = ACCENT_DIM;
    v.widgets.active.bg_stroke = Stroke::new(1.0, ACCENT);
    v.widgets.active.corner_radius = r;
    v.widgets.open.corner_radius = r;
    ctx.set_visuals_of(egui::Theme::Dark, v);
    ctx.set_theme(egui::Theme::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |s| {
        use FontFamily::*;
        s.text_styles = [
            (TextStyle::Small, FontId::new(11.5, Proportional)),
            (TextStyle::Body, FontId::new(14.0, Proportional)),
            (TextStyle::Button, FontId::new(13.5, Proportional)),
            (TextStyle::Heading, FontId::new(21.0, semibold())),
            (TextStyle::Monospace, FontId::new(12.5, Monospace)),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(8.0, 6.0);
        s.spacing.button_padding = egui::vec2(10.0, 5.0);
        s.spacing.interact_size.y = 26.0;
        s.spacing.window_margin = Margin::same(18);
    });
}

pub fn card() -> egui::Frame {
    egui::Frame::new().fill(SURFACE).stroke(Stroke::new(1.0, BORDER)).corner_radius(12).inner_margin(Margin::same(16))
}

pub fn banner(color: Color32) -> egui::Frame {
    egui::Frame::new()
        .fill(color.gamma_multiply(0.14))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.6)))
        .corner_radius(10)
        .inner_margin(Margin::symmetric(12, 8))
}

pub fn title(text: impl Into<String>, size: f32) -> RichText {
    RichText::new(text).family(semibold()).size(size)
}

pub fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text).color(MUTED)
}

pub fn icon(glyph: &str, size: f32, color: Color32) -> RichText {
    RichText::new(glyph).size(size).color(color)
}

pub fn primary(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(
        egui::Button::new(RichText::new(text).family(semibold()).color(BAR))
            .fill(ACCENT)
            .corner_radius(8)
            .min_size(egui::vec2(0.0, 30.0)),
    )
}

pub fn primary_enabled(ui: &mut egui::Ui, enabled: bool, text: &str) -> egui::Response {
    ui.add_enabled_ui(enabled, |ui| primary(ui, text)).inner
}

pub fn secondary(ui: &mut egui::Ui, text: &str) -> egui::Response {
    ui.add(egui::Button::new(text).corner_radius(8).min_size(egui::vec2(0.0, 30.0)))
}

/// Small rounded label.
pub fn chip(ui: &mut egui::Ui, text: impl Into<String>, fg: Color32) -> egui::Response {
    egui::Frame::new()
        .fill(fg.gamma_multiply(0.16))
        .corner_radius(99)
        .inner_margin(Margin::symmetric(8, 1))
        .show(ui, |ui| ui.label(RichText::new(text).size(11.5).color(fg)))
        .response
}

pub fn section(ui: &mut egui::Ui, text: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(text.to_uppercase()).size(11.0).color(MUTED).family(semibold()));
    ui.add_space(2.0);
}

const AVATAR: [Color32; 8] = [
    Color32::from_rgb(0x3b, 0x82, 0xc4),
    Color32::from_rgb(0x8b, 0x5c, 0xc8),
    Color32::from_rgb(0xc2, 0x5b, 0x8a),
    Color32::from_rgb(0xc7, 0x7a, 0x2e),
    Color32::from_rgb(0x2f, 0x9e, 0x6e),
    Color32::from_rgb(0x1f, 0x9b, 0xb0),
    Color32::from_rgb(0x6b, 0x7f, 0xd6),
    Color32::from_rgb(0xb0, 0x5a, 0x4a),
];

pub fn avatar_color(key: &str) -> Color32 {
    let h = key.bytes().fold(2166136261u32, |h, b| (h ^ b as u32).wrapping_mul(16777619));
    AVATAR[(h as usize) % AVATAR.len()]
}

pub fn initials(name: &str) -> String {
    let words: Vec<&str> =
        name.split(|c: char| c.is_whitespace() || c == ',' || c == '·').filter(|w| !w.is_empty()).collect();
    let mut s: String = words.iter().take(2).filter_map(|w| w.chars().find(|c| c.is_alphanumeric())).collect();
    if s.is_empty() {
        s.push('?');
    }
    s.to_uppercase()
}

/// Round avatar with initials (or an icon for system senders).
pub fn avatar(ui: &mut egui::Ui, name: &str, key: &str, size: f32, system: bool) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let p = ui.painter();
    if system {
        p.circle_filled(rect.center(), size / 2.0, SURFACE2);
        p.text(rect.center(), egui::Align2::CENTER_CENTER, ph::GEAR_SIX, FontId::proportional(size * 0.5), MUTED);
    } else {
        p.circle_filled(rect.center(), size / 2.0, avatar_color(key));
        p.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            initials(name),
            FontId::new(size * 0.38, semibold()),
            Color32::WHITE,
        );
    }
    resp
}

/// Two-column key/value grid with selectable monospace values; empty values are skipped.
pub fn kv(ui: &mut egui::Ui, id: &str, rows: &[(&str, String)]) {
    egui::Grid::new(id).num_columns(2).spacing([12.0, 5.0]).striped(false).show(ui, |ui| {
        for (k, v) in rows {
            if v.is_empty() {
                continue;
            }
            ui.label(RichText::new(*k).color(MUTED).size(12.0));
            ui.add(egui::Label::new(RichText::new(v).monospace().size(12.0)).wrap().selectable(true));
            ui.end_row();
        }
    });
}

pub fn reaction_emoji(key: &str) -> &str {
    match key {
        "like" => "👍",
        "heart" => "❤",
        "laugh" => "😆",
        "surprised" => "😮",
        "sad" => "😢",
        "angry" => "😠",
        "yes" | "check" => "✔",
        _ => key,
    }
}

/// Faint fingerprint-like ridges used as background decoration.
pub fn ridges(painter: &egui::Painter, center: egui::Pos2, max_r: f32, color: Color32) {
    let mut r = max_r * 0.12;
    let mut i = 0;
    while r < max_r {
        let (a1, a2) = match i % 3 {
            0 => (150.0f32, 400.0f32),
            1 => (170.0, 330.0),
            _ => (200.0, 470.0),
        };
        let n = 64;
        let pts: Vec<egui::Pos2> = (0..=n)
            .map(|k| {
                let a = (a1 + (a2 - a1) * k as f32 / n as f32).to_radians();
                center + egui::vec2(a.cos(), a.sin()) * r
            })
            .collect();
        painter.add(egui::Shape::line(pts, Stroke::new(max_r * 0.018, color)));
        r += max_r * 0.085;
        i += 1;
    }
}
