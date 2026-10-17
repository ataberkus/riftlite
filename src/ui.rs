//! egui UI: main window (sidebar navigation, one module per tab) and the click-through in-game overlay.

mod champ_select;
mod home;
mod icons;
mod live_tab;
mod matches;
mod overlay;
mod players;
mod post_game;
mod settings_tab;
mod stats;
mod theme;
mod widgets;

use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui::{
    self, Align, Align2, Area, CentralPanel, Color32, Frame, Layout, Margin, Order, Panel, RichText, ScrollArea, Sense,
    Stroke, StrokeKind, Ui, ViewportCommand, ViewportId, vec2,
};

use crate::live::LiveGame;
use crate::meta;
use crate::settings::{self, Settings};
use crate::worker::{Action, LcuStatus, Shared, Snapshot};
use icons::Icons;
use stats::queue_name;
pub use stats::date;
use theme::*;
use widgets::*;

/// Content column width on wide windows.
const CONTENT_MAX: f32 = 1180.0;
/// Below this window width the sidebar shows icons only.
const COMPACT_BELOW: f32 = 980.0;

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

impl Tab {
    fn label(self) -> &'static str {
        match self {
            Tab::Home => "Home",
            Tab::Profile => "Profile",
            Tab::League => "LoL matches",
            Tab::Tft => "TFT matches",
            Tab::ChampSelect => "Champ select",
            Tab::Live => "Live game",
            Tab::PostGame => "Post-game",
            Tab::Players => "Players",
            Tab::Champions => "Champions",
            Tab::Settings => "Settings",
        }
    }

    fn icon(self) -> Icon {
        match self {
            Tab::Home => Icon::Home,
            Tab::Profile => Icon::Profile,
            Tab::League => Icon::Swords,
            Tab::Tft => Icon::Hex,
            Tab::ChampSelect => Icon::Target,
            Tab::Live => Icon::Live,
            Tab::PostGame => Icon::Chart,
            Tab::Players => Icon::Search,
            Tab::Champions => Icon::Grid,
            Tab::Settings => Icon::Gear,
        }
    }

    fn subtitle(self, s: &Snapshot) -> String {
        match self {
            Tab::Home => "Your Riot games, client status and static data".into(),
            Tab::Profile => "Ranks, recent form, mastery and challenges".into(),
            Tab::League => "Recent League of Legends games".into(),
            Tab::Tft => "Recent Teamfight Tactics games".into(),
            Tab::ChampSelect => match &s.champ_select {
                Some(cs) => champ_select::cs_phase_label(cs).to_string(),
                None => "Builds, runes and picks while you draft".into(),
            },
            Tab::Live => match &s.live {
                Some(g) => format!("{} · {}", g.mode, stats::mmss(g.game_time)),
                None => "Live Client Data during your game".into(),
            },
            Tab::PostGame => "Your last game next to your usual games".into(),
            Tab::Players => "Look up a player on your region".into(),
            Tab::Champions => "Riot's recommended runes and your record per champion".into(),
            Tab::Settings => "Automation, overlay and data".into(),
        }
    }
}

/// Sidebar groups; Settings is pinned to the bottom.
const NAV: &[(&str, &[Tab])] = &[
    ("", &[Tab::Home, Tab::Profile]),
    ("Matches", &[Tab::League, Tab::Tft]),
    ("Game", &[Tab::ChampSelect, Tab::Live, Tab::PostGame]),
    ("Explore", &[Tab::Players, Tab::Champions]),
];

pub struct App {
    shared: Arc<Shared>,
    settings: Settings,
    autostart: bool,
    tab: Tab,
    seen_phase: String,
    notice: Option<(String, bool, Instant)>,
    seen_action: u64,
    start_minimized: bool,
    /// How the last main `ui` pass declared the overlay viewport (`None`: not declared).
    declared_overlay: Option<OverlayDecl>,
    expanded: HashSet<u64>,
    lookup_input: String,
    champ_filter: String,
    champ_selected: Option<i64>,
    /// OP.GG build group the user opened in champ select, per champion id.
    meta_group: Option<(i64, meta::Group)>,
    folder_input: String,
    hotkeys: Option<crate::tray::Hotkeys>,
    hotkeys_rx: Receiver<crate::tray::Hotkeys>,
    /// Logo, banner and Data Dragon images (shared with the overlay viewport).
    icons: Arc<Icons>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, start_minimized: bool, title: &'static str) -> Self {
        theme::install(&cc.egui_ctx);
        let icons = Icons::new(&cc.egui_ctx);
        let settings = Settings::load();
        let shared = crate::worker::spawn(cc.egui_ctx.clone(), settings.clone());
        let hotkeys_rx = crate::tray::spawn(cc.egui_ctx.clone(), shared.clone(), title);
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
            declared_overlay: None,
            expanded: HashSet::new(),
            lookup_input: String::new(),
            champ_filter: String::new(),
            champ_selected: None,
            meta_group: None,
            hotkeys: None,
            hotkeys_rx,
            icons,
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

