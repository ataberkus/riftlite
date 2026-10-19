//! Skill orders the player actually used, recorded from the Live Client API's
//! ability levels while riftlite watches a game, and replayed as a "next skill"
//! hint in later games on the same champion. Stored per champion in
//! `%LOCALAPPDATA%\riftlite\skills.json`; nothing leaves the machine.

use std::collections::HashMap;
use std::path::PathBuf;

const KEYS: [char; 4] = ['Q', 'W', 'E', 'R'];
/// Shorter recordings (early leave, remake) are not worth replaying.
const MIN_SAVED: usize = 6;

fn path() -> PathBuf {
    crate::settings::local_dir().join("skills.json")
}

/// Champion key (Data Dragon id) → skill order string such as `QEWQQRQEQE`.
fn load() -> HashMap<String, String> {
    std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn save(all: &HashMap<String, String>) -> Result<(), String> {
    let p = path();
    std::fs::create_dir_all(p.parent().expect("skills path has a parent")).map_err(|e| e.to_string())?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(all).expect("skills serialize"))
        .and_then(|_| std::fs::rename(&tmp, &p))
        .map_err(|e| e.to_string())
}

/// An order is recorded for the champion.
pub fn has(champion_key: &str) -> bool {
    load().contains_key(champion_key)
}

/// Saves an order learned from match history when the champion has none yet (a live
/// recording is never overwritten) and it is at least `MIN_SAVED` points long. Returns whether
/// it was saved.
pub fn backfill(champion_key: &str, order: &str) -> Result<bool, String> {
    let mut all = load();
    if !learn(&mut all, champion_key, order) {
        return Ok(false);
    }
    save(&all).map(|_| true).map_err(|e| format!("skills: {e}"))
}

fn learn(all: &mut HashMap<String, String>, champion_key: &str, order: &str) -> bool {
    if champion_key.is_empty() || order.chars().count() < MIN_SAVED || all.contains_key(champion_key) {
        return false;
    }
    all.insert(champion_key.to_string(), order.to_string());
    true
}

#[derive(Clone, PartialEq, Debug)]
pub struct SkillHint {
    /// Next key from the recorded order, when a point is unspent.
    pub next: Option<char>,
    /// The recorded order this game is following.
    pub recorded: String,
    /// Order taken so far this game.
    pub current: String,
}

#[derive(Default)]
pub struct Recorder {
    champion: String,
    last: [i64; 4],
    seq: String,
    reference: Option<String>,
}

impl Recorder {
    /// Feeds one live sample; returns the hint for the overlay.
    pub fn observe(&mut self, champion: &str, levels: [i64; 4], unspent: i64) -> Option<SkillHint> {
        if champion.is_empty() {
            return None;
        }
        if champion != self.champion {
            self.finish();
            self.champion = champion.to_string();
            self.reference = load().remove(champion);
        }
        for (i, (&now, &before)) in levels.iter().zip(self.last.iter()).enumerate() {
            for _ in before..now {
                self.seq.push(KEYS[i]);
            }
        }
        self.last = levels;
        let reference = self.reference.clone()?;
        let next = if unspent > 0 { reference.chars().nth(self.seq.chars().count()) } else { None };
        Some(SkillHint { next, recorded: reference, current: self.seq.clone() })
    }

    /// Game over: keep this game's order for the champion.
    pub fn finish(&mut self) {
        if self.seq.chars().count() >= MIN_SAVED && !self.champion.is_empty() {
            let mut all = load();
            all.insert(std::mem::take(&mut self.champion), std::mem::take(&mut self.seq));
            // Losing a recording is harmless; the next game records again.
            let _ = save(&all);
        }
        *self = Recorder::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_level_ups_in_order_and_hints_from_reference() {
        let mut r = Recorder { champion: "Ziggs".into(), reference: Some("QEWQQRQ".into()), ..Default::default() };
        assert_eq!(r.observe("Ziggs", [1, 0, 0, 0], 0).unwrap().next, None);
        // Two points at once (level 3 reached between polls) keep Q,W,E order.
        let h = r.observe("Ziggs", [1, 1, 1, 0], 1).unwrap();
        assert_eq!(h.current, "QWE");
        assert_eq!(h.next, Some('Q'));
    }

    #[test]
    fn backfill_fills_gaps_only_and_needs_a_full_enough_order() {
        let mut all = HashMap::from([("Ziggs".to_string(), "QWEQQRQ".to_string())]);
        // A recording exists (live or earlier backfill): match history never replaces it.
        assert!(!learn(&mut all, "Ziggs", "EQWEERE"));
        assert_eq!(all["Ziggs"], "QWEQQRQ");
        // A remake's five points are not worth replaying.
        assert!(!learn(&mut all, "Ahri", "QWEQQ"));
        assert!(!all.contains_key("Ahri"));
        assert!(learn(&mut all, "Ahri", "QWEQQR"));
        assert_eq!(all["Ahri"], "QWEQQR");
        assert!(!learn(&mut all, "", "QWEQQRQ"));
    }
}
