//! Player lookup and the champion browser.

use eframe::egui::{Align, Key, Layout, RichText, ScrollArea, TextEdit, Ui, vec2};

use super::champ_select::rec_page_lines;
use super::home::{lol_summary, mastery_list, ranked_list, void};
use super::matches::trunc;
use super::stats::{averages, position_label, queue_name};
use super::theme::*;
use super::widgets::*;
use super::App;
use crate::lcu::Friend;
use crate::worker::{Action, LcuStatus, Snapshot};
use crate::itemset;

impl App {
    pub(super) fn players(&mut self, ui: &mut Ui, s: &Snapshot) {
        let icons = self.icons.clone();
        let g = Gfx::new(&icons, &s.data);
        card(ui, |ui| {
            overline(ui, "Look up a player");
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                let r = ui.add(
                    TextEdit::singleline(&mut self.lookup_input)
                        .hint_text("Name#TAG")
                        .desired_width(300.0)
                        .font(body(14.0))
                        .margin(vec2(10.0, 7.0)),
                );
                let go = ui.add_enabled(s.busy.is_none(), Btn::primary("Look up").icon(Icon::Search)).clicked()
                    || (r.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)));
                if go && !self.lookup_input.trim().is_empty() {
                    self.request(Action::Lookup(self.lookup_input.trim().to_string()));
                }
            });
            small_note(ui, "Uses the League client's own lookup; works for players on your region.");
        });
        self.friends_card(ui, s, &g);
        let Some(r) = &s.lookup else { return };
        let l = match r {
            Err(e) => return void(error(ui, e)),
            Ok(l) => l,
        };
        ui.add_space(8.0);
        card(ui, |ui| {
            ui.horizontal(|ui| {
                g.profile(ui, l.summoner.profile_icon, &l.summoner.game_name, 64.0);
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        ui.label(RichText::new(&l.summoner.game_name).font(display(28.0)).color(TEXT));
                        ui.label(RichText::new(format!("#{}", l.summoner.tag_line)).font(display(28.0)).color(FAINT));
                    });
                    if l.summoner.level > 0 {
                        pill(ui, format!("Level {}", l.summoner.level), MUTED);
                    }
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui
                        .add(Btn::secondary("Spectate live game").icon(Icon::Eye))
                        .on_hover_text("Asks the League client to start spectating this player")
                        .clicked()
                    {
                        self.request(Action::Spectate(l.summoner.puuid.clone()));
                    }
                });
            });
        });
        ui.columns(2, |cols| {
            card(&mut cols[0], |ui| {
                card_head(ui, "Ranked", |_| {});
                ranked_list(ui, &l.ranked, false);
            });
            card(&mut cols[1], |ui| {
                card_head(ui, "Top mastery", |_| {});
                mastery_list(ui, &g, &l.mastery);
            });
        });
        if let Ok(h) = &l.history {
            card(ui, |ui| lol_summary(ui, &g, h));
        }
        section(ui, "Recent games");
        match &l.history {
            Err(e) => void(error(ui, e)),
            Ok(h) => {
                let puuid = l.summoner.puuid.clone();
                if let Some(id) = self.game_cards(ui, s, h, None, &puuid, "lookup") {
                    self.lookup(id);
                }
                ui.vertical_centered(|ui| {
                    if ui.add(Btn::secondary("Load 20 more").icon(Icon::ChevronDown)).clicked() {
                        self.request(Action::LookupMore);
                    }
                });
            }
        }
    }

    /// Friends signed in to League with their client status; look up or spectate them.
    fn friends_card(&mut self, ui: &mut Ui, s: &Snapshot, g: &Gfx) {
        let Some(friends) = &s.friends else { return };
        section_with(ui, "Friends online", |ui| {
            if ui.add(Btn::ghost("Refresh").icon(Icon::Refresh).small()).clicked() {
                self.request(Action::LoadFriends);
            }
        });
        let mut friends: Vec<&Friend> = match friends {
            Err(e) => return void(error(ui, e)),
            Ok(f) if f.is_empty() => return void(note(ui, "No friends are signed in to League.")),
            Ok(f) => f.iter().collect(),
        };
        // In a game first, then in champ select / queue, then idle.
        friends.sort_by_key(|f| (status_rank(&f.game_status), f.name.to_lowercase()));
        let mut look = None;
        card(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 4.0;
            for f in &friends {
                ui.horizontal(|ui| {
                    g.profile(ui, f.icon, &f.name, 30.0);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        trunc(ui, RichText::new(&f.name).font(semibold(12.5)).color(TEXT), 220.0);
                        let color = if status_rank(&f.game_status) == 0 { WIN } else { MUTED };
                        trunc(ui, RichText::new(friend_status(f, s)).size(11.5).color(color), 320.0);
                    });
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.add(Btn::ghost("Look up").icon(Icon::Search).small()).clicked() {
                            look = Some(f.name.clone());
                        }
                        if spectatable(&f.game_status)
                            && ui
                                .add(Btn::secondary("Spectate").icon(Icon::Eye).small())
                                .on_hover_text("Asks the League client to start spectating this friend")
                                .clicked()
                        {
                            self.request(Action::Spectate(f.puuid.clone()));
                        }
                    });
                });
            }
        });
        if let Some(id) = look {
            self.lookup(id);
        }
    }

    pub(super) fn champions(&mut self, ui: &mut Ui, s: &Snapshot) {
        if s.data.champions.is_empty() {
            return ui.horizontal(|ui| {
                ui.add(eframe::egui::Spinner::new().size(14.0).color(ACCENT));
                note(ui, "Loading Data Dragon…");
            })
            .inner;
        }
        let icons = self.icons.clone();
        let g = Gfx::new(&icons, &s.data);
        let history = s.lol_history.as_deref().unwrap_or(&[]);
        let filter = self.champ_filter.to_lowercase();
        let access = |id: i64| s.champion_access.iter().find(|c| c.id == id);
        let scope = self.champ_scope;
        let mut list: Vec<(i64, &str)> = s
            .data
            .champions
            .iter()
            .filter(|(_, c)| filter.is_empty() || c.name.to_lowercase().contains(&filter))
            .filter(|(id, _)| match scope {
                ChampScope::All => true,
                ChampScope::Owned => access(**id).is_some_and(|c| c.owned),
                ChampScope::Free => access(**id).is_some_and(|c| c.free),
            })
            .map(|(&id, c)| (id, c.name.as_str()))
            .collect();
        list.sort_by(|a, b| a.1.cmp(b.1));
        let narrow = ui.available_width() < 760.0;
        let list_w = if narrow { ui.available_width() } else { 300.0 };
        ui.horizontal_top(|ui| {
            ui.vertical(|ui| {
                ui.set_width(list_w);
                card(ui, |ui| {
                    ui.add(
                        TextEdit::singleline(&mut self.champ_filter)
                            .hint_text("Search champions")
                            .desired_width(f32::INFINITY)
                            .margin(vec2(10.0, 7.0)),
                    );
                    ui.add_space(4.0);
                    if !s.champion_access.is_empty() {
                        ui.horizontal(|ui| {
                            for (label, sc) in [("All", ChampScope::All), ("Owned", ChampScope::Owned), ("Free this week", ChampScope::Free)] {
                                if ui.add(Btn::secondary(label).small().selected(self.champ_scope == sc)).clicked() {
                                    self.champ_scope = sc;
                                }
                            }
                        });
                        let owned = s.champion_access.iter().filter(|c| c.owned).count();
                        let free: Vec<_> = s.champion_access.iter().filter(|c| c.free).collect();
                        let free_owned = free.iter().filter(|c| c.owned).count();
                        small_note(
                            ui,
                            format!(
                                "{} shown · you own {owned} · {} free this week, {} of them new to you",
                                list.len(),
                                free.len(),
                                free.len() - free_owned
                            ),
                        );
                    } else {
                        small_note(ui, format!("{} champions", list.len()));
                    }
                    ScrollArea::vertical().id_salt("champ-list").max_height(if narrow { 300.0 } else { 600.0 }).show(ui, |ui| {
                        ui.spacing_mut().item_spacing.y = 2.0;
                        for (id, name) in &list {
                            let picked = self.champ_selected == Some(*id);
                            let a = averages(history.iter().filter(|g| g.champion_id == *id));
                            let m = s.mastery.iter().find(|m| m.champion_id == *id);
                            let row = option_row(ui, picked, |ui| {
                                g.champ(ui, *id, 28.0);
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 0.0;
                                    trunc(ui, RichText::new(*name).font(semibold(12.5)).color(if picked { TEXT } else { BODY }), 130.0);
                                    if let Some(m) = m {
                                        ui.label(RichText::new(format!("M{} · {}k", m.level, m.points / 1000)).size(11.0).color(MUTED));
                                    }
                                });
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if let Some(a) = &a {
                                        ui.label(RichText::new(format!("{}G", a.n)).size(11.0).color(MUTED));
                                        ui.label(RichText::new(format!("{:.0}%", a.win_rate() * 100.0)).font(semibold(12.0)).color(wr_color(a.win_rate())));
                                    }
                                    match access(*id) {
                                        Some(c) if c.free && !c.owned => void(pill(ui, "Free", WIN)),
                                        Some(c) if c.free => void(pill(ui, "Free", MUTED)),
                                        _ => {}
                                    }
                                });
                            });
                            if row.clicked() && !picked {
                                self.champ_selected = Some(*id);
                                self.request(Action::BrowseChampion(*id));
                            }
                        }
                    });
                });
            });
            if !narrow {
                ui.add_space(4.0);
                ui.vertical(|ui| self.champion_detail(ui, s, &g));
            }
        });
        if narrow {
            self.champion_detail(ui, s, &g);
        }
    }

    fn champion_detail(&mut self, ui: &mut Ui, s: &Snapshot, g: &Gfx) {
        let Some(id) = self.champ_selected else {
            return empty_state(ui, Icon::Grid, "Pick a champion", "Riot's recommended runes and your own stats on it show here.", |_| {});
        };
        let history = s.lol_history.as_deref().unwrap_or(&[]);
        hero(ui, g.splash(id), 150.0, vec2(0.5, 0.25), |ui| {
            g.champ(ui, id, 56.0);
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(RichText::new(s.data.champion_name(id)).font(display(34.0)).color(TEXT));
                if let Some(m) = s.mastery.iter().find(|m| m.champion_id == id) {
                    ui.label(RichText::new(format!("Mastery {} · {} points", m.level, m.points)).color(BODY));
                }
            });
        });
        ui.add_space(8.0);
        card(ui, |ui| {
            card_head(ui, "Your games on it", |_| {});
            match averages(history.iter().filter(|g| g.champion_id == id)) {
                None => void(note(ui, format!("No games on it in your last {} matches", history.len()))),
                Some(a) => {
                    tiles(ui, |ui| {
                        stat_tile(ui, "Games", a.n.to_string(), TEXT, None);
                        stat_tile(ui, "Win rate", format!("{:.0}%", a.win_rate() * 100.0), wr_color(a.win_rate()), None);
                        stat_tile(ui, "KDA", format!("{:.2}", a.kda()), kda_color(a.kda()), None);
                        stat_tile(ui, "CS / min", format!("{:.1}", a.cs_min), TEXT, None);
                        stat_tile(ui, "Damage / min", format!("{:.0}", a.dmg_min), TEXT, None);
                    });
                }
            }
            if let Some(best) = itemset::best_game(history, id, None) {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    overline(ui, "Your best recent build");
                    g.items(ui, &best.items, 24.0);
                });
            }
        });
        section(ui, "Recommended runes");
        match &s.browse {
            Some((c, Ok(pages))) if *c == id => {
                for p in pages {
                    card(ui, |ui| {
                        ui.horizontal(|ui| {
                            g.perk(ui, p.perks[0], 36.0);
                            ui.vertical(|ui| {
                                ui.spacing_mut().item_spacing.y = 1.0;
                                ui.horizontal(|ui| {
                                    ui.label(RichText::new(s.data.perk_name(p.perks[0])).font(semibold(14.0)).color(TEXT));
                                    pill(ui, position_label(&p.position), CYAN);
                                });
                                ui.label(RichText::new(&p.label).color(MUTED));
                            });
                        });
                        ui.add_space(4.0);
                        rec_page_lines(ui, g, p);
                    });
                }
            }
            Some((c, Err(e))) if *c == id => void(error(ui, e)),
            _ if matches!(s.lcu, LcuStatus::Connected { .. }) => {
                ui.horizontal(|ui| {
                    ui.add(eframe::egui::Spinner::new().size(14.0).color(ACCENT));
                    note(ui, "Loading recommendations…");
                });
            }
            _ => void(note(ui, "Recommended runes need the League client running.")),
        }
    }
}

