// SPDX-License-Identifier: GPL-3.0-or-later
//! Desktop app: splash → welcome (choose a source) → viewer; export, verify, about.

mod dialogs;
mod theme;
mod updates;
mod viewer;
mod welcome;

use crate::discover::{self, Found};
use crate::evidence;
use crate::teams::{Dataset, Variant};
use eframe::egui;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
use std::time::Instant;

pub enum Start {
    Welcome,
    Path(PathBuf),
    Demo,
}

pub const LICENSE_NOTICE: &str = "ChatTrace Forensic\n\
Copyright (C) 2026 Luca Gibelli · Salamacchine (https://salamacchine.it)\n\n\
This program is free software: you can redistribute it and/or modify it under the terms of the GNU General \
Public License as published by the Free Software Foundation, either version 3 of the License, or (at your \
option) any later version.\n\n\
This program is distributed in the hope that it will be useful, but WITHOUT ANY WARRANTY; without even the \
implied warranty of MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE. See the GNU General Public License \
for more details.\n\n\
ChatTrace Forensic is an independent project and is not affiliated with, endorsed by or sponsored by \
Microsoft. Microsoft Teams and Microsoft Purview are trademarks of Microsoft Corporation.";

/// A loaded dataset plus indexes used by the viewer.
pub(crate) struct Loaded {
    pub ds: Dataset,
    pub source: Option<Found>,
    pub demo: bool,
    pub secs: f32,
    /// (account, conversation id) -> indexes into ds.messages
    pub by_conv: HashMap<(String, String), Vec<usize>>,
}

impl Loaded {
    fn new(ds: Dataset, source: Option<Found>, demo: bool, secs: f32) -> Self {
        let mut by_conv: HashMap<(String, String), Vec<usize>> = HashMap::new();
        for (i, m) in ds.messages.iter().enumerate() {
            by_conv.entry((m.account.clone(), m.conversation_id.clone())).or_default().push(i);
        }
        Loaded { ds, source, demo, secs, by_conv }
    }
    pub fn latest_count(&self) -> usize {
        self.ds.messages.iter().filter(|m| m.variant == Variant::Latest).count()
    }
}

pub(crate) struct ScanJob {
    pub root: PathBuf,
    pub cancel: Arc<AtomicBool>,
    pub progress: Arc<AtomicUsize>,
    pub rx: mpsc::Receiver<Vec<Found>>,
    /// Open automatically when exactly one store is found.
    pub auto_open: bool,
}

pub(crate) struct LoadJob {
    pub found: Found,
    pub started: Instant,
    pub rx: mpsc::Receiver<Result<Dataset, String>>,
}

pub(crate) enum Action {
    Open(Found),
    Scan(PathBuf, bool),
    CancelScan,
    ChooseFolder,
    Demo,
    Welcome,
    Reload,
    Export,
    Verify,
    About,
}

#[derive(Default)]
pub(crate) struct ViewState {
    pub account: Option<String>,
    pub conv_filter: String,
    pub conv: Option<(String, String)>,
    pub msg: Option<usize>,
    pub search: String,
    pub show_system: bool,
    pub show_earlier: bool,
    pub show_meta_only: bool,
    pub details: bool,
    pub details_tab_conv: bool,
    pub raw_cache: Option<(usize, String)>,
    pub scroll_top: bool,
}

pub struct App {
    splash_start: Option<f64>,
    local: Vec<Found>,
    teams_running: bool,
    teams_checked: Option<Instant>,
    path_input: String,
    scan: Option<ScanJob>,
    scan_result: Option<(PathBuf, Vec<Found>)>,
    load: Option<LoadJob>,
    data: Option<Loaded>,
    error: Option<String>,
    view: ViewState,
    export: Option<dialogs::ExportUi>,
    verify: Option<dialogs::VerifyUi>,
    about: bool,
    pub(crate) updater: updates::Updater,
    actions: Vec<Action>,
}

impl App {
    fn new(start: Start, ctx: &egui::Context) -> Self {
        let mut app = App {
            splash_start: None,
            local: discover::local(),
            teams_running: false,
            teams_checked: None,
            path_input: String::new(),
            scan: None,
            scan_result: None,
            load: None,
            data: None,
            error: None,
            view: ViewState { show_earlier: true, show_system: true, details: true, ..Default::default() },
            export: None,
            verify: None,
            about: false,
            updater: updates::Updater::new(ctx),
            actions: Vec::new(),
        };
        match start {
            Start::Welcome => {}
            Start::Path(p) => {
                app.path_input = p.display().to_string();
                app.actions.push(Action::Scan(p, true));
            }
            Start::Demo => app.actions.push(Action::Demo),
        }
        app
    }

    fn teams_running(&mut self) -> bool {
        if self.teams_checked.is_none_or(|t| t.elapsed().as_secs() >= 5) {
            self.teams_running = evidence::teams_running();
            self.teams_checked = Some(Instant::now());
        }
        self.teams_running
    }

