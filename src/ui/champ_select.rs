//! Champ select: teams, bans, ARAM cards and bench, OP.GG builds, Riot's
//! recommended pages, automation log, your record on the pick and suggestions, your games vs
//! the draft, pick ideas from your pool, and your pick's mastery, notes and challenges.

use eframe::egui::{Align, Align2, Color32, Label, Layout, RichText, Sense, Stroke, StrokeKind, Ui, UiBuilder, Vec2, vec2};

use super::home::void;
use super::live_tab::mastery_tag;
use super::matches::trunc;
use super::stats::{Avg, Comp, averages, mmss, now_ms, position, position_label, queue_name, scout, title};
use super::theme::*;
use super::widgets::*;
use super::App;
use crate::ddragon::StaticData;
use crate::lcu::{self, ChampSelect, Lobby, LolGame, RecPage};
use crate::matchups::Record;
use crate::meta::{self, Pick};
use crate::notes;
use crate::pool::{self, Reason};
use crate::worker::{Action, Snapshot};
use crate::itemset;

/// The champ select step in words; the LCU's `timer.phase` is an enum like `BAN_PICK`.
pub(super) fn cs_phase_label(cs: &ChampSelect) -> &str {
    match cs.phase.as_str() {
        _ if cs.picking_card() => "Pick a champion",
        "" => "…",
        "PLANNING" => "Declare intent",
        "BAN_PICK" => "Picks and bans",
        "FINALIZATION" => "Finalization",
        "GAME_STARTING" => "Game starting",
        other => other,
    }
}

/// Color of a Q/W/E/R key.
fn skill_color(key: &str) -> Color32 {
    match key {
        "Q" => CYAN,
        "W" => WIN,
        "E" => ACCENT_TEXT,
        "R" => SOON,
        _ => BODY,
    }
}

/// A small square key cap (`Q`, `W`, …).
fn key_cap(ui: &mut Ui, key: &str, size: f32) {
    let c = skill_color(key);
    let (r, _) = ui.allocate_exact_size(vec2(size, size), Sense::hover());
    ui.painter().rect(r, 4, alpha(c, 0.14), Stroke::new(1.0, alpha(c, 0.45)), StrokeKind::Inside);
    ui.painter().text(r.center(), Align2::CENTER_CENTER, key, semibold(size * 0.55), c);
}

/// Muted stats text with a "few games" tag when the margin is wide.
fn option_stats(ui: &mut Ui, stats: &str, few: bool) {
    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
        if few {
            pill(ui, "few games", MUTED);
        }
        ui.label(RichText::new(stats).size(11.5).color(if few { FAINT } else { MUTED }));
    });
}

/// Outer size of a [`pick_tile`]. Fixed, so the wrapping [`tiles`] row knows it before placing
/// the tile and moves it to the next row instead of squeezing it into the space left.
const PICK_TILE: Vec2 = vec2(184.0, 56.0);

/// A champion tile (ARAM cards, bench) with your record on it. Clickable unless `locked` (a bench
/// champion you can't swap to), which is drawn greyed out with a padlock.
fn pick_tile(ui: &mut Ui, g: &Gfx, champ: i64, record: Option<&Avg>, locked: bool) -> eframe::egui::Response {
    // Narrower than a tile only when the whole row is; then the tile takes the row.
    let size = vec2(PICK_TILE.x.min(ui.max_rect().width()), PICK_TILE.y);
    let (rect, resp) = ui.allocate_exact_size(size, if locked { Sense::hover() } else { Sense::click() });
    if !ui.is_rect_visible(rect) {
        return resp;
    }
    let hovered = resp.hovered() && !locked;
    ui.painter().rect(
        rect,
        10,
        if hovered { HOVER } else { RAISED },
        Stroke::new(1.0, if hovered { alpha(ACCENT, 0.7) } else { BORDER_STRONG }),
        StrokeKind::Inside,
    );
    let mut child = ui.new_child(UiBuilder::new().max_rect(rect.shrink2(vec2(10.0, 8.0))).layout(Layout::left_to_right(Align::Center)));
    // Selectable text would take clicks meant for the tile.
    child.style_mut().interaction.selectable_labels = false;
    if locked {
        g.champ_locked(&mut child, champ, 40.0);
    } else {
        g.champ(&mut child, champ, 40.0);
    }
    child.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 1.0;
        ui.add(Label::new(RichText::new(g.data.champion_name(champ)).font(semibold(13.0)).color(if locked { MUTED } else { TEXT })).truncate());
        match record {
            Some(a) => {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    ui.label(RichText::new(format!("{:.0}%", a.win_rate() * 100.0)).font(semibold(12.0)).color(wr_color(a.win_rate())));
                    ui.add(Label::new(RichText::new(format!("{}G · KDA {:.1}", a.n, a.kda())).size(11.5).color(MUTED)).truncate());
                });
            }
            None => {
                ui.add(Label::new(RichText::new("no recent games").size(11.5).color(FAINT)).truncate());
            }
        }
    });
    if locked { resp } else { resp.on_hover_cursor(eframe::egui::CursorIcon::PointingHand) }
}