    /// How the overlay viewport should be declared now; `None`: not shown.
    fn overlay_decl(&mut self, ctx: &egui::Context, s: &Snapshot) -> Option<OverlayDecl> {
        let edit = self.shared.overlay_edit.load(Ordering::Relaxed);
        let in_game = overlay_game(&s.live).is_some() || loading_screen(s);
        // Edit mode also shows it outside a game (with a sample game), so Settings → Overlay
        // can open it.
        if !self.settings.overlay_enabled || self.shared.overlay_hidden.load(Ordering::Relaxed) || !(edit || in_game) {
            return None;
        }
        let monitor_width = ctx.input(|i| i.viewport().monitor_size).map_or(1920.0, |m| m.x);
        let (pos, placed) = self.settings.resolve_overlay_pos(monitor_width);
        if placed {
            self.save_settings();
        }
        Some(OverlayDecl { edit, pos, size: self.settings.overlay_size })
    }

    /// Takes the layout the overlay changed in edit mode into these settings and saves it.
    fn sync_overlay_layout(&mut self) {
        if !self.shared.overlay_layout_dirty.swap(false, Ordering::Relaxed) {
            return;
        }
        let s = self.shared.settings.lock();
        self.settings.overlay_pos = s.overlay_pos;
        self.settings.overlay_size = s.overlay_size;
        self.settings.overlay_sections = s.overlay_sections;
        drop(s);
        self.save_settings();
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

/// How the overlay viewport is declared; a change needs a main pass to redeclare it.
#[derive(Clone, Copy, PartialEq)]
struct OverlayDecl {
    /// Edit mode: not click-through, dragged and resized by the user.
    edit: bool,
    /// Panel position and size in points (`Settings::overlay_pos`, `overlay_size`).
    pos: [f32; 2],
    size: [f32; 2],
}

impl OverlayDecl {
    /// What the builder sends to the window. In edit mode the window itself is moved and
    /// resized, so position and size are not redeclared (that would fight the drag).
    fn window_key(self) -> (bool, Option<([f32; 2], [f32; 2])>) {
        (self.edit, (!self.edit).then_some((self.pos, self.size)))
    }
}

pub const OVERLAY_TITLE: &str = "riftlite overlay";

pub fn overlay_id() -> ViewportId {
    ViewportId::from_hash_of("riftlite-overlay")
}

/// Turns overlay edit mode on or off (hotkey, tray menu, Settings, the overlay's Done
/// button). Turning it on also shows an overlay hidden with Ctrl+Shift+O.
pub fn set_overlay_edit(ctx: &egui::Context, shared: &Shared, on: bool) {
    shared.overlay_edit.store(on, Ordering::Relaxed);
    if on {
        shared.overlay_hidden.store(false, Ordering::Relaxed);
    }
    // The main pass redeclares the viewport; `logic` requests it even when minimized.
    ctx.request_repaint_of(ViewportId::ROOT);
    ctx.request_repaint_of(overlay_id());
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Phase transitions are handled here, not in `ui`, so they are seen while minimized.
        {
            let snap = self.shared.snap.lock();
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
        }
        // While minimized eframe skips `ui`, which is where the overlay viewport
        // is declared. Request one hidden pass only when it must appear, go away, or
        // change mode, place or size; its contents repaint on their own.
        self.sync_overlay_layout();
        let shared = self.shared.clone();
        let snap = shared.snap.lock();
        let want = self.overlay_decl(ctx, &snap);
        drop(snap);
        let minimized = ctx.input(|i| i.viewport().minimized.unwrap_or(false));
        if minimized && want.map(OverlayDecl::window_key) != self.declared_overlay.map(OverlayDecl::window_key) {
            ctx.send_viewport_cmd(ViewportCommand::RequestPaintWhileHidden);
        }
    }

    fn ui(&mut self, ui: &mut Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if std::mem::take(&mut self.start_minimized) {
            ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
        }
        if let Ok(h) = self.hotkeys_rx.try_recv() {
            self.hotkeys = Some(h);
        }
        let shared = self.shared.clone();
        let snap = shared.snap.lock();

        if let Some((seq, msg, ok)) = &snap.action_result {
            if *seq != self.seen_action {
                self.seen_action = *seq;
                self.notify(if *ok { Ok(msg.clone()) } else { Err(msg.clone()) });
            }
        }

        let compact = ctx.content_rect().width() < COMPACT_BELOW;
        self.sidebar(ui, &snap, compact);
        self.header(ui, &snap);
        Panel::bottom("status")
            .exact_size(30.0)
            .frame(Frame::new().fill(SIDEBAR).inner_margin(Margin::symmetric(16, 0)))
            .show(ui, |ui| {
                let r = ui.max_rect();
                ui.painter().hline(r.x_range(), r.top() - 0.5, Stroke::new(1.0, BORDER));
                ui.horizontal_centered(|ui| {
                    status_line(ui, &snap);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if !snap.data.version.is_empty() {
                            ui.label(RichText::new(format!("Data Dragon {}", snap.data.version)).size(11.5).color(FAINT));
                        }
                    });
                });
            });

