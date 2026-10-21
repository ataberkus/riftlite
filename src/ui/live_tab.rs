//! Live game tab (Live Client Data API) and enemy scouting shared with the overlay.

use eframe::egui::{Align, Align2, Color32, DragValue, Grid, Layout, RichText, Sense, Stroke, StrokeKind, Ui, vec2};

use super::matches::trunc;
use super::home::void;
use super::stats::{Scout, cs_baseline, kilo, live_puuid, mastery_on, mmss, position_label, scout, signed};
use super::theme::*;
use super::widgets::*;
use super::{App, loading_screen, overlay};
use crate::counters::CounterItem;
use crate::ddragon::StaticData;
use crate::goals::{self, GoalKind, Goals, Pace, Stats};
use crate::live::{FeedEvent, LiveGame, LivePlayer, Objective};
use crate::meta::MetaBuild;
use crate::styles::{KEYS, Rarity, SkillOrder, Style, StyleBuilds};
use crate::notes;
use crate::worker::{Action, Clip, Snapshot};

/// League in exclusive fullscreen: no other window, the overlay included, draws over it.
pub(super) const FULLSCREEN_WARNING: &str =
    "League is set to Fullscreen (WindowMode=0 in Config/game.cfg), which hides the overlay. Use Borderless or Windowed in the game's Video settings.";

/// Enemy (or, before teams are known, all) players of the gameflow session with their ranks.
pub(super) fn scouts(s: &Snapshot, my_team: Option<i64>) -> Vec<(i64, Option<Scout>)> {
    let Some(session) = &s.session else { return Vec::new() };
    let mine = my_team.or_else(|| {
        let me = s.summoner.as_ref()?.puuid.as_str();
        session.players.iter().find(|p| p.puuid == me).map(|p| p.team)
    });
    session
        .players
        .iter()
        .filter(|p| mine.is_none_or(|t| p.team != t))
        .map(|p| (p.champion_id, scout(s, &p.puuid)))
        .collect()
}

/// Objective timer text and color: soon (≤ 30 s) gold, up green, untimed muted.
pub(super) fn objective_text(o: &Objective, now: f64) -> (String, Color32) {
    let by = match o.taken_by.as_deref() {
        Some("ORDER") => " (blue took it)",
        Some("CHAOS") => " (red took it)",
        _ => "",
    };
    match o.ready_at {
        Some(t) if t > now => {
            let left = t - now;
            (format!("{} in {}{by}", o.label, mmss(left)), if left <= 30.0 { SOON } else { TEXT })
        }
        Some(_) => match o.until {
            Some(u) if !o.kill_tracked => (format!("{} spawned · gone by {}", o.label, mmss(u)), BODY),
            Some(u) => (format!("{} up until {}", o.label, mmss(u)), WIN),
            None => (format!("{} up", o.label), WIN),
        },
        None => (format!("{} down for {}", o.label, mmss(now - o.since)), MUTED),
    }
}

/// Item-value difference colored by sign.
pub(super) fn diff_color(v: i64) -> Color32 {
    match v {
        v if v > 250 => WIN,
        v if v < -250 => LOSS,
        _ => BODY,
    }
}

/// Color of a feed line: the side it favors.
pub(super) fn feed_color(e: &FeedEvent) -> Color32 {
    match e.team.as_deref() {
        Some("ORDER") => BLUE_TEAM,
        Some("CHAOS") => RED_TEAM,
        _ => MUTED,
    }
}

fn team_color(team: &str) -> Color32 {
    if team == "CHAOS" { RED_TEAM } else { BLUE_TEAM }
}

/// A callout stands out for its first minute.
pub(super) fn item_color(age: f64) -> Color32 {
    if age < 60.0 { SOON } else { BODY }
}

/// The OP.GG build picked in champ select and the index of its picked core, while it is
/// for `champion` (any champion when `None`, e.g. on the loading screen).
pub(super) fn picked_build(s: &Snapshot, champion: Option<i64>) -> Option<(&MetaBuild, usize)> {
    let m = s.meta.as_ref().filter(|m| champion.is_none_or(|c| c == m.champion_id))?;
    let Some(Ok(b)) = &m.build else { return None };
    Some((b, m.picks.build.min(b.cores.len() - 1)))
}

/// Blitz's playstyle builds for `champion`, when it has any.
pub(super) fn style_builds(s: &Snapshot, champion: Option<i64>) -> Option<&StyleBuilds> {
    let Some(Ok(b)) = &s.styles else { return None };
    Some(b).filter(|b| Some(b.champion_id) == champion && !b.styles.is_empty())
}