/// The party before champ select: members with rank and roles, and the queue timer.
fn lobby_card(ui: &mut Ui, s: &Snapshot, g: &Gfx, l: &Lobby) {
    section_with(ui, "Lobby", |ui| {
        if l.queue_id > 0 {
            pill(ui, queue_name(l.queue_id), MUTED);
        }
        if l.max_size > 0 {
            pill(ui, format!("{} / {}", l.members.len(), l.max_size), MUTED);
        }
        if let Some(q) = &s.queue {
            let text = match q.state.as_str() {
                "Searching" if q.estimate_s > 0.0 => format!("In queue {} · est. {}", mmss(q.in_queue_s), mmss(q.estimate_s)),
                "Searching" => format!("In queue {}", mmss(q.in_queue_s)),
                "Found" => "Match found".to_string(),
                other => format!("Queue: {other}"),
            };
            pill(ui, text, if q.state == "Found" { WIN } else { CYAN });
        }
    });
    card(ui, |ui| {
        let mut warned = false;
        if let Some((left, kind)) = &s.queue_penalty {
            warned = true;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                icon(ui, Icon::Clock, 14.0, SOON);
                ui.label(RichText::new(format!("Queue lockout · {} left", mmss(left.ceil()))).font(semibold(12.5)).color(SOON));
                let why = penalty_reason(kind);
                if !why.is_empty() {
                    ui.label(RichText::new(why).size(11.5).color(MUTED));
                }
            });
        }
        if s.ranked_restriction > 0 && matches!(l.queue_id, 420 | 440) {
            warned = true;
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                icon(ui, Icon::Lock, 14.0, SOON);
                ui.label(RichText::new(format!("Ranked restricted: {} left", n_games(s.ranked_restriction as u64))).font(semibold(12.5)).color(SOON));
                ui.label(RichText::new("Finished normal games count down the restriction.").size(11.5).color(MUTED));
            });
        }
        if warned {
            ui.add_space(4.0);
        }
        for m in &l.members {
            ui.horizontal(|ui| {
                let name = s.names.get(&m.puuid).map(String::as_str).filter(|n| !n.is_empty()).unwrap_or("…");
                g.profile(ui, m.icon, name, 34.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.label(RichText::new(if m.is_me { "you" } else { name }).font(semibold(13.0)).color(if m.is_me { CYAN } else { TEXT }));
                        if m.leader {
                            pill(ui, "leader", ACCENT_TEXT);
                        }
                    });
                    let roles: Vec<&str> =
                        m.positions.iter().filter(|p| !matches!(p.as_str(), "" | "UNSELECTED")).map(|p| position_label(p)).collect();
                    let mut line = format!("Level {}", m.level);
                    if !roles.is_empty() {
                        line += &format!(" · {}", roles.join(" / "));
                    }
                    ui.label(RichText::new(line).size(11.5).color(MUTED));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if let Some(r) = scout(s, &m.puuid) {
                        ui.label(RichText::new(&r.text).size(11.5).color(if r.tier.is_empty() { MUTED } else { tier_color(&r.tier) }));
                        rank_emblem(ui, &r.tier, 22.0);
                    }
                });
            });
        }
    });
}

/// The client's queue lockout `errorType` in words; unknown types in plain case.
fn penalty_reason(kind: &str) -> String {
    match kind {
        "QUEUE_DODGER" => "Dodge penalty (left a champ select)".into(),
        "LEAVER_BUSTED" => "Leaver penalty (low-priority queue)".into(),
        other => title(other),
    }
}

/// `1 game`, `3 games`.
fn n_games(n: u64) -> String {
    if n == 1 { "1 game".into() } else { format!("{n} games") }
}

/// `3–2 in 5 games`.
fn record_line(r: Record) -> String {
    format!("{}–{} in {}", r.wins, r.losses(), n_games(r.games))
}

/// While you have a ban to make: the champions your intended pick wins least against in lane
/// (OP.GG matchups), not banned yet and not on your team.
fn ban_hints(ui: &mut Ui, s: &Snapshot, cs: &ChampSelect, g: &Gfx, meta_on: bool) {
    section(ui, "Ban suggestions");
    let Some(mine) = cs.my_champion() else {
        return void(note(ui, "Hover or declare your pick to see the champions it does worst against."));
    };
    let name = s.data.champion_name(mine);
    let build = s.meta.as_ref().filter(|m| m.champion_id == mine).and_then(|m| m.build.as_ref()?.as_ref().ok());
    let Some(build) = build else {
        let why = if meta_on { format!("Loading OP.GG lane matchups for {name}…") } else { "Turn on OP.GG builds in Settings for lane-matchup ban suggestions.".into() };
        return void(note(ui, why));
    };
    let taken = |c: i64| cs.bans.contains(&c) || cs.my_team.iter().any(|p| p.champion_id == c);
    let mut worst: Vec<_> = build.counters.iter().filter(|m| !taken(m.champion_id) && m.games > 0).collect();
    worst.sort_by(|a, b| (a.wins as f64 / a.games as f64).total_cmp(&(b.wins as f64 / b.games as f64)));
    if worst.is_empty() {
        return void(note(ui, format!("OP.GG has no lane matchups for {name} in this queue.")));
    }
    card(ui, |ui| {
        tiles(ui, |ui| {
            for m in worst.iter().take(5) {
                inset_frame().show(ui, |ui| {
                    ui.horizontal(|ui| {
                        g.champ(ui, m.champion_id, 30.0);
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            ui.label(RichText::new(s.data.champion_name(m.champion_id)).font(semibold(12.5)).color(TEXT));
                            ui.label(RichText::new(m.stats()).size(11.5).color(MUTED));
                        });
                    });
                });
            }
        });
        small_note(ui, format!("{name}'s lane win rate against each (OP.GG, {} games or more), lowest first.", meta::MIN_GAMES));
    });
}

/// Your own record against the enemy picks (match details, any queue with lanes), and while you
/// have a ban to make, the enemy champions you have lost to most. Champions only: enemy players
/// stay anonymous.
fn draft_record(ui: &mut Ui, s: &Snapshot, cs: &ChampSelect, g: &Gfx) {
    let mine = cs.my_champion();
    let opp = s.opponent.as_ref().map(|o| o.champion_id);
    let rows: Vec<(i64, Record)> = cs
        .their_team
        .iter()
        .filter(|p| p.picked && p.champion_id > 0)
        .map(|p| (p.champion_id, s.matchups.record_vs(p.champion_id, None, false)))
        .filter(|(_, r)| r.games > 0)
        .collect();
    let bans = if cs.my_ban_pending {
        let mut exclude = cs.bans.clone();
        exclude.extend(cs.my_team.iter().chain(&cs.their_team).map(|p| p.champion_id).filter(|&c| c > 0));
        s.matchups.ban_hints(3, 3, &exclude)
    } else {
        Vec::new()
    };
    let filling = s.matchups.len() == 0;
    if rows.is_empty() && bans.is_empty() && !filling {
        return;
    }
    section(ui, "Your games vs this draft");
    card(ui, |ui| {
        if filling {
            small_note(ui, "Your record against each enemy champion fills in from your match history in the background.");
        }
        for &(enemy, all) in &rows {
            let lane = opp == Some(enemy);
            ui.horizontal(|ui| {
                g.champ(ui, enemy, 28.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        ui.label(RichText::new(format!("vs {}", s.data.champion_name(enemy))).font(semibold(12.5)).color(TEXT));
                        if lane {
                            pill(ui, "probable lane opponent", CYAN);
                        }
                        if all.few_games() {
                            pill(ui, "few games", MUTED);
                        }
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        let color = if all.few_games() { MUTED } else { wr_color(all.win_rate()) };
                        ui.label(RichText::new(record_line(all)).size(11.5).color(color));
                        let mut part = |label: String, r: Record| {
                            if r.games > 0 {
                                ui.label(RichText::new("·").size(11.5).color(FAINT));
                                ui.label(RichText::new(format!("{label} {}", record_line(r))).size(11.5).color(MUTED));
                            }
                        };
                        if let Some(m) = mine {
                            part(format!("on {}:", s.data.champion_name(m)), s.matchups.record_vs(enemy, Some(m), false));
                        }
                        if lane {
                            part("in lane:".into(), s.matchups.record_vs(enemy, None, true));
                        }
                    });
                });
            });
        }
        if !bans.is_empty() {
            if !rows.is_empty() {
                ui.add_space(6.0);
            }
            overline(ui, "Ban ideas from your own games");
            tiles(ui, |ui| {
                for &(c, r) in &bans {
                    inset_frame().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            g.champ(ui, c, 30.0);
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 1.0;
                                ui.label(RichText::new(s.data.champion_name(c)).font(semibold(12.5)).color(TEXT));
                                ui.label(RichText::new(format!("you won {} of {} vs it", r.wins, r.games)).size(11.5).color(MUTED));
                            });
                        });
                    });
                }
            });
            small_note(ui, "Options: enemy champions you have lost to most in your recorded games (any role, 3 games or more), not OP.GG's numbers.");
        }
    });
}

