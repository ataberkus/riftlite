//! Home and Profile tabs, plus the ranked / performance / mastery blocks the
//! Players tab reuses.

use eframe::egui::{Align, Color32, Label, Layout, Margin, RichText, Sense, Stroke, Ui, vec2};

use super::stats::{Avg, averages, cs_per_min, date, datetime, next_division, no_data_hint, now_ms, signed, streak, title};
use super::theme::*;
use super::widgets::*;
use super::App;
use crate::goals::GoalKind;
use crate::lcu::{self, Challenge, LolGame, Mastery, RankedQueue, TftGame};
use crate::lp;
use crate::meta::thousands;
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
        ranked_details(ui, s);
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
                mastery_list(ui, &g, &s.mastery, true);
                closest_milestones_block(ui, &g, &s.mastery);
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
                closest_challenges_block(ui, s);
            });
        });
        ui.columns(2, |cols| {
            card(&mut cols[0], |ui| goal_streaks(ui, s));
            card(&mut cols[1], |ui| toughest_matchups(ui, &g, s));
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

/// Queues whose LP riftlite logs after each game (League's end-of-game LP notification).
fn logs_lp(q: &RankedQueue) -> bool {
    matches!(q.queue.as_str(), "RANKED_SOLO_5x5" | "RANKED_FLEX_SR")
}

/// Anything the client reports beyond the rank itself: placements, series, decay, demotion,
/// season high or last season.
fn has_status(q: &RankedQueue) -> bool {
    q.provisional_left > 0
        || !q.mini_series.is_empty()
        || q.decay_days.is_some()
        || q.demotion_warning
        || !q.highest_tier.is_empty()
        || !q.previous_tier.is_empty()
}

/// A banner while ranked is restricted, then per queue: placements, series, decay, demotion and
/// season ranks from the client, and the LP chart from the LP log.
fn ranked_details(ui: &mut Ui, s: &Snapshot) {
    if s.ranked_restriction > 0 {
        restriction_banner(ui, s.ranked_restriction);
    }
    let rows: Vec<(&RankedQueue, Vec<lp::ChartPoint>)> = s
        .ranked
        .iter()
        .filter(|q| q.worth_showing())
        .map(|q| (q, lp::chart(&s.lp_log, &q.queue)))
        .filter(|(q, points)| has_status(q) || points.len() >= 2 || (logs_lp(q) && (q.is_ranked() || q.provisional_left > 0)))
        .collect();
    if rows.is_empty() {
        return;
    }
    card(ui, |ui| {
        card_head(ui, "Ranked details", |_| {});
        for (i, (q, points)) in rows.iter().enumerate() {
            if i > 0 {
                ui.add_space(4.0);
            }
            queue_status(ui, q);
            if points.len() >= 2 {
                lp_chart(ui, &s.lp_log, q, points);
            } else if logs_lp(q) {
                small_note(ui, "The LP chart fills in after ranked games riftlite sees end.");
            }
        }
        ui.add_space(2.0);
        small_note(ui, "Placements, series, decay and season ranks from the client. LP from the games riftlite saw end; dashed segments mark games or LP changes it missed.");
    });
}

fn restriction_banner(ui: &mut Ui, games: i64) {
    let frame = card_frame().fill(mix(SURFACE, SOON, 0.06)).stroke(Stroke::new(1.0, mix(BORDER, SOON, 0.35))).inner_margin(Margin::symmetric(14, 10));
    framed(ui, frame, |ui| {
        ui.horizontal_wrapped(|ui| {
            icon(ui, Icon::Lock, 16.0, SOON);
            ui.label(RichText::new(format!("Ranked restricted: {games} game{} left", plural(games))).font(semibold(13.5)).color(TEXT));
            small_note(ui, "Ranked queues open again once they are played in other queues.");
        });
    });
}

/// One queue's name with its placement, series, decay, demotion and season-rank chips.
fn queue_status(ui: &mut Ui, q: &RankedQueue) {
    ui.horizontal_wrapped(|ui| {
        ui.add_sized([120.0, 20.0], Label::new(RichText::new(q.label()).font(semibold(12.5)).color(TEXT)));
        if q.provisional_left > 0 {
            pill(ui, format!("{} placement{} left", q.provisional_left, plural(q.provisional_left)), ACCENT_TEXT);
        }
        if !q.mini_series.is_empty() {
            ui.label(RichText::new("Series").size(11.5).color(MUTED));
            series_dots(ui, &q.mini_series);
        }
        if let Some(days) = q.decay_days {
            let text = match days {
                ..=0 => "Decay due".to_string(),
                n => format!("Decay in {n} day{}", plural(n)),
            };
            pill(ui, text, SOON).on_hover_text("From the client's decay warning");
        }
        if q.demotion_warning {
            pill(ui, "Demotion risk", LOSS).on_hover_text("From the client's demotion warning");
        }
        for (caption, tier, division) in [("Season high", &q.highest_tier, &q.highest_division), ("Last season", &q.previous_tier, &q.previous_division)] {
            if let Some(rank) = rank_name(tier, division) {
                ui.label(RichText::new(caption).size(11.5).color(MUTED));
                ui.label(RichText::new(rank).font(semibold(12.0)).color(tier_color(tier)));
            }
        }
    });
}

/// Promotion series results as dots: wins, losses, games still to play.
fn series_dots(ui: &mut Ui, series: &str) {
    let wins = series.chars().filter(|&c| c == 'W').count();
    let losses = series.chars().filter(|&c| c == 'L').count();
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        for c in series.chars() {
            let (r, _) = ui.allocate_exact_size(vec2(9.0, 9.0), Sense::hover());
            let color = match c {
                'W' => WIN,
                'L' => LOSS,
                _ => FAINT,
            };
            ui.painter().circle_filled(r.center(), 4.0, color);
        }
    })
    .response
    .on_hover_text(format!("Series: {wins}W {losses}L of {}", series.chars().count()));
}