/// The Blitz style shown for `b`: the one picked in the Live tab (`pick`: champion id and
/// label), else the one you are building.
pub(super) fn active_style<'a>(b: &'a StyleBuilds, pick: Option<&(i64, String)>, owned: &[i64]) -> Option<(usize, &'a Style)> {
    let label = pick.filter(|(c, _)| *c == b.champion_id).map(|(_, l)| l.as_str());
    b.active(label, owned).map(|i| (i, &b.styles[i]))
}

/// The ability (`Q`…`R`) to level next by the active Blitz style's skill order, with the style's label.
pub(super) fn blitz_skill_hint(s: &Snapshot, game: &LiveGame, pick: Option<&(i64, String)>) -> Option<(char, String)> {
    let me = game.me()?;
    let b = style_builds(s, s.data.champion_by_key(&me.champion_key))?;
    let owned: Vec<i64> = me.items.iter().map(|it| it.id).collect();
    let (_, st) = active_style(b, pick, &owned)?;
    let k = st.skills.as_ref()?.next(game.abilities, game.my_level)?;
    Some((KEYS[k], st.label()))
}

/// Blitz's 18-level skill grid: one row per ability, the level number in the cell of the
/// ability taken there. Levels below `taken` are dimmed in; `next` (an index into the order) is highlighted.
fn skill_grid(ui: &mut Ui, sk: &SkillOrder, taken: usize, next: Option<usize>) {
    const CELL: f32 = 22.0;
    const GAP: f32 = 3.0;
    const KEY: f32 = 20.0;
    let step = CELL + GAP;
    let size = vec2(KEY + sk.order.len() as f32 * step, 4.0 * step - GAP);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter();
    for (row, key) in KEYS.iter().enumerate() {
        let y = rect.top() + row as f32 * step;
        let key_color = if row == 3 { SOON } else { TEXT };
        p.text(eframe::egui::pos2(rect.left() + KEY / 2.0 - 2.0, y + CELL / 2.0), Align2::CENTER_CENTER, key, semibold(12.0), key_color);
        for (i, &k) in sk.order.iter().enumerate() {
            let cell = eframe::egui::Rect::from_min_size(eframe::egui::pos2(rect.left() + KEY + i as f32 * step, y), vec2(CELL, CELL));
            if k != row {
                p.rect_filled(cell, 4, alpha(BORDER, 0.6));
                continue;
            }
            let (fill, text) = if Some(i) == next {
                (SOON, Color32::BLACK)
            } else if i < taken {
                (alpha(ACCENT, 0.55), TEXT)
            } else {
                (alpha(ACCENT, 0.18), BODY)
            };
            p.rect_filled(cell, 4, fill);
            p.text(cell.center(), Align2::CENTER_CENTER, (i + 1).to_string(), semibold(11.0), text);
        }
    }
}

/// Item tiles in build order; the ones in `owned` are outlined green.
pub(super) fn build_items(ui: &mut Ui, g: &Gfx, ids: &[i64], owned: &[i64], size: f32) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        for &id in ids {
            let r = g.item(ui, id, size);
            if owned.contains(&id) {
                ui.painter().rect_stroke(r.rect, 5, Stroke::new(1.5, WIN), StrokeKind::Inside);
            }
        }
    });
}

/// Counter-item options; the ones you hold (any variant) are outlined green.
pub(super) fn counter_items(ui: &mut Ui, g: &Gfx, items: &[CounterItem], size: f32) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        for it in items {
            let r = g.item(ui, it.id, size);
            if it.owned {
                ui.painter().rect_stroke(r.rect, 5, Stroke::new(1.5, WIN), StrokeKind::Inside);
            }
        }
    });
}

/// Your goals for `me`'s role (canonical position), when any is set.
pub(super) fn role_goals<'a>(s: &'a Snapshot, me: &LivePlayer) -> Option<&'a Goals> {
    match crate::lcu::canonical_position(&me.position) {
        "" => None,
        role => s.goals.goals(role),
    }
}

/// Each set goal against the game so far. The Live Client API reports no purchases, so
/// control wards are left to the post-game check.
pub(super) fn goal_pace(set: &Goals, game: &LiveGame, me: &LivePlayer) -> Vec<(GoalKind, f64, f64, Pace)> {
    let stats = Stats { cs: me.cs, deaths: me.deaths, vision: me.ward_score, control_wards: None, minutes: game.game_time / 60.0 };
    goals::live_pace(set, &stats)
}

