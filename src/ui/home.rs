//! Home and Profile tabs, plus the ranked / performance / mastery blocks the
//! Players tab reuses.

use eframe::egui::{Align, Color32, Layout, RichText, Ui, vec2};

use super::stats::{Avg, averages, cs_per_min, date, next_division, no_data_hint, now_ms, streak, title};
use super::theme::*;
use super::widgets::*;
use super::App;
use crate::lcu::{self, LolGame, Mastery, RankedQueue, TftGame};
use crate::lp;
use crate::riot::{self, Product};
use crate::worker::{Action, LcuStatus, Snapshot};

impl App {
    pub(super) fn home(&mut self, ui: &mut Ui, s: &Snapshot) {
        let icons = self.icons.clone();
        let g = Gfx::new(&icons, &s.data);
        hero(ui, Some(icons.banner), 168.0, vec2(1.0, 0.5), |ui| {
            if let Some(sum) = &s.summoner {
                g.profile(ui, sum.profile_icon, &sum.game_name, 64.0);
                ui.add_space(12.0);
            }
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                overline(ui, "riftlite");
                let hello = match &s.summoner {
                    Some(sum) if !sum.game_name.is_empty() => format!("Welcome back, {}", sum.game_name),
                    _ => "Welcome to riftlite".to_string(),
                };
                ui.label(RichText::new(hello).font(display(32.0)).color(TEXT));
                let (color, text) = match &s.lcu {
                    LcuStatus::Connected { .. } => (WIN, "League client connected"),
                    LcuStatus::ClientNotRunning => (MUTED, "League client not running"),
                    LcuStatus::Error(_) => (LOSS, "League client unreachable"),
                };
                ui.horizontal(|ui| {
                    dot(ui, color);
                    ui.label(RichText::new(text).color(BODY));
                });
            });
        });

        section(ui, "Games");
        ui.columns(2, |cols| {
            for (ui, p) in cols.iter_mut().zip(Product::ALL) {
                card(ui, |ui| {
                    let (running, what) = match p {
                        Product::League => match (s.running.league_game, s.running.league_client) {
                            (Some(pid), _) => (true, ("In game", format!("pid {pid}"))),
                            (None, Some(pid)) => (true, ("Client running", format!("pid {pid}"))),
                            _ => (false, ("Not running", String::new())),
                        },
                        Product::Tft => match (s.running.tft_game, s.running.tft_client) {
                            (Some(pid), _) => (true, ("In game", format!("pid {pid}"))),
                            (None, Some(pid)) => (true, ("Client running", format!("pid {pid}"))),
                            _ => (false, ("Not running", String::new())),
                        },
                    };
                    ui.horizontal(|ui| {
                        product_badge(ui, p);
                        ui.add_space(4.0);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 3.0;
                            ui.label(RichText::new(p.label()).font(semibold(15.0)).color(TEXT));
                            let r = pill(ui, what.0, if running { WIN } else { MUTED });
                            if !what.1.is_empty() {
                                r.on_hover_text(&what.1);
                            }
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            let can = s.installs.riot_client.is_some() && s.installs.get(p).is_some();
                            if ui.add_enabled(can && !running, Btn::primary("Launch").icon(Icon::Play)).clicked() {
                                let r = riot::launch(&s.installs, p).map(|_| format!("Launching {} via Riot Client", p.label()));
                                self.notify(r);
                            }
                        });
                    });
                    ui.add_space(6.0);
                    match s.installs.get(p) {
                        Some(i) => {
                            ui.horizontal(|ui| {
                                icon(ui, Icon::Folder, 14.0, MUTED);
                                ui.add(eframe::egui::Label::new(RichText::new(i.path.display().to_string()).color(BODY)).truncate());
                            });
                            let v = if i.verified { "client executable found" } else { "client executable missing" };
                            small_note(ui, format!("source: {} · {v}", i.source));
                        }
                        None => {
                            note(ui, "Not installed (set the folder in Settings if it is installed elsewhere)");
                        }
                    }
                });
            }
        });

        section(ui, "System");
        ui.columns(2, |cols| {
            card(&mut cols[0], |ui| {
                card_head(ui, "Riot Client", |ui| {
                    if let Some(pid) = s.running.riot_client {
                        pill(ui, "Running", WIN).on_hover_text(format!("pid {pid}"));
                    }
                });
                match &s.installs.riot_client {
                    Some(p) => {
                        ui.add(eframe::egui::Label::new(RichText::new(p.display().to_string()).color(BODY)).wrap());
                    }
                    None => {
                        error(ui, "RiotClientServices.exe not found; launching is unavailable");
                    }
                }
                if let Some(pid) = s.running.riot_client {
                    small_note(ui, format!("running (pid {pid})"));
                }
            });
            card(&mut cols[1], |ui| {
                card_head(ui, "Static data", |ui| {
                    if !s.data.version.is_empty() {
                        pill(ui, format!("Patch {}", s.data.version), ACCENT_TEXT);
                    }
                });
                match (&s.data_error, s.data.version.is_empty()) {
                    (Some(e), _) => void(error(ui, format!("Data Dragon: {e}"))),
                    (None, true) => {
                        ui.horizontal(|ui| {
                            ui.add(eframe::egui::Spinner::new().size(14.0).color(ACCENT));
                            note(ui, "Loading Data Dragon…");
                        });
                    }
                    (None, false) => {
                        ui.horizontal(|ui| {
                            for (label, n) in [("Champions", s.data.champions.len()), ("Items", s.data.items.len()), ("Runes", s.data.perks.len())] {
                                stat_tile(ui, label, n.to_string(), TEXT, None);
                            }
                        });
                        small_note(
                            ui,
                            format!(
                                "Data Dragon {}{}",
                                s.data.version,
                                if s.data.has_client_assets { " · client asset names merged" } else { "" }
                            ),
                        );
                    }
                }
            });
        });
    }

    pub(super) fn profile(&mut self, ui: &mut Ui, s: &Snapshot) {
        let icons = self.icons.clone();
        let g = Gfx::new(&icons, &s.data);
        let Some(sum) = &s.summoner else {
            empty_state(ui, Icon::Profile, "No profile yet", s.profile_error.as_deref().unwrap_or(no_data_hint(s)), |_| {});
            return;
        };
        let splash = s.mastery.first().and_then(|m| g.splash(m.champion_id)).or(Some(icons.banner));
        hero(ui, splash, 150.0, vec2(0.5, 0.2), |ui| {
            g.profile(ui, sum.profile_icon, &sum.game_name, 80.0);
            ui.add_space(14.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.label(RichText::new(&sum.game_name).font(display(32.0)).color(TEXT));
                    if !sum.tag_line.is_empty() {
                        ui.label(RichText::new(format!("#{}", sum.tag_line)).font(display(32.0)).color(MUTED));
                    }
                });
                ui.horizontal(|ui| {
                    pill(ui, format!("Level {}", sum.level), ACCENT_TEXT);
                    if let Some(q) = s.ranked.iter().find(|q| q.queue == "RANKED_SOLO_5x5" && q.is_ranked()) {
                        pill(ui, format!("Solo/Duo {}", q.rank()), tier_color(&q.tier));
                    }
                    if let Some(h) = s.honor_level {
                        pill(ui, format!("Honor {h}"), CYAN);
                    }
                    if let Some(w) = s.wallet {
                        let n = |v: i64| crate::meta::thousands(v.max(0) as u64);
                        pill(ui, format!("{} BE", n(w.blue_essence)), BODY);
                        pill(ui, format!("{} OE", n(w.orange_essence)), SOON);
                        pill(ui, format!("{} RP", n(w.rp)), ACCENT_TEXT);
                    }
                });
            });
        });

        ui.add_space(4.0);
        ui.columns(2, |cols| {
            for (ui, tft) in cols.iter_mut().zip([false, true]) {
                card(ui, |ui| {
                    card_head(ui, if tft { "Teamfight Tactics · ranked" } else { "League of Legends · ranked" }, |_| {});
                    ranked_list(ui, &s.ranked, tft);
                });
            }
        });
        ranked_progress(ui, s);
        let lol = s.lol_history.as_deref().unwrap_or(&[]);
        if !lol.is_empty() {
            card(ui, |ui| lol_summary(ui, &g, lol));
        }
        let tft = s.tft_history.as_deref().unwrap_or(&[]);
        if !tft.is_empty() {
            card(ui, |ui| tft_summary(ui, tft));
        }
        ui.columns(2, |cols| {
            card(&mut cols[0], |ui| {
                card_head(ui, "Champion mastery", |_| {});
                mastery_list(ui, &g, &s.mastery);
            });
            card(&mut cols[1], |ui| {
                card_head(ui, "Challenges", |_| {});
                match &s.challenges {
                    None => void(note(ui, "No challenge data")),
                    Some(c) => {
                        ui.horizontal(|ui| {
                            rank_emblem(ui, &c.level, 40.0);
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 0.0;
                                ui.label(RichText::new(title(&c.level)).font(display(20.0)).color(tier_color(&c.level)));
                                ui.label(RichText::new(format!("{} points · {} to next rank", c.score, c.to_next)).color(MUTED));
                            });
                        });
                        ui.add_space(6.0);
                        for cat in &c.categories {
                            ui.horizontal(|ui| {
                                ui.add_sized([110.0, 18.0], eframe::egui::Label::new(RichText::new(title(&cat.name)).color(BODY)).truncate());
                                ui.add_sized(
                                    [80.0, 18.0],
                                    eframe::egui::Label::new(RichText::new(title(&cat.level)).font(semibold(12.0)).color(tier_color(&cat.level))),
                                );
                                let frac = if cat.max > 0 { cat.current as f32 / cat.max as f32 } else { 0.0 };
                                let w = (ui.available_width() - 90.0).max(60.0);
                                let tier = tier_color(&cat.level);
                                progress(ui, frac, w, if tier == FAINT { ACCENT } else { tier });
                                ui.label(RichText::new(format!("{} / {}", cat.current, cat.max)).size(11.5).color(MUTED));
                            });
                        }
                    }
                }
            });
        });

        section_with(ui, "Missions", |ui| {
            let label = if s.missions.is_some() { "Reload missions" } else { "Load missions" };
            if ui.add(Btn::secondary(label).icon(Icon::Refresh).small()).clicked() {
                self.request(Action::LoadMissions);
            }
        });
        match &s.missions {
            None => void(note(ui, "Missions load on request.")),
            Some(Err(e)) => void(error(ui, e)),
            Some(Ok(m)) if m.is_empty() => void(note(ui, "No open League missions")),
            Some(Ok(m)) => card(ui, |ui| {
                for (i, mission) in m.iter().enumerate() {
                    if i > 0 {
                        ui.separator();
                    }
                    ui.horizontal(|ui| {
                        let right = 250.0;
                        ui.allocate_ui_with_layout(vec2((ui.available_width() - right).max(160.0), 0.0), Layout::top_down(Align::Min), |ui| {
                            ui.spacing_mut().item_spacing.y = 2.0;
                            ui.add(eframe::egui::Label::new(RichText::new(&mission.title).font(semibold(13.5)).color(TEXT)).wrap());
                            ui.add(eframe::egui::Label::new(RichText::new(&mission.objective).color(MUTED)).wrap());
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if mission.end_ms > 0 {
                                ui.label(RichText::new(format!("ends {}", date(mission.end_ms))).size(11.5).color(MUTED));
                            }
                            ui.label(RichText::new(format!("{} / {}", mission.current, mission.total)).font(semibold(12.0)).color(BODY));
                            let frac = (mission.current as f32 / mission.total.max(1) as f32).min(1.0);
                            progress(ui, frac, 110.0, if frac >= 1.0 { WIN } else { ACCENT });
                        });
                    });
                }
            }),
        }
    }
}

