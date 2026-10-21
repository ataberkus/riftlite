//! Settings: automation toggles, rune source, overlay, behaviour, goals, replays, League
//! folder, item sets.

use std::sync::atomic::Ordering;

use eframe::egui::{Align, DragValue, Grid, Layout, RichText, TextEdit, Ui, vec2};

use super::theme::*;
use super::widgets::*;
use super::live_tab::FULLSCREEN_WARNING;
use super::stats::position_label;
use super::{App, set_overlay_edit};
use crate::goals::Goals;
use crate::itemset;
use crate::riot::Product;
use crate::settings::{self, OVERLAY_MIN_SIZE, OVERLAY_SIZE, PageSource};
use crate::worker::{Action, Snapshot};

/// Canonical positions, in the client's order.
const ROLES: [&str; 5] = ["TOP", "JUNGLE", "MIDDLE", "BOTTOM", "UTILITY"];

impl App {
    pub(super) fn settings_tab(&mut self, ui: &mut Ui, s: &Snapshot) {
        let mut changed = false;
        section(ui, "Matchmaking");
        card(ui, |ui| {
            changed |= toggle(
                ui,
                &mut self.settings.auto_accept,
                "Accept the ready check automatically when a match is found",
                Some("Never overrides a check you already accepted or declined."),
            );
        });

        section(ui, "Champ select");
        card(ui, |ui| {
            changed |= toggle(
                ui,
                &mut self.settings.auto_runes,
                "On lock-in, apply a recommended rune page",
                Some("Written to one page named \"riftlite: <champion>\" (created if a slot is free, then reused); your own pages are never changed."),
            );
            changed |= toggle(ui, &mut self.settings.auto_spells, "On lock-in, apply recommended summoner spells (Flash keeps its key)", None);
            ui.add_space(4.0);
            inset_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Lock-in runes and spells from").font(semibold(13.0)).color(TEXT));
                ui.add_space(2.0);
                changed |= segmented(
                    ui,
                    &mut self.settings.page_source,
                    &[(PageSource::Riot, "Riot's recommended page"), (PageSource::OpGg, "OP.GG (the page and spells you picked)")],
                );
                ui.add_space(2.0);
                small_note(
                    ui,
                    "OP.GG needs \"Use OP.GG builds\" on. Lock-in then waits up to 20 s for OP.GG's answer; when it does not come, \
                     OP.GG has no page for the mode (Arena), or builds are off, Riot's page is applied and the reason is logged.",
                );
            });
            ui.add_space(4.0);
            changed |= toggle(ui, &mut self.settings.auto_item_set, "On lock-in, write an item set for the champion", None);
            changed |= toggle(
                ui,
                &mut self.settings.meta_builds,
                "Use OP.GG builds (aggregated from many players' games)",
                Some(
                    "On: champ select lists OP.GG's builds for your mode and lane (ranked and flex: Emerald+; ARAM, URF, Nexus Blitz, \
                     Arena: all games), grouped AP / AD / On-hit / Crit / Tank / Utility (from Data Dragon item stats), the best 3 of each, each with its \
                     win rate's 95% margin; wide margins are marked \"few games\". Builds in a group are ranked by the low end of that \
                     range, so a few lucky games cannot outrank a much-played build. You also pick OP.GG's skill order, rune page and \
                     spells, and see your lane win rate against enemy picks. The build you pick is remembered per champion and mode \
                     (%LOCALAPPDATA%\\riftlite\\builds.json). The item set holds the picked build, OP.GG's other starts and boots, the \
                     best build of each other group, and your best recent game in this mode. Only the champion id, mode and lane are \
                     sent to lol-api-champion.op.gg. Off, or when OP.GG is unreachable or has too few games: the build from your best \
                     recent game in this mode.",
                ),
            );
            changed |= toggle(
                ui,
                &mut self.settings.style_builds,
                "In game: Blitz builds for each playstyle",
                Some(
                    "In a game, the Live tab shows Blitz's builds for your champion like blitz.gg: the playstyles (AP, Bruiser, \
                     Crit / On-Hit, …) with games and win rate to pick from, and for the picked one (else the one you are \
                     building) starting items, summoner spells, build order, completed and situational items and the skill \
                     order with the next point to take; in ARAM Mayhem also Mayhem items and the best augments of each rarity \
                     on your champion. The overlay shows that playstyle's items and max order, and its \"level up\" hint follows \
                     Blitz's skill order. Blitz has tables for ranked solo/duo by lane (used for every Summoner's Rift game), \
                     ARAM (also used for ARAM Mayhem) and URF; in other modes, or off, the OP.GG build picked in champ select is \
                     shown. Only the champion id, queue and lane are sent to data.v2.iesdev.com; augment icons come from \
                     raw.communitydragon.org.",
                ),
            );
        });

        section(ui, "Overlay");
        card(ui, |ui| {
            changed |= toggle(
                ui,
                &mut self.settings.overlay_enabled,
                "Show the in-game overlay during League games",
                Some(
                    "A separate always-on-top, click-through window fed by Riot's Live Client Data API. \
                     Nothing is injected into the game; it is visible when League runs in Borderless or Windowed mode.",
                ),
            );
            changed |= toggle(
                ui,
                &mut self.settings.overlay_focus_only,
                "Only while the game window is focused",
                Some("Off: the overlay stays on top of every window while a game is running (useful for testing)."),
            );
            if let Some(h) = self.hotkeys {
                if h.overlay {
                    void_msg(ui, true, "Ctrl+Shift+O hides/shows it; the tray icon menu does the same.");
                } else {
                    void_msg(ui, false, "Ctrl+Shift+O is taken by another program; use the tray icon menu to hide/show it.");
                }
                if h.edit {
                    void_msg(ui, true, "Ctrl+Shift+E turns edit mode on and off in game; so does the tray icon menu.");
                } else {
                    void_msg(ui, false, "Ctrl+Shift+E is taken by another program; use the tray icon menu or the switch below for edit mode.");
                }
            }
            if s.window_mode == Some(0) {
                void_msg(ui, false, FULLSCREEN_WARNING);
            }
            ui.add_space(4.0);
            let mut edit = self.shared.overlay_edit.load(Ordering::Relaxed);
            if toggle(
                ui,
                &mut edit,
                "Edit mode: move, resize and pick sections on screen",
                Some(
                    "The overlay stops being click-through and gets an accent border: drag it to move it, drag the \
                     corner grip to resize it and tick the sections to show. Outside a game it shows sample data. \
                     Done, Ctrl+Shift+E or this switch ends it and it is click-through again. Not while ARAM Mayhem \
                     augment cards are on screen.",
                ),
            ) {
                set_overlay_edit(ui.ctx(), &self.shared, edit);
            }
            ui.add_space(4.0);
            inset_frame().show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.label(RichText::new("Position and size").font(semibold(13.0)).color(TEXT));
                let monitor_width = ui.ctx().input(|i| i.viewport().monitor_size).map_or(1920.0, |m| m.x);
                let (mut pos, placed) = self.settings.resolve_overlay_pos(monitor_width);
                changed |= placed;
                let mut size = self.settings.overlay_size;
                let mut reset = false;
                ui.horizontal(|ui| {
                    ui.spacing_mut().interact_size = vec2(80.0, 26.0);
                    // In edit mode the overlay window is the source; it writes these values.
                    ui.add_enabled_ui(!edit, |ui| {
                        changed |= ui.add(DragValue::new(&mut pos[0]).range(-16000.0..=16000.0).prefix("x ").suffix(" px")).changed();
                        changed |= ui.add(DragValue::new(&mut pos[1]).range(-16000.0..=16000.0).prefix("y ").suffix(" px")).changed();
                        changed |= ui.add(DragValue::new(&mut size[0]).range(OVERLAY_MIN_SIZE[0]..=4000.0).prefix("w ").suffix(" px")).changed();
                        changed |= ui.add(DragValue::new(&mut size[1]).range(OVERLAY_MIN_SIZE[1]..=4000.0).prefix("h ").suffix(" px")).changed();
                        reset = ui.add(Btn::ghost("Reset").small()).on_hover_text("Default size, top-right corner").clicked();
                    });
                });
                if reset {
                    // Placed at the top-right corner again on the next pass.
                    self.settings.overlay_pos = None;
                    self.settings.overlay_size = OVERLAY_SIZE;
                    changed = true;
                } else {
                    self.settings.overlay_pos = Some(pos);
                    self.settings.overlay_size = size;
                }
                small_note(
                    ui,
                    if edit {
                        "Drag the overlay itself while edit mode is on; these follow."
                    } else {
                        "Top-left corner, measured from the primary monitor's top-left (pixels at 100% display scale). \
                         Edit mode sets these by dragging."
                    },
                );
                ui.add_space(4.0);
                ui.label(RichText::new("Sections").font(semibold(13.0)).color(TEXT));
                ui.horizontal_wrapped(|ui| {
                    for (label, on) in self.settings.overlay_sections.toggles() {
                        changed |= ui.checkbox(on, label).changed();
                    }
                });
            });
            ui.add_space(4.0);
            changed |= toggle(
                ui,
                &mut self.settings.augment_tiers,
                "ARAM Mayhem: show augment tiers under the offered cards",
                Some(
                    "Riot's game and client APIs do not list the offered augments, so riftlite copies the card titles' strip \
                     of the screen (like a screenshot, about five times a second, only while a Mayhem game is focused) and reads it \
                     with Windows' built-in OCR in the client's language. The S–D tiers come from Blitz \
                     (data.v2.iesdev.com, the data behind blitz.gg's augment tier list and champion pages): the large \
                     badge is the tier on your champion, the small one over all champions; no game data is sent. \
                     Riot's developer policy disallows augment win rates in overlays; use at your own discretion.",
                ),
            );
        });

        section(ui, "Behaviour");
        card(ui, |ui| {
            changed |= toggle(
                ui,
                &mut self.settings.switch_tabs_automatically,
                "Switch to Champ select / Live game / Post-game tabs automatically",
                None,
            );
            if toggle(ui, &mut self.autostart, "Start with Windows (in the tray)", None) {
                if let Err(e) = settings::set_autostart(self.autostart) {
                    self.autostart = settings::autostart_enabled();
                    self.notify(Err(format!("Autostart: {e}")));
                }
            }
        });
        if changed {
            self.save_settings();
        }

        self.goals_card(ui, s);
        self.replays_card(ui, s);

        section(ui, "League folder");
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.add(
                    TextEdit::singleline(&mut self.folder_input)
                        .hint_text("detected automatically")
                        .desired_width((ui.available_width() - 110.0).clamp(200.0, 560.0))
                        .margin(vec2(10.0, 7.0)),
                );
                if ui.add(Btn::primary("Apply").icon(Icon::Folder)).clicked() {
                    let p = std::path::PathBuf::from(self.folder_input.trim());
                    if !self.folder_input.trim().is_empty() && !p.join(Product::League.client_exe()).is_file() {
                        self.notify(Err(format!("{} not found in that folder", Product::League.client_exe())));
                    } else {
                        self.settings.league_folder = self.folder_input.trim().to_string();
                        self.save_settings();
                        self.shared.refresh.store(true, Ordering::Relaxed);
                        self.notify(Ok("League folder saved".into()));
                    }
                }
            });
            if let Some(i) = s.installs.get(Product::League) {
                small_note(ui, format!("In use: {} ({})", i.path.display(), i.source));
            }
        });

        section(ui, "Item sets");
        card(ui, |ui| {
            let root = s.league_root();
            if ui.add_enabled(root.is_some(), Btn::danger("Remove all item sets created by riftlite").icon(Icon::Trash)).clicked() {
                let r = itemset::clear_all(root.as_deref().expect("button enabled only with a root"))
                    .map(|n| format!("Removed {n} riftlite item set(s)"));
                self.notify(r);
            }
        });

        section(ui, "Data");
        card(ui, |ui| {
            ui.horizontal(|ui| {
                icon(ui, Icon::Folder, 16.0, MUTED);
                ui.label(RichText::new("Settings, goals, notes, matchup record, LP history, recorded skill orders and cache:").color(BODY));
                ui.label(RichText::new(settings::local_dir().display().to_string()).font(semibold(12.5)).color(TEXT));
            });
        });
    }

    /// Per-role goals: edited in a draft, written only by Save or Clear.
    fn goals_card(&mut self, ui: &mut Ui, s: &Snapshot) {
        section(ui, "Goals");
        card(ui, |ui| {
            let mut role = ROLES.into_iter().find(|r| *r == self.goals_role).unwrap_or("MIDDLE");
            if segmented(ui, &mut role, &ROLES.map(|r| (r, position_label(r)))) {
                self.goals_role = role.to_string();
                self.goals_draft = None;
            }
            let stored = s.goals.goals(role).copied();
            let draft = self.goals_draft.get_or_insert_with(|| stored.unwrap_or_default());
            ui.add_space(4.0);
            Grid::new("goals-editor").num_columns(2).spacing([16.0, 6.0]).show(ui, |ui| {
                ui.spacing_mut().interact_size = vec2(72.0, 24.0);
                rate_goal(ui, "CS per minute", &mut draft.cs_per_min, 7.0, 12.0);
                count_goal(ui, "Most deaths", &mut draft.max_deaths, 4, 20);
                rate_goal(ui, "Vision score per minute", &mut draft.vision_per_min, 1.0, 5.0);
                count_goal(ui, "Control wards bought", &mut draft.control_wards, 2, 10);
            });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                let dirty = *draft != stored.unwrap_or_default();
                if ui.add_enabled(dirty, Btn::primary("Save").small()).clicked() {
                    self.shared.request(Action::SetGoals { role: role.to_string(), goals: *draft });
                }
                if ui.add_enabled(stored.is_some(), Btn::ghost("Clear").small()).clicked() {
                    *draft = Goals::default();
                    self.shared.request(Action::SetGoals { role: role.to_string(), goals: Goals::default() });
                }
                if dirty {
                    ui.label(RichText::new("Unsaved changes").size(11.5).color(SOON));
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    let streak = s.goals.streak(Some(role));
                    let color = if streak > 0 { WIN } else { MUTED };
                    ui.label(RichText::new(format!("Streak: {streak} game(s) with every goal met")).size(11.5).color(color));
                });
            });
            small_note(
                ui,
                "The Live tab and the overlay show how the game is going against these; Post-game checks them, \
                 control wards included (the live game does not report purchases). Kept on this PC (goals.json).",
            );
        });
    }

    /// The game's Replay API switch, written to game.cfg only by the button.
    fn replays_card(&mut self, ui: &mut Ui, s: &Snapshot) {
        section(ui, "Replays");
        card(ui, |ui| {
            ui.horizontal(|ui| {
                match s.replay_api {
                    Some(true) => status_msg(ui, true, "Replay API on"),
                    Some(false) => status_msg(ui, false, "Replay API off"),
                    None => status_msg(ui, false, "game.cfg not found"),
                };
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add_enabled(s.replay_api != Some(true), Btn::primary("Enable Replay API").small()).clicked() {
                        self.shared.request(Action::EnableReplayApi);
                    }
                });
            });
            small_note(
                ui,
                "Enabling writes EnableReplayApi=1 under [General] in <League>/Config/game.cfg, only when you click the button; \
                 it takes effect the next time the game starts. It lets riftlite jump in an open replay and record clips \
                 through the game's local Replay API (127.0.0.1:2999); nothing is sent anywhere.",
            );
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                icon(ui, Icon::Folder, 16.0, MUTED);
                ui.label(RichText::new("Clips:").color(BODY));
                ui.label(RichText::new(crate::replay::clips_dir().display().to_string()).font(semibold(12.5)).color(TEXT));
            });
        });
    }
}

