//! League and TFT match history, the shared game cards and the ten-player scoreboard.

use eframe::egui::{Align, Align2, Color32, Frame, Grid, Label, Layout, Margin, Rect, RichText, Sense, Stroke, Ui, vec2};

use super::home::{lol_summary, void};
use super::stats::{ago, cs_per_min, datetime, mmss, no_data_hint, queue_name};
use super::theme::*;
use super::widgets::*;
use super::{App, Tab};
use crate::ddragon::{StaticData, tft_label};
use crate::lcu::{LolGame, MatchDetail};
use crate::worker::{Action, Snapshot};
use crate::itemset;

/// A fixed-width column inside a row.
pub(super) fn col<R>(ui: &mut Ui, w: f32, f: impl FnOnce(&mut Ui) -> R) -> R {
    ui.vertical(|ui| {
        ui.set_width(w);
        ui.spacing_mut().item_spacing.y = 2.0;
        f(ui)
    })
    .inner
}

/// A single-line label cut to `w` points.
pub(super) fn trunc(ui: &mut Ui, text: RichText, w: f32) {
    ui.allocate_ui_with_layout(vec2(w, 18.0), Layout::left_to_right(Align::Center), |ui| {
        ui.set_max_width(w);
        ui.add(Label::new(text).truncate());
    });
}

/// A card tinted toward `color` with a colored stripe on its left edge.
pub(super) fn stripe_card<R>(ui: &mut Ui, color: Color32, f: impl FnOnce(&mut Ui) -> R) -> R {
    let frame = Frame::new()
        .fill(mix(SURFACE, color, 0.045))
        .stroke(Stroke::new(1.0, mix(BORDER, color, 0.16)))
        .corner_radius(RADIUS)
        .inner_margin(Margin { left: 18, right: 14, top: 12, bottom: 12 })
        .outer_margin(Margin { bottom: 6, ..Margin::ZERO });
    let r = frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        f(ui)
    });
    let rect = r.response.rect;
    let stripe = Rect::from_min_size(rect.min + vec2(0.0, 0.0), vec2(4.0, rect.height()));
    ui.painter().add(eframe::egui::epaint::RectShape::filled(
        stripe,
        eframe::egui::CornerRadius { nw: RADIUS, sw: RADIUS, ne: 0, se: 0 },
        color,
    ));
    r.inner
}

impl App {
    pub(super) fn lol_matches(&mut self, ui: &mut Ui, s: &Snapshot) {
        let games = match &s.lol_history {
            Err(e) => return void(error(ui, e)),
            Ok(g) if g.is_empty() => return empty_state(ui, Icon::Swords, "No games yet", no_data_hint(s), |_| {}),
            Ok(g) => g,
        };
        let icons = self.icons.clone();
        card(ui, |ui| lol_summary(ui, &Gfx::new(&icons, &s.data), games));
        section(ui, "Recent League games");
        let root = s.league_root();
        let me = s.summoner.as_ref().map_or("", |x| x.puuid.as_str());
        if let Some(id) = self.game_cards(ui, s, games, root.as_deref(), me, "self") {
            self.lookup(id);
        }
        ui.add_space(6.0);
        ui.vertical_centered(|ui| {
            if s.history_complete {
                note(ui, format!("All {} games the client keeps are shown.", games.len()));
            } else if ui.add(Btn::secondary(&format!("Load {} more", 20)).icon(Icon::ChevronDown)).clicked() {
                self.request(Action::MoreHistory);
            }
        });
    }

