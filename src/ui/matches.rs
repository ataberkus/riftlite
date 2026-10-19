//! League and TFT match history, the shared game cards and the ten-player scoreboard.

use eframe::egui::{Align, Align2, Color32, Frame, Grid, Label, Layout, Margin, Rect, RichText, Sense, Spinner, Stroke, Ui, vec2};

use super::home::{lol_summary, void};
use super::stats::{Avg, ago, averages, cs_per_min, datetime, mmss, no_data_hint, queue_name, signed};
use super::theme::*;
use super::widgets::*;
use super::{App, Tab};
use crate::ddragon::{StaticData, tft_label};
use crate::itemset;
use crate::lcu::{LANE_QUEUES, LolGame, MatchDetail};
use crate::timeline::{Death, sr_minimap};
use crate::worker::{Action, Clip, Snapshot};

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
        let g = Gfx::new(&icons, &s.data);
        card(ui, |ui| lol_summary(ui, &g, games));
        self.champion_table(ui, &g, games);
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

    /// Your averages per champion over the loaded history, sortable by any column.
    fn champion_table(&mut self, ui: &mut Ui, g: &Gfx, games: &[LolGame]) {
        const HEADS: [&str; 8] = ["Champion", "Games", "Win rate", "KDA", "CS / min", "Gold / min", "Damage / min", "Vision / min"];
        const SHOWN: usize = 8;
        let mut champs: Vec<i64> = games.iter().map(|x| x.champion_id).collect();
        champs.sort_unstable();
        champs.dedup();
        let mut rows: Vec<(i64, String, Avg)> = champs
            .into_iter()
            .filter_map(|c| Some((c, g.data.champion_name(c), averages(games.iter().filter(|x| x.champion_id == c))?)))
            .collect();
        let (col, desc) = self.champ_sort;
        let key = |a: &Avg| match col {
            1 => a.n as f64,
            2 => a.win_rate(),
            3 => a.kda(),
            4 => a.cs_min,
            5 => a.gold_min,
            6 => a.dmg_min,
            _ => a.vision_min,
        };
        rows.sort_by(|x, y| {
            let o = if col == 0 { x.1.cmp(&y.1) } else { key(&x.2).total_cmp(&key(&y.2)) };
            if desc { o.reverse() } else { o }
        });
        section_with(ui, "By champion", |ui| {
            if rows.len() > SHOWN {
                let label = if self.champ_table_all { "Show fewer".to_string() } else { format!("Show all {}", rows.len()) };
                if ui.add(Btn::ghost(&label).small()).clicked() {
                    self.champ_table_all ^= true;
                }
            }
        });
        card(ui, |ui| {
            Grid::new("champ-table").striped(true).spacing([18.0, 4.0]).min_row_height(28.0).show(ui, |ui| {
                for (i, h) in HEADS.iter().enumerate() {
                    let sorted = i == col;
                    let head = ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 3.0;
                        ui.label(RichText::new(h.to_uppercase()).font(semibold(10.5)).color(if sorted { TEXT } else { MUTED }));
                        if sorted {
                            icon(ui, if desc { Icon::ChevronDown } else { Icon::ChevronUp }, 10.0, TEXT);
                        }
                    });
                    let r = head.response.interact(Sense::click()).on_hover_cursor(eframe::egui::CursorIcon::PointingHand);
                    if r.on_hover_text("Sort by this column").clicked() {
                        // Numbers start highest first, names A to Z.
                        self.champ_sort = if sorted { (col, !desc) } else { (i, i != 0) };
                    }
                }
                ui.end_row();
                for (c, name, a) in rows.iter().take(if self.champ_table_all { usize::MAX } else { SHOWN }) {
                    ui.horizontal(|ui| {
                        g.champ(ui, *c, 24.0);
                        ui.label(RichText::new(name).font(semibold(12.5)).color(TEXT));
                    });
                    ui.label(RichText::new(a.n.to_string()).color(BODY));
                    ui.label(RichText::new(format!("{:.0}%", a.win_rate() * 100.0)).font(semibold(12.5)).color(wr_color(a.win_rate())));
                    ui.label(RichText::new(format!("{:.2}", a.kda())).color(kda_color(a.kda())));
                    for v in [a.cs_min, a.gold_min, a.dmg_min, a.vision_min] {
                        ui.label(RichText::new(if v >= 100.0 { format!("{v:.0}") } else { format!("{v:.1}") }).color(BODY));
                    }
                    ui.end_row();
                }
            });
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
                            let d = d.clone().with_me(viewer);
                            if let Some(id) = match_table(ui, &g, &d, &format!("{salt}-{}", game.game_id)) {
                                look = Some(id);
                            }
                            // Timelines and replays only for your own games, not a looked-up player's.
                            if s.summoner.as_ref().is_some_and(|x| !viewer.is_empty() && x.puuid == viewer) {
                                self.game_tools(ui, s, &g, &d);
                            }
                        }
                    }
                }
            });
        }
        look
    }

    /// The Timeline toggle and replay controls under one of your expanded games, then the
    /// timeline itself while it is open.
    fn game_tools(&mut self, ui: &mut Ui, s: &Snapshot, g: &Gfx, d: &MatchDetail) {
        let id = d.game_id;
        let open = self.timeline_open.contains(&id);
        ui.add_space(6.0);
        ui.horizontal_wrapped(|ui| {
            let toggle = Btn::ghost(if open { "Hide timeline" } else { "Timeline" }).icon(Icon::Chart).small().selected(open);
            if ui.add(toggle).on_hover_text("Skill order, gold lead, lane difference and where you died").clicked() {
                if open {
                    self.timeline_open.remove(&id);
                } else {
                    self.timeline_open.insert(id);
                    // Also retries a timeline that failed to load.
                    if !matches!(s.timelines.get(&id), Some(Ok(_))) {
                        self.request(Action::MatchTimeline(id));
                    }
                }
            }
            self.replay_controls(ui, s, id);
        });
        if self.timeline_open.contains(&id) {
            ui.add_space(6.0);
            self.timeline_view(ui, s, g, d);
        }
    }

    /// The client's replay state of one game and the action it allows.
    fn replay_controls(&self, ui: &mut Ui, s: &Snapshot, id: u64) {
        // States that may change on their own get a way to ask again.
        let recheck = match s.replays.get(&id) {
            None => {
                let ask = Btn::ghost("Replay").icon(Icon::Play).small();
                if ui.add(ask).on_hover_text("Ask the League client whether this game's replay can be downloaded or watched").clicked() {
                    self.request(Action::ReplayState(id));
                }
                false
            }
            Some(Err(e)) => {
                error(ui, e);
                true
            }
            Some(Ok(r)) => match r.state.as_str() {
                "download" | "retryDownload" => {
                    if ui.add(Btn::secondary("Download replay").icon(Icon::ArrowDown).small()).clicked() {
                        self.request(Action::ReplayDownload(id));
                    }
                    false
                }
                "checking" | "found" => {
                    ui.add(Spinner::new().size(12.0).color(ACCENT));
                    small_note(ui, "Checking…");
                    true
                }
                "downloading" => {
                    let pct = r.progress.clamp(0, 100);
                    progress(ui, pct as f32 / 100.0, 120.0, ACCENT);
                    small_note(ui, format!("Downloading {pct}%"));
                    false
                }
                "watch" => {
                    if ui.add(Btn::primary("Watch replay").icon(Icon::Play).small()).on_hover_text("Opens the replay in the game").clicked() {
                        self.request(Action::ReplayWatch(id));
                    }
                    false
                }
                "incompatible" => {
                    small_note(ui, "Recorded on another patch; the client can only play the current patch's replays");
                    false
                }
                "missingOrExpired" | "lost" => {
                    small_note(ui, "No longer available");
                    false
                }
                "unsupported" => {
                    small_note(ui, "This game has no replay");
                    false
                }
                "" => {
                    small_note(ui, "Replay state unknown");
                    true
                }
                other => {
                    small_note(ui, format!("Replay: {other}"));
                    true
                }
            },
        };
        if recheck && ui.add(Btn::ghost("Check again").icon(Icon::Refresh).small()).clicked() {
            self.request(Action::ReplayState(id));
        }
    }

    /// Skill order, team gold lead, lane difference at 10 and 15 minutes and your deaths, from
    /// the game's timeline; deaths can be jumped to and clipped while this game's replay is open.
    fn timeline_view(&self, ui: &mut Ui, s: &Snapshot, g: &Gfx, d: &MatchDetail) {
        let t = match s.timelines.get(&d.game_id) {
            None => {
                ui.horizontal(|ui| {
                    ui.add(Spinner::new().size(14.0).color(ACCENT));
                    note(ui, "Loading timeline…");
                });
                return;
            }
            Some(Err(e)) => return void(error(ui, e)),
            Some(Ok(t)) => t,
        };
        let Some(me) = d.me().filter(|m| m.participant_id > 0) else {
            return void(small_note(ui, "This game's details do not say which player you were, so its timeline cannot be read."));
        };
        let pid = me.participant_id;
        let champ = |id: i64| d.players.iter().find(|p| id > 0 && p.participant_id == id).map(|p| p.champion_id);
        inset_frame().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 4.0;
            let skills = t.skill_order(pid);
            if !skills.is_empty() {
                overline(ui, "Skill order");
                skill_row(ui, &skills);
                ui.add_space(8.0);
            }

            let team: Vec<i64> = d.players.iter().filter(|p| p.team == me.team && p.participant_id > 0).map(|p| p.participant_id).collect();
            let lead: Vec<LinePoint> = t.gold_lead(&team).into_iter().map(|(m, v)| LinePoint { x: m as f64, y: v as f64, gap_before: false }).collect();
            if lead.len() >= 2 {
                overline(ui, "Team gold lead");
                let size = vec2(ui.available_width().min(640.0), 90.0);
                line_chart(ui, &lead, size, WIN, Some(Baseline { value: 0.0, below: Some(LOSS) }), &|i| {
                    format!("{:.0} min · {}", lead[i].x, gold_k(lead[i].y))
                });
                let (lo, hi) = lead.iter().fold((0.0f64, 0.0f64), |(lo, hi), p| (lo.min(p.y), hi.max(p.y)));
                let mut caption = vec![format!("{} at the end", gold_k(lead.last().map_or(0.0, |p| p.y)))];
                if hi > 0.0 {
                    caption.push(format!("biggest lead {}", gold_k(hi)));
                }
                if lo < 0.0 {
                    caption.push(format!("biggest deficit {}", gold_k(lo)));
                }
                small_note(ui, caption.join(" · "));
                ui.add_space(8.0);
            }

            let opponent = d.players.iter().find(|p| !me.position.is_empty() && p.team != me.team && p.position == me.position && p.participant_id > 0);
            if let Some(opp) = opponent {
                let diffs: Vec<_> = [10, 15].into_iter().filter_map(|m| t.lane_diff(pid, opp.participant_id, m)).collect();
                if !diffs.is_empty() {
                    overline(ui, "Lane difference");
                    ui.horizontal_wrapped(|ui| {
                        small_note(ui, "vs");
                        g.champ(ui, opp.champion_id, 20.0);
                        for x in diffs {
                            let text = format!("@{} {} gold · {} CS · {} XP", x.minute, signed(x.gold), signed(x.cs), signed(x.xp));
                            ui.add_space(8.0);
                            ui.label(RichText::new(text).font(semibold(12.5)).color(if x.gold >= 0 { WIN } else { LOSS }));
                        }
                    });
                    ui.add_space(8.0);
                }
            }

            let deaths = t.deaths(pid);
            // Only an open replay as long as this game is this game's replay.
            let playback = s.replay_playback.filter(|p| (p.length - d.duration_s as f64).abs() <= 15.0);
            let recording = matches!(s.clip, Some(Clip::Recording { .. }));
            let died = |x: &Death| {
                let who = champ(x.killer).map_or_else(|| "a turret, minion or monster".to_string(), |c| s.data.champion_name(c));
                let help = if x.assists.is_empty() { String::new() } else { format!(" (+{})", x.assists.len()) };
                format!("Died {} to {who}{help}", mmss(x.time_ms as f64 / 1000.0))
            };
            overline(ui, &format!("Deaths ({})", deaths.len()));
            ui.horizontal_top(|ui| {
                let on_rift = d.map_id == 11 || (d.map_id == 0 && LANE_QUEUES.contains(&d.queue_id));
                if on_rift {
                    // Kills without a position would all pile up in a corner.
                    let placed = |v: &[Death]| v.iter().filter(|x| x.x != 0 || x.y != 0).cloned().collect::<Vec<_>>();
                    let shown = placed(&deaths);
                    let marks: Vec<(f32, f32)> = shown.iter().map(|x| sr_minimap(x.x, x.y)).collect();
                    let kills: Vec<(f32, f32)> = placed(&t.takedowns(pid)).iter().map(|x| sr_minimap(x.x, x.y)).collect();
                    death_map(ui, g.tex(s.data.map_image(11), 512), 220.0, &marks, &kills, &|i| died(&shown[i]));
                    ui.add_space(12.0);
                }
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 4.0;
                    if deaths.is_empty() {
                        small_note(ui, "No deaths this game.");
                    }
                    for x in &deaths {
                        let at = x.time_ms as f64 / 1000.0;
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(mmss(at)).font(semibold(12.5)).color(TEXT)).on_hover_text(died(x));
                            match champ(x.killer) {
                                Some(c) => void(g.champ(ui, c, 20.0)),
                                None => void(small_note(ui, "Executed")),
                            }
                            if !x.assists.is_empty() {
                                small_note(ui, format!("+{}", x.assists.len())).on_hover_text("Enemies who assisted");
                            }
                            if playback.is_some() {
                                if ui.add(Btn::ghost("Jump").icon(Icon::Play).small()).on_hover_text("Moves the replay to 10 seconds before this death").clicked() {
                                    self.request(Action::ReplaySeek((at - 10.0).max(0.0)));
                                }
                                let (start, end) = ((at - self.clip_span.0).max(0.0), at + self.clip_span.1);
                                let clip = ui
                                    .add_enabled(!recording, Btn::ghost("Clip").icon(Icon::Save).small())
                                    .on_hover_text(format!("Records {}–{} of the replay into your clips folder", mmss(start), mmss(end)));
                                if clip.clicked() {
                                    self.request(Action::ReplayClip { start, end });
                                }
                            }
                        });
                    }
                    if playback.is_none() && !deaths.is_empty() {
                        ui.add_space(2.0);
                        small_note(
                            ui,
                            if s.replay_api != Some(true) {
                                "Turn on the Replay API in Settings to jump to deaths and record clips"
                            } else {
                                "Watch this game's replay to jump to deaths and clip them"
                            },
                        );
                    }
                    if playback.is_some() {
                        match &s.clip {
                            Some(Clip::Recording { start, end, current }) => void(ui.horizontal(|ui| {
                                ui.add(Spinner::new().size(12.0).color(LOSS));
                                small_note(ui, format!("Recording {}–{} (at {})", mmss(*start), mmss(*end), mmss(*current)));
                            })),
                            Some(Clip::Saved(path)) => void(status_msg(ui, true, format!("Clip saved to {path}"))),
                            Some(Clip::Failed(msg)) => void(error(ui, format!("Clip failed: {msg}"))),
                            None => {}
                        }
                    }
                });
            });
        });
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

/// Skill points in level order as small key chips, ultimates highlighted; hover shows the level.
fn skill_row(ui: &mut Ui, order: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(3.0, 3.0);
        for (i, key) in order.chars().enumerate() {
            let ult = key == 'R';
            let (r, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::hover());
            let (fill, stroke, text) = if ult { (alpha(ACCENT, 0.25), alpha(ACCENT, 0.7), ACCENT_TEXT) } else { (HOVER, BORDER_STRONG, BODY) };
            ui.painter().rect(r, 5, fill, Stroke::new(1.0, stroke), eframe::egui::StrokeKind::Inside);
            ui.painter().text(r.center(), Align2::CENTER_CENTER, key, semibold(11.5), text);
            resp.on_hover_text(format!("Level {}", i + 1));
        }
    });
}

/// `+1.2k` / `-0.4k` gold.
fn gold_k(v: f64) -> String {
    format!("{}{:.1}k", if v < 0.0 { "-" } else { "+" }, v.abs() / 1000.0)
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