        CentralPanel::default().frame(Frame::new().fill(BG)).show(ui, |ui| {
            ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                let avail = ui.available_width();
                let pad = if compact { 16.0 } else { 28.0 };
                let width = (avail - 2.0 * pad).min(CONTENT_MAX);
                let left = ((avail - width) / 2.0).max(pad);
                ui.add_space(18.0);
                ui.horizontal_top(|ui| {
                    ui.add_space(left);
                    ui.vertical(|ui| {
                        ui.set_width(width);
                        match self.tab {
                            Tab::Home => self.home(ui, &snap),
                            Tab::Profile => self.profile(ui, &snap),
                            Tab::League => self.lol_matches(ui, &snap),
                            Tab::Tft => self.tft_matches(ui, &snap),
                            Tab::ChampSelect => self.champ_select(ui, &snap),
                            Tab::Live => self.live_tab(ui, &snap),
                            Tab::PostGame => self.post_game(ui, &snap),
                            Tab::Players => self.players(ui, &snap),
                            Tab::Champions => self.champions(ui, &snap),
                            Tab::Settings => self.settings_tab(ui, &snap),
                        }
                        ui.add_space(28.0);
                    });
                });
            });
        });

        self.toast(&ctx);
        let decl = self.overlay_decl(&ctx, &snap);
        drop(snap);
        let prev = std::mem::replace(&mut self.declared_overlay, decl);
        // eframe applies a redeclared place, size or callback without repainting the overlay
        // (only an actual resize does), so badges could wait for the next live poll.
        if decl.is_some() && prev.is_some() && decl.map(OverlayDecl::window_key) != prev.map(OverlayDecl::window_key) {
            ctx.request_repaint_of(overlay_id());
        }
        let was_declared = prev.is_some();
        if let Some(decl) = decl {
            self.show_overlay(&ctx, !was_declared, decl);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        // Transparent so the overlay viewport can be see-through; every panel
        // in the main window paints an opaque fill.
        [0.0; 4]
    }
}

