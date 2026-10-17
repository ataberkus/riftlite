//! The click-through in-game overlay: a separate always-on-top viewport.
//!
//! Edit mode (Ctrl+Shift+E, tray menu, Settings) makes it clickable: drag to move, the
//! corner grip resizes, checkboxes pick the sections. The window itself is moved and
//! resized; its new place is written to `Shared::settings` and saved by the main window.

use eframe::egui::{
    self, Align, Color32, CursorIcon, Frame, Id, Layout, Margin, Rect, ResizeDirection, RichText, Sense, Shadow, Stroke,
    StrokeKind, Ui, UiBuilder, ViewportBuilder, ViewportCommand, ViewportId, pos2, vec2,
};

use super::live_tab::{diff_color, objective_text, scouts};
use super::stats::{cs_baseline, kilo, mmss, signed};
use super::theme::*;
use super::widgets::*;
use super::{App, OVERLAY_TITLE, OverlayDecl, loading_screen, overlay_game, overlay_id, set_overlay_edit};
use crate::live::{LiveGame, LiveItem, LivePlayer, Objective};
use crate::settings::{OVERLAY_MIN_SIZE, OverlaySections};
use crate::worker::{Shared, Snapshot};

/// The overlay lists enemy ranks while the game is this young (loading screen and laning start).
const SCOUT_SECS: f64 = 90.0;
/// Empty space around the panel inside the viewport (room for its shadow).
const INSET: f32 = 6.0;
/// Side of the edit-mode resize grip in the bottom-right corner.
const GRIP: f32 = 18.0;

impl App {
    /// Deferred viewport: it has its own callback and is repainted by the
    /// worker (`request_repaint_of(overlay_id())`), so it keeps updating while
    /// the main window is minimized and eframe skips the main `ui` pass.
    ///
    /// `created`: the previous main pass did not declare the overlay, so eframe creates
    /// a new window for it after this pass.
    pub(super) fn show_overlay(&mut self, ctx: &egui::Context, created: bool, decl: OverlayDecl) {
        let pos = pos2(decl.pos[0], decl.pos[1]);
        let size = vec2(decl.size[0], decl.size[1]);
        let mut builder = ViewportBuilder::default()
            .with_title(OVERLAY_TITLE)
            .with_decorations(false)
            .with_transparent(true)
            .with_always_on_top()
            .with_mouse_passthrough(!decl.edit)
            .with_taskbar(false)
            .with_resizable(decl.edit)
            .with_min_inner_size(OVERLAY_MIN_SIZE)
            .with_active(false);
        // In edit mode the user moves and resizes the window itself; redeclaring the
        // (one pass old) saved place would fight the drag.
        if !decl.edit || created {
            builder = builder.with_position(pos).with_inner_size(size);
        }
        // egui-winit gives every undecorated window a DWM drop shadow on Windows, and no
        // builder option turns it off. Over the game that shadow outlines the otherwise
        // invisible panel box, so the overlay callback switches it off once per window.
        let shadow_off = Id::unique("riftlite-overlay-shadow-off");
        if created {
            ctx.data_mut(|d| d.remove::<bool>(shadow_off));
        }
        let shared = self.shared.clone();
        let icons = self.icons.clone();
        ctx.show_viewport_deferred(overlay_id(), builder, move |ui, _class| {
            let ctx = ui.ctx().clone();
            let done = ctx.data(|d| d.get_temp::<bool>(shadow_off).unwrap_or(false));
            if !done && crate::win::remove_window_shadow(OVERLAY_TITLE) {
                ctx.data_mut(|d| d.insert_temp(shadow_off, true));
            }
            if decl.edit {
                save_window_place(&ctx, &shared);
            } else {
                ctx.data_mut(|d| d.remove::<bool>(moved_id()));
            }
            let sections = shared.settings.lock().overlay_sections;
            let snap = shared.snap.lock();
            let live = overlay_game(&snap.live);
            let loading = loading_screen(&snap);
            // Edit mode outside a game: a sample game shows every section.
            let sample = (decl.edit && live.is_none() && !loading).then(sample_game);
            let game = live.or(sample.as_ref());
            if game.is_none() && !loading {
                return;
            }
            let g = Gfx::new(&icons, &snap.data);
            // The window's actual size, which follows a resize in edit mode.
            let outer = ctx.content_rect();
            let panel_rect = outer.shrink(INSET);
            ui.scope_builder(UiBuilder::new().max_rect(panel_rect), |ui| {
                // Edit mode fills the box; content that does not fit is cut at its border.
                ui.set_clip_rect(if decl.edit { panel_rect } else { outer });
                ui.style_mut().interaction.selectable_labels = false;
                let move_area = decl.edit.then(|| ui.interact(outer, Id::unique("riftlite-overlay-move"), Sense::drag()));
                Frame::new()
                    .fill(Color32::from_rgba_unmultiplied(10, 13, 19, 222))
                    .stroke(Stroke::new(1.0, alpha(ACCENT, 0.35)))
                    .corner_radius(10)
                    .shadow(Shadow { offset: [0, 2], blur: 6, spread: 0, color: Color32::from_black_alpha(90) })
                    .inner_margin(Margin::symmetric(12, 10))
                    .show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        if decl.edit {
                            // The whole box, so its size is visible while resizing.
                            ui.set_min_height(ui.available_height());
                            edit_bar(ui, &shared, sample.is_some());
                        }
                        overlay_contents(ui, &g, &snap, game, sections, sample.is_some());
                    });
                if let Some(area) = move_area {
                    move_and_resize(ui, &area, panel_rect);
                }
            });
        });
    }
}