/// `Gold II`, `Master`; `None` without a tier.
fn rank_name(tier: &str, division: &str) -> Option<String> {
    if matches!(tier, "" | "NONE" | "UNRANKED") {
        return None;
    }
    Some(if division.is_empty() || matches!(tier, "MASTER" | "GRANDMASTER" | "CHALLENGER") { title(tier) } else { format!("{} {division}", title(tier)) })
}

/// LP over one queue's logged games, oldest first, one step per game; hover shows the game.
fn lp_chart(ui: &mut Ui, log: &[lp::LpEntry], q: &RankedQueue, points: &[lp::ChartPoint]) {
    // The entries `lp::chart` kept, in its order: this queue, a known rank.
    let entries: Vec<&lp::LpEntry> = log.iter().filter(|e| e.queue == q.queue && lp::ladder(&e.tier, &e.division, e.lp).is_some()).collect();
    let line: Vec<LinePoint> = points.iter().enumerate().map(|(i, p)| LinePoint { x: i as f64, y: p.value as f64, gap_before: p.gap_before }).collect();
    let label = |i: usize| {
        let p = &points[i];
        let rank = entries.get(i).and_then(|e| rank_name(&e.tier, &e.division).map(|r| format!("{r} · {} LP\n", e.lp))).unwrap_or_default();
        format!("{rank}{} LP · {} · {}", signed(p.delta), if p.win { "Win" } else { "Loss" }, datetime(p.at_ms))
    };
    let tier = tier_color(&q.tier);
    ui.add_space(2.0);
    overline(ui, &format!("LP · {} logged games", points.len()));
    line_chart(ui, &line, vec2(ui.available_width(), 84.0), if tier == FAINT { ACCENT } else { tier }, None, &label);
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

/// Mastery rows: portrait, name, level, points, last played. `milestones` adds a line on the next
/// level and season milestone; only for your own champions, never other players' grades.
pub(super) fn mastery_list(ui: &mut Ui, g: &Gfx, mastery: &[Mastery], milestones: bool) {
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
        if let Some(line) = milestones.then(|| milestone_text(m)).flatten() {
            ui.horizontal(|ui| {
                // Lines up with the name, past the portrait.
                ui.add_space(38.0);
                ui.add(Label::new(RichText::new(line).size(11.0).color(BODY)).wrap());
            });
            ui.add_space(2.0);
        }
    }
}

