use std::{
    collections::HashSet,
    path::PathBuf,
    sync::mpsc::{channel, Receiver, Sender},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};

mod pad_view;
mod preview;

use eframe::egui::{self, Color32, RichText};
use winbridge_core::{
    config::name_from_exe,
    controllers::{self, Controller, Kind, PadState},
    doctor::{self, Check, Status},
    launch::{self, which},
    layout::Layout,
    mapping::{self, Wizard},
    setup::{self, SetupState},
    Game, GamescopeConfig, Library,
};

const ACCENT: Color32 = Color32::from_rgb(52, 199, 140);
const WARN: Color32 = Color32::from_rgb(240, 180, 60);
const BAD: Color32 = Color32::from_rgb(235, 87, 87);
const INFO: Color32 = Color32::from_rgb(110, 160, 235);
const MUTED: Color32 = Color32::from_rgb(150, 156, 168);
const CARD: Color32 = Color32::from_rgb(34, 37, 46);

#[derive(PartialEq, Clone, Copy)]
enum Page {
    Library,
    Health,
    Controllers,
}

enum Event {
    Checks(Vec<Check>),
    Picked(PathBuf),
    SetupDone(Result<(), String>),
    SetupChecked(SetupState),
    Exited { id: String, name: String, success: bool, seconds: u64, log: PathBuf },
    LaunchFailed(String),
}

enum Action {
    Play(String),
    Remove(String),
    Save(Game),
    Find(String),
    UseExe(String, PathBuf),
}

/// A running "press a button" test on one controller.
struct Live {
    uniq_key: String,
    kind: Kind,
    keys: Vec<u16>,
    layout: Layout,
    rx: Receiver<(u16, u16, i32)>,
    stop: Arc<AtomicBool>,
    state: PadState,
}

impl Drop for Live {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// The "set up buttons" wizard running for one controller.
struct MapSession {
    key: String,
    pad: Controller,
    rx: Receiver<(u16, u16, i32)>,
    stop: Arc<AtomicBool>,
    wizard: Wizard,
}

impl Drop for MapSession {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

enum MapAct {
    Start(Controller),
    Skip,
    Cancel,
    Save,
    Reset(Controller),
}

struct Notice {
    text: String,
    bad: bool,
    log: Option<PathBuf>,
    at: Instant,
}

struct App {
    lib: Library,
    games: Vec<Game>,
    checks: Vec<Check>,
    checking: bool,
    pads: Vec<Controller>,
    last_pad_scan: Instant,
    tools: HashSet<String>,
    page: Page,
    running: HashSet<String>,
    notice: Option<Notice>,
    confirm_remove: Option<String>,
    finder: Option<(String, Vec<PathBuf>)>,
    setting_up: bool,
    setup_state: Option<SetupState>,
    setup_dismissed: bool,
    need_recheck: bool,
    live: Option<Live>,
    mapping: Option<MapSession>,
    /// Text being typed in each game's advanced fields: (Steam id, Proton version).
    drafts: std::collections::HashMap<String, (String, String)>,
    tx: Sender<Event>,
    rx: Receiver<Event>,
}

fn main() -> eframe::Result {
    if let Some(r) = preview::maybe_run() {
        return r;
    }
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([940.0, 640.0])
            .with_min_inner_size([700.0, 480.0])
            .with_title("WinBridge")
            .with_app_id("winbridge-gui")
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native("WinBridge", options, Box::new(|cc| Ok(Box::new(App::new(cc)))))
}

impl App {
    fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut v = egui::Visuals::dark();
        v.panel_fill = Color32::from_rgb(22, 24, 30);
        v.window_fill = Color32::from_rgb(28, 31, 39);
        v.selection.bg_fill = ACCENT.gamma_multiply(0.5);
        v.hyperlink_color = ACCENT;
        cc.egui_ctx.set_visuals(v);
        cc.egui_ctx.global_style_mut(|s| {
            s.spacing.item_spacing = egui::vec2(10.0, 10.0);
            s.spacing.button_padding = egui::vec2(12.0, 7.0);
            s.visuals.widgets.inactive.corner_radius = 8.into();
            s.visuals.widgets.hovered.corner_radius = 8.into();
            s.visuals.widgets.active.corner_radius = 8.into();
        });

        let lib = Library::open().expect("could not create WinBridge folders");
        let (tx, rx) = channel();
        let mut app = Self {
            games: lib.list().unwrap_or_default(),
            lib,
            checks: vec![],
            checking: false,
            pads: controllers::detect(),
            last_pad_scan: Instant::now(),
            tools: HashSet::new(),
            page: Page::Library,
            running: HashSet::new(),
            notice: None,
            confirm_remove: None,
            finder: None,
            setting_up: false,
            setup_state: None,
            setup_dismissed: false,
            need_recheck: false,
            live: None,
            mapping: None,
            drafts: Default::default(),
            tx,
            rx,
        };
        app.refresh_tools();
        app.run_health_check(&cc.egui_ctx);
        // First run: quietly find out what's missing; the user then installs it with one click.
        app.check_setup(&cc.egui_ctx);
        // "Open with WinBridge" from the file manager: add the .exe that was passed in.
        if let Some(arg) = std::env::args().nth(1).filter(|a| !a.starts_with("--")) {
            let p = PathBuf::from(arg);
            if p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")) {
                app.add_exe(p);
            }
        }
        app
    }

