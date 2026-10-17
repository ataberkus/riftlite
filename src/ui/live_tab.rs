//! Live game tab (Live Client Data API) and enemy scouting shared with the overlay.

use eframe::egui::{Align, Align2, Color32, Grid, Layout, RichText, Sense, Stroke, StrokeKind, Ui, vec2};

use super::matches::trunc;
use super::home::void;
use super::stats::{Scout, cs_baseline, kilo, live_puuid, mastery_on, mmss, scout, signed};
use super::theme::*;
use super::widgets::*;
use super::{App, loading_screen};
use crate::live::{FeedEvent, LiveGame, LivePlayer, Objective};
use crate::meta::MetaBuild;
use crate::worker::Snapshot;

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

impl App {
    pub(super) fn live_tab(&mut self, ui: &mut Ui, s: &Snapshot) {
        let icons = self.icons.clone();
        let g = Gfx::new(&icons, &s.data);
        let Some(game) = &s.live else {
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
            return;
        };
        live_header(ui, &g, s, game);
        if self.settings.overlay_enabled && s.window_mode == Some(0) {
            ui.add_space(6.0);
            error(ui, FULLSCREEN_WARNING);
        }
        let me = game.me();
        if let Some((b, sel)) = picked_build(s, me.and_then(|p| s.data.champion_by_key(&p.champion_key))) {
            let owned: Vec<i64> = me.map(|p| p.items.iter().map(|it| it.id).collect()).unwrap_or_default();
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
            });
        }
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
