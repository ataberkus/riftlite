//! egui UI: main window tabs and the click-through in-game overlay.

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, CentralPanel, Color32, Frame, Grid, Layout, Margin, Panel, ProgressBar, RichText, ScrollArea, Ui,
    ViewportBuilder, ViewportCommand, ViewportId,
};

use crate::ddragon::{StaticData, tft_label};
use crate::lcu::{LolGame, MatchDetail, MatchPlayer, RankedQueue, RecPage, TftGame};
use crate::live::{LiveGame, LivePlayer};
use crate::riot::{self, Product};
use crate::settings::{self, Settings};
use crate::worker::{Action, LcuStatus, Shared, Snapshot};
use crate::{itemset, win};

const ACCENT: Color32 = Color32::from_rgb(0x4f, 0xb8, 0xff);
const WIN: Color32 = Color32::from_rgb(0x52, 0xc4, 0x7a);
const LOSS: Color32 = Color32::from_rgb(0xe0, 0x5c, 0x5c);
const MUTED: Color32 = Color32::from_gray(150);
const SOON: Color32 = Color32::from_rgb(0xff, 0xc8, 0x4a);
/// The overlay lists enemy ranks while the game is this young (loading screen and laning start).
const SCOUT_SECS: f64 = 90.0;

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Home,
    Profile,
    League,
    Tft,
    ChampSelect,
    Live,
    PostGame,
    Players,
    Champions,
    Settings,
}

pub struct App {
    shared: Arc<Shared>,
    settings: Settings,
    autostart: bool,
    tab: Tab,
    seen_phase: String,
    notice: Option<(String, bool, Instant)>,
    seen_action: u64,
    start_minimized: bool,
    /// Whether the last main `ui` pass declared the overlay viewport.
    overlay_declared: bool,
    expanded: HashSet<u64>,
    lookup_input: String,
    champ_filter: String,
    champ_selected: Option<i64>,
    folder_input: String,
    hotkey: Option<bool>,
    hotkey_rx: Receiver<bool>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, start_minimized: bool, title: &'static str) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.selection.bg_fill = ACCENT.linear_multiply(0.45);
        visuals.hyperlink_color = ACCENT;
        cc.egui_ctx.set_theme(egui::Theme::Dark);
        cc.egui_ctx.set_visuals_of(egui::Theme::Dark, visuals);
        let settings = Settings::load();
        let shared = crate::worker::spawn(cc.egui_ctx.clone(), settings.clone());
        let hotkey_rx = crate::tray::spawn(cc.egui_ctx.clone(), shared.clone(), title);
        App {
            folder_input: settings.league_folder.clone(),
            shared,
            settings,
            autostart: settings::autostart_enabled(),
            tab: Tab::Home,
            seen_phase: String::new(),
            notice: None,
            seen_action: 0,
            start_minimized,
            overlay_declared: false,
            expanded: HashSet::new(),
            lookup_input: String::new(),
            champ_filter: String::new(),
            champ_selected: None,
            hotkey: None,
            hotkey_rx,
        }
    }

    fn notify(&mut self, r: Result<String, String>) {
        self.notice = Some(match r {
            Ok(m) => (m, true, Instant::now()),
            Err(m) => (m, false, Instant::now()),
        });
    }

    fn save_settings(&mut self) {
        *self.shared.settings.lock() = self.settings.clone();
        if let Err(e) = self.settings.save() {
            self.notify(Err(format!("Could not save settings: {e}")));
        }
    }

    fn request(&self, a: Action) {
        self.shared.request(a);
    }
}

fn overlay_game(live: &Option<LiveGame>) -> Option<&LiveGame> {
    live.as_ref().filter(|g| g.mode != "TFT" && !g.players.is_empty())
}

/// Game process up, players known, Live Client API not answering yet: the loading screen.
fn loading_screen(s: &Snapshot) -> bool {
    s.live.is_none()
        && s.running.league_game.is_some()
        && matches!(s.phase.as_str(), "GameStart" | "InProgress")
        && s.session.as_ref().is_some_and(|g| !g.players.is_empty() && !queue_name(g.queue_id).starts_with("TFT"))
}

fn overlay_wanted(settings: &Settings, shared: &Shared, s: &Snapshot) -> bool {
    settings.overlay_enabled
        && !shared.overlay_hidden.load(Ordering::Relaxed)
        && (overlay_game(&s.live).is_some() || loading_screen(s))
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // While minimized eframe skips `ui`, which is where the overlay viewport
        // is declared. Request one hidden pass only when it must appear or go away;
        // its contents repaint on their own.
        let want = overlay_wanted(&self.settings, &self.shared, &self.shared.snap.lock());
        let minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
        if minimized && want != self.overlay_declared {
            ctx.send_viewport_cmd(ViewportCommand::RequestPaintWhileHidden);
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if std::mem::take(&mut self.start_minimized) {
            ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
        }
        if let Ok(ok) = self.hotkey_rx.try_recv() {
            self.hotkey = Some(ok);
        }
        let shared = self.shared.clone();
        let snap = shared.snap.lock();

        if let Some((seq, msg, ok)) = &snap.action_result {
            if *seq != self.seen_action {
                self.seen_action = *seq;
                self.notify(if *ok { Ok(msg.clone()) } else { Err(msg.clone()) });
            }
        }
        if snap.phase != self.seen_phase {
            if self.settings.switch_tabs_automatically {
                match snap.phase.as_str() {
                    "ChampSelect" => self.tab = Tab::ChampSelect,
                    "InProgress" => self.tab = Tab::Live,
                    "EndOfGame" if matches!(self.tab, Tab::Live | Tab::ChampSelect) => self.tab = Tab::PostGame,
                    _ => {}
                }
            }
            self.seen_phase = snap.phase.clone();
        }

        Panel::top("nav").show(ui, |ui| {
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("riftlite").strong().size(18.0).color(ACCENT));
                ui.add_space(12.0);
                for (t, label) in [
                    (Tab::Home, "Home"),
                    (Tab::Profile, "Profile"),
                    (Tab::League, "LoL matches"),
                    (Tab::Tft, "TFT matches"),
                    (Tab::ChampSelect, "Champ select"),
                    (Tab::Live, "Live game"),
                    (Tab::PostGame, "Post-game"),
                    (Tab::Players, "Players"),
                    (Tab::Champions, "Champions"),
                    (Tab::Settings, "Settings"),
                ] {
                    ui.selectable_value(&mut self.tab, t, label);
                }
            });
            ui.add_space(4.0);
        });

        Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                status_line(ui, &snap);
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.button("Refresh").clicked() {
                        self.shared.refresh.store(true, Ordering::Relaxed);
                    }
                    if let Some((msg, ok, at)) = &self.notice {
                        if at.elapsed() < Duration::from_secs(8) {
                            ui.colored_label(if *ok { WIN } else { LOSS }, msg);
                        }
                    }
                    if let Some(b) = snap.busy {
                        ui.colored_label(MUTED, b);
                    }
                });
            });
        });

        CentralPanel::default().show(ui, |ui| {
            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| match self.tab {
                Tab::Home => self.home(ui, &snap),
                Tab::Profile => self.profile(ui, &snap),
                Tab::League => self.lol_matches(ui, &snap),
                Tab::Tft => tft_matches(ui, &snap),
                Tab::ChampSelect => self.champ_select(ui, &snap),
                Tab::Live => live_tab(ui, &snap),
                Tab::PostGame => self.post_game(ui, &snap),
                Tab::Players => self.players(ui, &snap),
                Tab::Champions => self.champions(ui, &snap),
                Tab::Settings => self.settings_tab(ui, &snap),
            });
        });

        self.overlay_declared = overlay_wanted(&self.settings, &self.shared, &snap);
        drop(snap);
        if self.overlay_declared {
            self.show_overlay(&ctx);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // Transparent so the overlay viewport can be see-through; every panel
        // in the main window paints an opaque fill.
        [0.0; 4]
    }
}