    fn refresh_tools(&mut self) {
        self.tools = ["umu-run", "gamemoded", "gamescope", "mangohud"]
            .iter()
            .filter(|t| which(t).is_some())
            .map(|t| t.to_string())
            .collect();
    }

    fn engine_ready(&self) -> bool {
        self.tools.contains("umu-run")
    }

    fn run_health_check(&mut self, ctx: &egui::Context) {
        if self.checking {
            return;
        }
        self.checking = true;
        let (tx, ctx, paths) = (self.tx.clone(), ctx.clone(), self.lib.paths.clone());
        std::thread::spawn(move || {
            let _ = tx.send(Event::Checks(doctor::run_checks(&paths)));
            ctx.request_repaint();
        });
    }

    fn check_setup(&mut self, ctx: &egui::Context) {
        let (tx, ctx, paths) = (self.tx.clone(), ctx.clone(), self.lib.paths.clone());
        std::thread::spawn(move || {
            let _ = tx.send(Event::SetupChecked(setup::check(&paths)));
            ctx.request_repaint();
        });
    }

    fn start_setup(&mut self, ctx: &egui::Context) {
        if self.setting_up {
            return;
        }
        self.setting_up = true;
        let (tx, ctx, paths) = (self.tx.clone(), ctx.clone(), self.lib.paths.clone());
        std::thread::spawn(move || {
            let r = winbridge_core::setup::install_engine(&paths, false, false).map(|_| ()).map_err(|e| e.to_string());
            let _ = tx.send(Event::SetupDone(r));
            ctx.request_repaint();
        });
    }

    fn say(&mut self, text: impl Into<String>, bad: bool, log: Option<PathBuf>) {
        self.notice = Some(Notice { text: text.into(), bad, log, at: Instant::now() });
    }

    fn pick_file(&self, ctx: &egui::Context) {
        let (tx, ctx) = (self.tx.clone(), ctx.clone());
        std::thread::spawn(move || {
            if let Some(f) = rfd::FileDialog::new()
                .set_title("Choose a game or installer")
                .add_filter("Windows programs", &["exe"])
                .pick_file()
            {
                let _ = tx.send(Event::Picked(f));
                ctx.request_repaint();
            }
        });
    }

    fn add_exe(&mut self, exe: PathBuf) {
        let name = name_from_exe(&exe);
        match self.lib.add(Game::new(&name, exe)) {
            Ok(g) => {
                self.say(format!("Added “{}”. Press Play to start it.", g.name), false, None);
                self.games = self.lib.list().unwrap_or_default();
            }
            Err(e) => self.say(format!("Couldn't add that file: {e}"), true, None),
        }
    }

    fn play(&mut self, id: &str, ctx: &egui::Context) {
        let game = match self.lib.get(id) {
            Ok(g) => g,
            Err(e) => return self.say(e.to_string(), true, None),
        };
        let tools = self.tools.clone();
        let plan = match launch::plan(&game, &self.lib.paths, &|t| tools.contains(t)) {
            Ok(p) => p,
            Err(e) => {
                self.page = Page::Health;
                return self.say(e.to_string(), true, None);
            }
        };
        let (tx, ctx, paths) = (self.tx.clone(), ctx.clone(), self.lib.paths.clone());
        self.running.insert(game.id.clone());
        self.say(
            format!("Starting {}… the first launch can take a minute while the game engine sets up.", game.name),
            false,
            None,
        );
        std::thread::spawn(move || {
            match launch::spawn(&game, &paths, &plan).and_then(|r| r.wait()) {
                Ok((info, log)) => {
                    let _ = tx.send(Event::Exited {
                        id: game.id,
                        name: game.name,
                        success: info.success,
                        seconds: info.seconds,
                        log,
                    });
                }
                Err(e) => {
                    let _ = tx.send(Event::Exited {
                        id: game.id.clone(),
                        name: game.name.clone(),
                        success: false,
                        seconds: 0,
                        log: PathBuf::new(),
                    });
                    let _ = tx.send(Event::LaunchFailed(e.to_string()));
                }
            }
            ctx.request_repaint();
        });
    }