    fn apply(&mut self, ctx: &egui::Context, action: Action) {
        match action {
            Action::Open(found) => {
                let (tx, rx) = mpsc::channel();
                let f = found.clone();
                let c = ctx.clone();
                std::thread::spawn(move || {
                    let r = crate::teams::load(&f.leveldb, f.blob.as_deref()).map_err(|e| format!("{e:#}"));
                    let _ = tx.send(r);
                    c.request_repaint();
                });
                self.error = None;
                self.load = Some(LoadJob { found, started: Instant::now(), rx });
            }
            Action::Scan(root, auto_open) => {
                if let Some(s) = &self.scan {
                    s.cancel.store(true, Ordering::Relaxed);
                }
                let cancel = Arc::new(AtomicBool::new(false));
                let progress = Arc::new(AtomicUsize::new(0));
                let (tx, rx) = mpsc::channel();
                let (r, ca, pr, c) = (root.clone(), cancel.clone(), progress.clone(), ctx.clone());
                std::thread::spawn(move || {
                    let v = discover::discover(&r, &ca, &mut |n| {
                        pr.store(n, Ordering::Relaxed);
                        c.request_repaint();
                    });
                    let _ = tx.send(v);
                    c.request_repaint();
                });
                self.scan_result = None;
                self.error = None;
                self.scan = Some(ScanJob { root, cancel, progress, rx, auto_open });
            }
            Action::CancelScan => {
                if let Some(s) = self.scan.take() {
                    s.cancel.store(true, Ordering::Relaxed);
                }
            }
            Action::ChooseFolder => {
                if let Some(p) = rfd::FileDialog::new().set_title("Choose a disk or folder to search").pick_folder() {
                    self.path_input = p.display().to_string();
                    self.apply(ctx, Action::Scan(p, false));
                }
            }
            Action::Demo => {
                self.data = Some(Loaded::new(crate::demo::dataset(), None, true, 0.0));
                self.view = ViewState { show_earlier: true, show_system: true, details: true, ..Default::default() };
                self.select_first_conversation();
                // Preselect the edited message so the version history is visible.
                if let Some(d) = &self.data {
                    self.view.msg =
                        d.ds.messages.iter().position(|m| m.edited.is_some() && m.variant == Variant::Latest);
                }
            }
            Action::Welcome => {
                self.data = None;
                self.error = None;
                self.local = discover::local();
            }
            Action::Reload => {
                if let Some(f) = self.data.as_ref().and_then(|d| d.source.clone()) {
                    self.apply(ctx, Action::Open(f));
                }
            }
            Action::Export => {
                if let Some(d) = &self.data {
                    self.export = Some(dialogs::ExportUi::new(d));
                }
            }
            Action::Verify => self.verify = Some(dialogs::VerifyUi::default()),
            Action::About => self.about = true,
        }
    }

    fn select_first_conversation(&mut self) {
        if let Some(d) = &self.data {
            self.view.conv = d.ds.conversations.first().map(|c| (c.account.clone(), c.id.clone()));
            self.view.msg = None;
        }
    }

    fn poll(&mut self, ctx: &egui::Context) {
        if let Some(s) = &self.scan
            && let Ok(v) = s.rx.try_recv()
        {
            let s = self.scan.take().unwrap();
            if s.auto_open && v.len() == 1 {
                self.actions.push(Action::Open(v[0].clone()));
            }
            self.scan_result = Some((s.root, v));
        }
        if let Some(l) = &self.load
            && let Ok(r) = l.rx.try_recv()
        {
            let l = self.load.take().unwrap();
            match r {
                Ok(ds) => {
                    let keep = self
                        .data
                        .as_ref()
                        .is_some_and(|d| d.source.as_ref().map(|s| &s.leveldb) == Some(&l.found.leveldb));
                    let secs = l.started.elapsed().as_secs_f32();
                    self.data = Some(Loaded::new(ds, Some(l.found), false, secs));
                    if !keep {
                        self.view =
                            ViewState { show_earlier: true, show_system: true, details: true, ..Default::default() };
                        self.select_first_conversation();
                    } else {
                        self.view.raw_cache = None;
                    }
                }
                Err(e) => self.error = Some(e),
            }
        }
        // Folders dropped on the window.
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).filter(|p| !p.as_os_str().is_empty()).collect()
        });
        if let Some(p) = dropped.into_iter().next() {
            let p = if p.is_file() { p.parent().map(|x| x.to_path_buf()).unwrap_or(p) } else { p };
            if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip")) {
                self.verify = Some(dialogs::VerifyUi::default());
            } else {
                self.path_input = p.display().to_string();
                self.actions.push(Action::Scan(p, true));
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        self.poll(&ctx);
        self.updater.poll();
        for a in std::mem::take(&mut self.actions) {
            self.apply(&ctx, a);
        }
        if self.updater.busy()
            || self.scan.is_some()
            || self.load.is_some()
            || self.export.as_ref().is_some_and(|e| e.running())
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        let now = ctx.input(|i| i.time);
        let start = *self.splash_start.get_or_insert(now);
        if welcome::splash(ui, now - start) {
            return;
        }

        if self.data.is_some() {
            viewer::show(self, ui);
        } else {
            welcome::show(self, ui);
        }

        if let Some(e) = &mut self.export {
            let src = self.data.as_ref().and_then(|d| d.source.clone());
            let running = src.as_ref().is_some_and(|s| s.live_local) && self.teams_running;
            if !e.show(&ctx, running) {
                self.export = None;
            }
        }
        if let Some(v) = &mut self.verify
            && !v.show(&ctx)
        {
            self.verify = None;
        }
        if self.about && !dialogs::about(&ctx, &mut self.updater) {
            self.about = false;
        }
        self.updater.show_dialog(&ctx);
    }
}

pub fn run(start: Start) -> anyhow::Result<()> {
    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../../assets/icon-256.png")).ok();
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([1360.0, 880.0])
        .with_min_inner_size([980.0, 620.0])
        .with_title(crate::APP_NAME)
        .with_app_id("chattrace");
    if let Some(i) = icon {
        viewport = viewport.with_icon(Arc::new(i));
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(
        crate::APP_NAME,
        options,
        Box::new(move |cc| {
            egui_extras::install_image_loaders(&cc.egui_ctx);
            theme::install(&cc.egui_ctx);
            Ok(Box::new(App::new(start, &cc.egui_ctx)))
        }),
    )
    .map_err(|e| anyhow::anyhow!("{e}"))
}