/// Statuses before the game itself: the client has no game to spectate yet.
fn pre_game(status: &str) -> bool {
    matches!(status, "outOfGame" | "inQueue" | "championSelect" | "") || status.starts_with("hosting")
}

fn spectatable(status: &str) -> bool {
    !pre_game(status)
}

fn status_rank(status: &str) -> u8 {
    match status {
        s if spectatable(s) => 0,
        "outOfGame" | "" => 2,
        _ => 1,
    }
}

/// `In game · Ranked Solo/Duo · Ahri`, `Champion select`, `Away`…
fn friend_status(f: &Friend, s: &Snapshot) -> String {
    let mut parts = Vec::new();
    match f.game_status.as_str() {
        "" | "outOfGame" => parts.push(match f.availability.as_str() {
            "away" => "Away".to_string(),
            "dnd" => "Busy".to_string(),
            _ => "Online".to_string(),
        }),
        st => parts.push(humanize(st)),
    }
    if !matches!(f.game_status.as_str(), "" | "outOfGame") && f.queue_id > 0 {
        parts.push(queue_name(f.queue_id));
    }
    if spectatable(&f.game_status) && f.champion_id > 0 {
        parts.push(s.data.champion_name(f.champion_id));
    }
    parts.join(" · ")
}

/// `championSelect` → `Champion select`.
fn humanize(camel: &str) -> String {
    let mut out = String::new();
    for (i, c) in camel.chars().enumerate() {
        if i == 0 {
            out.extend(c.to_uppercase());
        } else if c.is_uppercase() {
            out.push(' ');
            out.extend(c.to_lowercase());
        } else {
            out.push(c);
        }
    }
    out
}

/// Which champions the browser lists.
#[derive(Clone, Copy, PartialEq, Default, Debug)]
pub(super) enum ChampScope {
    #[default]
    All,
    Owned,
    /// This week's free rotation.
    Free,
}