    fn drain_events(&mut self) {
        while let Ok(ev) = self.rx.try_recv() {
            match ev {
                Event::Checks(c) => {
                    self.checks = c;
                    self.checking = false;
                    self.refresh_tools();
                }
                Event::Picked(p) => self.add_exe(p),
                Event::SetupChecked(s) => self.setup_state = Some(s),
                Event::SetupDone(r) => {
                    self.setting_up = false;
                    self.refresh_tools();
                    self.checking = false;
                    match r {
                        Ok(()) => self.say("The game engine is installed. You're ready to play!", false, None),
                        Err(e) => {
                            let log = Some(self.lib.paths.logs_dir().join("setup.log"));
                            self.say(format!("Setup didn't finish. {e}"), true, log)
                        }
                    }
                    self.need_recheck = true;
                }
                Event::LaunchFailed(e) => self.say(e, true, None),
                Event::Exited { id, name, success, seconds, log } => {
                    self.running.remove(&id);
                    if success {
                        self.say(format!("{name} closed."), false, None);
                    } else if seconds < 20 && !log.as_os_str().is_empty() {
                        self.say(
                            format!("{name} stopped right after starting. This usually means the game needs a different setting, or isn't supported on Linux (some games block it)."),
                            true,
                            Some(log),
                        );
                    } else if !log.as_os_str().is_empty() {
                        self.say(format!("{name} closed unexpectedly."), true, Some(log));
                    }
                }
            }
        }
    }

    fn apply(&mut self, action: Action, ctx: &egui::Context) {
        match action {
            Action::Play(id) => self.play(&id, ctx),
            Action::Save(g) => {
                if let Err(e) = self.lib.save(&g) {
                    self.say(format!("Couldn't save settings: {e}"), true, None);
                }
                self.games = self.lib.list().unwrap_or_default();
            }
            Action::Remove(id) => {
                self.drafts.remove(&id);
                match self.lib.remove(&id, false) {
                    Ok(()) => {
                        self.say("Removed. Save files were kept on disk in case you add the game back.", false, None)
                    }
                    Err(e) => self.say(e.to_string(), true, None),
                }
                self.games = self.lib.list().unwrap_or_default();
            }
            Action::Find(id) => {
                if let Ok(g) = self.lib.get(&id) {
                    let found = launch::scan_installed_exes(&g.prefix(&self.lib.paths));
                    if found.is_empty() {
                        self.say(
                            "Nothing installed yet. Press Play to run the installer first, finish it, then try again.",
                            false,
                            None,
                        );
                    } else {
                        self.finder = Some((id, found));
                    }
                }
            }
            Action::UseExe(id, exe) => {
                if let Ok(mut g) = self.lib.get(&id) {
                    g.name = name_from_exe(&exe);
                    g.exe = exe;
                    let _ = self.lib.save(&g);
                    self.say(format!("“{}” is ready. Press Play!", g.name), false, None);
                }
                self.games = self.lib.list().unwrap_or_default();
            }
        }
    }
}