fn status_line(ui: &mut Ui, s: &Snapshot) {
    let (color, text) = match &s.lcu {
        LcuStatus::ClientNotRunning => (MUTED, "League client not running".to_string()),
        LcuStatus::Connected { port, source } => {
            let phase = if s.phase.is_empty() { "…" } else { s.phase.as_str() };
            (WIN, format!("LCU connected · port {port} (via {source}) · phase {phase}"))
        }
        LcuStatus::Error(e) => (LOSS, e.clone()),
    };
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    ui.painter().circle_filled(rect.center(), 4.0, color);
    ui.label(text);
    if let Some(q) = s.session.as_ref().and_then(|g| g.description.as_deref()) {
        ui.label(RichText::new(format!("· {q}")).color(MUTED));
    }
}

fn section(ui: &mut Ui, title: &str) {
    ui.add_space(10.0);
    ui.label(RichText::new(title).strong().size(16.0));
    ui.add_space(4.0);
}

fn card<R>(ui: &mut Ui, f: impl FnOnce(&mut Ui) -> R) -> R {
    Frame::group(ui.style()).inner_margin(Margin::same(10)).show(ui, f).inner
}

impl App {
    fn home(&mut self, ui: &mut Ui, s: &Snapshot) {
        section(ui, "Games");
        for p in Product::ALL {
            card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(p.label()).strong().size(15.0));
                    let (running, what) = match p {
                        Product::League => match (s.running.league_game, s.running.league_client) {
                            (Some(pid), _) => (true, format!("in game (pid {pid})")),
                            (None, Some(pid)) => (true, format!("client running (pid {pid})")),
                            _ => (false, "not running".into()),
                        },
                        Product::Tft => match (s.running.tft_game, s.running.tft_client) {
                            (Some(pid), _) => (true, format!("in game (pid {pid})")),
                            (None, Some(pid)) => (true, format!("client running (pid {pid})")),
                            _ => (false, "not running".into()),
                        },
                    };
                    ui.colored_label(if running { WIN } else { MUTED }, what);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let can = s.installs.riot_client.is_some() && s.installs.get(p).is_some();
                        if ui.add_enabled(can && !running, egui::Button::new("Launch")).clicked() {
                            let r = riot::launch(&s.installs, p).map(|_| format!("Launching {} via Riot Client", p.label()));
                            self.notify(r);
                        }
                    });
                });
                match s.installs.get(p) {
                    Some(i) => {
                        ui.label(format!("Installed at {}", i.path.display()));
                        let v = if i.verified { "client executable found" } else { "client executable missing" };
                        ui.label(RichText::new(format!("source: {} · {v}", i.source)).color(MUTED));
                    }
                    None => {
                        ui.colored_label(MUTED, "Not installed (set the folder in Settings if it is installed elsewhere)");
                    }
                }
            });
            ui.add_space(6.0);
        }
        section(ui, "Riot Client");
        match &s.installs.riot_client {
            Some(p) => ui.label(p.display().to_string()),
            None => ui.colored_label(LOSS, "RiotClientServices.exe not found; launching is unavailable"),
        };
        if let Some(pid) = s.running.riot_client {
            ui.colored_label(WIN, format!("running (pid {pid})"));
        }
        section(ui, "Static data");
        match (&s.data_error, s.data.version.is_empty()) {
            (Some(e), _) => ui.colored_label(LOSS, format!("Data Dragon: {e}")),
            (None, true) => ui.colored_label(MUTED, "Loading Data Dragon…"),
            (None, false) => ui.label(format!(
                "Data Dragon {} · {} champions · {} items · {} runes{}",
                s.data.version,
                s.data.champions.len(),
                s.data.items.len(),
                s.data.perks.len(),
                if s.data.has_client_assets { " · client asset names merged" } else { "" }
            )),
        };
    }

    fn profile(&mut self, ui: &mut Ui, s: &Snapshot) {
        let Some(sum) = &s.summoner else {
            section(ui, "Profile");
            ui.colored_label(MUTED, s.profile_error.as_deref().unwrap_or(no_data_hint(s)));
            return;
        };
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(&sum.game_name).strong().size(22.0));
            if !sum.tag_line.is_empty() {
                ui.label(RichText::new(format!("#{}", sum.tag_line)).size(22.0).color(MUTED));
            }
            ui.label(RichText::new(format!("level {}", sum.level)).color(MUTED));
        });
        ui.columns(2, |cols| {
            for (ui, tft) in cols.iter_mut().zip([false, true]) {
                card(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(if tft { "Teamfight Tactics" } else { "League of Legends" }).strong());
                    ranked_grid(ui, &s.ranked, tft);
                    ui.add_space(6.0);
                    if tft {
                        tft_summary(ui, s.tft_history.as_deref().unwrap_or(&[]));
                    } else {
                        lol_summary(ui, &s.data, s.lol_history.as_deref().unwrap_or(&[]));
                    }
                });
            }
        });
        ui.columns(2, |cols| {
            card(&mut cols[0], |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Champion mastery").strong());
                mastery_grid(ui, &s.data, &s.mastery, "mastery-self");
            });
            card(&mut cols[1], |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Challenges").strong());
                match &s.challenges {
                    None => void(ui.colored_label(MUTED, "No challenge data")),
                    Some(c) => {
                        ui.label(format!("{} · {} points ({} to next rank)", title(&c.level), c.score, c.to_next));
                        Grid::new("challenges").spacing([12.0, 3.0]).show(ui, |ui| {
                            for cat in &c.categories {
                                ui.label(RichText::new(title(&cat.name)).color(MUTED));
                                ui.label(title(&cat.level));
                                ui.add(
                                    ProgressBar::new(if cat.max > 0 { cat.current as f32 / cat.max as f32 } else { 0.0 })
                                        .desired_width(140.0)
                                        .text(format!("{} / {}", cat.current, cat.max)),
                                );
                                ui.end_row();
                            }
                        });
                    }
                }
            });
        });
        section(ui, "Missions");
        if ui.button(if s.missions.is_some() { "Reload missions" } else { "Load missions" }).clicked() {
            self.request(Action::LoadMissions);
        }
        match &s.missions {
            None => {}
            Some(Err(e)) => void(ui.colored_label(LOSS, e)),
            Some(Ok(m)) if m.is_empty() => void(ui.colored_label(MUTED, "No open League missions")),
            Some(Ok(m)) => {
                Grid::new("missions").striped(true).spacing([12.0, 4.0]).show(ui, |ui| {
                    for mission in m {
                        ui.label(RichText::new(&mission.title).strong());
                        ui.label(&mission.objective);
                        ui.add(
                            ProgressBar::new((mission.current as f32 / mission.total as f32).min(1.0))
                                .desired_width(160.0)
                                .text(format!("{} / {}", mission.current, mission.total)),
                        );
                        ui.label(RichText::new(if mission.end_ms > 0 { format!("ends {}", date(mission.end_ms)) } else { String::new() }).color(MUTED));
                        ui.end_row();
                    }
                });
            }
        }
    }

    fn lol_matches(&mut self, ui: &mut Ui, s: &Snapshot) {
        section(ui, "Recent League games");
        let games = match &s.lol_history {
            Err(e) => return void(ui.colored_label(LOSS, e)),
            Ok(g) if g.is_empty() => return void(ui.colored_label(MUTED, no_data_hint(s))),
            Ok(g) => g,
        };
        let root = s.league_root();
        let me = s.summoner.as_ref().map_or("", |x| x.puuid.as_str());
        if let Some(id) = self.game_cards(ui, s, games, root.as_deref(), me, "self") {
            self.lookup(id);
        }
        ui.add_space(6.0);
        if s.history_complete {
            ui.colored_label(MUTED, format!("All {} games the client keeps are shown.", games.len()));
        } else if ui.button(format!("Load {} more", 20)).clicked() {
            self.request(Action::MoreHistory);
        }
    }

    /// One card per game with an expandable 10-player table; returns a Riot ID the user chose to look up.
    fn game_cards(
        &mut self,
        ui: &mut Ui,
        s: &Snapshot,
        games: &[LolGame],
        root: Option<&std::path::Path>,
        viewer: &str,
        salt: &str,
    ) -> Option<String> {
        let mut look = None;
        for g in games {
            card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.colored_label(if g.win { WIN } else { LOSS }, RichText::new(if g.win { "Win " } else { "Loss" }).strong());
                    ui.label(RichText::new(s.data.champion_name(g.champion_id)).strong());
                    ui.label(format!("{} / {} / {} ({:.1})", g.kills, g.deaths, g.assists, g.kda()));
                    let mins = (g.duration_s as f64 / 60.0).max(1.0);
                    ui.label(RichText::new(format!("CS {} ({:.1}/m) · {:.1}k gold · {}", g.cs, g.cs as f64 / mins, g.gold as f64 / 1000.0, mmss(g.duration_s as f64))).color(MUTED));
                    ui.label(RichText::new(format!("{} items", g.items[..6].iter().filter(|&&i| i > 0).count())).color(MUTED))
                        .on_hover_text(item_list(&s.data, &g.items));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let open = self.expanded.contains(&g.game_id);
                        if ui.button(if open { "Hide" } else { "Details" }).clicked() {
                            if open {
                                self.expanded.remove(&g.game_id);
                            } else {
                                self.expanded.insert(g.game_id);
                                if !matches!(s.match_details.get(&g.game_id), Some(Ok(_))) {
                                    self.request(Action::MatchDetail(g.game_id));
                                }
                            }
                        }
                        if let Some(root) = root {
                            if ui
                                .add_enabled(!s.data.champions.is_empty(), egui::Button::new("Save item set"))
                                .on_hover_text("Write this game's items as an item set the League client shows in the shop")
                                .clicked()
                            {
                                self.write_item_set(Some(root), &s.data, g);
                            }
                        }
                        ui.label(RichText::new(format!("{} · {}", queue_name(g.queue_id), datetime(g.created_ms))).color(MUTED));
                    });
                });
                if self.expanded.contains(&g.game_id) {
                    match s.match_details.get(&g.game_id) {
                        None => void(ui.colored_label(MUTED, "Loading…")),
                        Some(Err(e)) => void(ui.colored_label(LOSS, e)),
                        Some(Ok(d)) => {
                            if let Some(id) = match_table(ui, &s.data, &d.clone().with_me(viewer), &format!("{salt}-{}", g.game_id)) {
                                look = Some(id);
                            }
                        }
                    }
                }
            });
            ui.add_space(3.0);
        }
        look
    }

    fn lookup(&mut self, riot_id: String) {
        self.lookup_input = riot_id.clone();
        self.request(Action::Lookup(riot_id));
        self.tab = Tab::Players;
    }

    fn write_item_set(&mut self, root: Option<&std::path::Path>, data: &StaticData, g: &LolGame) {
        let r = root
            .ok_or_else(|| "League install not found".to_string())
            .and_then(|root| itemset::write_from_game(root, data, g))
            .map(|p| format!("Item set written: {}", p.display()));
        self.notify(r);
    }

    fn champ_select(&mut self, ui: &mut Ui, s: &Snapshot) {
        let Some(cs) = &s.champ_select else {
            section(ui, "Champ select");
            ui.colored_label(MUTED, "Not in champ select. This tab opens automatically when one starts.");
            if s.phase == "Lobby" && s.session.as_ref().is_some_and(|g| matches!(g.queue_id, 480 | 490)) {
                section(ui, "Swiftplay");
                ui.label("Swiftplay picks both champions before queueing; riftlite can fill both slots with Riot's recommended runes and spells.");
                if ui.button("Apply recommended runes to my Swiftplay picks").clicked() {
                    self.request(Action::SwiftplayRunes);
                }
            }
            return;
        };
        section(ui, &format!("Champ select · {}", if cs.phase.is_empty() { "…" } else { &cs.phase }));
        ui.columns(2, |cols| {
            for (ui, (title, team)) in cols.iter_mut().zip([("Your team", &cs.my_team), ("Enemy team", &cs.their_team)]) {
                card(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(RichText::new(title).strong());
                    Grid::new(title).spacing([12.0, 4.0]).show(ui, |ui| {
                        for p in team {
                            ui.label(RichText::new(position(&p.position)).color(MUTED));
                            let champ = RichText::new(s.data.champion_name(p.champion_id));
                            ui.label(if p.is_me { champ.strong().color(ACCENT) } else if p.picked { champ.strong() } else { champ.color(MUTED) });
                            ui.label(RichText::new(if p.is_me { "you" } else { p.name.as_str() }).color(MUTED));
                            ui.label(rank_of(s, &p.puuid).unwrap_or_default());
                            ui.end_row();
                        }
                    });
                });
            }
        });
        if !cs.bans.is_empty() {
            let bans: Vec<_> = cs.bans.iter().map(|&b| s.data.champion_name(b)).collect();
            ui.label(format!("Bans: {}", bans.join(", ")));
        }
        let history = s.lol_history.as_deref().unwrap_or(&[]);
        if cs.bench_enabled {
            section(ui, "Bench");
            if cs.bench.is_empty() {
                ui.colored_label(MUTED, "Bench is empty");
            }
            ui.horizontal_wrapped(|ui| {
                for &c in &cs.bench {
                    let a = averages(history.iter().filter(|g| g.champion_id == c));
                    let label = match &a {
                        Some(a) => format!("{} · {}G {:.0}%", s.data.champion_name(c), a.n, a.win_rate() * 100.0),
                        None => s.data.champion_name(c),
                    };
                    if ui.button(label).on_hover_text("Swap to this champion (your record on it from recent games)").clicked() {
                        self.request(Action::BenchSwap(c));
                    }
                }
            });
        }

        section(ui, "Riot recommended runes");
        if cs.my_champion().is_none() {
            ui.colored_label(MUTED, "Hover or lock a champion to see Riot's recommended pages.");
        } else if s.rec_pages.is_empty() {
            ui.colored_label(MUTED, "No recommended page for this pick and mode.");
        }
        for (i, p) in s.rec_pages.iter().enumerate() {
            card(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(s.data.perk_name(p.perks[0])).strong());
                    ui.label(RichText::new(&p.label).color(MUTED));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Apply spells").clicked() {
                            self.request(Action::ApplySpells(i));
                        }
                        if ui.button("Apply runes").clicked() {
                            self.request(Action::ApplyRunes(i));
                        }
                    });
                });
                rec_page_lines(ui, &s.data, p);
            });
        }
        if !s.champ_log.is_empty() {
            section(ui, "Automation");
            for (m, ok) in &s.champ_log {
                ui.colored_label(if *ok { WIN } else { LOSS }, m);
            }
        }

        section(ui, "Your history on this pick");
        let Some(champ) = cs.my_champion() else {
            ui.colored_label(MUTED, "Hover or lock a champion to see your recent games on it.");
            suggestions(ui, s, history);
            return;
        };
        let on: Vec<_> = history.iter().filter(|g| g.champion_id == champ).collect();
        let name = s.data.champion_name(champ);
        if on.is_empty() {
            ui.label(format!("No {name} games in your last {} matches.", history.len()));
        } else {
            let wins = on.iter().filter(|g| g.win).count();
            let kda = on.iter().map(|g| g.kda()).sum::<f64>() / on.len() as f64;
            ui.label(format!("{name}: {} games · {wins} wins · avg KDA {kda:.2}", on.len()));
        }
        if let Some(best) = itemset::best_game(history, champ) {
            ui.label(RichText::new(format!("Best recent build: {}", item_list(&s.data, &best.items))).color(MUTED));
            let root = s.league_root();
            if ui.button(format!("Save as {name} item set")).clicked() {
                self.write_item_set(root.as_deref(), &s.data, best);
            }
        }
        suggestions(ui, s, history);
    }

    fn post_game(&mut self, ui: &mut Ui, s: &Snapshot) {
        let Some(pg) = &s.post_game else {
            section(ui, "Post-game");
            ui.colored_label(MUTED, "Opens after a game with that game's stats and how it compares to your usual games.");
            return;
        };
        let d = &pg.detail;
        let Some(me) = d.me() else {
            section(ui, "Post-game");
            return void(ui.colored_label(MUTED, "You were not a player in this game."));
        };
        ui.add_space(10.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(if me.win { "Victory" } else { "Defeat" }).strong().size(22.0).color(if me.win { WIN } else { LOSS }));
            let q = if d.queue_id > 0 { queue_name(d.queue_id) } else { d.mode.clone() };
            ui.label(RichText::new(format!("{q} · {} · {}", mmss(d.duration_s as f64), datetime(d.created_ms))).color(MUTED));
        });
        if let Some(lp) = &pg.lp {
            let sign = if lp.delta >= 0 { "+" } else { "" };
            let rank = RankedQueue { queue: lp.queue.clone(), tier: lp.tier.clone(), division: lp.division.clone(), lp: lp.lp, wins: 0, losses: 0 };
            ui.label(RichText::new(format!("{sign}{} LP · {} now {}", lp.delta, rank.label(), rank.rank())).strong().color(if lp.delta >= 0 { WIN } else { LOSS }));
        }
        ui.label(format!(
            "{} · {} / {} / {} · CS {} · {:.1}k gold · {:.1}k damage",
            s.data.champion_name(me.champion_id),
            me.kills,
            me.deaths,
            me.assists,
            me.cs,
            me.gold as f64 / 1000.0,
            me.damage as f64 / 1000.0
        ));

        section(ui, "Insights");
        let history = s.lol_history.as_deref().unwrap_or(&[]);
        let others = || history.iter().filter(|g| g.game_id != d.game_id);
        let same_queue = averages(others().filter(|g| d.queue_id > 0 && g.queue_id == d.queue_id)).filter(|a| a.n >= 3);
        let (base, what) = match same_queue {
            Some(a) => (Some(a), format!("your other {} games", queue_name(d.queue_id))),
            None => (averages(others()), "your recent games".to_string()),
        };
        match base {
            None => void(ui.colored_label(MUTED, "Play a few more games for a baseline.")),
            Some(a) => {
                let mins = (d.duration_s as f64 / 60.0).max(1.0);
                let team_kills = d.team_sum(me.team, |p| p.kills).max(1);
                let team_dmg = d.team_sum(me.team, |p| p.damage).max(1);
                ui.label(RichText::new(format!("Compared with {what} ({} games)", a.n)).color(MUTED));
                Grid::new("insights").striped(true).spacing([16.0, 4.0]).show(ui, |ui| {
                    for h in ["", "This game", "Your average", ""] {
                        ui.label(RichText::new(h).color(MUTED));
                    }
                    ui.end_row();
                    let rows = [
                        ("KDA", (me.kills + me.assists) as f64 / me.deaths.max(1) as f64, a.kda(), true),
                        ("Deaths", me.deaths as f64, a.deaths, false),
                        ("CS / min", me.cs as f64 / mins, a.cs_min, true),
                        ("Gold / min", me.gold as f64 / mins, a.gold_min, true),
                        ("Damage / min", me.damage as f64 / mins, a.dmg_min, true),
                        ("Vision / min", me.vision as f64 / mins, a.vision_min, true),
                    ];
                    for (label, now, avg, higher_better) in rows {
                        if now == 0.0 && avg == 0.0 {
                            continue;
                        }
                        ui.label(label);
                        ui.label(RichText::new(format!("{now:.1}")).strong());
                        ui.label(format!("{avg:.1}"));
                        ui.label(trend(now, avg, higher_better));
                        ui.end_row();
                    }
                });
                ui.label(format!(
                    "Kill participation {:.0}% · {:.0}% of your team's damage",
                    (me.kills + me.assists) as f64 * 100.0 / team_kills as f64,
                    me.damage as f64 * 100.0 / team_dmg as f64
                ));
            }
        }
        let root = s.league_root();
        if root.is_some() && ui.button("Save this game's build as an item set").clicked() {
            let g = as_lol_game(d, me);
            self.write_item_set(root.as_deref(), &s.data, &g);
        }
        section(ui, "Scoreboard");
        if let Some(id) = match_table(ui, &s.data, d, "postgame") {
            self.lookup(id);
        }
    }

    fn players(&mut self, ui: &mut Ui, s: &Snapshot) {
        section(ui, "Look up a player");
        ui.horizontal(|ui| {
            let r = ui.add(egui::TextEdit::singleline(&mut self.lookup_input).hint_text("Name#TAG").desired_width(240.0));
            let go = ui.add_enabled(s.busy.is_none(), egui::Button::new("Look up")).clicked()
                || (r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)));
            if go && !self.lookup_input.trim().is_empty() {
                self.request(Action::Lookup(self.lookup_input.trim().to_string()));
            }
        });
        ui.label(RichText::new("Uses the League client's own lookup; works for players on your region.").color(MUTED));
        let Some(r) = &s.lookup else { return };
        let l = match r {
            Err(e) => return void(ui.colored_label(LOSS, e)),
            Ok(l) => l,
        };
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(RichText::new(&l.summoner.game_name).strong().size(20.0));
            ui.label(RichText::new(format!("#{}", l.summoner.tag_line)).size(20.0).color(MUTED));
            if l.summoner.level > 0 {
                ui.label(RichText::new(format!("level {}", l.summoner.level)).color(MUTED));
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Spectate live game").on_hover_text("Asks the League client to start spectating this player").clicked() {
                    self.request(Action::Spectate(l.summoner.puuid.clone()));
                }
            });
        });
        ui.columns(2, |cols| {
            card(&mut cols[0], |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Ranked").strong());
                ranked_grid(ui, &l.ranked, false);
                if let Ok(h) = &l.history {
                    ui.add_space(6.0);
                    lol_summary(ui, &s.data, h);
                }
            });
            card(&mut cols[1], |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Top mastery").strong());
                mastery_grid(ui, &s.data, &l.mastery, "mastery-lookup");
            });
        });
        section(ui, "Recent games");
        match &l.history {
            Err(e) => void(ui.colored_label(LOSS, e)),
            Ok(h) => {
                let puuid = l.summoner.puuid.clone();
                if let Some(id) = self.game_cards(ui, s, h, None, &puuid, "lookup") {
                    self.lookup(id);
                }
                if ui.button("Load 20 more").clicked() {
                    self.request(Action::LookupMore);
                }
            }
        }
    }

    fn champions(&mut self, ui: &mut Ui, s: &Snapshot) {
        section(ui, "Champions");
        if s.data.champions.is_empty() {
            return void(ui.colored_label(MUTED, "Loading Data Dragon…"));
        }
        ui.add(egui::TextEdit::singleline(&mut self.champ_filter).hint_text("Search").desired_width(220.0));
        let history = s.lol_history.as_deref().unwrap_or(&[]);
        let filter = self.champ_filter.to_lowercase();
        let mut list: Vec<(i64, &str)> = s
            .data
            .champions
            .iter()
            .filter(|(_, c)| filter.is_empty() || c.name.to_lowercase().contains(&filter))
            .map(|(&id, c)| (id, c.name.as_str()))
            .collect();
        list.sort_by(|a, b| a.1.cmp(b.1));
        ui.columns(2, |cols| {
            ScrollArea::vertical().id_salt("champ-list").max_height(520.0).show(&mut cols[0], |ui| {
                Grid::new("champ-grid").striped(true).spacing([12.0, 3.0]).show(ui, |ui| {
                    for (id, name) in &list {
                        let picked = self.champ_selected == Some(*id);
                        if ui.selectable_label(picked, *name).clicked() {
                            self.champ_selected = Some(*id);
                            self.request(Action::BrowseChampion(*id));
                        }
                        let a = averages(history.iter().filter(|g| g.champion_id == *id));
                        ui.label(RichText::new(a.map(|a| format!("{}G {:.0}%", a.n, a.win_rate() * 100.0)).unwrap_or_default()).color(MUTED));
                        let m = s.mastery.iter().find(|m| m.champion_id == *id);
                        ui.label(RichText::new(m.map(|m| format!("M{} · {}k", m.level, m.points / 1000)).unwrap_or_default()).color(MUTED));
                        ui.end_row();
                    }
                });
            });
            let ui = &mut cols[1];
            let Some(id) = self.champ_selected else {
                ui.colored_label(MUTED, "Pick a champion for Riot's recommended runes and your own stats on it.");
                return;
            };
            ui.label(RichText::new(s.data.champion_name(id)).strong().size(18.0));
            match averages(history.iter().filter(|g| g.champion_id == id)) {
                None => void(ui.colored_label(MUTED, format!("No games on it in your last {} matches", history.len()))),
                Some(a) => void(ui.label(format!(
                    "You: {} games · {:.0}% wins · KDA {:.2} · {:.1} CS/min · {:.0} damage/min",
                    a.n,
                    a.win_rate() * 100.0,
                    a.kda(),
                    a.cs_min,
                    a.dmg_min
                ))),
            }
            if let Some(best) = itemset::best_game(history, id) {
                ui.label(RichText::new(format!("Your best recent build: {}", item_list(&s.data, &best.items))).color(MUTED));
            }
            match &s.browse {
                Some((c, Ok(pages))) if *c == id => {
                    for p in pages {
                        card(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(position_label(&p.position)).strong());
                                ui.label(RichText::new(&p.label).color(MUTED));
                            });
                            rec_page_lines(ui, &s.data, p);
                        });
                    }
                }
                Some((c, Err(e))) if *c == id => void(ui.colored_label(LOSS, e)),
                _ if matches!(s.lcu, LcuStatus::Connected { .. }) => void(ui.colored_label(MUTED, "Loading recommendations…")),
                _ => void(ui.colored_label(MUTED, "Recommended runes need the League client running.")),
            }
        });
    }

    fn settings_tab(&mut self, ui: &mut Ui, s: &Snapshot) {
        section(ui, "Matchmaking");
        let mut changed = ui.checkbox(&mut self.settings.auto_accept, "Accept the ready check automatically when a match is found").changed();
        ui.label(RichText::new("Never overrides a check you already accepted or declined.").color(MUTED));
        section(ui, "Champ select");
        changed |= ui.checkbox(&mut self.settings.auto_runes, "On lock-in, apply Riot's recommended rune page").changed();
        ui.label(
            RichText::new("Written to one page named \"riftlite: <champion>\" (created if a slot is free, then reused); your own pages are never changed.")
                .color(MUTED),
        );
        changed |= ui.checkbox(&mut self.settings.auto_spells, "On lock-in, apply Riot's recommended summoner spells (Flash keeps its key)").changed();
        changed |= ui.checkbox(&mut self.settings.auto_item_set, "On lock-in, write an item set from your best recent game on the champion").changed();
        section(ui, "Overlay");
        changed |= ui.checkbox(&mut self.settings.overlay_enabled, "Show the in-game overlay during League games").changed();
        ui.label(
            RichText::new(
                "A separate always-on-top, click-through window fed by Riot's Live Client Data API. \
                 Nothing is injected into the game; it is visible when League runs in Borderless or Windowed mode.",
            )
            .color(MUTED),
        );
        ui.label(match self.hotkey {
            Some(true) => "Ctrl+Shift+O hides/shows it; the tray icon menu does the same.".to_string(),
            Some(false) => "Ctrl+Shift+O is taken by another program; use the tray icon menu to hide/show it.".to_string(),
            None => String::new(),
        });
        ui.horizontal(|ui| {
            ui.label("Offset from top-right corner:");
            changed |= ui.add(egui::DragValue::new(&mut self.settings.overlay_margin_right).range(0.0..=4000.0).suffix(" px right")).changed();
            changed |= ui.add(egui::DragValue::new(&mut self.settings.overlay_margin_top).range(0.0..=4000.0).suffix(" px down")).changed();
        });
        section(ui, "Behaviour");
        changed |= ui
            .checkbox(&mut self.settings.switch_tabs_automatically, "Switch to Champ select / Live game / Post-game tabs automatically")
            .changed();
        if changed {
            self.save_settings();
        }
        if ui.checkbox(&mut self.autostart, "Start with Windows (minimized)").changed() {
            if let Err(e) = settings::set_autostart(self.autostart) {
                self.autostart = settings::autostart_enabled();
                self.notify(Err(format!("Autostart: {e}")));
            }
        }
        section(ui, "League folder");
        ui.horizontal(|ui| {
            ui.add(egui::TextEdit::singleline(&mut self.folder_input).hint_text("detected automatically").desired_width(360.0));
            if ui.button("Apply").clicked() {
                let p = std::path::PathBuf::from(self.folder_input.trim());
                if !self.folder_input.trim().is_empty() && !p.join(Product::League.client_exe()).is_file() {
                    self.notify(Err(format!("{} not found in that folder", Product::League.client_exe())));
                } else {
                    self.settings.league_folder = self.folder_input.trim().to_string();
                    self.save_settings();
                    self.shared.refresh.store(true, Ordering::Relaxed);
                    self.notify(Ok("League folder saved".into()));
                }
            }
        });
        if let Some(i) = s.installs.get(Product::League) {
            ui.label(RichText::new(format!("In use: {} ({})", i.path.display(), i.source)).color(MUTED));
        }
        section(ui, "Item sets");
        let root = s.league_root();
        if ui.add_enabled(root.is_some(), egui::Button::new("Remove all item sets created by riftlite")).clicked() {
            let r = itemset::clear_all(root.as_deref().expect("button enabled only with a root"))
                .map(|n| format!("Removed {n} riftlite item set(s)"));
            self.notify(r);
        }
        section(ui, "Data");
        ui.label(format!("Settings, recorded skill orders and cache: {}", settings::local_dir().display()));
    }

    /// Deferred viewport: it has its own callback and is repainted by the
    /// worker (`request_repaint_of(overlay_id())`), so it keeps updating while
    /// the main window is minimized and eframe skips the main `ui` pass.
    fn show_overlay(&mut self, ctx: &egui::Context) {
        const SIZE: [f32; 2] = [330.0, 300.0];
        let monitor = ctx.input(|i| i.viewport().monitor_size).unwrap_or(egui::vec2(1920.0, 1080.0));
        let pos = [monitor.x - SIZE[0] - self.settings.overlay_margin_right, self.settings.overlay_margin_top];
        let builder = ViewportBuilder::default()
            .with_title("riftlite overlay")
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_mouse_passthrough(true)
            .with_taskbar(false)
            .with_resizable(false)
            .with_active(false)
            .with_position(pos)
            .with_inner_size(SIZE);
        let shared = self.shared.clone();
        ctx.show_viewport_deferred(overlay_id(), builder, move |ui, _class| {
            let snap = shared.snap.lock();
            let game = overlay_game(&snap.live);
            if game.is_none() && !loading_screen(&snap) {
                return;
            }
            // Only the content gets a backdrop; the rest of the viewport stays fully transparent.
            CentralPanel::default().frame(Frame::NONE).show(ui, |ui| {
                Frame::new()
                    .fill(Color32::from_black_alpha(185))
                    .corner_radius(8)
                    .inner_margin(Margin::same(10))
                    .show(ui, |ui| overlay_contents(ui, &snap, game));
            });
        });
    }
}