/// Progress on one of your champions: marks toward the next level, points to it, and the grades
/// the next season milestone still needs. `None` when the client sent none of it.
pub(super) fn milestone_text(m: &Mastery) -> Option<String> {
    let mut parts = Vec::new();
    if m.marks_required > 0 {
        parts.push(format!("{}/{} marks", m.marks.clamp(0, m.marks_required), m.marks_required));
    }
    if m.points_to_next > 0 {
        parts.push(format!("{} pts to M{}", thousands(m.points_to_next as u64), m.level + 1));
    }
    let needed = m.grades_needed();
    if !needed.is_empty() {
        parts.push(format!("next milestone needs {}", grades_text(&needed)));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// `S- ×1, A ×2`
fn grades_text(needed: &[(String, i64)]) -> String {
    needed.iter().map(|(g, n)| format!("{g} ×{n}")).collect::<Vec<_>>().join(", ")
}

/// Your champions closest to their next season milestone: fewest grades still missing, then most
/// points. Champions without requirements, or with none missing, are left out.
fn closest_milestones(mastery: &[Mastery], n: usize) -> Vec<(&Mastery, Vec<(String, i64)>)> {
    let mut v: Vec<_> = mastery
        .iter()
        .filter(|m| !m.required_grades.is_empty())
        .map(|m| (m, m.grades_needed()))
        .filter(|(_, need)| !need.is_empty())
        .collect();
    v.sort_by_key(|(m, need)| (need.iter().map(|(_, c)| c).sum::<i64>(), std::cmp::Reverse(m.points), m.champion_id));
    v.truncate(n);
    v
}

/// The "Closest milestones" list under the Profile's mastery rows; nothing without any.
fn closest_milestones_block(ui: &mut Ui, g: &Gfx, mastery: &[Mastery]) {
    let closest = closest_milestones(mastery, 3);
    if closest.is_empty() {
        return;
    }
    ui.add_space(4.0);
    ui.separator();
    overline(ui, "Closest milestones");
    for (m, need) in closest {
        ui.horizontal_wrapped(|ui| {
            g.champ(ui, m.champion_id, 22.0);
            ui.label(RichText::new(g.data.champion_name(m.champion_id)).font(semibold(12.5)).color(TEXT));
            ui.label(RichText::new(format!("needs {}", grades_text(&need))).size(11.5).color(BODY));
            if m.milestone_reward_marks > 0 {
                ui.label(RichText::new(format!("· reward {} mark{}", m.milestone_reward_marks, plural(m.milestone_reward_marks))).size(11.5).color(MUTED));
            }
        });
    }
    small_note(ui, "Grades from your own games this season.");
}

/// Your open challenges closest to their next level; nothing when the list is missing or empty.
fn closest_challenges_block(ui: &mut Ui, s: &Snapshot) {
    let Some(Ok(list)) = &s.challenge_list else { return };
    let closest = lcu::closest_challenges(list, 5);
    if closest.is_empty() {
        return;
    }
    ui.add_space(4.0);
    ui.separator();
    overline(ui, "Closest to the next level");
    for c in closest {
        challenge_row(ui, c);
    }
}

/// Name, level → next level, description and progress of one challenge.
pub(super) fn challenge_row(ui: &mut Ui, c: &Challenge) {
    let next = tier_color(&c.next_level);
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(&c.name).font(semibold(12.5)).color(TEXT));
        ui.label(RichText::new(format!("{} → {}", title(&c.level), title(&c.next_level))).font(semibold(11.5)).color(if next == FAINT { MUTED } else { next }));
    });
    if !c.description.is_empty() {
        small_note(ui, &c.description);
    }
    ui.horizontal(|ui| {
        let w = (ui.available_width() - 120.0).max(60.0);
        progress(ui, c.progress() as f32, w, if next == FAINT { ACCENT } else { next });
        ui.label(RichText::new(format!("{} / {}", challenge_value(c.value), challenge_value(c.next_threshold))).size(11.5).color(MUTED));
    });
    ui.add_space(3.0);
}

/// Challenge values: whole numbers with thousands separators, else one decimal.
fn challenge_value(v: f64) -> String {
    if v >= 0.0 && v.fract() == 0.0 { thousands(v as u64) } else { format!("{v:.1}") }
}