pub(super) fn void<T>(_: T) {}

/// Ranked Solo/Duo and Flex: the current streak, wins to the next division from the LP log,
/// and this session's results.
fn ranked_progress(ui: &mut Ui, s: &Snapshot) {
    let history = s.lol_history.as_deref().unwrap_or(&[]);
    let rows: Vec<(&RankedQueue, Option<(bool, usize)>)> = [("RANKED_SOLO_5x5", 420), ("RANKED_FLEX_SR", 440)]
        .into_iter()
        .filter_map(|(name, id)| {
            let q = s.ranked.iter().find(|q| q.queue == name && q.is_ranked())?;
            Some((q, streak(history.iter().filter(|g| g.queue_id == id))))
        })
        .collect();
    let session = lp::session(&s.lp_log, now_ms());
    if rows.is_empty() && session.is_empty() {
        return;
    }
    card(ui, |ui| {
        card_head(ui, "Ranked progress", |_| {});
        for (q, st) in rows {
            ui.horizontal_wrapped(|ui| {
                ui.add_sized([120.0, 20.0], eframe::egui::Label::new(RichText::new(q.label()).font(semibold(12.5)).color(TEXT)));
                match st {
                    Some((won, n)) if n >= 2 => void(pill(ui, format!("{n}-{} streak", if won { "win" } else { "loss" }), result_color(won))),
                    Some((won, _)) => void(pill(ui, if won { "won last game" } else { "lost last game" }, MUTED)),
                    None => {}
                }
                let Some(next) = next_division(&q.tier, &q.division) else { return };
                match lp::avg_gain(&s.lp_log, &q.queue) {
                    Some((avg, n)) => {
                        let wins = (((100 - q.lp).max(1) as f64) / avg).ceil() as i64;
                        ui.label(RichText::new(format!("≈{wins} win{} to {next}", if wins == 1 { "" } else { "s" })).color(BODY));
                        ui.label(RichText::new(format!("at +{avg:.0} LP per win (your last {n} recorded)")).size(11.5).color(MUTED));
                    }
                    None => void(ui.label(RichText::new(format!("{} LP to {next}", 100 - q.lp)).color(BODY))),
                }
            });
        }
        if !session.is_empty() {
            ui.add_space(4.0);
            session_line(ui, session);
        }
        ui.add_space(2.0);
        small_note(ui, "Streaks from match history (remakes skipped). LP per win from the LP riftlite saw after your ranked games; the client keeps no LP history.");
    });
}