fn void_msg(ui: &mut Ui, ok: bool, text: &str) {
    ui.horizontal(|ui| {
        ui.add_space(10.0);
        status_msg(ui, ok, text);
    });
}

/// A per-minute goal row: the checkbox turns it on (at `default`) or off; values move in tenths.
fn rate_goal(ui: &mut Ui, label: &str, goal: &mut Option<f64>, default: f64, max: f64) {
    goal_check(ui, label, goal, default);
    let mut v = goal.unwrap_or(default);
    if ui.add_enabled(goal.is_some(), DragValue::new(&mut v).range(0.0..=max).speed(0.05).fixed_decimals(1)).changed() {
        *goal = Some((v * 10.0).round() / 10.0);
    }
    ui.end_row();
}

/// A count goal row: the checkbox turns it on (at `default`) or off.
fn count_goal(ui: &mut Ui, label: &str, goal: &mut Option<i64>, default: i64, max: i64) {
    goal_check(ui, label, goal, default);
    let mut v = goal.unwrap_or(default);
    if ui.add_enabled(goal.is_some(), DragValue::new(&mut v).range(0..=max).speed(0.1)).changed() {
        *goal = Some(v);
    }
    ui.end_row();
}

fn goal_check<T: Copy>(ui: &mut Ui, label: &str, goal: &mut Option<T>, default: T) {
    let mut on = goal.is_some();
    if ui.checkbox(&mut on, RichText::new(label).color(BODY)).changed() {
        *goal = on.then_some(default);
    }
}