    /// One card per game with an expandable 10-player table; returns a Riot ID the user chose to look up.
    pub(super) fn game_cards(
        &mut self,
        ui: &mut Ui,
        s: &Snapshot,
        games: &[LolGame],
        root: Option<&std::path::Path>,
        viewer: &str,
        salt: &str,
    ) -> Option<String> {
        let icons = self.icons.clone();
        let g = Gfx::new(&icons, &s.data);
        let mut look = None;
        for game in games {
            let color = result_color(game.win);
            stripe_card(ui, color, |ui| {
                let wide = ui.available_width() > 700.0;
                ui.horizontal(|ui| {
                    col(ui, 100.0, |ui| {
                        ui.label(RichText::new(if game.win { "Victory" } else { "Defeat" }).font(semibold(14.0)).color(color));
                        trunc(ui, RichText::new(queue_name(game.queue_id)).size(12.0).color(BODY), 100.0);
                        ui.label(RichText::new(ago(game.created_ms)).size(11.5).color(MUTED)).on_hover_text(datetime(game.created_ms));
                        ui.label(RichText::new(mmss(game.duration_s as f64)).size(11.5).color(MUTED));
                    });
                    g.champ(ui, game.champion_id, 52.0);
                    ui.add_space(4.0);
                    col(ui, 118.0, |ui| {
                        kda(ui, game.kills, game.deaths, game.assists, 19.0);
                        let kda = game.kda();
                        ui.label(RichText::new(format!("{kda:.2} KDA")).font(semibold(12.0)).color(kda_color(kda)));
                        trunc(ui, RichText::new(s.data.champion_name(game.champion_id)).size(11.5).color(MUTED), 118.0);
                    });
                    col(ui, 92.0, |ui| {
                        ui.label(RichText::new(format!("{} CS", game.cs)).font(semibold(13.0)).color(TEXT));
                        ui.label(RichText::new(format!("{:.1} / min", cs_per_min(game))).size(11.5).color(MUTED));
                        ui.label(RichText::new(format!("{:.1}k gold", game.gold as f64 / 1000.0)).size(11.5).color(MUTED));
                    });
                    if wide {
                        g.items(ui, &game.items, 26.0);
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let open = self.expanded.contains(&game.game_id);
                        let toggle = Btn::ghost(if open { "Hide" } else { "Details" }).icon(if open { Icon::ChevronUp } else { Icon::ChevronDown }).small();
                        if ui.add(toggle).clicked() {
                            if open {
                                self.expanded.remove(&game.game_id);
                            } else {
                                self.expanded.insert(game.game_id);
                                if !matches!(s.match_details.get(&game.game_id), Some(Ok(_))) {
                                    self.request(Action::MatchDetail(game.game_id));
                                }
                            }
                        }
                        if let Some(root) = root {
                            if ui
                                .add_enabled(!s.data.champions.is_empty(), Btn::ghost("Item set").icon(Icon::Save).small())
                                .on_hover_text("Write this game's items as an item set the League client shows in the shop")
                                .clicked()
                            {
                                self.write_item_set(Some(root), &s.data, game);
                            }
                        }
                    });
                });
                if !wide {
                    ui.add_space(4.0);
                    g.items(ui, &game.items, 24.0);
                }
                if self.expanded.contains(&game.game_id) {
                    ui.add_space(6.0);
                    ui.separator();
                    match s.match_details.get(&game.game_id) {
                        None => {
                            ui.horizontal(|ui| {
                                ui.add(eframe::egui::Spinner::new().size(14.0).color(ACCENT));
                                note(ui, "Loading…");
                            });
                        }
                        Some(Err(e)) => void(error(ui, e)),
                        Some(Ok(d)) => {
                            if let Some(id) = match_table(ui, &g, &d.clone().with_me(viewer), &format!("{salt}-{}", game.game_id)) {
                                look = Some(id);
                            }
                        }
                    }
                }
            });
        }
        look
    }

    pub(super) fn lookup(&mut self, riot_id: String) {
        self.lookup_input = riot_id.clone();
        self.request(Action::Lookup(riot_id));
        self.tab = Tab::Players;
    }

    pub(super) fn write_item_set(&mut self, root: Option<&std::path::Path>, data: &StaticData, g: &LolGame) {
        let r = root
            .ok_or_else(|| "League install not found".to_string())
            .and_then(|root| itemset::write_from_game(root, data, g))
            .map(|p| format!("Item set written: {}", p.display()));
        self.notify(r);
    }

    pub(super) fn tft_matches(&mut self, ui: &mut Ui, s: &Snapshot) {
        let games = match &s.tft_history {
            Err(e) => return void(error(ui, e)),
            Ok(g) if g.is_empty() => return empty_state(ui, Icon::Hex, "No TFT games yet", no_data_hint(s), |_| {}),
            Ok(g) => g,
        };
        card(ui, |ui| super::home::tft_summary(ui, games));
        section(ui, "Recent TFT games");
        for game in games {
            let color = placement_color(game.placement);
            stripe_card(ui, color, |ui| {
                ui.horizontal(|ui| {
                    let (r, _) = ui.allocate_exact_size(vec2(52.0, 52.0), Sense::hover());
                    ui.painter().rect(r, 10, alpha(color, 0.16), Stroke::new(1.0, alpha(color, 0.5)), eframe::egui::StrokeKind::Inside);
                    ui.painter().text(r.center(), Align2::CENTER_CENTER, format!("#{}", game.placement), display(24.0), color);
                    ui.add_space(4.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        ui.label(RichText::new(queue_name(game.queue_id)).font(semibold(14.0)).color(TEXT));
                        ui.label(RichText::new(format!("Set {} · level {} · {}", game.set, game.level, mmss(game.length_s as f64))).color(MUTED));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Min), |ui| {
                        ui.label(RichText::new(ago(game.datetime_ms)).size(11.5).color(MUTED)).on_hover_text(datetime(game.datetime_ms));
                    });
                });
                ui.add_space(6.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                    for t in game.traits.iter().take(6) {
                        trait_chip(ui, &format!("{} {}", t.1, tft_label(&t.0)), t.2);
                    }
                });
                ui.add_space(2.0);
                ui.horizontal_wrapped(|ui| {
                    ui.spacing_mut().item_spacing = vec2(4.0, 4.0);
                    for u in &game.units {
                        unit_chip(ui, tft_label(&u.0), u.1.max(1));
                    }
                });
            });
        }
    }
}