/// This play session's ranked results per queue, from the LP log.
pub(super) fn session_line(ui: &mut Ui, session: &[lp::LpEntry]) {
    let mut queues: Vec<&str> = session.iter().map(|e| e.queue.as_str()).collect();
    queues.sort_unstable();
    queues.dedup();
    for q in queues {
        let games: Vec<_> = session.iter().filter(|e| e.queue == q).collect();
        let wins = games.iter().filter(|e| e.win).count();
        let net: i64 = games.iter().map(|e| e.delta).sum();
        ui.horizontal_wrapped(|ui| {
            overline(ui, &format!("This session · {}", lcu::ranked_label(q)));
            ui.label(RichText::new(format!("{wins}W {}L", games.len() - wins)).font(semibold(12.5)).color(TEXT));
            ui.label(RichText::new(format!("{}{net} LP", if net >= 0 { "+" } else { "" })).font(semibold(12.5)).color(result_color(net >= 0)));
            ui.spacing_mut().item_spacing.x = 3.0;
            for e in &games {
                pill(ui, format!("{}{}", if e.delta >= 0 { "+" } else { "" }, e.delta), result_color(e.win));
            }
        });
    }
}

/// A rounded square badge with the product's initials.
fn product_badge(ui: &mut Ui, p: Product) {
    let (rect, _) = ui.allocate_exact_size(vec2(44.0, 44.0), eframe::egui::Sense::hover());
    let (top, bottom, text) = match p {
        Product::League => (Color32::from_rgb(0xE0, 0xB4, 0x5A), Color32::from_rgb(0x8A, 0x5E, 0x1C), "LoL"),
        Product::Tft => (Color32::from_rgb(0x9C, 0x7C, 0xFF), Color32::from_rgb(0x4A, 0x2F, 0xB0), "TFT"),
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 11, bottom);
    let upper = eframe::egui::Rect::from_min_size(rect.min, vec2(rect.width(), rect.height() * 0.55));
    painter.add(eframe::egui::epaint::RectShape::filled(
        upper,
        eframe::egui::CornerRadius { nw: 11, ne: 11, sw: 0, se: 0 },
        mix(top, bottom, 0.25),
    ));
    painter.text(rect.center(), eframe::egui::Align2::CENTER_CENTER, text, display(17.0), Color32::WHITE);
}