/// Set once the user starts moving or resizing the overlay in this edit session; from
/// then on the window's place is the layout to save.
fn moved_id() -> Id {
    Id::unique("riftlite-overlay-moved")
}

/// Edit mode: dragging the panel moves the window, the corner grip resizes it (both
/// native, so the OS keeps it under the cursor).
fn move_and_resize(ui: &mut Ui, area: &egui::Response, panel: Rect) {
    let ctx = ui.ctx().clone();
    let grip = Rect::from_min_max(panel.max - vec2(GRIP, GRIP), panel.max);
    let resize = ui.interact(grip, Id::unique("riftlite-overlay-resize"), Sense::drag());
    let p = ui.painter();
    // On top of the content, so the box stays outlined when the content is taller.
    p.rect_stroke(panel, 10, Stroke::new(1.5, ACCENT), StrokeKind::Inside);
    for i in 1..=3 {
        let d = i as f32 * 4.5;
        p.line_segment([pos2(grip.max.x - d, grip.max.y - 3.0), pos2(grip.max.x - 3.0, grip.max.y - d)], Stroke::new(1.5, ACCENT));
    }
    if resize.hovered() || resize.dragged() {
        ctx.set_cursor_icon(CursorIcon::ResizeSouthEast);
    } else if area.hovered() || area.dragged() {
        ctx.set_cursor_icon(CursorIcon::Move);
    }
    if resize.drag_started() {
        ctx.data_mut(|d| d.insert_temp(moved_id(), true));
        ctx.send_viewport_cmd(ViewportCommand::BeginResize(ResizeDirection::SouthEast));
    } else if area.drag_started() {
        ctx.data_mut(|d| d.insert_temp(moved_id(), true));
        ctx.send_viewport_cmd(ViewportCommand::StartDrag);
    }
}

/// After a move or resize in edit mode, writes the window's place into the shared
/// settings for the main window to save.
fn save_window_place(ctx: &egui::Context, shared: &Shared) {
    if !ctx.data(|d| d.get_temp::<bool>(moved_id()).unwrap_or(false)) {
        return;
    }
    let Some((outer, inner)) = ctx.input(|i| Some((i.viewport().outer_rect?, i.viewport().inner_rect?))) else { return };
    let pos = [outer.min.x.round(), outer.min.y.round()];
    let size = [inner.width().round(), inner.height().round()];
    let mut s = shared.settings.lock();
    if s.overlay_pos != Some(pos) || s.overlay_size != size {
        s.overlay_pos = Some(pos);
        s.overlay_size = size;
        drop(s);
        layout_changed(ctx, shared);
    }
}