/// Actual vs target: "6.8 / 7.0" for CS, "0.74 / 0.80" for vision (a 1-decimal rounding
/// could show a missed target as met), "2 / 4 max" for deaths.
pub(super) fn goal_value(kind: GoalKind, target: f64, actual: f64) -> String {
    match kind {
        GoalKind::CsPerMin => format!("{actual:.1} / {target:.1}"),
        GoalKind::VisionPerMin => format!("{actual:.2} / {target:.2}"),
        GoalKind::MaxDeaths => format!("{actual:.0} / {target:.0} max"),
        GoalKind::ControlWards => format!("{actual:.0} / {target:.0}"),
    }
}

pub(super) fn pace_color(p: Pace) -> Color32 {
    match p {
        Pace::Ahead => WIN,
        Pace::OnTrack => mix(WIN, BODY, 0.35),
        Pace::Behind => SOON,
        Pace::Failed => LOSS,
    }
}

fn pace_label(kind: GoalKind, p: Pace) -> &'static str {
    match (kind, p) {
        (GoalKind::MaxDeaths, Pace::OnTrack) => "under the limit",
        (GoalKind::MaxDeaths, Pace::Behind) => "at the limit",
        (GoalKind::MaxDeaths, Pace::Failed) => "over the limit",
        (_, Pace::Ahead) => "ahead",
        (_, Pace::OnTrack) => "on track",
        (_, Pace::Behind) => "behind",
        (_, Pace::Failed) => "missed",
    }
}

/// Champion ids for notes: yours, the enemies' and your lane opponent's; `None` until your
/// champion is known.
pub(super) fn note_champions(data: &StaticData, game: &LiveGame) -> Option<(i64, Vec<i64>, Option<i64>)> {
    let me = game.me()?;
    let mine = data.champion_by_key(&me.champion_key)?;
    let enemies = game.players.iter().filter(|p| p.team != me.team).filter_map(|p| data.champion_by_key(&p.champion_key)).collect();
    let lane = game.lane_opponent().and_then(|p| data.champion_by_key(&p.champion_key));
    Some((mine, enemies, lane))
}