fn dot(ui: &mut egui::Ui, color: Color32) {
    let (r, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
    ui.painter().circle_filled(r.center(), 5.0, color);
}

fn looks_like_installer(g: &Game) -> bool {
    let n = g.exe.file_name().and_then(|n| n.to_str()).unwrap_or("").to_ascii_lowercase();
    ["setup", "install", "unpack"].iter().any(|k| n.contains(k)) || n.ends_with(".msi")
}

impl eframe::App for App {
    fn ui(&mut self, root: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = root.ctx().clone();
        let ctx = &ctx;
        self.drain_events();
        if let Some(l) = self.live.as_mut() {
            while let Ok((t, c, v)) = l.rx.try_recv() {
                l.state.apply(t, c, v);
            }
        }
        if let Some(m) = self.mapping.as_mut() {
            while let Ok((t, c, v)) = m.rx.try_recv() {
                m.wizard.feed(t, c, v);
            }
        }
        if self.page != Page::Controllers {
            self.live = None;
            self.mapping = None;
        }
        if std::mem::take(&mut self.need_recheck) {
            self.run_health_check(ctx);
            self.check_setup(ctx);
        }

        // Drag & drop: the easiest way for anyone to add a game.
        let dropped: Vec<PathBuf> = ctx.input(|i| i.raw.dropped_files.iter().map(|f| f.path().to_path_buf()).collect());
        for p in dropped {
            if p.extension().is_some_and(|e| e.eq_ignore_ascii_case("exe")) {
                self.add_exe(p);
                self.page = Page::Library;
            } else {
                self.say("Only Windows programs (.exe files) can be added.", true, None);
            }
        }

        if self.last_pad_scan.elapsed() > Duration::from_secs(2) {
            self.pads = controllers::detect();
            self.last_pad_scan = Instant::now();
        }
        ctx.request_repaint_after(Duration::from_secs(2));

        if self.notice.as_ref().is_some_and(|n| !n.bad && n.log.is_none() && n.at.elapsed() > Duration::from_secs(8)) {
            self.notice = None;
        }

        let ready = self.engine_ready() && !self.checks.iter().any(|c| c.status == Status::Fail);

        egui::Panel::left("nav").exact_size(190.0).show(root, |ui| {
            ui.add_space(14.0);
            ui.label(RichText::new("WinBridge").size(24.0).strong().color(ACCENT));
            ui.label(RichText::new("Windows games, on Linux").color(MUTED).size(12.0));
            ui.add_space(18.0);
            for (page, label) in
                [(Page::Library, "My Games"), (Page::Health, "Health Check"), (Page::Controllers, "Controllers")]
            {
                let sel = self.page == page;
                if ui
                    .add_sized([166.0, 36.0], egui::Button::new(RichText::new(label).size(15.0)).selected(sel))
                    .clicked()
                {
                    self.page = page;
                }
            }
            ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if self.checking {
                        ui.spinner();
                        ui.label(RichText::new("Checking…").color(MUTED));
                    } else if ready {
                        dot(ui, ACCENT);
                        ui.label("Ready to play");
                    } else {
                        dot(ui, BAD);
                        ui.label("Setup needed");
                    }
                });
            });
        });

        if self.notice.is_some() {
            egui::Panel::bottom("notice").show(root, |ui| {
                let mut dismiss = false;
                let mut open_log = None;
                if let Some(n) = &self.notice {
                    ui.horizontal_wrapped(|ui| {
                        dot(ui, if n.bad { BAD } else { ACCENT });
                        ui.label(&n.text);
                        if let Some(log) = &n.log {
                            if ui.button("Open log").clicked() {
                                open_log = Some(log.clone());
                            }
                        }
                        if ui.button("Dismiss").clicked() {
                            dismiss = true;
                        }
                    });
                }
                if let Some(l) = open_log {
                    let _ = std::process::Command::new("xdg-open").arg(l).spawn();
                }
                if dismiss {
                    self.notice = None;
                }
            });
        }

        let mut actions: Vec<Action> = Vec::new();
        let mut want_pick = false;
        let mut want_recheck = false;

        egui::CentralPanel::default().show(root, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| match self.page {
                Page::Library => self.ui_library(ui, &mut actions, &mut want_pick),
                Page::Health => self.ui_health(ui, &mut want_recheck),
                Page::Controllers => self.ui_controllers(ui),
            });
        });

        // “Find installed game” dialog.
        if let Some((id, found)) = self.finder.clone() {
            let mut open = true;
            egui::Window::new("Which one is the game?")
                .open(&mut open)
                .collapsible(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    ui.label(
                        RichText::new("These programs were installed. Pick the one that starts the game.").color(MUTED),
                    );
                    for p in &found {
                        let shown = p.to_string_lossy().split("drive_c/").last().unwrap_or("").to_string();
                        if ui.button(shown).clicked() {
                            actions.push(Action::UseExe(id.clone(), p.clone()));
                            self.finder = None;
                        }
                    }
                });
            if !open {
                self.finder = None;
            }
        }

        if want_pick {
            self.pick_file(ctx);
        }
        if want_recheck {
            self.run_health_check(ctx);
        }
        if !actions.is_empty() {
            ctx.request_repaint();
        }
        for a in actions {
            self.apply(a, ctx);
        }
    }
}