/// Queues whose champion you do not choose freely (ARAM, ARURF) or whose results are
/// placements rather than wins (Arena); left out of your pick pool.
const NOT_IN_POOL: [i64; 6] = [450, 2400, 900, 1010, 1700, 1710];

/// While your pick is open: champions from your own pool, each with the reasons it could fit
/// this draft (`pool::ideas`). Options to weigh, never a single pick. Not in Arena.
fn pick_ideas(ui: &mut Ui, s: &Snapshot, cs: &ChampSelect, g: &Gfx) {
    if matches!(cs.queue_id, 1700 | 1710) || s.session.as_ref().is_some_and(|x| x.map_id == 30) {
        return;
    }
    let history = s.lol_history.as_deref().unwrap_or(&[]);
    let mut champs: Vec<pool::PoolChamp> = Vec::new();
    for game in history.iter().filter(|x| !NOT_IN_POOL.contains(&x.queue_id)) {
        match champs.iter_mut().find(|p| p.champion_id == game.champion_id) {
            Some(p) => {
                p.games += 1;
                p.wins += game.win as u64;
            }
            None => champs.push(pool::PoolChamp { champion_id: game.champion_id, games: 1, wins: game.win as u64 }),
        }
    }
    let team = Comp::of(&s.data, cs.my_team.iter().filter(|p| !p.is_me && p.picked).map(|p| p.champion_id));
    let warnings = team.warnings();
    let needs = pool::Needs {
        magic: warnings.contains(&"All physical damage"),
        physical: warnings.contains(&"All magic damage"),
        tank: warnings.contains(&"No tank"),
    };
    let position = lcu::canonical_position(cs.me().map_or("", |me| me.position.as_str()));
    // Without OP.GG's matchups the opponent still brings your own record against it.
    let opponent = s.opponent.as_ref().map(|o| {
        let counters: &[meta::Matchup] = match &o.build {
            Some(Ok(b)) => &b.counters,
            _ => &[],
        };
        (o.champion_id, counters)
    });
    let record_vs = |mine: i64, enemy: i64| {
        let r = s.matchups.record_vs(enemy, Some(mine), true);
        (r.games, r.wins)
    };
    let input = pool::Input {
        pool: &champs,
        pickable: &s.pickable,
        position,
        positions: &s.positions,
        needs,
        data: &s.data,
        opponent,
        record_vs: &record_vs,
    };
    let taken = |c: i64| cs.bans.contains(&c) || cs.my_team.iter().chain(&cs.their_team).any(|p| !p.is_me && p.picked && p.champion_id == c);
    let ideas: Vec<pool::Idea> = pool::ideas(&input).into_iter().filter(|i| !taken(i.champion_id)).take(5).collect();
    if ideas.is_empty() {
        return;
    }
    section(ui, "Pick ideas");
    card(ui, |ui| {
        overline(ui, "Options from your own pool, with the reasons each could fit");
        if let Some(o) = &s.opponent {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 6.0;
                ui.label(RichText::new("Probable lane opponent").size(11.5).color(MUTED));
                g.champ(ui, o.champion_id, 22.0);
                ui.label(RichText::new(s.data.champion_name(o.champion_id)).font(semibold(12.5)).color(TEXT));
            });
        }
        ui.add_space(4.0);
        for idea in &ideas {
            ui.horizontal(|ui| {
                g.champ(ui, idea.champion_id, 34.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 6.0;
                        ui.label(RichText::new(s.data.champion_name(idea.champion_id)).font(semibold(13.0)).color(TEXT));
                        ui.label(RichText::new(format!("{} · {} won", n_games(idea.games), idea.wins)).size(11.5).color(MUTED));
                    });
                    ui.horizontal_wrapped(|ui| {
                        ui.spacing_mut().item_spacing.x = 4.0;
                        if idea.reasons.is_empty() {
                            ui.label(RichText::new("from your record on it").size(11.5).color(FAINT));
                        }
                        for r in &idea.reasons {
                            let (text, color) = reason_chip(r, position, &s.data);
                            pill(ui, text, color);
                        }
                    });
                });
            });
        }
        ui.add_space(2.0);
        small_note(
            ui,
            "From your recent games (ARAM, ARURF and Arena left out), Riot's position list, your team's damage mix and the probable lane opponent. Ideas to weigh, not instructions.",
        );
    });
}

/// A pick idea's reason as chip text and color.
fn reason_chip(r: &Reason, position: &str, data: &StaticData) -> (String, Color32) {
    match r {
        Reason::Role => (format!("Riot lists it for {}", position_label(position)), CYAN),
        Reason::Magic => ("adds magic damage".into(), ACCENT_TEXT),
        Reason::Physical => ("adds physical damage".into(), ACCENT_TEXT),
        Reason::Tank => ("adds a frontliner".into(), ACCENT_TEXT),
        Reason::Counters { opponent, games, opponent_wins } => {
            let pct = *opponent_wins as f64 * 100.0 / (*games).max(1) as f64;
            (format!("OP.GG: {} wins {pct:.0}% of {games} games vs it", data.champion_name(*opponent)), BODY)
        }
        Reason::Record { opponent, games, wins } => (format!("you're {wins}–{} vs {} on it", games - wins, data.champion_name(*opponent)), BODY),
    }
}