pub fn overlay_id() -> ViewportId {
    ViewportId::from_hash_of("riftlite-overlay")
}

fn void<T>(_: T) {}

fn no_data_hint(s: &Snapshot) -> &'static str {
    match s.lcu {
        LcuStatus::Connected { .. } if s.summoner.is_none() => "Waiting for the League client to finish logging in…",
        LcuStatus::Connected { .. } => "No games found.",
        _ => "Start the League client to load match history.",
    }
}

fn title(s: &str) -> String {
    let lower = s.to_lowercase().replace('_', " ");
    let mut c = lower.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

fn ranked_grid(ui: &mut Ui, ranked: &[RankedQueue], tft: bool) {
    Grid::new(("ranked", tft, ranked.len())).spacing([12.0, 4.0]).show(ui, |ui| {
        let queues: Vec<_> = ranked.iter().filter(|q| q.is_tft() == tft && q.worth_showing()).collect();
        if queues.is_empty() {
            ui.colored_label(MUTED, "No ranked data");
            ui.end_row();
        }
        for q in queues {
            ui.label(RichText::new(q.label()).color(MUTED));
            ui.label(RichText::new(q.rank()).strong());
            let total = q.wins + q.losses;
            if total > 0 {
                ui.label(format!("{}W {}L ({:.0}%)", q.wins, q.losses, q.wins as f64 * 100.0 / total as f64));
            } else {
                ui.label("");
            }
            ui.end_row();
        }
    });
}

fn mastery_grid(ui: &mut Ui, data: &StaticData, mastery: &[crate::lcu::Mastery], salt: &str) {
    if mastery.is_empty() {
        return void(ui.colored_label(MUTED, "No mastery data"));
    }
    Grid::new(salt).spacing([12.0, 3.0]).show(ui, |ui| {
        for m in mastery.iter().take(10) {
            ui.label(RichText::new(data.champion_name(m.champion_id)).strong());
            ui.label(format!("level {}", m.level));
            ui.label(RichText::new(format!("{} pts", m.points)).color(MUTED));
            ui.label(RichText::new(if m.last_play_ms > 0 { date(m.last_play_ms) } else { String::new() }).color(MUTED));
            ui.end_row();
        }
    });
}

fn lol_summary(ui: &mut Ui, data: &StaticData, games: &[LolGame]) {
    if games.is_empty() {
        return;
    }
    let wins = games.iter().filter(|g| g.win).count();
    let kda = games.iter().map(LolGame::kda).sum::<f64>() / games.len() as f64;
    ui.label(format!("Last {} games: {wins} wins ({:.0}%) · avg KDA {kda:.2}", games.len(), wins as f64 * 100.0 / games.len() as f64));
    let mut by_champ: Vec<(i64, usize, usize)> = Vec::new();
    for g in games {
        match by_champ.iter_mut().find(|c| c.0 == g.champion_id) {
            Some(c) => {
                c.1 += 1;
                c.2 += g.win as usize;
            }
            None => by_champ.push((g.champion_id, 1, g.win as usize)),
        }
    }
    by_champ.sort_by(|a, b| b.1.cmp(&a.1));
    let top: Vec<_> = by_champ.iter().take(5).map(|c| format!("{} {}G {}W", data.champion_name(c.0), c.1, c.2)).collect();
    ui.label(RichText::new(format!("Most played: {}", top.join(" · "))).color(MUTED));
}

/// Picks from your own record: champions you have won with recently, best first.
fn suggestions(ui: &mut Ui, s: &Snapshot, history: &[LolGame]) {
    let mut champs: Vec<i64> = history.iter().map(|g| g.champion_id).collect();
    champs.sort_unstable();
    champs.dedup();
    let mut rows: Vec<(i64, Avg)> = champs
        .into_iter()
        .filter(|c| s.pickable.is_empty() || s.pickable.contains(c))
        .filter_map(|c| Some((c, averages(history.iter().filter(|g| g.champion_id == c)).filter(|a| a.n >= 2)?)))
        .collect();
    if rows.is_empty() {
        return;
    }
    rows.sort_by(|a, b| b.1.win_rate().total_cmp(&a.1.win_rate()).then(b.1.n.cmp(&a.1.n)));
    section(ui, "Your strongest available picks");
    let list: Vec<_> = rows
        .iter()
        .take(6)
        .map(|(c, a)| format!("{} {:.0}% ({}G, KDA {:.1})", s.data.champion_name(*c), a.win_rate() * 100.0, a.n, a.kda()))
        .collect();
    ui.label(list.join(" · "));
    ui.label(RichText::new(format!("From your last {} games; owned and not banned.", history.len())).color(MUTED));
}

fn rec_page_lines(ui: &mut Ui, data: &StaticData, p: &RecPage) {
    let names = |ids: &[i64]| ids.iter().map(|&i| data.perk_name(i)).collect::<Vec<_>>().join(", ");
    ui.label(format!("{}: {}", data.perk_name(p.primary_style), names(&p.perks[..4])));
    ui.label(format!("{}: {}", data.perk_name(p.sub_style), names(&p.perks[4..6])));
    ui.label(RichText::new(format!("Shards: {}", names(&p.perks[6..]))).color(MUTED));
    if !p.spells.contains(&0) {
        ui.label(RichText::new(format!("Spells: {} + {}", data.spell_name(p.spells[0]), data.spell_name(p.spells[1]))).color(MUTED));
    }
}

/// Ten-player table; returns a Riot ID whose "Look up" button was pressed.
fn match_table(ui: &mut Ui, data: &StaticData, d: &MatchDetail, salt: &str) -> Option<String> {
    let mut look = None;
    let mut teams: Vec<i64> = d.players.iter().map(|p| p.team).collect();
    teams.sort_unstable();
    teams.dedup();
    for team in teams {
        let won = d.players.iter().any(|p| p.team == team && p.win);
        let kills = d.team_sum(team, |p| p.kills);
        let gold = d.team_sum(team, |p| p.gold);
        ui.add_space(4.0);
        ui.colored_label(
            if won { WIN } else { LOSS },
            format!("{} · {} · {kills} kills · {:.1}k gold", if team == 100 { "Blue" } else { "Red" }, if won { "won" } else { "lost" }, gold as f64 / 1000.0),
        );
        Grid::new((salt, team)).striped(true).spacing([12.0, 3.0]).show(ui, |ui| {
            for h in ["Champion", "Player", "K / D / A", "CS", "Damage", "Gold", "Items", ""] {
                ui.label(RichText::new(h).color(MUTED));
            }
            ui.end_row();
            for p in d.players.iter().filter(|p| p.team == team) {
                let champ = RichText::new(data.champion_name(p.champion_id));
                ui.label(if p.is_me { champ.strong().color(ACCENT) } else { champ.strong() });
                ui.label(&p.name);
                ui.label(format!("{} / {} / {}", p.kills, p.deaths, p.assists));
                ui.label(p.cs.to_string());
                ui.label(format!("{:.1}k", p.damage as f64 / 1000.0));
                ui.label(format!("{:.1}k", p.gold as f64 / 1000.0));
                ui.label(RichText::new(format!("{} items", p.items[..6].iter().filter(|&&i| i > 0).count())).color(MUTED))
                    .on_hover_text(item_list(data, &p.items));
                if !p.is_me && p.name.contains('#') && ui.small_button("Look up").clicked() {
                    look = Some(p.name.clone());
                }
                ui.end_row();
            }
        });
    }
    look
}

fn as_lol_game(d: &MatchDetail, p: &MatchPlayer) -> LolGame {
    LolGame {
        game_id: d.game_id,
        created_ms: d.created_ms,
        duration_s: d.duration_s,
        queue_id: d.queue_id,
        champion_id: p.champion_id,
        win: p.win,
        kills: p.kills,
        deaths: p.deaths,
        assists: p.assists,
        cs: p.cs,
        gold: p.gold,
        damage: p.damage,
        damage_taken: p.damage_taken,
        vision: p.vision,
        items: p.items,
    }
}

/// Per-game averages of the player's own games.
#[derive(Default, Clone, Copy)]
struct Avg {
    n: usize,
    wins: usize,
    kills: f64,
    deaths: f64,
    assists: f64,
    cs_min: f64,
    gold_min: f64,
    dmg_min: f64,
    vision_min: f64,
}

impl Avg {
    fn kda(&self) -> f64 {
        (self.kills + self.assists) / self.deaths.max(1.0)
    }

    fn win_rate(&self) -> f64 {
        self.wins as f64 / self.n.max(1) as f64
    }
}

fn averages<'a>(games: impl IntoIterator<Item = &'a LolGame>) -> Option<Avg> {
    let mut a = Avg::default();
    for g in games {
        let m = (g.duration_s as f64 / 60.0).max(1.0);
        a.n += 1;
        a.wins += g.win as usize;
        a.kills += g.kills as f64;
        a.deaths += g.deaths as f64;
        a.assists += g.assists as f64;
        a.cs_min += g.cs as f64 / m;
        a.gold_min += g.gold as f64 / m;
        a.dmg_min += g.damage as f64 / m;
        a.vision_min += g.vision as f64 / m;
    }
    if a.n == 0 {
        return None;
    }
    let n = a.n as f64;
    for v in [&mut a.kills, &mut a.deaths, &mut a.assists, &mut a.cs_min, &mut a.gold_min, &mut a.dmg_min, &mut a.vision_min] {
        *v /= n;
    }
    Some(a)
}