impl App {
    fn sidebar(&mut self, ui: &mut Ui, s: &Snapshot, compact: bool) {
        let icons = self.icons.clone();
        let g = Gfx::new(&icons, &s.data);
        let width = if compact { 68.0 } else { 224.0 };
        let margin = if compact { Margin::symmetric(10, 16) } else { Margin { left: 14, right: 14, top: 18, bottom: 12 } };
        Panel::left("sidebar")
            .exact_size(width)
            .resizable(false)
            .frame(Frame::new().fill(SIDEBAR).inner_margin(margin))
            .show(ui, |ui| {
                let full = ui.max_rect().expand2(vec2(margin.right as f32, margin.top as f32));
                ui.painter().vline(full.right() - 0.5, full.y_range(), Stroke::new(1.0, BORDER));
                // Brand.
                ui.horizontal(|ui| {
                    if compact {
                        ui.add_space((ui.available_width() - 34.0) / 2.0);
                    } else {
                        ui.add_space(4.0);
                    }
                    let (r, _) = ui.allocate_exact_size(vec2(34.0, 34.0), Sense::hover());
                    ui.painter().image(icons.logo.id, r, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), Color32::WHITE);
                    if !compact {
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.label(RichText::new("riftlite").font(display(24.0)).color(TEXT));
                            ui.label(RichText::new("League companion").size(11.0).color(MUTED));
                        });
                    }
                });
                ui.add_space(18.0);
                for (group, tabs) in NAV {
                    if !group.is_empty() {
                        ui.add_space(10.0);
                        if compact {
                            let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), Sense::hover());
                            ui.painter().hline(r.shrink2(vec2(8.0, 0.0)).x_range(), r.center().y, Stroke::new(1.0, BORDER));
                            ui.add_space(6.0);
                        } else {
                            ui.horizontal(|ui| {
                                ui.add_space(10.0);
                                overline(ui, group);
                            });
                            ui.add_space(2.0);
                        }
                    }
                    for &t in *tabs {
                        let badge = match t {
                            Tab::Live if s.live.is_some() => Some(("LIVE", LOSS)),
                            Tab::ChampSelect if s.champ_select.is_some() => Some(("NOW", CYAN)),
                            Tab::PostGame if s.post_game.is_some() && s.phase == "EndOfGame" => Some(("NEW", WIN)),
                            _ => None,
                        };
                        if nav_item(ui, t, self.tab == t, compact, badge).clicked() {
                            self.tab = t;
                        }
                    }
                }
                ui.with_layout(Layout::bottom_up(Align::Min), |ui| {
                    self.summoner_chip(ui, &g, s, compact);
                    ui.add_space(6.0);
                    if nav_item(ui, Tab::Settings, self.tab == Tab::Settings, compact, None).clicked() {
                        self.tab = Tab::Settings;
                    }
                });
            });
    }

    /// Profile icon, name and Solo/Duo rank; opens the Profile tab.
    fn summoner_chip(&mut self, ui: &mut Ui, g: &Gfx, s: &Snapshot, compact: bool) {
        let Some(sum) = &s.summoner else {
            let (color, text) = match &s.lcu {
                LcuStatus::Connected { .. } => (SOON, "Logging in…"),
                LcuStatus::ClientNotRunning => (MUTED, "Client offline"),
                LcuStatus::Error(_) => (LOSS, "Client error"),
            };
            ui.horizontal(|ui| {
                ui.add_space(if compact { (ui.available_width() - 10.0) / 2.0 } else { 10.0 });
                dot(ui, color);
                if !compact {
                    ui.label(RichText::new(text).size(12.0).color(MUTED));
                }
            });
            return;
        };
        let solo = s.ranked.iter().find(|r| r.queue == "RANKED_SOLO_5x5");
        let bg = ui.painter().add(egui::Shape::Noop);
        let inner = Frame::new().inner_margin(if compact { Margin::same(6) } else { Margin::symmetric(8, 8) }).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                if compact {
                    ui.add_space((ui.available_width() - 34.0) / 2.0);
                }
                g.profile(ui, sum.profile_icon, &sum.game_name, 34.0);
                if !compact {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.add(egui::Label::new(RichText::new(&sum.game_name).font(semibold(13.0)).color(TEXT)).truncate());
                        match solo.filter(|r| r.is_ranked()) {
                            Some(r) => ui.label(RichText::new(r.rank()).size(11.5).color(tier_color(&r.tier))),
                            None => ui.label(RichText::new(format!("Level {}", sum.level)).size(11.5).color(MUTED)),
                        };
                    });
                }
            });
        });
        let rect = inner.response.rect;
        let resp = ui.interact(rect, ui.scope_id().with("summoner-chip"), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
        let fill = if resp.hovered() || self.tab == Tab::Profile { HOVER } else { RAISED };
        ui.painter().set(bg, egui::epaint::RectShape::new(rect, 10, fill, Stroke::new(1.0, BORDER), StrokeKind::Inside));
        if resp.on_hover_text(format!("{}#{}", sum.game_name, sum.tag_line)).clicked() {
            self.tab = Tab::Profile;
        }
    }

    /// Page title, subtitle, busy indicator and Refresh.
    fn header(&mut self, ui: &mut Ui, s: &Snapshot) {
        Panel::top("header").frame(Frame::new().fill(BG).inner_margin(Margin { left: 28, right: 24, top: 18, bottom: 14 })).show(ui, |ui| {
            let r = ui.max_rect();
            ui.painter().hline((r.left() - 28.0)..=(r.right() + 24.0), r.bottom() + 14.5, Stroke::new(1.0, BORDER));
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    ui.label(RichText::new(self.tab.label()).font(display(30.0)).color(TEXT));
                    ui.label(RichText::new(self.tab.subtitle(s)).color(MUTED));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(Btn::secondary("Refresh").icon(Icon::Refresh)).on_hover_text("Reload everything from the League client").clicked() {
                        self.shared.refresh.store(true, Ordering::Relaxed);
                    }
                    if let Some(b) = s.busy {
                        ui.add_space(6.0);
                        ui.label(RichText::new(b).color(BODY));
                        ui.add(egui::Spinner::new().size(14.0).color(ACCENT));
                    }
                });
            });
        });
    }

    /// The last action's result, bottom-right, fading out after 8 s.
    fn toast(&mut self, ctx: &egui::Context) {
        const SHOW: f32 = 8.0;
        let Some((msg, ok, at)) = &self.notice else { return };
        let age = at.elapsed().as_secs_f32();
        if age >= SHOW {
            self.notice = None;
            return;
        }
        let opacity = ((SHOW - age) / 0.6).min(1.0);
        let color = result_color(*ok);
        Area::new(egui::Id::unique("toast")).anchor(Align2::RIGHT_BOTTOM, vec2(-20.0, -44.0)).order(Order::Foreground).interactable(false).show(
            ctx,
            |ui| {
                ui.multiply_opacity(opacity);
                Frame::new()
                    .fill(RAISED)
                    .stroke(Stroke::new(1.0, alpha(color, 0.55)))
                    .corner_radius(10)
                    .shadow(egui::Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(120) })
                    .inner_margin(Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        ui.set_max_width(420.0);
                        status_msg(ui, *ok, msg.as_str());
                    });
            },
        );
        ctx.request_repaint_after(Duration::from_millis(if opacity < 1.0 { 30 } else { 250 }));
    }
}