/// Your mastery on the pick: level, points and marks to the next level, grades the next
/// milestone still needs, and the best grade so far.
fn mastery_lines(ui: &mut Ui, m: &lcu::Mastery) {
    overline(ui, "Mastery");
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;
        ui.label(RichText::new(format!("Level {}", m.level)).font(semibold(13.0)).color(if m.level >= 10 { SOON } else { TEXT }));
        if m.points_to_next > 0 {
            ui.label(RichText::new(format!("{} pts to level {}", m.points_to_next, m.level + 1)).size(11.5).color(MUTED));
        }
        if m.marks_required > 0 {
            ui.label(RichText::new(format!("marks {}/{}", m.marks, m.marks_required)).size(11.5).color(MUTED));
        }
        if !m.highest_grade.is_empty() {
            pill(ui, format!("best grade {}", m.highest_grade), ACCENT_TEXT);
        }
    });
    let needed = m.grades_needed();
    if !needed.is_empty() {
        let list: Vec<String> = needed.iter().map(|(grade, n)| format!("{grade} ×{n}")).collect();
        small_note(ui, format!("Next milestone needs: {}", list.join(", ")));
    }
}

/// Open challenges a game on the pick can advance: name, description, value toward the next
/// threshold and the level it reaches.
fn challenge_lines(ui: &mut Ui, list: &[&lcu::Challenge]) {
    let num = |v: f64| if v.fract() == 0.0 { format!("{v:.0}") } else { format!("{v:.1}") };
    overline(ui, "Challenges a game on it can advance");
    for c in list {
        ui.horizontal(|ui| {
            ui.label(RichText::new(&c.name).font(semibold(12.5)).color(TEXT));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                ui.label(RichText::new(format!("{} → {}", title(&c.level), title(&c.next_level))).size(11.5).color(MUTED));
            });
        });
        if !c.description.is_empty() {
            small_note(ui, &c.description);
        }
        ui.horizontal(|ui| {
            let width = (ui.available_width() - 90.0).max(60.0);
            progress(ui, c.progress() as f32, width, ACCENT);
            ui.label(RichText::new(format!("{} / {}", num(c.value), num(c.next_threshold))).size(11.0).color(MUTED));
        });
        ui.add_space(2.0);
    }
}