/// One row per ranked queue: tier emblem, rank, LP, win/loss bar.
pub(super) fn ranked_list(ui: &mut Ui, ranked: &[RankedQueue], tft: bool) {
    let queues: Vec<_> = ranked.iter().filter(|q| q.is_tft() == tft && q.worth_showing()).collect();
    if queues.is_empty() {
        ui.horizontal(|ui| {
            rank_emblem(ui, "", 44.0);
            note(ui, "No ranked data");
        });
        return;
    }
    for q in queues {
        ui.horizontal(|ui| {
            rank_emblem(ui, &q.tier, 48.0);
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                overline(ui, q.label());
                let rank = q.rank();
                let (main, lp) = rank.split_once(" · ").unwrap_or((rank.as_str(), ""));
                ui.horizontal(|ui| {
                    let color = if q.is_ranked() { tier_color(&q.tier) } else { MUTED };
                    ui.label(RichText::new(main).font(display(20.0)).color(color));
                    if !lp.is_empty() {
                        ui.label(RichText::new(lp).font(semibold(12.5)).color(BODY));
                    }
                });
                let total = q.wins + q.losses;
                if total > 0 {
                    ui.horizontal(|ui| {
                        wr_bar(ui, q.wins, q.losses, 110.0);
                        let wr = q.wins as f64 / total as f64;
                        ui.label(RichText::new(format!("{:.0}%", wr * 100.0)).font(semibold(12.0)).color(wr_color(wr)));
                        ui.label(RichText::new(format!("{}W {}L", q.wins, q.losses)).size(11.5).color(MUTED));
                    });
                }
            });
        });
        ui.add_space(4.0);
    }
}