fn trend(now: f64, avg: f64, higher_better: bool) -> RichText {
    if avg <= 0.0 {
        return RichText::new("");
    }
    let pct = (now - avg) * 100.0 / avg;
    let good = (pct >= 0.0) == higher_better;
    let text = format!("{}{pct:.0}%", if pct >= 0.0 { "+" } else { "" });
    if pct.abs() < 5.0 { RichText::new(text).color(MUTED) } else { RichText::new(text).color(if good { WIN } else { LOSS }) }
}

/// Solo/Duo rank (Flex if unranked in Solo) of a scouted player.
fn rank_of(s: &Snapshot, puuid: &str) -> Option<String> {
    let q = s.ranks.get(puuid)?.as_ref().ok()?;
    let pick = |name: &str| q.iter().find(|r| r.queue == name && r.is_ranked());
    Some(match pick("RANKED_SOLO_5x5").map(|r| (r, "")).or_else(|| pick("RANKED_FLEX_SR").map(|r| (r, " (Flex)"))) {
        None => "Unranked".into(),
        Some((r, flex)) => {
            let total = r.wins + r.losses;
            let wr = if total > 0 { format!(" · {:.0}% of {total}", r.wins as f64 * 100.0 / total as f64) } else { String::new() };
            format!("{}{flex}{wr}", r.rank())
        }
    })
}