impl App {
    pub(super) fn champ_select(&mut self, ui: &mut Ui, s: &Snapshot) {
        let icons = self.icons.clone();
        let g = Gfx::new(&icons, &s.data);
        let Some(cs) = &s.champ_select else {
            match &s.lobby {
                Some(l) => lobby_card(ui, s, &g, l),
                None => empty_state(ui, Icon::Target, "Not in champ select", "This tab opens automatically when one starts.", |_| {}),
            }
            if s.phase == "Lobby" && s.session.as_ref().is_some_and(|g| matches!(g.queue_id, 480 | 490)) {
                section(ui, "Swiftplay");
                card(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.allocate_ui_with_layout(vec2((ui.available_width() - 320.0).max(200.0), 0.0), Layout::top_down(Align::Min), |ui| {
                            note(
                                ui,
                                "Swiftplay picks both champions before queueing; riftlite can fill both slots with Riot's recommended runes and spells.",
                            );
                        });
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            if ui.add(Btn::primary("Apply recommended runes to my Swiftplay picks").icon(Icon::Bolt)).clicked() {
                                self.request(Action::SwiftplayRunes);
                            }
                        });
                    });
                });
            }
            return;
        };
        ui.horizontal(|ui| {
            pill(ui, cs_phase_label(cs), ACCENT_TEXT);
            let left = (cs.phase_ends_ms - now_ms()) as f64 / 1000.0;
            if cs.phase_ends_ms > 0 && left > 0.0 {
                pill(ui, format!("{} left", mmss(left.ceil())), if left <= 10.0 { SOON } else { BODY });
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(250));
            }
            if cs.queue_id > 0 {
                pill(ui, queue_name(cs.queue_id), MUTED);
            }
            if cs.locked {
                pill(ui, "Locked in", WIN);
            }
        });
        ui.add_space(6.0);
        ui.columns(2, |cols| {
            for (ui, (title, team, color)) in
                cols.iter_mut().zip([("Your team", &cs.my_team, BLUE_TEAM), ("Enemy team", &cs.their_team, RED_TEAM)])
            {
                card(ui, |ui| {
                    ui.horizontal(|ui| {
                        let (r, _) = ui.allocate_exact_size(vec2(3.0, 14.0), Sense::hover());
                        ui.painter().rect_filled(r, 1, color);
                        overline(ui, title);
                    });
                    ui.add_space(2.0);
                    for p in team {
                        ui.horizontal(|ui| {
                            let (r, _) = ui.allocate_exact_size(vec2(38.0, 38.0), Sense::hover());
                            if p.champion_id > 0 {
                                let tint = if p.picked || p.is_me { Color32::WHITE } else { Color32::from_gray(110) };
                                g.paint_champ(ui.painter(), r, p.champion_id, tint);
                            } else {
                                ui.painter().rect(r, 8, RAISED, Stroke::new(1.0, BORDER), StrokeKind::Inside);
                            }
                            if p.is_me {
                                ui.painter().rect_stroke(r.expand(1.5), 9, Stroke::new(1.5, CYAN), StrokeKind::Outside);
                            }
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 1.0;
                                let champ = RichText::new(s.data.champion_name(p.champion_id)).font(semibold(13.0));
                                ui.label(if p.is_me {
                                    champ.color(CYAN)
                                } else if p.picked {
                                    champ.color(TEXT)
                                } else {
                                    champ.color(MUTED)
                                });
                                ui.horizontal(|ui| {
                                    ui.spacing_mut().item_spacing.x = 4.0;
                                    ui.label(RichText::new(position(&p.position)).size(11.5).color(MUTED));
                                    let who = if p.is_me { "you" } else { p.name.as_str() };
                                    if !who.is_empty() {
                                        ui.label(RichText::new("·").size(11.5).color(FAINT));
                                        trunc(ui, RichText::new(who).size(11.5).color(BODY), 120.0);
                                    }
                                    if p.champion_id > 0 && !p.puuid.is_empty() {
                                        mastery_tag(ui, s, &p.puuid, p.champion_id);
                                    }
                                });
                            });
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                if let Some(r) = scout(s, &p.puuid) {
                                    ui.label(RichText::new(&r.text).size(11.5).color(if r.tier.is_empty() { MUTED } else { tier_color(&r.tier) }));
                                    rank_emblem(ui, &r.tier, 22.0);
                                }
                            });
                        });
                    }
                    let comp = Comp::of(&s.data, team.iter().filter(|p| p.picked || p.is_me).map(|p| p.champion_id));
                    if comp.n > 0 {
                        ui.add_space(4.0);
                        ui.horizontal_wrapped(|ui| {
                            ui.label(RichText::new(comp.summary()).size(11.5).color(MUTED))
                                .on_hover_text("From Riot's champion ratings and class tags (Data Dragon), not this patch's builds");
                            for w in comp.warnings() {
                                pill(ui, w, SOON);
                            }
                        });
                    }
                });
            }
        });
        if !cs.bans.is_empty() {
            ui.horizontal_wrapped(|ui| {
                overline(ui, "Bans");
                ui.add_space(4.0);
                for &b in &cs.bans {
                    let (r, resp) = ui.allocate_exact_size(vec2(28.0, 28.0), Sense::hover());
                    g.paint_champ(ui.painter(), r, b, Color32::from_gray(120));
                    ui.painter().line_segment([r.left_bottom() + vec2(4.0, -4.0), r.right_top() + vec2(-4.0, 4.0)], Stroke::new(2.0, LOSS));
                    resp.on_hover_text(s.data.champion_name(b));
                }
            });
        }
        if cs.my_ban_pending {
            ban_hints(ui, s, cs, &g, self.settings.meta_builds);
        }
        draft_record(ui, s, cs, &g);
        let history = s.lol_history.as_deref().unwrap_or(&[]);
        let record = |c: i64| averages(history.iter().filter(|g| g.champion_id == c));
        if cs.picking_card() {
            section(ui, "Pick a champion");
            if cs.card_choices.is_empty() {
                ui.horizontal(|ui| {
                    ui.add(eframe::egui::Spinner::new().size(14.0).color(ACCENT));
                    note(ui, "Waiting for the client's champion cards…");
                });
            }
            tiles(ui, |ui| {
                for &c in &cs.card_choices {
                    let resp = pick_tile(ui, &g, c, record(c).as_ref(), false);
                    if resp.on_hover_text("Pick this champion (your record on it from recent games)").clicked() {
                        self.request(Action::PickCard(c));
                    }
                }
            });
        }

        if cs.bench_enabled {
            section(ui, "Bench");
            if cs.bench.is_empty() {
                note(ui, "Bench is empty");
            }
            tiles(ui, |ui| {
                for &c in &cs.bench {
                    // The bench carries no lock state; the client locks champions you can't pick
                    // (not owned or rented). Until the pickable list loads, nothing is locked.
                    let locked = !s.pickable.is_empty() && !s.pickable.contains(&c);
                    let resp = pick_tile(ui, &g, c, record(c).as_ref(), locked);
                    if locked {
                        resp.on_hover_text("Locked: you don't own this champion, so you can't swap to it");
                    } else if resp.on_hover_text("Swap to this champion (your record on it from recent games)").clicked() {
                        self.request(Action::BenchSwap(c));
                    }
                }
            });
        }

        if !cs.locked && !cs.picking_card() {
            pick_ideas(ui, s, cs, &g);
        }
        self.meta_build(ui, s, cs, &g);

        section(ui, "Recommended runes");
        if cs.my_champion().is_none() {
            note(ui, "Hover or lock a champion to see Riot's recommended pages.");
        } else if s.rec_pages.is_empty() {
            note(ui, "No recommended page for this pick and mode.");
        }
        for (i, p) in s.rec_pages.iter().enumerate() {
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    g.perk(ui, p.perks[0], 40.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.label(RichText::new(s.data.perk_name(p.perks[0])).font(semibold(14.0)).color(TEXT));
                        ui.label(RichText::new(&p.label).color(MUTED));
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(Btn::secondary("Apply spells").small()).clicked() {
                            self.request(Action::ApplySpells(i));
                        }
                        if ui.add(Btn::primary("Apply runes").small()).clicked() {
                            self.request(Action::ApplyRunes(i));
                        }
                    });
                });
                ui.add_space(4.0);
                rec_page_lines(ui, &g, p);
            });
        }
        if !s.champ_log.is_empty() {
            section(ui, "Automation");
            card(ui, |ui| {
                for (m, ok) in &s.champ_log {
                    status_msg(ui, *ok, m);
                }
            });
        }

        section(ui, "Your history on this pick");
        let Some(champ) = cs.my_champion() else {
            note(ui, "Hover or lock a champion to see your recent games on it.");
            suggestions(ui, s, &g, history);
            return;
        };
        let name = s.data.champion_name(champ);
        card(ui, |ui| {
            ui.horizontal(|ui| {
                g.champ(ui, champ, 48.0);
                ui.add_space(4.0);
                let on: Vec<_> = history.iter().filter(|g| g.champion_id == champ).collect();
                if on.is_empty() {
                    note(ui, format!("No {name} games in your last {} matches.", history.len()));
                } else {
                    let wins = on.iter().filter(|g| g.win).count();
                    let kda = on.iter().map(|g| g.kda()).sum::<f64>() / on.len() as f64;
                    let wr = wins as f64 / on.len() as f64;
                    stat_tile(ui, "Games", on.len().to_string(), TEXT, Some(&name));
                    stat_tile(ui, "Wins", wins.to_string(), wr_color(wr), Some(&format!("{:.0}%", wr * 100.0)));
                    stat_tile(ui, "Avg KDA", format!("{kda:.2}"), kda_color(kda), None);
                }
            });
            // Same mode as the lock-in item set, so the button saves what lock-in would use.
            let mode = meta::mode_for(cs.queue_id, s.session.as_ref().map_or(0, |g| g.map_id)).ok();
            if let Some(best) = itemset::best_game(history, champ, mode) {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    overline(ui, "Best recent build in this mode");
                    g.items(ui, &best.items, 24.0);
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let root = s.league_root();
                        if ui.add(Btn::secondary(&format!("Save as {name} item set")).icon(Icon::Save).small()).clicked() {
                            self.write_item_set(root.as_deref(), &s.data, best);
                        }
                    });
                });
            }
        });
        self.my_pick_card(ui, s, cs, champ);
        suggestions(ui, s, &g, history);
    }

    /// Your pick at a glance: mastery toward the next level and milestone, your notes on it and on
    /// the probable lane matchup (plus other saved notes for this draft, read-only), and the
    /// challenges a game on it can advance.
    fn my_pick_card(&mut self, ui: &mut Ui, s: &Snapshot, cs: &ChampSelect, champ: i64) {
        let name = s.data.champion_name(champ);
        let mastery = s.mastery.iter().find(|m| m.champion_id == champ);
        let mode = match s.session.as_ref().map_or(0, |x| x.map_id) {
            12 => "ARAM",
            30 => "CHERRY",
            _ => "CLASSIC",
        };
        let challenges: Vec<&lcu::Challenge> = match &s.challenge_list {
            Some(Ok(list)) => lcu::champion_challenges(list, champ, mode).into_iter().take(3).collect(),
            _ => Vec::new(),
        };
        let opp = s.opponent.as_ref().map(|o| o.champion_id);
        let enemies: Vec<i64> = cs.their_team.iter().filter(|p| p.picked && p.champion_id > 0).map(|p| p.champion_id).collect();
        let own_key = notes::champ_key(champ);
        let vs_key = opp.map(|o| notes::vs_key(champ, o));
        let others: Vec<(String, String, String)> = s
            .notes
            .for_game(champ, &enemies, opp, &s.data)
            .into_iter()
            .filter(|(k, _, _)| *k != own_key && Some(k) != vs_key.as_ref())
            .collect();
        section(ui, &format!("{name}: mastery, notes and challenges"));
        card(ui, |ui| {
            let notes_col = |this: &mut App, ui: &mut Ui| {
                overline(ui, "Your notes");
                let mut boxes = vec![(own_key.clone(), format!("{name} notes"))];
                if let (Some(o), Some(k)) = (opp, &vs_key) {
                    boxes.push((k.clone(), format!("{name} vs {}", s.data.champion_name(o))));
                }
                for (key, heading) in &boxes {
                    if let Some((key, text)) = note_box(ui, &mut this.note_edit, key, heading, s.notes.get(key)) {
                        this.request(Action::SaveNote { key, text });
                    }
                    ui.add_space(4.0);
                }
                for (_, heading, text) in &others {
                    ui.label(RichText::new(heading).font(semibold(12.5)).color(TEXT));
                    ui.add(Label::new(RichText::new(text).color(BODY)).wrap());
                    ui.add_space(4.0);
                }
            };
            if mastery.is_none() && challenges.is_empty() {
                return notes_col(self, ui);
            }
            ui.columns(2, |cols| {
                if let Some(m) = mastery {
                    mastery_lines(&mut cols[0], m);
                    cols[0].add_space(6.0);
                }
                if !challenges.is_empty() {
                    challenge_lines(&mut cols[0], &challenges);
                }
                notes_col(self, &mut cols[1]);
            });
        });
    }

    /// OP.GG's builds for the current pick (Settings → "Use OP.GG builds"): the champion's OP.GG
    /// summary, build types and builds best first (Arena: most played first), then the picked
    /// build's items, skill orders, rune pages, spells and lane matchups. Clicking an option picks
    /// it for the OP.GG rune page and the lock-in set.
    fn meta_build(&mut self, ui: &mut Ui, s: &Snapshot, cs: &ChampSelect, g: &Gfx) {
        if !self.settings.meta_builds {
            return;
        }
        section(ui, "OP.GG builds");
        let Some(m) = &s.meta else {
            return void(note(ui, "Hover or lock a champion to load OP.GG's builds for it."));
        };
        let name = s.data.champion_name(m.champion_id);
        let loading = |ui: &mut Ui, text: String| {
            ui.horizontal(|ui| {
                ui.add(eframe::egui::Spinner::new().size(14.0).color(ACCENT));
                note(ui, text);
            });
        };
        let build = match (&m.query, &m.build) {
            (Err(e), _) => return void(note(ui, format!("OP.GG: {e}; the lock-in item set uses your own games."))),
            (Ok(q), None) if s.data.item_stats.is_empty() => {
                return loading(ui, format!("Waiting for Data Dragon's item data to load OP.GG {} builds for {name}…", q.label()));
            }
            (Ok(q), None) => return loading(ui, format!("Loading OP.GG {} builds for {name}…", q.label())),
            (_, Some(Err(e))) => {
                error(ui, format!("{e}; the lock-in item set uses your own games."));
                if ui.add(Btn::secondary("Retry OP.GG").icon(Icon::Refresh).small()).clicked() {
                    self.request(Action::RetryMeta);
                }
                return;
            }
            (_, Some(Ok(b))) => b,
        };
        let p = m.picks;
        let sel = p.build.min(build.cores.len() - 1);
        let groups = build.groups();
        // The group the user opened for this champion, else the first one holding the selected build.
        let opened = self.meta_group.filter(|&(c, g)| c == m.champion_id && groups.contains(&g)).map(|(_, g)| g);
        let Some(shown) = opened.or_else(|| groups.iter().copied().find(|&g| build.cores[sel].in_group(g))) else {
            return;
        };
        let arrows = |ui: &mut Ui, ids: &[i64], size: f32| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 3.0;
                for (k, &id) in ids.iter().enumerate() {
                    if k > 0 {
                        ui.label(RichText::new("›").color(FAINT));
                    }
                    g.item(ui, id, size);
                }
            });
        };
        card(ui, |ui| {
            ui.horizontal(|ui| {
                g.champ(ui, m.champion_id, 36.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 1.0;
                    ui.label(RichText::new(format!("{name} · OP.GG {}", build.query.label())).font(semibold(14.0)).color(TEXT));
                    let order = if build.query.mode == "arena" { ", most played first" } else { "" };
                    ui.label(RichText::new(format!("{} builds{order}", build.cores.len())).color(MUTED));
                });
            });
            if !build.patch_matches(&s.data.version) {
                error(ui, format!("OP.GG's data is for patch {}; the game is on {}.", build.patch, s.data.version));
            }
            if cs.queue_id == 2400 {
                small_note(ui, "ARAM Mayhem: these are OP.GG's ARAM builds; Mayhem's augments can change which items are strong.");
            }
            if build.query.mode == "arena" {
                small_note(ui, "Arena: options with pick rate and games, most played first; Riot does not allow Arena win rates. Augments are not shown.");
            }
            if let Some(sum) = &build.summary {
                ui.add_space(6.0);
                opgg_summary(ui, sum, &name, build.query);
            }
            ui.add_space(6.0);
            ui.horizontal_wrapped(|ui| {
                ui.spacing_mut().item_spacing.x = 4.0;
                for gr in groups {
                    let members = build.in_group(gr);
                    if ui.add(Btn::secondary(&format!("{} ({})", gr.label(), members.len())).small().selected(shown == gr)).clicked() {
                        self.meta_group = Some((m.champion_id, gr));
                        if let Some(&best) = members.first().filter(|&&b| b != sel) {
                            self.request(Action::SelectMeta(Pick::Build(best)));
                        }
                    }
                }
            });
            ui.add_space(2.0);
            for i in build.in_group(shown) {
                let c = &build.cores[i];
                let few = c.items.few_games();
                let row = option_row(ui, i == sel, |ui| {
                    ui.add_sized([96.0, 20.0], eframe::egui::Label::new(RichText::new(c.label()).font(semibold(12.5)).color(if few { MUTED } else { TEXT })));
                    arrows(ui, &c.items.ids, 26.0);
                    option_stats(ui, &c.items.stats(), few);
                });
                if row.clicked() && i != sel {
                    self.request(Action::SelectMeta(Pick::Build(i)));
                }
            }
            ui.add_space(6.0);
            let chosen = &build.cores[sel];
            inset_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(chosen.label()).font(semibold(14.0)).color(TEXT));
                    ui.label(RichText::new(chosen.items.stats()).size(11.5).color(MUTED));
                });
                ui.add_space(4.0);
                let row = |ui: &mut Ui, label: &str, f: &mut dyn FnMut(&mut Ui)| {
                    ui.horizontal_wrapped(|ui| {
                        ui.add_sized([84.0, 22.0], eframe::egui::Label::new(RichText::new(label.to_uppercase()).font(semibold(10.5)).color(MUTED)));
                        f(ui);
                    });
                };
                if let Some((st, others)) = build.starters.split_first() {
                    row(ui, "Starting", &mut |ui| {
                        g.items(ui, &st.ids, 24.0);
                        for o in others.iter().take(2) {
                            ui.label(RichText::new("or").color(FAINT));
                            g.items(ui, &o.ids, 20.0);
                            ui.label(RichText::new(format!("({})", o.stats())).size(11.0).color(MUTED));
                        }
                    });
                }
                row(ui, "Core", &mut |ui| arrows(ui, &chosen.items.ids, 24.0));
                if let Some((b, others)) = build.boots.split_first() {
                    row(ui, "Boots", &mut |ui| {
                        g.items(ui, &b.ids, 24.0);
                        ui.label(RichText::new(format!("({})", b.stats())).size(11.0).color(MUTED));
                        for o in others {
                            ui.label(RichText::new("or").color(FAINT));
                            g.items(ui, &o.ids, 20.0);
                            ui.label(RichText::new(format!("({})", o.stats())).size(11.0).color(MUTED));
                        }
                    });
                }
                row(ui, "Full build", &mut |ui| g.items(ui, &build.full_build(sel, &s.data), 24.0));
                let situational = build.situational(sel, &s.data);
                if !situational.is_empty() {
                    row(ui, "Situational", &mut |ui| g.items(ui, &situational, 22.0));
                }
            });
            if !build.skills.is_empty() {
                ui.add_space(6.0);
                overline(ui, "Skill order");
                for (i, sk) in build.skills.iter().enumerate() {
                    let few = sk.few_games();
                    let row = option_row(ui, i == p.skills, |ui| {
                        let pr = build.skill_priority(i);
                        if !pr.is_empty() {
                            ui.spacing_mut().item_spacing.x = 3.0;
                            for (k, key) in pr.iter().enumerate() {
                                if k > 0 {
                                    ui.label(RichText::new("›").color(FAINT));
                                }
                                key_cap(ui, key, 22.0);
                            }
                            ui.add_space(10.0);
                        }
                        ui.spacing_mut().item_spacing.x = 2.0;
                        for key in &sk.order {
                            key_cap(ui, key, 16.0);
                        }
                        option_stats(ui, &sk.stats(), few);
                    });
                    if row.on_hover_text(sk.order.join(" ")).clicked() && i != p.skills {
                        self.request(Action::SelectMeta(Pick::Skills(i)));
                    }
                }
            }
            if !build.runes.is_empty() {
                ui.add_space(6.0);
                overline(ui, "Rune page, best fit for the picked build first (offered below as the OP.GG page)");
                let order = build.rune_order(sel);
                for (rank, &i) in order.iter().enumerate() {
                    let r = &build.runes[i];
                    let few = r.few_games();
                    // Pages often share keystone and styles, so every rune is shown to tell them apart.
                    let runes = |ids: &[i64]| ids.iter().map(|&id| s.data.perk_name(id)).collect::<Vec<_>>().join(", ");
                    let names = format!(
                        "{}: {} · {}: {}",
                        s.data.perk_name(r.primary_page_id),
                        runes(&r.primary_rune_ids),
                        s.data.perk_name(r.secondary_page_id),
                        runes(&r.secondary_rune_ids)
                    );
                    let row = option_row(ui, i == p.runes, |ui| {
                        ui.spacing_mut().item_spacing.x = 3.0;
                        g.perk(ui, r.primary_page_id, 18.0);
                        for (k, &id) in r.primary_rune_ids.iter().enumerate() {
                            g.perk(ui, id, if k == 0 { 30.0 } else { 22.0 });
                        }
                        ui.add_space(8.0);
                        g.perk(ui, r.secondary_page_id, 18.0);
                        for &id in &r.secondary_rune_ids {
                            g.perk(ui, id, 22.0);
                        }
                        if rank == 0 && order.len() > 1 {
                            ui.add_space(6.0);
                            pill(ui, "suggested", ACCENT_TEXT).on_hover_text(format!("For the {} build: {}", chosen.label(), chosen.rune_fit_note()));
                        }
                        option_stats(ui, &r.stats(), few);
                    });
                    if row.on_hover_text(names).clicked() && i != p.runes {
                        self.request(Action::SelectMeta(Pick::Runes(i)));
                    }
                }
            }
            if !build.spells.is_empty() {
                ui.add_space(6.0);
                overline(ui, "Summoner spells");
                for (i, sp) in build.spells.iter().enumerate() {
                    let few = sp.few_games();
                    let row = option_row(ui, i == p.spells, |ui| {
                        g.spell(ui, sp.ids[0], 24.0);
                        g.spell(ui, sp.ids[1], 24.0);
                        ui.label(
                            RichText::new(format!("{} + {}", s.data.spell_name(sp.ids[0]), s.data.spell_name(sp.ids[1])))
                                .color(if few { MUTED } else { BODY }),
                        );
                        option_stats(ui, &sp.stats(), few);
                    });
                    if row.clicked() && i != p.spells {
                        self.request(Action::SelectMeta(Pick::Spells(i)));
                    }
                }
            }
            if !build.counters.is_empty() {
                ui.add_space(6.0);
                overline(ui, "Lane matchups");
                let mine = lcu::perk_position(cs.me().map_or("", |me| me.position.as_str()));
                let enemies: Vec<_> = cs.their_team.iter().filter(|e| e.picked && e.champion_id > 0).collect();
                if enemies.is_empty() {
                    note(ui, format!("Enemy picks show here with {name}'s lane win rate against them."));
                }
                for e in enemies {
                    let lane = mine != "NONE" && lcu::perk_position(&e.position) == mine;
                    let enemy = s.data.champion_name(e.champion_id);
                    ui.horizontal(|ui| {
                        g.champ(ui, e.champion_id, 24.0);
                        ui.label(RichText::new(format!("vs {enemy}")).font(semibold(12.5)).color(TEXT));
                        if lane {
                            pill(ui, "your lane", CYAN);
                        }
                        match build.matchup(e.champion_id) {
                            Some(mu) => ui.label(RichText::new(mu.stats()).color(BODY)),
                            None => ui.label(RichText::new("no OP.GG lane data").color(MUTED)),
                        };
                    });
                }
            }
            let sample = match build.query.mode {
                "ranked" | "flex" => "Emerald+ games",
                _ => "all games",
            };
            let patch = if build.patch.is_empty() { String::new() } else { format!("patch {}, ", build.patch) };
            let margin = if build.query.mode == "arena" { "Most played first" } else { "± is the win rate's 95% margin" };
            ui.add_space(6.0);
            small_note(ui, format!("Data: OP.GG (op.gg), {patch}{sample}. {margin}. Types from Data Dragon item stats."));
        });
    }
}