impl App {
    fn setup_banner(&mut self, ui: &mut egui::Ui) {
        let needed: Option<Vec<String>> = match &self.setup_state {
            Some(SetupState::Needed(lines)) => Some(setup::describe(lines)),
            _ => None,
        };
        let engine_missing = !self.engine_ready();
        let unsupported = matches!(self.setup_state, Some(SetupState::Unsupported)) && engine_missing;
        if needed.is_none() && !unsupported {
            return;
        }
        // "Not now" hides the offer unless games genuinely can't start.
        if self.setup_dismissed && !engine_missing && !self.setting_up {
            return;
        }
        let ctx = ui.ctx().clone();
        egui::Frame::new().fill(WARN.gamma_multiply(0.22)).corner_radius(10).inner_margin(12).show(ui, |ui| {
            ui.set_width(ui.available_width());
            if self.setting_up {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label("Installing… if a window asks for your password, enter it. This can take a few minutes.");
                });
            } else if unsupported {
                ui.label("This Linux distribution isn't supported by the automatic setup yet. Open the Health Check to see what to install by hand.");
            } else if let Some(items) = needed {
                ui.label(RichText::new("One-time setup").strong());
                ui.label("To play Windows games WinBridge needs a few things installed on your system (graphics support, the game engine and controller support). It installs them for you, using your system's package manager.");
                egui::CollapsingHeader::new("What will be installed").show(ui, |ui| {
                    for i in items {
                        ui.label(RichText::new(format!("•  {i}")).size(12.0).color(MUTED));
                    }
                });
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(RichText::new("Install everything").strong().color(Color32::BLACK)).fill(ACCENT)).clicked() {
                        self.start_setup(&ctx);
                    }
                    if !engine_missing && ui.button("Not now").clicked() {
                        self.setup_dismissed = true;
                    }
                    ui.label(RichText::new("You'll be asked for your password once.").color(MUTED).size(12.0));
                });
            }
        });
    }

    fn ui_library(&mut self, ui: &mut egui::Ui, actions: &mut Vec<Action>, want_pick: &mut bool) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.heading("My Games");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add(egui::Button::new(RichText::new("＋  Add a game").strong().color(Color32::BLACK)).fill(ACCENT))
                    .clicked()
                {
                    *want_pick = true;
                }
            });
        });

        self.setup_banner(ui);

        if self.games.is_empty() {
            ui.add_space(40.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("Drop a game here").size(26.0).strong());
                ui.label(
                    RichText::new("Drag a Windows game or installer (.exe) onto this window,\nor press “Add a game”.")
                        .color(MUTED),
                );
            });
            return;
        }

        let games = self.games.clone();
        for g in games {
            let running = self.running.contains(&g.id);
            let mut edited = g.clone();
            let mut changed = false;
            egui::Frame::new().fill(CARD).corner_radius(12).inner_margin(14).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&g.name).size(18.0).strong());
                        ui.label(RichText::new(g.exe.file_name().and_then(|n| n.to_str()).unwrap_or("")).color(MUTED).size(12.0));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if running {
                            ui.spinner();
                            ui.label(RichText::new("Running").color(ACCENT));
                        } else if ui
                            .add_enabled(self.engine_ready(), egui::Button::new(RichText::new("▶  Play").size(16.0).strong().color(Color32::BLACK)).fill(ACCENT).min_size(egui::vec2(110.0, 36.0)))
                            .clicked()
                        {
                            actions.push(Action::Play(g.id.clone()));
                        }
                    });
                });

                if looks_like_installer(&g) {
                    ui.horizontal_wrapped(|ui| {
                        dot(ui, WARN);
                        ui.label("This looks like an installer. Press Play, finish the install, then:");
                        if ui.button("Find installed game").clicked() {
                            actions.push(Action::Find(g.id.clone()));
                        }
                    });
                }

                egui::CollapsingHeader::new("Settings").id_salt(&g.id).show(ui, |ui| {
                    changed |= ui.checkbox(&mut edited.gamemode, "Performance boost (GameMode)").changed();
                    changed |= ui.checkbox(&mut edited.mangohud, "Show FPS counter").changed();

                    let has_gs = self.tools.contains("gamescope");
                    let mut limit = edited.gamescope.as_ref().and_then(|gs| gs.fps_limit).is_some();
                    ui.horizontal(|ui| {
                        if ui.add_enabled(has_gs, egui::Checkbox::new(&mut limit, "Limit frame rate for smoother play")).changed() {
                            edited.gamescope = limit.then(|| GamescopeConfig { fps_limit: Some(60), ..Default::default() });
                            changed = true;
                        }
                        if let Some(gs) = edited.gamescope.as_mut() {
                            let mut f = gs.fps_limit.unwrap_or(60);
                            if ui.add(egui::DragValue::new(&mut f).range(20..=360).suffix(" fps")).changed() {
                                gs.fps_limit = Some(f);
                                changed = true;
                            }
                        }
                    });
                    if !has_gs {
                        ui.label(RichText::new("Needs Gamescope (see Health Check).").color(MUTED).size(12.0));
                    }

                    ui.collapsing("Advanced", |ui| {
                        ui.label(RichText::new("Steam game ID: applies known fixes for that game (find it in the game's Steam store URL). Leave empty if unsure.").color(MUTED).size(12.0));
                        // Typed text lives in a per-game draft; rebuilding it from the saved value on every
                        // frame would throw away each keystroke before the field loses focus and saves.
                        let (mut sid, mut p) = self.drafts.get(&g.id).cloned().unwrap_or_else(|| {
                            (
                                g.game_id.strip_prefix("umu-").filter(|s| *s != "0").unwrap_or("").to_string(),
                                g.proton.clone().unwrap_or_default(),
                            )
                        });
                        let sid_response = ui.text_edit_singleline(&mut sid);
                        if sid_response.lost_focus() {
                            sid.retain(|c| c.is_ascii_digit());
                            edited.game_id = format!("umu-{}", if sid.is_empty() { "0" } else { &sid });
                            changed = true;
                        }
                        ui.label(RichText::new("Compatibility version. Leave empty for the recommended one; set it once a game works to keep it from changing.").color(MUTED).size(12.0));
                        let proton_response = ui.text_edit_singleline(&mut p);
                        if proton_response.lost_focus() {
                            p = p.trim().to_string();
                            edited.proton = (!p.is_empty()).then(|| p.clone());
                            changed = true;
                        }
                        self.drafts.insert(g.id.clone(), (sid, p));
                    });

                    ui.add_space(4.0);
                    if self.confirm_remove.as_deref() == Some(g.id.as_str()) {
                        ui.horizontal(|ui| {
                            ui.label("Remove from library? (save files are kept)");
                            if ui.button(RichText::new("Yes, remove").color(BAD)).clicked() {
                                actions.push(Action::Remove(g.id.clone()));
                                self.confirm_remove = None;
                            }
                            if ui.button("Cancel").clicked() {
                                self.confirm_remove = None;
                            }
                        });
                    } else if ui.button("Remove").clicked() {
                        self.confirm_remove = Some(g.id.clone());
                    }
                });
            });
            if changed {
                actions.push(Action::Save(edited));
            }
        }
    }

    fn ui_health(&mut self, ui: &mut egui::Ui, want_recheck: &mut bool) {
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.heading("Health Check");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.add_enabled(!self.checking, egui::Button::new("Check again")).clicked() {
                    *want_recheck = true;
                }
            });
        });
        ui.label(
            RichText::new("Makes sure your computer is set up to run Windows games. Fix the red items first.")
                .color(MUTED),
        );
        self.setup_banner(ui);

        let mut order = self.checks.clone();
        order.sort_by_key(|c| match c.status {
            Status::Fail => 0,
            Status::Warn => 1,
            Status::Info => 2,
            Status::Ok => 3,
        });
        for c in order {
            let color = match c.status {
                Status::Ok => ACCENT,
                Status::Warn => WARN,
                Status::Fail => BAD,
                Status::Info => INFO,
            };
            egui::Frame::new().fill(CARD).corner_radius(10).inner_margin(12).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    dot(ui, color);
                    ui.label(RichText::new(c.title).strong());
                });
                ui.label(RichText::new(&c.message).color(MUTED));
                if let Some(fix) = &c.fix {
                    if c.status != Status::Ok {
                        ui.label(RichText::new("How to fix — paste this in a terminal:").size(12.0));
                        ui.horizontal(|ui| {
                            egui::Frame::new()
                                .fill(Color32::from_rgb(16, 18, 23))
                                .corner_radius(6)
                                .inner_margin(8)
                                .show(ui, |ui| {
                                    ui.add(egui::Label::new(RichText::new(fix).monospace().size(12.0)).wrap());
                                });
                            if ui.button("Copy").clicked() {
                                ui.ctx().copy_text(fix.clone());
                            }
                        });
                    }
                }
            });
        }
    }

    fn start_live(&mut self, pad: &Controller, ctx: &egui::Context) {
        let Some(path) = pad.event_path.clone() else { return };
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let c = ctx.clone();
        let ranges = mapping::read_abs_ranges(&path, &pad.caps.abs);
        match controllers::watch(path, stop.clone(), tx, move || c.request_repaint()) {
            Ok(()) => {
                self.live = Some(Live { uniq_key: pad_key(pad), kind: pad.kind, keys: pad.caps.keys.clone(), layout: Layout::resolve(pad, &self.lib.paths), rx, stop, state: PadState::with_axes(&ranges) });
            }
            Err(e) => self.say(
                format!("Couldn't open the controller for testing ({e}). If it's a permission problem, see the tip on the controller card."),
                true,
                None,
            ),
        }
    }

    fn start_mapping(&mut self, pad: &Controller, ctx: &egui::Context) {
        let Some(path) = pad.event_path.clone() else { return };
        self.live = None;
        let ranges = mapping::read_abs_ranges(&path, &pad.caps.abs);
        let (tx, rx) = channel();
        let stop = Arc::new(AtomicBool::new(false));
        let c = ctx.clone();
        match controllers::watch(path, stop.clone(), tx, move || c.request_repaint()) {
            Ok(()) => {
                self.mapping = Some(MapSession {
                    key: pad_key(pad),
                    pad: pad.clone(),
                    rx,
                    stop,
                    wizard: Wizard::new(pad.caps.clone(), ranges),
                });
            }
            Err(e) => self.say(format!("Couldn't open the controller ({e})."), true, None),
        }
    }

    fn apply_map_action(&mut self, act: MapAct, ctx: &egui::Context) {
        match act {
            MapAct::Start(p) => self.start_mapping(&p, ctx),
            MapAct::Skip => {
                if let Some(m) = self.mapping.as_mut() {
                    m.wizard.skip();
                }
            }
            MapAct::Cancel => self.mapping = None,
            MapAct::Save => {
                if let Some(m) = self.mapping.take() {
                    let p = &m.pad;
                    let guid = mapping::sdl_guid(p.bus, p.vendor, p.product, p.version, &p.raw_name);
                    let line = m.wizard.mapping_line(&guid, &p.name);
                    match mapping::save(&self.lib.paths, p.vendor, p.product, &line) {
                        Ok(()) => self.say(
                            format!("Saved the button layout for {}. Games you start from now on will use it (restart a game that's already running).", p.name),
                            false,
                            None,
                        ),
                        Err(e) => self.say(format!("Couldn't save the layout: {e}"), true, None),
                    }
                }
            }
            MapAct::Reset(p) => {
                let _ = mapping::remove(&self.lib.paths, p.vendor, p.product);
                self.say(format!("Removed the custom button layout for {}.", p.name), false, None);
            }
        }
    }

    fn ui_controllers(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let paths = self.lib.paths.clone();
        let mut act: Option<MapAct> = None;
        ui.add_space(8.0);
        ui.heading("Controllers");
        ui.label(RichText::new("Xbox, PlayStation, Switch Pro and most other controllers work over USB or Bluetooth. Plug one in or pair it in your system's Bluetooth settings and it appears here.").color(MUTED));

        if self.pads.is_empty() {
            egui::Frame::new().fill(CARD).corner_radius(10).inner_margin(14).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label("No controller detected.");
                ui.label(RichText::new("Not showing up? Try another cable (some only charge), another USB port, or switch the controller on.").color(MUTED).size(12.0));
            });
        }

        let pads = self.pads.clone();
        for (idx, p) in pads.iter().enumerate() {
            let same_name = pads.iter().filter(|o| o.name == p.name).count();
            let nth = pads[..=idx].iter().filter(|o| o.name == p.name).count();
            let title = if same_name > 1 { format!("{} — port {nth}", p.name) } else { p.name.clone() };
            let key = pad_key(p);
            let testing = self.live.as_ref().is_some_and(|l| l.uniq_key == key);
            egui::Frame::new().fill(CARD).corner_radius(12).inner_margin(14).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (badge, color) = match p.kind {
                        Kind::Xbox => ("XBOX", Color32::from_rgb(60, 190, 90)),
                        Kind::PlayStation => ("PS", Color32::from_rgb(70, 120, 230)),
                        Kind::Nintendo => ("NINTENDO", Color32::from_rgb(230, 70, 70)),
                        Kind::Generic => ("PAD", MUTED),
                    };
                    egui::Frame::new().fill(color.gamma_multiply(0.3)).corner_radius(6).inner_margin(egui::Margin::symmetric(8, 4)).show(ui, |ui| {
                        ui.label(RichText::new(badge).strong().color(color).size(12.0));
                    });
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&title).strong());
                        let mut line = p.connection.label().to_string();
                        if let Some(b) = &p.battery {
                            line += &format!("  ·  Battery {}%{}", b.percent, if b.charging { " (charging)" } else { "" });
                        }
                        ui.label(RichText::new(line).color(MUTED).size(12.0));
                    });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if p.event_path.is_some() {
                            let label = if testing { "Stop test" } else { "Test buttons" };
                            if ui.button(label).clicked() {
                                if testing {
                                    self.live = None;
                                } else {
                                    self.start_live(p, &ctx);
                                }
                            }
                        }
                    });
                });
                for h in &p.hints {
                    ui.horizontal_wrapped(|ui| {
                        dot(ui, WARN);
                        ui.label(RichText::new(h).size(13.0));
                    });
                }
                let mapping_here = self.mapping.as_ref().is_some_and(|m| m.key == key);
                if mapping_here {
                    if let Some(m) = &self.mapping {
                        draw_wizard(ui, m, &mut act);
                    }
                } else if p.event_path.is_some() {
                    ui.horizontal(|ui| {
                        if ui
                            .button("Set up buttons")
                            .on_hover_text("Use this if a game puts buttons in the wrong places or ignores this controller. Takes about a minute.")
                            .clicked()
                        {
                            act = Some(MapAct::Start(p.clone()));
                        }
                        if mapping::has_saved(&paths, p.vendor, p.product) && ui.button("Reset buttons").clicked() {
                            act = Some(MapAct::Reset(p.clone()));
                        }
                    });
                }
                if testing {
                    if let Some(l) = &self.live {
                        draw_live(ui, l);
                    }
                }
            });
        }
        if let Some(a) = act {
            self.apply_map_action(a, &ctx);
            ctx.request_repaint();
        }
    }
}