impl App {
    pub(super) fn live_tab(&mut self, ui: &mut Ui, s: &Snapshot) {
        let icons = self.icons.clone();
        let g = Gfx::new(&icons, &s.data);
        // Overlay edit mode outside a game previews the same sample game here.
        let sample = (s.live.is_none() && self.shared.overlay_edit.load(std::sync::atomic::Ordering::Relaxed)).then(|| overlay::sample_game(&s.data));
        let Some(game) = s.live.as_ref().or(sample.as_ref()) else {
            let msg = if s.running.any_game() {
                "Game process found; waiting for the Live Client Data API (port 2999)…"
            } else {
                "No game running."
            };
            empty_state(ui, Icon::Live, "Live game", msg, |_| {});
            if loading_screen(s) && !scouts(s, None).is_empty() {
                section(ui, "Players");
                card(ui, |ui| loading_players(ui, &g, s));
            }
            self.open_replay(ui, s);
            return;
        };
        if sample.is_some() {
            status_msg(ui, true, "Sample game: the overlay is in edit mode and no game is running.");
        }
        live_header(ui, &g, s, game);
        if self.settings.overlay_enabled && s.window_mode == Some(0) {
            ui.add_space(6.0);
            error(ui, FULLSCREEN_WARNING);
        }
        self.open_replay(ui, s);
        let me = game.me();
        if let Some(me) = me {
            live_goals(ui, s, game, me);
        }
        let champ = me.and_then(|p| s.data.champion_by_key(&p.champion_key));
        let owned: Vec<i64> = me.map(|p| p.items.iter().map(|it| it.id).collect()).unwrap_or_default();
        if let Some(b) = style_builds(s, champ) {
            self.blitz_builds(ui, &g, s, game, b, &owned);
        } else if let Some((b, sel)) = picked_build(s, champ) {
            let core = &b.cores[sel];
            section(ui, "Your build");
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("OP.GG {} · {}", b.query.label(), core.label())).font(semibold(13.0)).color(TEXT));
                    ui.label(RichText::new(core.items.stats()).size(11.5).color(MUTED));
                });
                ui.add_space(4.0);
                build_items(ui, &g, &b.full_build(sel, &s.data), &owned, 28.0);
                let situational = b.situational(sel, &s.data);
                if !situational.is_empty() {
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("SITUATIONAL").font(semibold(10.5)).color(MUTED));
                        build_items(ui, &g, &situational, &owned, 22.0);
                    });
                }
                if let Some(Err(e)) = &s.styles {
                    ui.add_space(2.0);
                    note(ui, &format!("Playstyle builds unavailable: {e}"));
                }
            });
        } else if let Some(Err(e)) = &s.styles {
            section(ui, "Builds by playstyle");
            card(ui, |ui| {
                error(ui, e);
            });
        }
        if !game.counters.is_empty() {
            section(ui, "Counter items");
            card(ui, |ui| {
                for c in &game.counters {
                    ui.horizontal(|ui| {
                        counter_items(ui, &g, &c.items, 26.0);
                        ui.add_space(4.0);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            ui.label(RichText::new(c.threat.label()).font(semibold(12.5)).color(TEXT));
                            ui.label(RichText::new(&c.reason).size(11.5).color(BODY));
                        });
                    });
                }
                ui.add_space(2.0);
                note(ui, "From the enemies' current items (Data Dragon stats); options, not a build order.");
            });
        }
        self.live_notes(ui, s, game);
        if !game.objectives.is_empty() {
            section(ui, "Objectives");
            card(ui, |ui| {
                tiles(ui, |ui| {
                    ui.spacing_mut().item_spacing = vec2(8.0, 6.0);
                    for o in &game.objectives {
                        let (text, color) = objective_text(o, game.game_time);
                        inset_frame().show(ui, |ui| {
                            ui.horizontal(|ui| {
                                icon(ui, Icon::Clock, 14.0, color);
                                ui.label(RichText::new(text).font(semibold(12.5)).color(color));
                            });
                        });
                    }
                });
            });
        }
        if !game.feed.is_empty() {
            section(ui, "Recent events");
            card(ui, |ui| {
                for e in game.feed.iter().take(10) {
                    ui.horizontal(|ui| {
                        ui.add_sized([44.0, 18.0], eframe::egui::Label::new(RichText::new(mmss(e.time)).size(11.5).color(MUTED)));
                        let (r, _) = ui.allocate_exact_size(vec2(3.0, 14.0), Sense::hover());
                        ui.painter().rect_filled(r, 1, feed_color(e));
                        ui.label(RichText::new(&e.text).color(BODY));
                    });
                }
            });
        }
        if !game.item_callouts.is_empty() {
            section(ui, "Enemy items finished");
            card(ui, |ui| {
                for c in game.item_callouts.iter().take(10) {
                    ui.horizontal(|ui| {
                        ui.add_sized([44.0, 18.0], eframe::egui::Label::new(RichText::new(mmss(c.time)).size(11.5).color(MUTED)));
                        g.champ_key(ui, &c.champion_key, &c.champion, 20.0);
                        ui.label(RichText::new(&c.champion).font(semibold(12.5)).color(TEXT));
                        g.item(ui, c.item, 20.0);
                        ui.label(RichText::new(s.data.item_name(c.item)).color(item_color(game.game_time - c.time)));
                    });
                }
            });
        }
        let mut teams: Vec<&str> = game.players.iter().map(|p| p.team.as_str()).collect();
        teams.sort();
        teams.dedup();
        for team in teams {
            let label = match team {
                "ORDER" => "Blue side",
                "CHAOS" => "Red side",
                t => t,
            };
            let color = team_color(team);
            ui.add_space(10.0);
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(3.0, 16.0), Sense::hover());
                    ui.painter().rect_filled(r, 1, color);
                    ui.label(RichText::new(label).font(semibold(14.0)).color(color));
                    ui.label(RichText::new(format!("{} kills · {} item value", game.team_kills(team), kilo(game.team_gold(team)))).color(MUTED));
                });
                ui.add_space(4.0);
                Grid::new(team).striped(true).spacing([12.0, 4.0]).min_row_height(34.0).show(ui, |ui| {
                    for h in ["", "Player", "Spells", "Rank", "Lvl", "K / D / A", "CS", "Wards", "Items"] {
                        ui.label(RichText::new(h.to_uppercase()).font(semibold(10.5)).color(MUTED));
                    }
                    ui.end_row();
                    for p in game.players.iter().filter(|p| p.team == team) {
                        let (r, resp) = ui.allocate_exact_size(vec2(32.0, 32.0), Sense::hover());
                        let id = s.data.champion_by_key(&p.champion_key).unwrap_or(0);
                        g.paint_champ(ui.painter(), r, id, if p.dead { Color32::from_gray(90) } else { Color32::WHITE });
                        if p.dead {
                            ui.painter().text(r.center(), Align2::CENTER_CENTER, format!("{:.0}", p.respawn), display(16.0), LOSS);
                        }
                        if p.is_me {
                            ui.painter().rect_stroke(r.expand(1.5), 8, Stroke::new(1.5, CYAN), StrokeKind::Outside);
                        }
                        resp.on_hover_text(&p.champion);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 0.0;
                            let name = RichText::new(&p.name).font(semibold(12.5));
                            trunc(ui, if p.is_me { name.color(CYAN) } else { name.color(TEXT) }, 150.0);
                            if p.dead {
                                ui.label(RichText::new(format!("{} · dead {:.0}s", p.champion, p.respawn)).size(11.0).color(LOSS));
                            } else {
                                ui.label(RichText::new(&p.champion).size(11.0).color(MUTED));
                            }
                        });
                        loadout(ui, &g, p, 20.0);
                        ui.horizontal(|ui| {
                            let puuid = live_puuid(s, p);
                            if let Some(r) = puuid.and_then(|id| scout(s, id)) {
                                rank_emblem(ui, &r.tier, 20.0);
                                ui.label(RichText::new(r.text).size(11.5).color(if r.tier.is_empty() { MUTED } else { tier_color(&r.tier) }));
                            }
                            if let (Some(id), Some(champ)) = (puuid, s.data.champion_by_key(&p.champion_key)) {
                                mastery_tag(ui, s, id, champ);
                            }
                        });
                        ui.label(RichText::new(p.level.to_string()).font(semibold(13.0)).color(BODY));
                        kda(ui, p.kills, p.deaths, p.assists, 13.5);
                        ui.label(RichText::new(p.cs.to_string()).color(BODY));
                        ui.label(RichText::new(format!("{:.0}", p.ward_score)).color(BODY));
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 3.0;
                            for it in &p.items {
                                g.item(ui, it.id, 22.0).on_hover_text(&it.name);
                            }
                            ui.add_space(4.0);
                            ui.label(RichText::new(kilo(p.item_gold())).size(11.5).color(SOON));
                        });
                        ui.end_row();
                    }
                });
            });
        }
    }

    /// A replay open in the game (Replay API on): its clock and a clip around the current moment.
    fn open_replay(&mut self, ui: &mut Ui, s: &Snapshot) {
        let Some(p) = s.replay_playback else { return };
        let recording = matches!(s.clip, Some(Clip::Recording { .. }));
        section(ui, "Replay");
        card(ui, |ui| {
            ui.horizontal(|ui| {
                icon(ui, Icon::Clock, 14.0, MUTED);
                ui.label(RichText::new(format!("Replay {} / {}", mmss(p.time), mmss(p.length))).font(semibold(13.5)).color(TEXT));
                if p.paused {
                    pill(ui, "Paused", MUTED);
                } else if (p.speed - 1.0).abs() > 0.01 {
                    pill(ui, format!("{:.2}×", p.speed), MUTED);
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let (start, end) = ((p.time - self.clip_span.0).max(0.0), p.time + self.clip_span.1);
                    let clip = ui
                        .add_enabled(!recording, Btn::primary("Clip around now").small())
                        .on_hover_text(format!("Records {}–{}", mmss(start), mmss(end)));
                    if clip.clicked() {
                        self.shared.request(Action::ReplayClip { start, end });
                    }
                });
            });
            ui.horizontal(|ui| {
                ui.spacing_mut().interact_size = vec2(64.0, 22.0);
                ui.label(RichText::new("Clip span").size(11.5).color(MUTED));
                ui.add(DragValue::new(&mut self.clip_span.0).range(0.0..=120.0).speed(0.5).suffix(" s before"));
                ui.add(DragValue::new(&mut self.clip_span.1).range(0.0..=120.0).speed(0.5).suffix(" s after"));
            });
            match &s.clip {
                Some(Clip::Recording { start, end, current }) => {
                    ui.horizontal(|ui| {
                        progress(ui, ((current - start) / (end - start).max(1.0)) as f32, 120.0, ACCENT);
                        ui.label(RichText::new(format!("Recording {}–{} (at {}); keep the replay playing", mmss(*start), mmss(*end), mmss(*current))).size(11.5).color(BODY));
                    });
                }
                Some(Clip::Saved(path)) => {
                    status_msg(ui, true, format!("Clip saved: {path}"));
                }
                Some(Clip::Failed(e)) => {
                    error(ui, format!("Clip failed: {e}"));
                }
                None => {
                    small_note(ui, format!("Clips go to {}.", crate::replay::clips_dir().display()));
                }
            }
        });
    }

    /// Your notes on your champion and this game's matchups. Your champion's and the lane
    /// matchup's notes can be added here; other matchup notes show when they exist.
    fn live_notes(&mut self, ui: &mut Ui, s: &Snapshot, game: &LiveGame) {
        let Some((mine, enemies, lane)) = note_champions(&s.data, game) else { return };
        let name = s.data.champion_name(mine);
        let mut rows: Vec<(String, String, Option<String>)> = vec![(notes::champ_key(mine), name.clone(), None)];
        if let Some(o) = lane {
            rows.push((notes::vs_key(mine, o), format!("{name} vs {}", s.data.champion_name(o)), None));
        }
        for (key, title, text) in s.notes.for_game(mine, &enemies, lane, &s.data) {
            if let Some(r) = rows.iter_mut().find(|r| r.0 == key) {
                r.2 = Some(text);
            } else {
                rows.push((key, title, Some(text)));
            }
        }
        section(ui, "Notes");
        card(ui, |ui| {
            for (i, (key, title, text)) in rows.iter().enumerate() {
                if i > 0 {
                    ui.add_space(6.0);
                }
                if let Some((key, text)) = note_box(ui, &mut self.note_edit, key, title, text.as_deref()) {
                    self.shared.request(Action::SaveNote { key, text });
                }
            }
            ui.add_space(2.0);
            small_note(ui, "Kept on this PC; the overlay shows them during the first minute and a half.");
        });
    }
}