/// OP.GG's numbers for the picked champion: one line, then its win rate by game length
/// (bars around 50 %) and by recent patch. Arena has neither chart (no win rates).
fn opgg_summary(ui: &mut Ui, sum: &meta::ChampSummary, name: &str, query: meta::Query) {
    inset_frame().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.add(Label::new(RichText::new(sum.line(name, query)).size(12.0).color(BODY)).wrap());
        let lengths = &sum.game_lengths;
        let trend = &sum.trend;
        if lengths.is_empty() && trend.len() < 2 {
            return;
        }
        ui.add_space(4.0);
        let w = ((ui.available_width() - 16.0) / 2.0).max(140.0);
        ui.horizontal_top(|ui| {
            ui.spacing_mut().item_spacing.x = 16.0;
            if !lengths.is_empty() {
                // OP.GG's buckets: under the next start, between starts, and the last one onwards.
                let labels: Vec<String> = lengths
                    .iter()
                    .enumerate()
                    .map(|(i, &(from, _))| match lengths.get(i + 1) {
                        Some(&(next, _)) if from == 0 => format!("<{next}"),
                        Some(&(next, _)) => format!("{from}–{next}"),
                        None => format!("{from}+"),
                    })
                    .collect();
                let bars: Vec<(&str, f64)> = labels.iter().zip(lengths).map(|(l, &(_, wr))| (l.as_str(), wr)).collect();
                ui.vertical(|ui| {
                    ui.set_width(w);
                    overline(ui, "Win rate by game length (min)");
                    let axis = BarAxis { base: 0.5, span: 0.1, above: WIN, below: LOSS };
                    bar_chart(ui, &bars, vec2(w, 64.0), axis, &|i: usize| format!("{} min: {:.1}% WR", labels[i], lengths[i].1 * 100.0));
                });
            }
            if trend.len() >= 2 {
                let points: Vec<LinePoint> = trend.iter().enumerate().map(|(i, &(_, wr))| LinePoint { x: i as f64, y: wr, gap_before: false }).collect();
                ui.vertical(|ui| {
                    ui.set_width(w);
                    overline(ui, "Win rate by patch");
                    let label = |i: usize| format!("Patch {}: {:.1}% WR", trend[i].0, trend[i].1 * 100.0);
                    line_chart(ui, &points, vec2(w, 48.0), CYAN, Some(Baseline { value: 0.5, below: None }), &label);
                    let (first, last) = (&trend[0].0, &trend[trend.len() - 1].0);
                    ui.label(RichText::new(format!("{first} → {last} · dashed line 50%")).size(11.0).color(MUTED));
                });
            }
        });
    });
}