fn layout_changed(ctx: &egui::Context, shared: &Shared) {
    shared.overlay_layout_dirty.store(true, std::sync::atomic::Ordering::Relaxed);
    ctx.request_repaint_of(ViewportId::ROOT);
}

/// Edit-mode header: hint, Done button and the section checkboxes.
fn edit_bar(ui: &mut Ui, shared: &Shared, sample: bool) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("EDIT LAYOUT").font(semibold(10.5)).color(ACCENT_TEXT));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui.button(RichText::new("Done").font(semibold(11.5))).clicked() {
                set_overlay_edit(ui.ctx(), shared, false);
            }
        });
    });
    ui.label(RichText::new("Drag to move, corner to resize. Ctrl+Shift+E or Done to finish.").size(10.5).color(MUTED));
    if sample {
        ui.label(RichText::new("No game running: sample data.").size(10.5).color(SOON));
    }
    let before = shared.settings.lock().overlay_sections;
    let mut sections = before;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(8.0, 2.0);
        for (label, on) in sections.toggles() {
            ui.checkbox(on, RichText::new(label).size(11.0).color(BODY));
        }
    });
    if sections != before {
        shared.settings.lock().overlay_sections = sections;
        layout_changed(ui.ctx(), shared);
        ui.ctx().request_repaint();
    }
    rule(ui);
}

/// A caption + value pair on one compact line.
fn stat(ui: &mut Ui, caption: &str, value: RichText) {
    ui.label(RichText::new(caption.to_uppercase()).font(semibold(9.5)).color(MUTED));
    ui.label(value);
}

fn header(ui: &mut Ui, g: &Gfx, right: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        let t = g.icons.logo;
        let (r, _) = ui.allocate_exact_size(vec2(18.0, 18.0), egui::Sense::hover());
        ui.painter().image(t.id, r, egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)), Color32::WHITE);
        ui.label(RichText::new("riftlite").font(semibold(12.5)).color(ACCENT_TEXT));
        ui.with_layout(Layout::right_to_left(Align::Center), right);
    });
}

fn rule(ui: &mut Ui) {
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), 1.0), egui::Sense::hover());
    ui.painter().rect_filled(r, 0, alpha(BORDER_STRONG, 0.9));
}

/// A rule between groups, once anything is drawn above.
fn separate(ui: &mut Ui, above: &mut bool) {
    if std::mem::replace(above, true) {
        rule(ui);
    }
}