/// Puuid of a live player, matched through the gameflow session by champion and team.
fn live_puuid<'a>(s: &'a Snapshot, p: &LivePlayer) -> Option<&'a str> {
    let champ = s.data.champion_by_key(&p.champion_key)?;
    let team = if p.team == "ORDER" { 100 } else { 200 };
    s.session.as_ref()?.players.iter().find(|x| x.champion_id == champ && x.team == team).map(|x| x.puuid.as_str())
}

/// Your CS/min baseline for this pick: same champion (2+ games), else the queue, else everything.
fn cs_baseline(s: &Snapshot, champion_key: &str) -> Option<(f64, String)> {
    let history = s.lol_history.as_deref().ok()?;
    let champ = s.data.champion_by_key(champion_key);
    if let Some(a) = averages(history.iter().filter(|g| Some(g.champion_id) == champ)).filter(|a| a.n >= 2) {
        return Some((a.cs_min, format!("your {} avg", s.data.champion_name(champ?))));
    }
    let queue = s.session.as_ref().map_or(0, |g| g.queue_id);
    if let Some(a) = averages(history.iter().filter(|g| queue > 0 && g.queue_id == queue)).filter(|a| a.n >= 3) {
        return Some((a.cs_min, format!("your {} avg", queue_name(queue))));
    }
    averages(history).map(|a| (a.cs_min, "your avg".to_string()))
}