/// Picks from your own record: champions you have won with recently, best first.
fn suggestions(ui: &mut Ui, s: &Snapshot, g: &Gfx, history: &[LolGame]) {
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
    tiles(ui, |ui| {
        for (c, a) in rows.iter().take(6) {
            inset_frame().show(ui, |ui| {
                ui.horizontal(|ui| {
                    g.champ(ui, *c, 34.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 1.0;
                        ui.label(RichText::new(s.data.champion_name(*c)).font(semibold(12.5)).color(TEXT));
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.label(RichText::new(format!("{:.0}%", a.win_rate() * 100.0)).font(semibold(12.0)).color(wr_color(a.win_rate())));
                            ui.label(RichText::new(format!("{}G · KDA {:.1}", a.n, a.kda())).size(11.5).color(MUTED));
                        });
                    });
                });
            });
        }
    });
    small_note(ui, format!("From your last {} games; owned and not banned.", history.len()));
}

/// A recommended page: rune paths and runes as icons, shards and spells.
pub(super) fn rec_page_lines(ui: &mut Ui, g: &Gfx, p: &RecPage) {
    let names = |ids: &[i64]| ids.iter().map(|&i| g.data.perk_name(i)).collect::<Vec<_>>().join(", ");
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        g.perk(ui, p.primary_style, 20.0);
        ui.label(RichText::new(g.data.perk_name(p.primary_style)).font(semibold(12.0)).color(BODY)).on_hover_text(names(&p.perks[..4]));
        for &id in &p.perks[..4] {
            g.perk(ui, id, 26.0);
        }
        ui.add_space(12.0);
        g.perk(ui, p.sub_style, 20.0);
        ui.label(RichText::new(g.data.perk_name(p.sub_style)).font(semibold(12.0)).color(BODY)).on_hover_text(names(&p.perks[4..6]));
        for &id in &p.perks[4..6] {
            g.perk(ui, id, 26.0);
        }
    });
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(format!("Shards: {}", names(&p.perks[6..]))).size(11.5).color(MUTED));
        if !p.spells.contains(&0) {
            ui.add_space(8.0);
            g.spell(ui, p.spells[0], 20.0);
            g.spell(ui, p.spells[1], 20.0);
            ui.label(RichText::new(format!("{} + {}", g.data.spell_name(p.spells[0]), g.data.spell_name(p.spells[1]))).size(11.5).color(MUTED));
        }
    });
}