/// Mastery rows: portrait, name, level, points, last played.
pub(super) fn mastery_list(ui: &mut Ui, g: &Gfx, mastery: &[Mastery]) {
    if mastery.is_empty() {
        return void(note(ui, "No mastery data"));
    }
    for m in mastery.iter().take(10) {
        ui.horizontal(|ui| {
            g.champ(ui, m.champion_id, 30.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(RichText::new(g.data.champion_name(m.champion_id)).font(semibold(13.0)).color(TEXT));
                ui.label(RichText::new(if m.last_play_ms > 0 { format!("played {}", date(m.last_play_ms)) } else { String::new() }).size(11.0).color(MUTED));
            });
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(format!("{} pts", m.points)).color(MUTED));
                pill(ui, format!("M{}", m.level), if m.level >= 10 { SOON } else { ACCENT_TEXT }).on_hover_text(format!("Mastery level {}", m.level));
            });
        });
    }
}

/// Recent League form: win rate, KDA and CS tiles, KDA and CS/min trend lines, results and most played.
pub(super) fn lol_summary(ui: &mut Ui, g: &Gfx, games: &[LolGame]) {
    let Some(a) = averages(games) else { return };
    let wins = games.iter().filter(|g| g.win).count();
    card_head(ui, &format!("League · last {} games", games.len()), |_| {});
    tiles(ui, |ui| {
        stat_tile(ui, "Win rate", format!("{:.0}%", a.win_rate() * 100.0), wr_color(a.win_rate()), Some(&format!("{wins}W {}L", games.len() - wins)));
        stat_tile(ui, "KDA", format!("{:.2}", a.kda()), kda_color(a.kda()), Some(&format!("{:.1} / {:.1} / {:.1}", a.kills, a.deaths, a.assists)));
        stat_tile(ui, "CS / min", format!("{:.1}", a.cs_min), TEXT, Some(&format!("{:.0} gold / min", a.gold_min)));
        stat_tile(ui, "Damage / min", format!("{:.0}", a.dmg_min), TEXT, Some(&format!("{:.1} vision / min", a.vision_min)));
    });
    ui.add_space(6.0);
    // History is newest first; trends read left to right.
    let kda: Vec<f64> = games.iter().rev().map(LolGame::kda).collect();
    let cs: Vec<f64> = games.iter().rev().map(cs_per_min).collect();
    ui.columns(2, |cols| {
        for (ui, (label, values, avg, color)) in cols.iter_mut().zip([("KDA trend", &kda, a.kda(), CYAN), ("CS / min trend", &cs, a.cs_min, ACCENT)]) {
            inset_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    overline(ui, label);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let last = values.last().copied().unwrap_or(0.0);
                        ui.label(RichText::new(format!("last {last:.1}")).size(11.5).color(MUTED));
                    });
                });
                let w = ui.available_width();
                sparkline(ui, values, vec2(w, 46.0), color, Some(avg), |v| format!("{v:.1}"));
            });
        }
    });
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        overline(ui, "Results");
        result_strip(ui, games.iter().rev().map(|g| g.win));
    });
    let mut by_champ: Vec<(i64, usize, usize)> = Vec::new();
    for game in games {
        match by_champ.iter_mut().find(|c| c.0 == game.champion_id) {
            Some(c) => {
                c.1 += 1;
                c.2 += game.win as usize;
            }
            None => by_champ.push((game.champion_id, 1, game.win as usize)),
        }
    }
    by_champ.sort_by(|a, b| b.1.cmp(&a.1));
    ui.add_space(4.0);
    overline(ui, "Most played");
    tiles(ui, |ui| {
        for &(c, n, w) in by_champ.iter().take(5) {
            inset_frame().show(ui, |ui| {
                ui.horizontal(|ui| {
                    g.champ(ui, c, 32.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(RichText::new(g.data.champion_name(c)).font(semibold(12.5)).color(TEXT));
                        let wr = w as f64 / n as f64;
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.label(RichText::new(format!("{n}G {w}W")).size(11.5).color(MUTED));
                            ui.label(RichText::new(format!("{:.0}%", wr * 100.0)).font(semibold(11.5)).color(wr_color(wr)));
                        });
                    });
                });
            });
        }
    });
    let positions: Vec<(&str, Avg)> = ["Top", "Jungle", "Mid", "Bot", "Support"]
        .into_iter()
        .filter_map(|p| Some((p, averages(games.iter().filter(|g| g.position == p))?)))
        .collect();
    if !positions.is_empty() {
        ui.add_space(4.0);
        overline(ui, "By position");
        tiles(ui, |ui| {
            for (p, a) in &positions {
                inset_frame().show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.label(RichText::new(*p).font(semibold(12.5)).color(TEXT));
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.label(RichText::new(format!("{:.0}%", a.win_rate() * 100.0)).font(semibold(11.5)).color(wr_color(a.win_rate())));
                            ui.label(RichText::new(format!("{}G · KDA {:.1}", a.n, a.kda())).size(11.5).color(MUTED));
                        });
                    });
                });
            }
        });
        small_note(ui, "Summoner's Rift games only; the position is Riot's guess from where you played (match timeline) and is often wrong for roaming or lane swaps.");
    }
}

