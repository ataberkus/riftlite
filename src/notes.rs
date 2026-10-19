//! Your notes on champions and lane matchups, kept in `%LOCALAPPDATA%\riftlite\notes.json`;
//! nothing leaves the machine.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::ddragon::StaticData;

/// Note key → text.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(transparent)]
pub struct Notes {
    notes: BTreeMap<String, String>,
}

/// Key of the note on one of your champions.
pub fn champ_key(champion: i64) -> String {
    format!("champ:{champion}")
}

/// Key of the note on your champion against an enemy champion.
pub fn vs_key(mine: i64, theirs: i64) -> String {
    format!("vs:{mine}:{theirs}")
}

fn path() -> PathBuf {
    crate::settings::local_dir().join("notes.json")
}

impl Notes {
    pub fn load() -> Notes {
        std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let p = path();
        std::fs::create_dir_all(p.parent().expect("notes path has a parent")).map_err(|e| format!("notes: {e}"))?;
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).expect("notes serialize"))
            .and_then(|_| std::fs::rename(&tmp, &p))
            .map_err(|e| format!("notes: {e}"))
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.notes.get(key).map(String::as_str)
    }

    /// Stores the trimmed text; an empty text removes the note.
    pub fn set(&mut self, key: &str, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            self.notes.remove(key);
        } else {
            self.notes.insert(key.to_string(), text.to_string());
        }
    }

    /// Notes for a game as (key, title, text): your champion's note, then the lane opponent's
    /// matchup note, then the other enemies' matchup notes in `enemies` order. Missing notes are
    /// left out.
    pub fn for_game(&self, mine: i64, enemies: &[i64], lane_opponent: Option<i64>, data: &StaticData) -> Vec<(String, String, String)> {
        let mine_name = data.champion_name(mine);
        let champ = std::iter::once((champ_key(mine), mine_name.clone()));
        let others = enemies.iter().copied().filter(|&e| Some(e) != lane_opponent);
        let matchups = lane_opponent.into_iter().chain(others).map(|e| (vs_key(mine, e), format!("{mine_name} vs {}", data.champion_name(e))));
        champ.chain(matchups).filter_map(|(key, title)| self.get(&key).map(|text| text.to_string()).map(|text| (key, title, text))).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ddragon::Champion;

    fn data() -> StaticData {
        let mut d = StaticData::default();
        for (id, name) in [(103, "Ahri"), (238, "Zed"), (7, "LeBlanc"), (1, "Annie")] {
            d.champions.insert(id, Champion { name: name.into(), ..Default::default() });
        }
        d
    }

    #[test]
    fn for_game_puts_champion_then_lane_opponent_then_other_enemies() {
        let mut n = Notes::default();
        n.set(&vs_key(103, 1), "stay out of stun range");
        n.set(&vs_key(103, 238), "  buy Seeker's early  ");
        n.set(&champ_key(103), "max Q");
        n.set(&champ_key(238), "not my champion this game");
        n.set(&vs_key(238, 103), "the other way round");
        let got = n.for_game(103, &[1, 7, 238], Some(238), &data());
        assert_eq!(
            got,
            [
                ("champ:103".to_string(), "Ahri".to_string(), "max Q".to_string()),
                // The lane opponent comes first even though it is listed last; text is trimmed.
                ("vs:103:238".to_string(), "Ahri vs Zed".to_string(), "buy Seeker's early".to_string()),
                ("vs:103:1".to_string(), "Ahri vs Annie".to_string(), "stay out of stun range".to_string()),
            ]
        );
        // Without a lane opponent the enemies keep their order.
        let keys: Vec<String> = n.for_game(103, &[238, 1], None, &data()).into_iter().map(|x| x.0).collect();
        assert_eq!(keys, ["champ:103", "vs:103:238", "vs:103:1"]);
    }

    #[test]
    fn setting_blank_text_removes_the_note() {
        let mut n = Notes::default();
        n.set(&champ_key(103), "max Q");
        n.set(&champ_key(103), "   ");
        assert_eq!(n.get(&champ_key(103)), None);
        assert!(n.for_game(103, &[], None, &data()).is_empty());
    }
}