/// Your goals for your role and how this game is going; nothing without goals for the role.
fn live_goals(ui: &mut Ui, s: &Snapshot, game: &LiveGame, me: &LivePlayer) {
    let Some(set) = role_goals(s, me) else { return };
    let role = crate::lcu::canonical_position(&me.position);
    section(ui, "Goals");
    card(ui, |ui| {
        Grid::new("live-goals").spacing([16.0, 4.0]).show(ui, |ui| {
            for (kind, target, actual, pace) in goal_pace(set, game, me) {
                ui.label(RichText::new(kind.label()).color(BODY));
                ui.label(RichText::new(goal_value(kind, target, actual)).font(semibold(13.0)).color(TEXT));
                ui.label(RichText::new(pace_label(kind, pace)).font(semibold(12.0)).color(pace_color(pace)));
                ui.end_row();
            }
            if let Some(n) = set.control_wards {
                ui.label(RichText::new(GoalKind::ControlWards.label()).color(BODY));
                ui.label(RichText::new(n.to_string()).font(semibold(13.0)).color(TEXT));
                ui.label(RichText::new("checked after the game").size(11.5).color(MUTED));
                ui.end_row();
            }
        });
        ui.add_space(2.0);
        let streak = s.goals.streak(Some(role));
        let streak = if streak > 0 { format!(" · {streak} game(s) in a row with every goal met") } else { String::new() };
        small_note(ui, format!("{} goals (Settings), so far vs target{streak}.", position_label(role)));
    });
}