fn draw_wizard(ui: &mut egui::Ui, m: &MapSession, act: &mut Option<MapAct>) {
    let (done, total) = m.wizard.progress();
    ui.add_space(6.0);
    ui.add(
        egui::ProgressBar::new(done as f32 / total as f32).text(format!("Step {} of {total}", (done + 1).min(total))),
    );
    if let Some(target) = m.wizard.current() {
        ui.label(RichText::new(target.prompt()).size(17.0).strong().color(ACCENT));
        ui.label(RichText::new("Press it once and let go. Don't have that button? Skip it.").color(MUTED).size(12.0));
        ui.horizontal(|ui| {
            if ui.button("Skip this one").clicked() {
                *act = Some(MapAct::Skip);
            }
            if ui.button("Cancel").clicked() {
                *act = Some(MapAct::Cancel);
            }
        });
    } else {
        ui.label(
            RichText::new(format!("All done — {} of {total} inputs set.", m.wizard.mapped_count()))
                .size(16.0)
                .strong()
                .color(ACCENT),
        );
        ui.horizontal(|ui| {
            if ui
                .add(egui::Button::new(RichText::new("Save layout").strong().color(Color32::BLACK)).fill(ACCENT))
                .clicked()
            {
                *act = Some(MapAct::Save);
            }
            if ui.button("Discard").clicked() {
                *act = Some(MapAct::Cancel);
            }
        });
    }
}