/// `sample`: `game` is `sample_game()`; sections fed by the snapshot get sample rows too.
fn overlay_contents(ui: &mut Ui, g: &Gfx, s: &Snapshot, game: Option<&LiveGame>, show: OverlaySections, sample: bool) {
    ui.spacing_mut().item_spacing = vec2(6.0, 4.0);
    let Some(game) = game else {
        if show.clock {
            header(ui, g, |ui| {
                ui.label(RichText::new("LOADING").font(semibold(10.5)).color(SOON));
            });
        }
        if show.scouts {
            overlay_scouts(ui, g, s, None);
        }
        return;
    };
    if show.clock {
        header(ui, g, |ui| {
            ui.label(RichText::new(mmss(game.game_time)).font(display(20.0)).color(TEXT));
        });
    }
    let Some(me) = game.me() else { return };
    let mut above = show.clock;
    let hint = if sample { Some('Q') } else { s.skill_hint.as_ref().and_then(|h| h.next) }.filter(|_| show.skill_hint);
    if show.kda || show.kills_gold || show.item_diff || hint.is_some() {
        separate(ui, &mut above);
    }
    if show.kda {
        let mins = (game.game_time / 60.0).max(1.0);
        let cs_min = me.cs as f64 / mins;
        let bench = cs_baseline(s, &me.champion_key).map(|(a, _)| a);
        ui.horizontal(|ui| {
            g.champ_key(ui, &me.champion_key, &me.champion, 30.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                kda(ui, me.kills, me.deaths, me.assists, 16.0);
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    let c = match bench {
                        Some(a) if cs_min >= a => WIN,
                        Some(_) => LOSS,
                        None => BODY,
                    };
                    ui.label(RichText::new(format!("{} CS", me.cs)).font(semibold(11.5)).color(TEXT));
                    ui.label(RichText::new(format!("{cs_min:.1}/m")).font(semibold(11.5)).color(c));
                    if let Some(a) = bench {
                        ui.label(RichText::new(format!("avg {a:.1}")).size(11.0).color(MUTED));
                    }
                });
            });
        });
    }
    let enemy = if me.team == "ORDER" { "CHAOS" } else { "ORDER" };
    let (blue, red) = if me.team == "ORDER" { (BLUE_TEAM, RED_TEAM) } else { (RED_TEAM, BLUE_TEAM) };
    if show.kills_gold {
        ui.horizontal(|ui| {
            stat(ui, "Kills", RichText::new(game.team_kills(&me.team).to_string()).font(semibold(13.0)).color(blue));
            ui.label(RichText::new("–").color(FAINT));
            ui.label(RichText::new(game.team_kills(enemy).to_string()).font(semibold(13.0)).color(red));
            ui.add_space(6.0);
            stat(ui, "Gold", RichText::new(format!("{:.0}", game.my_gold)).font(semibold(13.0)).color(SOON));
        });
    }
    if show.item_diff {
        let (mine, theirs) = (game.team_gold(&me.team), game.team_gold(enemy));
        ui.horizontal(|ui| {
            stat(ui, "Items", RichText::new(format!("{} vs {}", kilo(mine), kilo(theirs))).size(12.0).color(BODY));
            ui.label(RichText::new(signed(mine - theirs)).font(semibold(12.0)).color(diff_color(mine - theirs)));
            if let Some(opp) = game.lane_opponent() {
                let d = me.item_gold() - opp.item_gold();
                ui.add_space(4.0);
                stat(ui, "Lane", RichText::new(signed(d)).font(semibold(12.0)).color(diff_color(d)));
            }
        });
    }
    if let Some(next) = hint {
        ui.horizontal(|ui| {
            icon(ui, Icon::Bolt, 13.0, SOON);
            ui.label(RichText::new(format!("Level up {next} (your last {} order)", me.champion)).font(semibold(12.0)).color(SOON));
        });
    }
    let objectives: Vec<_> = game
        .objectives
        .iter()
        // A respawned objective stays listed for one minute, then drops off until the next kill.
        .filter(|o| show.objectives && !o.ready_at.is_some_and(|t| game.game_time - t > 60.0))
        .collect();
    if !objectives.is_empty() {
        separate(ui, &mut above);
        for o in objectives {
            let (text, color) = objective_text(o, game.game_time);
            ui.horizontal(|ui| {
                icon(ui, Icon::Clock, 12.0, color);
                ui.label(RichText::new(text).size(12.0).color(color));
            });
        }
    }
    let dead: Vec<_> = game.players.iter().filter(|p| show.dead_enemies && p.team == enemy && p.dead).collect();
    if !dead.is_empty() {
        separate(ui, &mut above);
        ui.horizontal_wrapped(|ui| {
            icon(ui, Icon::Skull, 14.0, LOSS);
            ui.label(RichText::new("DEAD").font(semibold(9.5)).color(LOSS));
            for p in dead {
                g.champ_key(ui, &p.champion_key, &p.champion, 20.0);
                ui.label(RichText::new(format!("{:.0}s", p.respawn)).font(semibold(11.5)).color(LOSS));
            }
        });
    }
    if show.scouts {
        if sample {
            sample_scouts(ui, g, game, enemy);
        } else if game.game_time < SCOUT_SECS {
            overlay_scouts(ui, g, s, Some(if me.team == "ORDER" { 100 } else { 200 }));
        }
    }
}

