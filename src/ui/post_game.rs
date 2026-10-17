//! Post-game: result, LP change, this game against your usual games, and the scoreboard.

use eframe::egui::{Align, Grid, Layout, RichText, Ui, vec2};

use super::home::{session_line, void};
use super::matches::match_table;
use super::stats::{as_lol_game, averages, datetime, mmss, now_ms, queue_name};
use super::theme::*;
use super::widgets::*;
use super::App;
use crate::lcu::{MatchPlayer, RankedQueue};
use crate::lp;
use crate::worker::{Action, Snapshot};

impl App {
    pub(super) fn post_game(&mut self, ui: &mut Ui, s: &Snapshot) {
        let Some(pg) = &s.post_game else {
            return empty_state(
                ui,
                Icon::Chart,
                "No finished game yet",
                "Opens after a game with that game's stats and how it compares to your usual games.",
                |_| {},
            );
        };
        let d = &pg.detail;
        let Some(me) = d.me() else {
            return empty_state(ui, Icon::Chart, "Post-game", "You were not a player in this game.", |_| {});
        };
        let icons = self.icons.clone();
        let g = Gfx::new(&icons, &s.data);
        let color = result_color(me.win);
        let q = if d.queue_id > 0 { queue_name(d.queue_id) } else { d.mode.clone() };
        hero(ui, g.splash(me.champion_id), 150.0, vec2(0.5, 0.3), |ui| {
            g.champ(ui, me.champion_id, 72.0);
            ui.add_space(8.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.label(RichText::new(if me.win { "VICTORY" } else { "DEFEAT" }).font(display(36.0)).color(color));
                ui.label(RichText::new(format!("{} · {q}", s.data.champion_name(me.champion_id))).font(semibold(14.0)).color(TEXT));
                ui.label(RichText::new(format!("{} · {}", mmss(d.duration_s as f64), datetime(d.created_ms))).color(BODY));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if let Some(lp) = &pg.lp {
                    let up = lp.delta >= 0;
                    let rank = RankedQueue {
                        queue: lp.queue.clone(),
                        tier: lp.tier.clone(),
                        division: lp.division.clone(),
                        lp: lp.lp,
                        wins: 0,
                        losses: 0,
                    };
                    ui.vertical(|ui| {
                        ui.with_layout(Layout::top_down(Align::Max), |ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            let sign = if up { "+" } else { "" };
                            ui.label(RichText::new(format!("{sign}{} LP", lp.delta)).font(display(30.0)).color(result_color(up)));
                            ui.label(RichText::new(format!("{} now {}", rank.label(), rank.rank())).color(TEXT));
                        });
                    });
                    rank_emblem(ui, &lp.tier, 52.0);
                }
            });
        });
        ui.add_space(10.0);
        let mins = (d.duration_s as f64 / 60.0).max(1.0);
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
            stat_tile(ui, "CS", me.cs.to_string(), TEXT, Some(&format!("{:.1} / min", me.cs as f64 / mins)));
            stat_tile(ui, "Gold", format!("{:.1}k", me.gold as f64 / 1000.0), SOON, None);
            stat_tile(ui, "Damage", format!("{:.1}k", me.damage as f64 / 1000.0), LOSS, None);
            stat_tile(ui, "Vision", me.vision.to_string(), CYAN, None);
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            g.items(ui, &me.items, 30.0);
            if matches!(s.phase.as_str(), "EndOfGame" | "PreEndOfGame") {
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add_enabled(s.busy.is_none(), Btn::primary("Play again").icon(Icon::Play))
                        .on_hover_text("Leaves the end of game screen for a lobby of the same queue")
                        .clicked()
                    {
                        self.request(Action::PlayAgain);
                    }
                });
            }
        });
        let session = lp::session(&s.lp_log, now_ms());
        if !session.is_empty() {
            ui.add_space(6.0);
            session_line(ui, session);
        }

        let root = s.league_root();
        section_with(ui, "Insights", |ui| {
            if root.is_some() && ui.add(Btn::secondary("Save this game's build as an item set").icon(Icon::Save).small()).clicked() {
                let lg = as_lol_game(d, me);
                self.write_item_set(root.as_deref(), &s.data, &lg);
            }
        });
        let history = s.lol_history.as_deref().unwrap_or(&[]);
        let others = || history.iter().filter(|g| g.game_id != d.game_id);
        let same_queue = averages(others().filter(|g| d.queue_id > 0 && g.queue_id == d.queue_id)).filter(|a| a.n >= 3);
        let (base, what) = match same_queue {
            Some(a) => (Some(a), format!("your other {} games", queue_name(d.queue_id))),
            None => (averages(others()), "your recent games".to_string()),
        };
        card(ui, |ui| match base {
            None => void(note(ui, "Play a few more games for a baseline.")),
            Some(a) => {
                let team = |f: fn(&MatchPlayer) -> i64| d.team_sum(me.team, f).max(1) as f64;
                card_head(ui, &format!("Compared with {what} ({} games)", a.n), |_| {});
                ui.add_space(4.0);
                Grid::new("insights").striped(true).spacing([28.0, 6.0]).min_col_width(80.0).show(ui, |ui| {
                    for h in ["", "This game", "Your average", ""] {
                        ui.label(RichText::new(h.to_uppercase()).font(semibold(10.5)).color(MUTED));
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
                        ui.label(RichText::new(label).color(BODY));
                        ui.label(RichText::new(format!("{now:.1}")).font(semibold(14.0)).color(TEXT));
                        ui.label(RichText::new(format!("{avg:.1}")).color(MUTED));
                        trend(ui, now, avg, higher_better);
                        ui.end_row();
                    }
                });
                ui.add_space(10.0);
                let shares = [
                    ("Kill participation", (me.kills + me.assists) as f64 / team(|p| p.kills), CYAN),
                    ("Share of your team's damage", me.damage as f64 / team(|p| p.damage), ACCENT),
                    ("Share of your team's gold", me.gold as f64 / team(|p| p.gold), SOON),
                    ("Share of damage taken", me.damage_taken as f64 / team(|p| p.damage_taken), LOSS),
                    ("Share of your team's vision", me.vision as f64 / team(|p| p.vision), WIN),
                ];
                tiles(ui, |ui| {
                    for (label, frac, c) in shares {
                        ui.vertical(|ui| {
                            ui.set_width(260.0);
                            ui.horizontal(|ui| {
                                overline(ui, label);
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    ui.label(RichText::new(format!("{:.0}%", frac * 100.0)).font(semibold(13.0)).color(TEXT));
                                });
                            });
                            progress(ui, frac as f32, 260.0, c);
                        });
                        ui.add_space(24.0);
                    }
                });
            }
        });
        section(ui, "Scoreboard");
        if let Some(id) = card(ui, |ui| match_table(ui, &g, d, "postgame")) {
            self.lookup(id);
        }
    }
}