/// A live player's summoner spells and keystone, as far as the game names them.
pub(super) fn loadout(ui: &mut Ui, g: &Gfx, p: &LivePlayer, size: f32) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for key in &p.spells {
            if let Some(id) = g.data.spell_by_key(key) {
                g.spell(ui, id, size);
            }
        }
        if p.keystone > 0 {
            g.perk(ui, p.keystone, size + 2.0);
        }
    });
}

/// Your champion, level, timer and the headline numbers.
fn live_header(ui: &mut Ui, g: &Gfx, s: &Snapshot, game: &LiveGame) {
    card(ui, |ui| {
        let me = game.me();
        ui.horizontal(|ui| {
            match me {
                Some(me) => {
                    g.champ_key(ui, &me.champion_key, &me.champion, 60.0);
                    ui.add_space(6.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(RichText::new(&me.champion).font(display(28.0)).color(TEXT));
                        ui.horizontal(|ui| {
                            pill(ui, format!("Level {}", game.my_level), CYAN);
                            pill(ui, &game.mode, MUTED);
                        });
                    });
                }
                None => {
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&game.mode).font(display(28.0)).color(TEXT));
                        if game.mode == "TFT" {
                            ui.label(RichText::new(format!("TFT · level {} · {:.0} gold", game.my_level, game.my_gold)).color(BODY));
                        }
                    });
                }
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.vertical(|ui| {
                    ui.with_layout(Layout::top_down(Align::Max), |ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        overline(ui, "Game time");
                        ui.label(RichText::new(mmss(game.game_time)).font(display(34.0)).color(TEXT));
                    });
                });
            });
        });
        let Some(me) = me else { return };
        ui.add_space(8.0);
        let mins = (game.game_time / 60.0).max(1.0);
        let cs_min = me.cs as f64 / mins;
        let bench = cs_baseline(s, &me.champion_key);
        let enemy = if me.team == "ORDER" { "CHAOS" } else { "ORDER" };
        let (mine, theirs) = (game.team_gold(&me.team), game.team_gold(enemy));
        tiles(ui, |ui| {
            let ratio = (me.kills + me.assists) as f64 / me.deaths.max(1) as f64;
            inset_frame().show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    overline(ui, "K / D / A");
                    kda(ui, me.kills, me.deaths, me.assists, 24.0);
                    ui.label(RichText::new(format!("{ratio:.2} KDA")).size(11.5).color(kda_color(ratio)));
                });
            });
            inset_frame().show(ui, |ui| {
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    overline(ui, "CS");
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(me.cs.to_string()).font(display(24.0)).color(TEXT));
                        let c = match &bench {
                            Some((avg, _)) if cs_min >= *avg => WIN,
                            Some(_) => LOSS,
                            None => BODY,
                        };
                        ui.label(RichText::new(format!("{cs_min:.1}/m")).font(semibold(13.0)).color(c));
                    });
                    if let Some((avg, what)) = &bench {
                        ui.label(RichText::new(format!("benchmark {avg:.1} ({what})")).size(11.5).color(MUTED));
                    }
                });
            });
            stat_tile(ui, "Gold", format!("{:.0}", game.my_gold), SOON, None);
            stat_tile(ui, "Team item value", signed(mine - theirs), diff_color(mine - theirs), Some(&format!("{} vs {}", kilo(mine), kilo(theirs))));
            if let Some(opp) = game.lane_opponent() {
                let d = me.item_gold() - opp.item_gold();
                stat_tile(ui, "Lane", signed(d), diff_color(d), Some(&format!("you {} vs {} {}", kilo(me.item_gold()), opp.champion, kilo(opp.item_gold()))));
            }
        });
        if let Some(h) = &s.skill_hint {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                icon(ui, Icon::Bolt, 14.0, SOON);
                ui.label(RichText::new(format!("Skill order this game: {} · your last game on {}: {}", h.current, me.champion, h.recorded)).color(BODY));
            });
        }
    });
}