/// Compact enemy ranks.
fn overlay_scouts(ui: &mut Ui, g: &Gfx, s: &Snapshot, my_team: Option<i64>) {
    let rows = scouts(s, my_team);
    if rows.is_empty() {
        return;
    }
    ui.label(RichText::new("ENEMY RANKS").font(semibold(9.5)).color(MUTED));
    for (champ, r) in rows {
        let rank = r.as_ref().map(|r| (r.text.as_str(), r.tier.as_str()));
        scout_row(ui, |ui| g.champ(ui, champ, 18.0), &s.data.champion_name(champ), rank);
    }
}

/// One enemy rank row; `rank`: display text and tier, `None` while it loads.
fn scout_row(ui: &mut Ui, champ: impl FnOnce(&mut Ui) -> egui::Response, name: &str, rank: Option<(&str, &str)>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 5.0;
        champ(ui);
        ui.label(RichText::new(name).size(11.5).color(BODY));
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| match rank {
            Some((text, tier)) => {
                ui.label(RichText::new(text).size(11.5).color(if tier.is_empty() { MUTED } else { tier_color(tier) }));
            }
            None => {
                ui.label(RichText::new("…").color(MUTED));
            }
        });
    });
}

/// The enemy ranks section of the sample game.
fn sample_scouts(ui: &mut Ui, g: &Gfx, game: &LiveGame, enemy: &str) {
    ui.label(RichText::new("ENEMY RANKS").font(semibold(9.5)).color(MUTED));
    for (p, rank) in game.players.iter().filter(|p| p.team == enemy).zip([("Gold II", "GOLD"), ("Emerald IV", "EMERALD"), ("Platinum I", "PLATINUM")]) {
        scout_row(ui, |ui| g.champ_key(ui, &p.champion_key, &p.champion, 18.0), &p.champion, Some(rank));
    }
}

/// A mid-game Summoner's Rift game for edit mode outside a game, so every section shows.
fn sample_game() -> LiveGame {
    let player = |champ: &str, team: &str, position: &str, kda: [i64; 3], cs: i64, items: i64, respawn: f64, is_me: bool| LivePlayer {
        name: champ.to_string(),
        champion: champ.to_string(),
        champion_key: champ.to_string(),
        team: team.to_string(),
        position: position.to_string(),
        level: 11,
        kills: kda[0],
        deaths: kda[1],
        assists: kda[2],
        cs,
        ward_score: 0.0,
        items: vec![LiveItem { id: 0, name: String::new(), price: items, count: 1, consumable: false }],
        dead: respawn > 0.0,
        respawn,
        is_me,
    };
    let game_time = 14.0 * 60.0 + 32.0;
    LiveGame {
        game_time,
        mode: "CLASSIC".to_string(),
        map: crate::live::SUMMONERS_RIFT,
        my_gold: 1240.0,
        my_level: 11,
        abilities: [4, 2, 4, 1],
        players: vec![
            player("Ahri", "ORDER", "MIDDLE", [4, 1, 3], 128, 6100, 0.0, true),
            player("Garen", "ORDER", "TOP", [2, 2, 1], 110, 5200, 0.0, false),
            player("Jinx", "ORDER", "BOTTOM", [3, 1, 4], 131, 5900, 0.0, false),
            player("Zed", "CHAOS", "MIDDLE", [2, 3, 1], 119, 5400, 18.0, false),
            player("Darius", "CHAOS", "TOP", [3, 2, 0], 105, 5600, 0.0, false),
            player("Caitlyn", "CHAOS", "BOTTOM", [1, 3, 2], 124, 5000, 24.0, false),
        ],
        objectives: vec![
            Objective { label: "Dragon".to_string(), ready_at: Some(game_time + 95.0), since: game_time - 205.0, taken_by: Some("ORDER".to_string()) },
            Objective { label: "Baron".to_string(), ready_at: Some(game_time + 328.0), since: 0.0, taken_by: None },
        ],
        dragons: (1, 0),
    }
}
