//! Player lookup and the champion browser.

use eframe::egui::{Align, Key, Layout, RichText, ScrollArea, TextEdit, Ui, vec2};

use super::champ_select::rec_page_lines;
use super::home::{lol_summary, mastery_list, ranked_list, void};
use super::matches::trunc;
use super::stats::{averages, position_label};
use super::theme::*;
use super::widgets::*;
use super::App;
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
        let mut list: Vec<(i64, &str)> = s
            .data
            .champions
            .iter()
            .filter(|(_, c)| filter.is_empty() || c.name.to_lowercase().contains(&filter))
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
                    small_note(ui, format!("{} champions", list.len()));
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