/// An active TFT trait: bronze, silver, gold, then prismatic by style.
fn trait_chip(ui: &mut Ui, text: &str, style: i64) {
    let color = match style {
        1 => Color32::from_rgb(0xC0, 0x7D, 0x55),
        2 => Color32::from_rgb(0xA9, 0xB9, 0xC9),
        3 => Color32::from_rgb(0xEC, 0xBC, 0x5C),
        _ => Color32::from_rgb(0x8F, 0xE3, 0xFF),
    };
    pill(ui, text, color);
}

/// A board unit with its star level.
fn unit_chip(ui: &mut Ui, name: &str, stars: i64) {
    let star_color = match stars {
        1 => Color32::from_rgb(0xC0, 0x7D, 0x55),
        2 => Color32::from_rgb(0xC9, 0xD6, 0xE3),
        _ => SOON,
    };
    let galley = ui.painter().layout_no_wrap(name.to_string(), body(12.0), BODY);
    let star = 9.0;
    let size = vec2(galley.size().x + 16.0 + stars as f32 * (star + 1.0) + 4.0, 22.0);
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let p = ui.painter();
    p.rect(rect, 6, RAISED, Stroke::new(1.0, BORDER), eframe::egui::StrokeKind::Inside);
    p.galley(eframe::egui::pos2(rect.left() + 8.0, rect.center().y - galley.size().y / 2.0), galley.clone(), BODY);
    let mut x = rect.left() + 8.0 + galley.size().x + 5.0;
    for _ in 0..stars {
        paint_star(p, eframe::egui::pos2(x + star / 2.0, rect.center().y), star / 2.0, star_color);
        x += star + 1.0;
    }
}

/// Ten-player table; returns a Riot ID whose "Look up" button was pressed.
pub(super) fn match_table(ui: &mut Ui, g: &Gfx, d: &MatchDetail, salt: &str) -> Option<String> {
    let mut look = None;
    let mut teams: Vec<i64> = d.players.iter().map(|p| p.team).collect();
    teams.sort_unstable();
    teams.dedup();
    let max_dmg = d.players.iter().map(|p| p.damage).max().unwrap_or(1).max(1);
    for team in teams {
        let won = d.players.iter().any(|p| p.team == team && p.win);
        let kills = d.team_sum(team, |p| p.kills);
        let gold = d.team_sum(team, |p| p.gold);
        let team_color = if team == 100 { BLUE_TEAM } else { RED_TEAM };
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(3.0, 16.0), Sense::hover());
            ui.painter().rect_filled(r, 1, team_color);
            ui.label(RichText::new(if team == 100 { "Blue team" } else { "Red team" }).font(semibold(13.5)).color(team_color));
            pill(ui, if won { "Victory" } else { "Defeat" }, result_color(won));
            ui.label(RichText::new(format!("{kills} kills · {:.1}k gold", gold as f64 / 1000.0)).color(MUTED));
        });
        Grid::new((salt, team)).striped(true).spacing([10.0, 4.0]).min_row_height(30.0).show(ui, |ui| {
            for h in ["", "Player", "K / D / A", "CS", "Damage", "Gold", "Items", ""] {
                ui.label(RichText::new(h.to_uppercase()).font(semibold(10.5)).color(MUTED));
            }
            ui.end_row();
            for p in d.players.iter().filter(|p| p.team == team) {
                g.champ(ui, p.champion_id, 26.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    let name = RichText::new(&p.name).font(semibold(12.5));
                    trunc(ui, if p.is_me { name.color(CYAN) } else { name.color(TEXT) }, 150.0);
                    ui.label(RichText::new(g.data.champion_name(p.champion_id)).size(11.0).color(MUTED));
                });
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    kda(ui, p.kills, p.deaths, p.assists, 14.0);
                    let kda = (p.kills + p.assists) as f64 / p.deaths.max(1) as f64;
                    ui.label(RichText::new(format!("{kda:.2}")).size(11.0).color(kda_color(kda)));
                });
                ui.label(RichText::new(p.cs.to_string()).color(BODY));
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.label(RichText::new(format!("{:.1}k", p.damage as f64 / 1000.0)).color(BODY));
                    progress(ui, p.damage as f32 / max_dmg as f32, 64.0, alpha(team_color, 0.85));
                });
                ui.label(RichText::new(format!("{:.1}k", p.gold as f64 / 1000.0)).color(BODY));
                g.items(ui, &p.items, 20.0);
                if !p.is_me && p.name.contains('#') {
                    if ui.add(Btn::ghost("Look up").icon(Icon::Search).small()).clicked() {
                        look = Some(p.name.clone());
                    }
                } else {
                    ui.label("");
                }
                ui.end_row();
            }
        });
    }
    look
}