fn tft_summary(ui: &mut Ui, games: &[TftGame]) {
    if games.is_empty() {
        return;
    }
    let avg = games.iter().map(|g| g.placement as f64).sum::<f64>() / games.len() as f64;
    let top4 = games.iter().filter(|g| (1..=4).contains(&g.placement)).count();
    let firsts = games.iter().filter(|g| g.placement == 1).count();
    ui.label(format!("Last {} games: avg placement {avg:.2} · top 4 {top4} · 1st {firsts}", games.len()));
}

fn tft_matches(ui: &mut Ui, s: &Snapshot) {
    section(ui, "Recent TFT games");
    let games = match &s.tft_history {
        Err(e) => return void(ui.colored_label(LOSS, e)),
        Ok(g) if g.is_empty() => return void(ui.colored_label(MUTED, no_data_hint(s))),
        Ok(g) => g,
    };
    for g in games {
        card(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let color = match g.placement {
                    1 => Color32::GOLD,
                    2..=4 => WIN,
                    _ => LOSS,
                };
                ui.label(RichText::new(format!("#{}", g.placement)).strong().size(18.0).color(color));
                ui.label(queue_name(g.queue_id));
                ui.label(RichText::new(format!("Set {} · level {} · {}", g.set, g.level, mmss(g.length_s as f64))).color(MUTED));
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new(datetime(g.datetime_ms)).color(MUTED));
                });
            });
            let traits: Vec<_> = g.traits.iter().take(6).map(|t| format!("{} {}", t.1, tft_label(&t.0))).collect();
            ui.label(format!("Traits: {}", traits.join(" · ")));
            let units: Vec<_> = g.units.iter().map(|u| format!("{}{}", tft_label(&u.0), "★".repeat(u.1.max(1) as usize))).collect();
            ui.label(RichText::new(format!("Board: {}", units.join(", "))).color(MUTED));
        });
        ui.add_space(4.0);
    }
}