/// One sidebar entry: icon, label, optional badge; accent bar and tint when selected.
fn nav_item(ui: &mut Ui, tab: Tab, selected: bool, compact: bool, badge: Option<(&str, Color32)>) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 38.0), Sense::click());
    let resp = resp.on_hover_cursor(egui::CursorIcon::PointingHand);
    let hovered = resp.hovered();
    let p = ui.painter();
    if selected {
        p.rect(rect, 9, alpha(ACCENT, 0.16), Stroke::new(1.0, alpha(ACCENT, 0.28)), StrokeKind::Inside);
        let bar = egui::Rect::from_min_size(rect.left_top() + vec2(0.0, 9.0), vec2(3.0, rect.height() - 18.0));
        p.rect_filled(bar, 2, ACCENT);
    } else if hovered {
        p.rect_filled(rect, 9, HOVER);
    }
    let fg = if selected {
        ACCENT_TEXT
    } else if hovered {
        TEXT
    } else {
        MUTED
    };
    let icon_rect = if compact {
        egui::Rect::from_center_size(rect.center(), vec2(18.0, 18.0))
    } else {
        egui::Rect::from_center_size(egui::pos2(rect.left() + 22.0, rect.center().y), vec2(18.0, 18.0))
    };
    paint_icon(p, icon_rect, tab.icon(), fg);
    if !compact {
        p.text(
            egui::pos2(rect.left() + 42.0, rect.center().y),
            Align2::LEFT_CENTER,
            tab.label(),
            semibold(13.5),
            if selected || hovered { TEXT } else { BODY },
        );
    }
    if let Some((text, color)) = badge {
        if compact {
            p.circle_filled(icon_rect.right_top() + vec2(2.0, 0.0), 4.0, color);
        } else {
            let galley = p.layout_no_wrap(text.to_string(), semibold(9.5), color);
            let size = galley.size() + vec2(10.0, 4.0);
            let r = egui::Rect::from_min_size(egui::pos2(rect.right() - 8.0 - size.x, rect.center().y - size.y / 2.0), size);
            p.rect(r, (size.y / 2.0) as u8, alpha(color, 0.16), Stroke::new(1.0, alpha(color, 0.5)), StrokeKind::Inside);
            p.galley(r.center() - galley.size() / 2.0, galley, color);
        }
    }
    let resp = if compact { resp.on_hover_text(tab.label()) } else { resp };
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Tab, true, selected, tab.label()));
    resp
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
    dot(ui, color);
    ui.label(RichText::new(text).size(12.0).color(BODY));
    if let Some(q) = s.session.as_ref().and_then(|g| g.description.as_deref()) {
        ui.label(RichText::new(format!("· {q}")).size(12.0).color(MUTED));
    }
}