/// Recent TFT form: average placement, top 4s, firsts and a placement strip.
pub(super) fn tft_summary(ui: &mut Ui, games: &[TftGame]) {
    if games.is_empty() {
        return;
    }
    let avg = games.iter().map(|g| g.placement as f64).sum::<f64>() / games.len() as f64;
    let top4 = games.iter().filter(|g| (1..=4).contains(&g.placement)).count();
    let firsts = games.iter().filter(|g| g.placement == 1).count();
    card_head(ui, &format!("Teamfight Tactics · last {} games", games.len()), |_| {});
    tiles(ui, |ui| {
        stat_tile(ui, "Avg placement", format!("{avg:.2}"), if avg <= 4.0 { WIN } else { LOSS }, None);
        stat_tile(ui, "Top 4", top4.to_string(), TEXT, Some(&format!("{:.0}%", top4 as f64 * 100.0 / games.len() as f64)));
        stat_tile(ui, "1st", firsts.to_string(), if firsts > 0 { SOON } else { TEXT }, None);
    });
    ui.add_space(4.0);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        for game in games.iter().rev() {
            let c = placement_color(game.placement);
            let (r, _) = ui.allocate_exact_size(vec2(22.0, 22.0), eframe::egui::Sense::hover());
            ui.painter().rect_filled(r, 5, c.gamma_multiply(0.2));
            ui.painter().text(r.center(), eframe::egui::Align2::CENTER_CENTER, game.placement.to_string(), display(14.0), c);
        }
    });
}