fn kilo(g: i64) -> String {
    format!("{:.1}k", g as f64 / 1000.0)
}

fn signed(v: i64) -> String {
    if v >= 0 { format!("+{v}") } else { v.to_string() }
}

fn live_tab(ui: &mut Ui, s: &Snapshot) {
    let Some(g) = &s.live else {
        section(ui, "Live game");
        let msg = if s.running.any_game() {
            "Game process found; waiting for the Live Client Data API (port 2999)…"
        } else {
            "No game running."
        };
        ui.colored_label(MUTED, msg);
        if loading_screen(s) {
            scouting(ui, s, None);
        }
        return;
    };
    section(ui, &format!("Live game · {} · {}", g.mode, mmss(g.game_time)));
    if let Some(me) = g.me() {
        let mins = (g.game_time / 60.0).max(1.0);
        ui.label(format!(
            "You: {} · level {} · {}/{}/{} · CS {} ({:.1}/min) · {:.0} gold",
            me.champion, g.my_level, me.kills, me.deaths, me.assists, me.cs, me.cs as f64 / mins, g.my_gold
        ));
        if let Some((avg, what)) = cs_baseline(s, &me.champion_key) {
            ui.label(RichText::new(format!("CS/min benchmark: {avg:.1} ({what})")).color(MUTED));
        }
        let enemy = if me.team == "ORDER" { "CHAOS" } else { "ORDER" };
        let (mine, theirs) = (g.team_gold(&me.team), g.team_gold(enemy));
        ui.label(format!("Item value: your team {} vs {} ({})", kilo(mine), kilo(theirs), signed(mine - theirs)));
        if let Some(opp) = g.lane_opponent() {
            ui.label(format!("Lane: you {} vs {} {} ({})", kilo(me.item_gold()), opp.champion, kilo(opp.item_gold()), signed(me.item_gold() - opp.item_gold())));
        }
        if let Some(h) = &s.skill_hint {
            ui.label(RichText::new(format!("Skill order this game: {} · your last game on {}: {}", h.current, me.champion, h.recorded)).color(MUTED));
        }
    } else if g.mode == "TFT" {
        ui.label(format!("TFT · level {} · {:.0} gold", g.my_level, g.my_gold));
    }
    if !g.objectives.is_empty() {
        section(ui, "Objectives");
        for o in &g.objectives {
            let (text, color) = objective_text(o, g.game_time);
            ui.colored_label(color, text);
        }
    }
    let mut teams: Vec<&str> = g.players.iter().map(|p| p.team.as_str()).collect();
    teams.sort();
    teams.dedup();
    for team in teams {
        let label = match team {
            "ORDER" => "Blue side",
            "CHAOS" => "Red side",
            t => t,
        };
        section(ui, &format!("{label} · {} kills · {} item value", g.team_kills(team), kilo(g.team_gold(team))));
        Grid::new(team).striped(true).spacing([14.0, 4.0]).show(ui, |ui| {
            for h in ["Champion", "Player", "Rank", "Lvl", "K / D / A", "CS", "Wards", "Items"] {
                ui.label(RichText::new(h).color(MUTED));
            }
            ui.end_row();
            for p in g.players.iter().filter(|p| p.team == team) {
                let champ = RichText::new(&p.champion);
                ui.label(if p.is_me { champ.strong().color(ACCENT) } else { champ.strong() });
                if p.dead {
                    ui.colored_label(LOSS, format!("{} (dead {:.0}s)", p.name, p.respawn));
                } else {
                    ui.label(&p.name);
                }
                ui.label(live_puuid(s, p).and_then(|id| rank_of(s, id)).unwrap_or_default());
                ui.label(p.level.to_string());
                ui.label(format!("{} / {} / {}", p.kills, p.deaths, p.assists));
                ui.label(p.cs.to_string());
                ui.label(format!("{:.0}", p.ward_score));
                let names: Vec<&str> = p.items.iter().map(|i| i.name.as_str()).collect();
                ui.label(RichText::new(format!("{} · {}", kilo(p.item_gold()), names.join(", "))).color(MUTED));
                ui.end_row();
            }
        });
    }
}

