//! Champ select: teams, bans, ARAM cards and bench, OP.GG builds, Riot's
//! recommended pages, automation log, your record on the pick and suggestions.

use eframe::egui::{Align, Align2, Color32, Frame, Layout, Margin, RichText, Sense, Stroke, StrokeKind, Ui, vec2};

use super::home::void;
use super::matches::trunc;
use super::stats::{Avg, averages, position, queue_name, scout};
use super::theme::*;
use super::widgets::*;
use super::App;
use crate::lcu::{self, ChampSelect, LolGame, RecPage};
use crate::meta::{self, Pick};
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

/// A clickable champion tile (ARAM cards, bench, suggestions) with your record on it.
fn pick_tile(ui: &mut Ui, g: &Gfx, champ: i64, record: Option<&Avg>) -> eframe::egui::Response {
    let bg = ui.painter().add(eframe::egui::Shape::Noop);
    let inner = Frame::new().inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.horizontal(|ui| {
            g.champ(ui, champ, 40.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                ui.label(RichText::new(g.data.champion_name(champ)).font(semibold(13.0)).color(TEXT));
                match record {
                    Some(a) => {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 4.0;
                            ui.label(RichText::new(format!("{:.0}%", a.win_rate() * 100.0)).font(semibold(12.0)).color(wr_color(a.win_rate())));
                            ui.label(RichText::new(format!("{}G · KDA {:.1}", a.n, a.kda())).size(11.5).color(MUTED));
                        });
                    }
                    None => {
                        ui.label(RichText::new("no recent games").size(11.5).color(FAINT));
                    }
                }
            });
            ui.add_space(4.0);
        });
    });
    let rect = inner.response.rect;
    let resp = ui.interact(rect, inner.response.id.with("pick"), Sense::click());
    let hovered = resp.hovered();
    ui.painter().set(
        bg,
        eframe::egui::epaint::RectShape::new(
            rect,
            10,
            if hovered { HOVER } else { RAISED },
            Stroke::new(1.0, if hovered { alpha(ACCENT, 0.7) } else { BORDER_STRONG }),
            StrokeKind::Inside,
        ),
    );
    resp.on_hover_cursor(eframe::egui::CursorIcon::PointingHand)
}

impl App {
    pub(super) fn champ_select(&mut self, ui: &mut Ui, s: &Snapshot) {
        let icons = self.icons.clone();
        let g = Gfx::new(&icons, &s.data);
        let Some(cs) = &s.champ_select else {
            empty_state(ui, Icon::Target, "Not in champ select", "This tab opens automatically when one starts.", |_| {});
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
                    let resp = pick_tile(ui, &g, c, record(c).as_ref());
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
                    let resp = pick_tile(ui, &g, c, record(c).as_ref());
                    if resp.on_hover_text("Swap to this champion (your record on it from recent games)").clicked() {
                        self.request(Action::BenchSwap(c));
                    }
                }
            });
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
        suggestions(ui, s, &g, history);
    }

    /// OP.GG's builds for the current pick (Settings → "Use OP.GG builds"): build types and
    /// builds best first, then the picked build's items, skill orders, rune pages, spells and
    /// lane matchups. Clicking an option picks it for the OP.GG rune page and the lock-in set.
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
                    ui.label(RichText::new(format!("{} builds", build.cores.len())).color(MUTED));
                });
            });
            if !build.patch_matches(&s.data.version) {
                error(ui, format!("OP.GG's data is for patch {}; the game is on {}.", build.patch, s.data.version));
            }
            if cs.queue_id == 2400 {
                small_note(ui, "ARAM Mayhem: these are OP.GG's ARAM builds; Mayhem's augments can change which items are strong.");
            }
            if build.query.mode == "arena" {
                small_note(ui, "Arena: wins as OP.GG counts them, with average placement. Augments are not shown.");
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
                    ui.horizontal(|ui| {
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
                overline(ui, "Rune page (offered below as the OP.GG page)");
                for (i, r) in build.runes.iter().enumerate() {
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
            ui.add_space(6.0);
            small_note(ui, format!("Data: OP.GG (op.gg), {patch}{sample}. ± is the win rate's 95% margin. Types from Data Dragon item stats."));
        });
    }
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