/// Games in a row with every goal met, and per goal kind set in any role.
fn goal_streaks(ui: &mut Ui, s: &Snapshot) {
    card_head(ui, "Goal streaks", |_| {});
    let set: Vec<_> = s.goals.roles.values().filter(|g| !g.is_empty()).collect();
    if set.is_empty() {
        return void(note(ui, "No goals set yet. Add per-role goals in Settings → Goals; riftlite checks them after each game it sees end."));
    }
    let kinds = [
        (GoalKind::CsPerMin, set.iter().any(|g| g.cs_per_min.is_some())),
        (GoalKind::MaxDeaths, set.iter().any(|g| g.max_deaths.is_some())),
        (GoalKind::VisionPerMin, set.iter().any(|g| g.vision_per_min.is_some())),
        (GoalKind::ControlWards, set.iter().any(|g| g.control_wards.is_some())),
    ];
    let row = |ui: &mut Ui, label: &str, n: usize| {
        ui.horizontal(|ui| {
            ui.label(RichText::new(label).font(semibold(12.5)).color(TEXT));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(format!("{n} in a row")).font(semibold(12.5)).color(if n > 0 { WIN } else { MUTED }));
            });
        });
    };
    row(ui, "All goals met", s.goals.streak(None));
    for (kind, _) in kinds.into_iter().filter(|(_, on)| *on) {
        row(ui, kind.label(), s.goals.kind_streak(kind));
    }
    small_note(
        ui,
        if s.goals.results.is_empty() {
            "No game checked yet; streaks start with your next game."
        } else {
            "Your newest games in a row that met the goal; games without that goal are skipped."
        },
    );
}

/// The enemy champions you have lost to more often than beaten (3+ games), as your own record.
fn toughest_matchups(ui: &mut Ui, g: &Gfx, s: &Snapshot) {
    card_head(ui, "Toughest matchups", |_| {});
    if s.matchups.len() == 0 {
        return void(note(ui, "Fills in as riftlite reads your match details."));
    }
    let worst = s.matchups.ban_hints(3, 5, &[]);
    if worst.is_empty() {
        return void(note(ui, "No enemy champion you have lost to more often than beaten in 3+ games."));
    }
    for (id, r) in worst {
        let few = r.few_games();
        ui.horizontal(|ui| {
            g.champ(ui, id, 26.0);
            ui.label(RichText::new(g.data.champion_name(id)).font(semibold(12.5)).color(if few { BODY } else { TEXT }));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(format!("{} games", r.games)).size(11.5).color(MUTED));
                ui.label(RichText::new(format!("{}–{}", r.wins, r.losses())).font(semibold(12.5)).color(if few { MUTED } else { LOSS }));
                if few {
                    pill(ui, "few games", MUTED);
                }
            });
        });
    }
    small_note(ui, "Your wins–losses against them, any lane and any of your champions, from the matches riftlite read.");
}

fn plural(n: i64) -> &'static str {
    if n == 1 { "" } else { "s" }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn champ(id: i64, points: i64, required: &[(&str, i64)], earned: &[&str]) -> Mastery {
        Mastery {
            champion_id: id,
            points,
            required_grades: required.iter().map(|(g, n)| (g.to_string(), *n)).collect(),
            grades: earned.iter().map(|g| g.to_string()).collect(),
            ..Mastery::default()
        }
    }

    #[test]
    fn closest_milestones_put_fewest_missing_grades_first() {
        let list = [
            champ(1, 900, &[("A", 3)], &[]),
            champ(2, 100, &[("A", 3)], &["A", "S"]),
            // B is below A, so two are still missing.
            champ(3, 500, &[("A", 3)], &["A+", "B"]),
            // Requirements met: nothing to show.
            champ(4, 800, &[("A", 2)], &["A", "S"]),
            champ(5, 999, &[], &[]),
            // One missing like champion 2, with more points.
            champ(6, 700, &[("S-", 1)], &[]),
        ];
        let ids: Vec<i64> = closest_milestones(&list, 3).iter().map(|(m, _)| m.champion_id).collect();
        assert_eq!(ids, [6, 2, 3]);
    }
}