/// Enemy (or, before teams are known, all) ranks from the gameflow session.
fn scouting(ui: &mut Ui, s: &Snapshot, my_team: Option<i64>) {
    let Some(session) = &s.session else { return };
    let mine = my_team.or_else(|| {
        let me = s.summoner.as_ref()?.puuid.as_str();
        session.players.iter().find(|p| p.puuid == me).map(|p| p.team)
    });
    let enemies: Vec<_> = session.players.iter().filter(|p| mine.is_none_or(|t| p.team != t)).collect();
    if enemies.is_empty() {
        return;
    }
    ui.label(RichText::new("Enemy ranks").strong());
    for p in enemies {
        let rank = rank_of(s, &p.puuid).unwrap_or_else(|| "…".into());
        ui.label(format!("{} · {rank}", s.data.champion_name(p.champion_id)));
    }
}

fn objective_text(o: &crate::live::Objective, now: f64) -> (String, Color32) {
    let by = match o.taken_by.as_deref() {
        Some("ORDER") => " (blue took it)",
        Some("CHAOS") => " (red took it)",
        _ => "",
    };
    match o.ready_at {
        Some(t) if t > now => {
            let left = t - now;
            (format!("{} in {}{by}", o.label, mmss(left)), if left <= 30.0 { SOON } else { Color32::from_gray(235) })
        }
        Some(_) => (format!("{} up", o.label), WIN),
        None => (format!("{} down for {}", o.label, mmss(now - o.since)), MUTED),
    }
}

fn overlay_contents(ui: &mut Ui, s: &Snapshot, g: Option<&LiveGame>) {
    ui.style_mut().visuals.override_text_color = Some(Color32::from_gray(235));
    let Some(g) = g else {
        ui.label(RichText::new("riftlite · loading").color(ACCENT).strong());
        scouting(ui, s, None);
        return;
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new("riftlite").color(ACCENT).strong());
        ui.label(RichText::new(mmss(g.game_time)).strong().color(Color32::WHITE));
    });
    let Some(me) = g.me() else { return };
    let mins = (g.game_time / 60.0).max(1.0);
    let bench = cs_baseline(s, &me.champion_key).map(|(a, _)| format!(" · avg {a:.1}")).unwrap_or_default();
    ui.label(format!("{}/{}/{} · CS {} ({:.1}/m{bench})", me.kills, me.deaths, me.assists, me.cs, me.cs as f64 / mins));
    let enemy = if me.team == "ORDER" { "CHAOS" } else { "ORDER" };
    ui.label(format!("Kills {} – {} · Gold {:.0}", g.team_kills(&me.team), g.team_kills(enemy), g.my_gold));
    let (mine, theirs) = (g.team_gold(&me.team), g.team_gold(enemy));
    let mut items = format!("Items {} vs {} ({})", kilo(mine), kilo(theirs), signed(mine - theirs));
    if let Some(opp) = g.lane_opponent() {
        items += &format!(" · lane {}", signed(me.item_gold() - opp.item_gold()));
    }
    ui.label(items);
    if let Some(next) = s.skill_hint.as_ref().and_then(|h| h.next) {
        ui.colored_label(SOON, format!("Level up {next} (your last {} order)", me.champion));
    }
    for o in &g.objectives {
        // A respawned objective stays listed for one minute, then drops off until the next kill.
        if o.ready_at.is_some_and(|t| g.game_time - t > 60.0) {
            continue;
        }
        let (text, color) = objective_text(o, g.game_time);
        ui.colored_label(color, text);
    }
    let dead: Vec<_> = g.players.iter().filter(|p| p.team == enemy && p.dead).map(|p| format!("{} {:.0}s", p.champion, p.respawn)).collect();
    if !dead.is_empty() {
        ui.colored_label(LOSS, format!("Dead: {}", dead.join(", ")));
    }
    if g.game_time < SCOUT_SECS {
        scouting(ui, s, Some(if me.team == "ORDER" { 100 } else { 200 }));
    }
}

fn item_list(data: &StaticData, items: &[i64; 7]) -> String {
    let names: Vec<_> = items.iter().filter(|&&i| i > 0).map(|&i| data.item_name(i)).collect();
    if names.is_empty() { "no items".into() } else { names.join(", ") }
}

fn position(p: &str) -> &str {
    match p {
        "top" => "Top",
        "jungle" => "Jungle",
        "middle" => "Mid",
        "bottom" => "Bot",
        "utility" => "Support",
        "" => "—",
        other => other,
    }
}

fn position_label(p: &str) -> &str {
    match p {
        "TOP" => "Top",
        "JUNGLE" => "Jungle",
        "MIDDLE" => "Mid",
        "BOTTOM" => "Bot",
        "UTILITY" => "Support",
        "NONE" => "Any role",
        other => other,
    }
}

pub fn queue_name(id: i64) -> String {
    match id {
        400 => "Normal Draft",
        420 => "Ranked Solo/Duo",
        430 => "Normal Blind",
        440 => "Ranked Flex",
        450 => "ARAM",
        2400 => "ARAM Mayhem",
        480 | 490 => "Swiftplay",
        700 => "Clash",
        900 | 1010 => "ARURF",
        1700 | 1710 => "Arena",
        1090 => "TFT Normal",
        1100 => "TFT Ranked",
        1130 => "TFT Hyper Roll",
        1160 => "TFT Double Up",
        1210 => "TFT Choncc's Treasure",
        0 => "Custom",
        _ => return format!("Queue {id}"),
    }
    .into()
}

pub fn mmss(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

pub fn date(unix_ms: i64) -> String {
    let t = win::local_time(unix_ms);
    format!("{:04}-{:02}-{:02}", t.0, t.1, t.2)
}

pub fn datetime(unix_ms: i64) -> String {
    let t = win::local_time(unix_ms);
    format!("{:04}-{:02}-{:02} {:02}:{:02}", t.0, t.1, t.2, t.3, t.4)
}