/// A player's mastery on their champion as a small tag; nothing until it is loaded.
pub(super) fn mastery_tag(ui: &mut Ui, s: &Snapshot, puuid: &str, champion: i64) {
    if let Some((text, level)) = mastery_on(s, puuid, champion) {
        pill(ui, text, if level >= 10 { SOON } else if level == 0 { FAINT } else { MUTED }).on_hover_text("Champion mastery level and points on this champion");
    }
}

/// Loading screen: both teams of the gameflow session with rank and mastery on their champion.
fn loading_players(ui: &mut Ui, g: &Gfx, s: &Snapshot) {
    let Some(session) = &s.session else { return };
    ui.columns(2, |cols| {
        for (ui, (team, label, color)) in cols.iter_mut().zip([(100, "Blue side", BLUE_TEAM), (200, "Red side", RED_TEAM)]) {
            ui.label(RichText::new(label).font(semibold(13.0)).color(color));
            for p in session.players.iter().filter(|p| p.team == team) {
                ui.horizontal(|ui| {
                    g.champ(ui, p.champion_id, 28.0);
                    ui.label(RichText::new(s.data.champion_name(p.champion_id)).font(semibold(13.0)).color(TEXT));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        mastery_tag(ui, s, &p.puuid, p.champion_id);
                        match scout(s, &p.puuid) {
                            Some(r) => {
                                ui.label(RichText::new(&r.text).size(11.5).color(if r.tier.is_empty() { MUTED } else { tier_color(&r.tier) }));
                                rank_emblem(ui, &r.tier, 20.0);
                            }
                            None => void(ui.label(RichText::new("…").color(MUTED))),
                        }
                    });
                });
            }
        }
    });
}

/// A captioned group in the playstyle detail.
fn labeled(ui: &mut Ui, caption: &str, add: impl FnOnce(&mut Ui)) {
    ui.vertical(|ui| {
        overline(ui, caption);
        ui.add_space(2.0);
        add(ui);
    });
}

/// Item tiles with `›` between them, wrapping.
fn item_chain(ui: &mut Ui, g: &Gfx, ids: &[i64], owned: &[i64], size: f32) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for (i, &id) in ids.iter().enumerate() {
            if i > 0 {
                ui.label(RichText::new("›").color(FAINT));
            }
            build_items(ui, g, &[id], owned, size);
        }
    });
}