fn pad_key(p: &Controller) -> String {
    format!(
        "{}:{}:{}:{}",
        p.vendor,
        p.product,
        p.uniq,
        p.event_path.as_ref().map(|e| e.display().to_string()).unwrap_or_default()
    )
}

/// Buttons light up when pressed; sticks and triggers move. Lets anyone confirm a controller works.
fn draw_live(ui: &mut egui::Ui, l: &Live) {
    ui.add_space(6.0);
    ui.label(
        RichText::new("Press buttons and move the sticks — the matching part of the controller lights up.")
            .color(MUTED)
            .size(12.0),
    );

    if l.layout.is_known() {
        let style = pad_view::style_for(l.kind, &l.layout.name);
        ui.vertical_centered(|ui| pad_view::draw(ui, style, l.kind, &l.layout, &l.state));
        let note = match l.layout.origin {
            winbridge_core::layout::Origin::Standard => format!("{} layout", l.kind.label()),
            winbridge_core::layout::Origin::Saved => "Showing your saved button layout".to_string(),
            winbridge_core::layout::Origin::Sdl => {
                "Layout taken from SDL's controller database — the same one games use".to_string()
            }
            winbridge_core::layout::Origin::Unknown => String::new(),
        };
        ui.vertical_centered(|ui| ui.label(RichText::new(note).color(MUTED).size(11.0)));
        return;
    }
    ui.label(RichText::new("WinBridge doesn't know this controller's layout yet, so raw buttons are shown. Use “Set up buttons” to teach it.").color(WARN).size(12.0));

    // Show exactly the buttons this controller has (standard pads and generic joysticks differ).
    let mut codes: Vec<u16> = l.keys.iter().copied().filter(|c| *c >= 0x100).collect();
    codes.sort_unstable();
    ui.horizontal_wrapped(|ui| {
        for code in codes {
            let name = controllers::button_name(l.kind, code);
            let on = l.state.pressed(code);
            let (fill, text) = if on { (ACCENT, Color32::BLACK) } else { (Color32::from_rgb(48, 52, 64), MUTED) };
            ui.add(egui::Button::new(RichText::new(name).strong().color(text)).fill(fill).sense(egui::Sense::hover()));
        }
        let (dx, dy) = l.state.dpad();
        let arrow = match (dx, dy) {
            (0, 0) => "D-pad".to_string(),
            (x, y) => format!(
                "D-pad {}{}",
                match y {
                    -1 => "↑",
                    1 => "↓",
                    _ => "",
                },
                match x {
                    -1 => "←",
                    1 => "→",
                    _ => "",
                }
            ),
        };
        let on = (dx, dy) != (0, 0);
        let fill = if on { ACCENT } else { Color32::from_rgb(48, 52, 64) };
        ui.add(
            egui::Button::new(RichText::new(arrow).strong().color(if on { Color32::BLACK } else { MUTED }))
                .fill(fill)
                .sense(egui::Sense::hover()),
        );
    });

    ui.horizontal(|ui| {
        stick(ui, "Left stick", l.state.stick(l.kind, controllers::ABS_X, controllers::ABS_Y));
        if l.kind == Kind::Generic {
            // Generic joysticks (e.g. PlayStation adapters) put the second stick on Z / Rz and have
            // digital triggers, so there are no trigger bars.
            stick(ui, "Right stick", l.state.stick(l.kind, controllers::ABS_Z, controllers::ABS_RZ));
        } else {
            stick(ui, "Right stick", l.state.stick(l.kind, controllers::ABS_RX, controllers::ABS_RY));
            ui.vertical(|ui| {
                ui.add(
                    egui::ProgressBar::new(l.state.trigger(controllers::ABS_Z))
                        .desired_width(120.0)
                        .text("Left trigger"),
                );
                ui.add(
                    egui::ProgressBar::new(l.state.trigger(controllers::ABS_RZ))
                        .desired_width(120.0)
                        .text("Right trigger"),
                );
            });
        }
    });
}

fn stick(ui: &mut egui::Ui, label: &str, (x, y): (f32, f32)) {
    ui.vertical(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(84.0, 84.0), egui::Sense::hover());
        let c = rect.center();
        ui.painter().circle_stroke(c, 40.0, egui::Stroke::new(1.5, MUTED));
        ui.painter().circle_filled(c + egui::vec2(x, y) * 32.0, 9.0, ACCENT);
        ui.label(RichText::new(label).size(11.0).color(MUTED));
    });
}