impl App {
    /// Blitz's builds for your champion, laid out like blitz.gg: the playstyles to pick from,
    /// then the picked one's starting items, spells, build order, completed and situational
    /// items and skill order; ARAM Mayhem adds the champion's augments.
    fn blitz_builds(&self, ui: &mut Ui, g: &Gfx, s: &Snapshot, game: &LiveGame, b: &StyleBuilds, owned: &[i64]) {
        let pick = self.shared.style_pick.lock().clone();
        let Some((active, st)) = active_style(b, pick.as_ref(), owned) else { return };
        let patch = if b.patch.is_empty() { String::new() } else { format!(" · patch {}", b.patch) };
        section_with(ui, "Builds by playstyle", |ui| {
            small_note(ui, format!("Blitz {}{patch}", b.query.label()));
        });
        card(ui, |ui| {
            for (i, other) in b.styles.iter().enumerate() {
                let row = option_row(ui, i == active, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(other.label()).font(semibold(13.0)).color(TEXT));
                        build_items(ui, g, &other.completed[..other.completed.len().min(2)], owned, 20.0);
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            ui.label(RichText::new(other.record.text()).size(11.5).color(MUTED));
                        });
                    });
                });
                if row.on_hover_text("Show this playstyle here and on the overlay").clicked() {
                    *self.shared.style_pick.lock() = Some((b.champion_id, other.label()));
                    ui.ctx().request_repaint_of(super::overlay_id());
                }
            }
            ui.add_space(10.0);
            ui.horizontal_top(|ui| {
                if !st.starting.is_empty() {
                    labeled(ui, "Starting items", |ui| build_items(ui, g, &st.starting, owned, 30.0));
                    ui.add_space(24.0);
                }
                if let Some((spells, record)) = &st.spells {
                    labeled(ui, "Summoner spells", |ui| {
                        ui.horizontal(|ui| {
                            for &id in spells {
                                g.spell(ui, id, 30.0);
                            }
                            ui.label(RichText::new(record.text()).size(11.5).color(MUTED));
                        });
                    });
                }
            });
            ui.add_space(8.0);
            labeled(ui, "Build order", |ui| item_chain(ui, g, &st.build_order, owned, 30.0));
            ui.add_space(8.0);
            labeled(ui, "Completed items", |ui| build_items(ui, g, &st.completed, owned, 34.0));
            let mayhem_items: Vec<i64> = b.mayhem.as_ref().map(|m| m.items.iter().map(|&(id, _)| id).collect()).unwrap_or_default();
            if !st.situational.is_empty() || !mayhem_items.is_empty() {
                ui.add_space(8.0);
                labeled(ui, "Situational items", |ui| {
                    ui.horizontal_wrapped(|ui| {
                        build_items(ui, g, &st.situational, owned, 28.0);
                        if !mayhem_items.is_empty() {
                            ui.add_space(6.0);
                            ui.label(RichText::new("MAYHEM").font(semibold(10.5)).color(SOON)).on_hover_text("ARAM Mayhem items Blitz rates on this champion");
                            build_items(ui, g, &mayhem_items, owned, 28.0);
                        }
                    });
                });
            }
            if let Some(sk) = &st.skills {
                ui.add_space(10.0);
                labeled(ui, "Ability max order", |ui| {
                    ui.horizontal(|ui| {
                        let max = sk.max_order().iter().map(char::to_string).collect::<Vec<_>>().join(" › ");
                        ui.label(RichText::new(max).font(display(18.0)).color(TEXT));
                        ui.add_space(8.0);
                        ui.label(RichText::new(sk.record.text()).size(11.5).color(MUTED));
                        if let Some(k) = sk.next(game.abilities, game.my_level) {
                            ui.add_space(12.0);
                            icon(ui, Icon::Bolt, 13.0, SOON);
                            ui.label(RichText::new(format!("Level up {} next", KEYS[k])).font(semibold(13.0)).color(SOON));
                        }
                    });
                    ui.add_space(4.0);
                    let taken = game.abilities.iter().sum::<i64>().max(0) as usize;
                    skill_grid(ui, sk, taken, sk.next_step(game.abilities, game.my_level));
                });
            }
            ui.add_space(6.0);
            note(ui, "Most played first; picks the playstyle you are building until you choose one. Items you hold are outlined.");
        });
        let Some(m) = &b.mayhem else { return };
        section(ui, &format!("Augments for {}", s.data.champion_name(b.champion_id)));
        card(ui, |ui| match &m.augments {
            Err(e) => {
                error(ui, e);
            }
            Ok(_) => {
                ui.columns(3, |cols| {
                    for (ui, rarity) in cols.iter_mut().zip(Rarity::ORDER) {
                        overline(ui, rarity.label());
                        ui.add_space(2.0);
                        for a in m.best(rarity) {
                            ui.horizontal(|ui| {
                                g.augment(ui, a.icon.as_deref(), &a.name, 28.0);
                                let color = overlay::augment_tier_color(Some(a.tier));
                                ui.label(RichText::new(a.tier.letter()).font(display(15.0)).color(color));
                                ui.add(eframe::egui::Label::new(RichText::new(&a.name).size(12.5).color(BODY)).truncate());
                            });
                        }
                    }
                });
                ui.add_space(4.0);
                note(ui, "Blitz's tier for each augment on this champion, best first.");
            }
        });
    }
}
