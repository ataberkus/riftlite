//! LP after each ranked game riftlite saw end, from the client's LP-change notification.
//! Kept in `%LOCALAPPDATA%\riftlite\lp.json` (newest last, capped); nothing leaves the machine.
//! The client keeps no LP history itself, so games played without riftlite are missing.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Entries kept; older ones are dropped.
const MAX: usize = 500;
/// A pause longer than this ends a play session.
const SESSION_GAP_MS: i64 = 3 * 60 * 60 * 1000;
/// Recent wins averaged for the LP-per-win estimate.
const GAIN_SAMPLE: usize = 20;

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct LpEntry {
    pub game_id: u64,
    /// When the game ended (Unix ms).
    pub at_ms: i64,
    /// `RANKED_SOLO_5x5`, `RANKED_FLEX_SR`…
    pub queue: String,
    pub tier: String,
    pub division: String,
    /// LP after the game.
    pub lp: i64,
    pub delta: i64,
    pub win: bool,
}

fn path() -> PathBuf {
    crate::settings::local_dir().join("lp.json")
}

pub fn load() -> Vec<LpEntry> {
    std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Adds a game (once per game id) and returns the whole log.
pub fn record(entry: LpEntry) -> Result<Vec<LpEntry>, String> {
    let mut all = load();
    if all.iter().any(|e| e.game_id == entry.game_id) {
        return Ok(all);
    }
    all.push(entry);
    let excess = all.len().saturating_sub(MAX);
    all.drain(..excess);
    let p = path();
    std::fs::create_dir_all(p.parent().expect("lp path has a parent")).map_err(|e| e.to_string())?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&all).expect("lp serialize"))
        .and_then(|_| std::fs::rename(&tmp, &p))
        .map_err(|e| e.to_string())?;
    Ok(all)
}

/// The current play session: the trailing games without a long pause, if the last one is recent.
pub fn session(all: &[LpEntry], now_ms: i64) -> &[LpEntry] {
    if all.last().is_none_or(|e| now_ms - e.at_ms > SESSION_GAP_MS) {
        return &[];
    }
    let start = all.windows(2).rposition(|w| w[1].at_ms - w[0].at_ms > SESSION_GAP_MS).map_or(0, |i| i + 1);
    &all[start..]
}

/// Average LP of the recent wins in a queue and how many wins that is; `None` without any.
pub fn avg_gain(all: &[LpEntry], queue: &str) -> Option<(f64, usize)> {
    let gains: Vec<i64> = all.iter().rev().filter(|e| e.queue == queue && e.win && e.delta > 0).take(GAIN_SAMPLE).map(|e| e.delta).collect();
    (!gains.is_empty()).then(|| (gains.iter().sum::<i64>() as f64 / gains.len() as f64, gains.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(at_ms: i64) -> LpEntry {
        LpEntry { game_id: at_ms as u64, at_ms, queue: "RANKED_SOLO_5x5".into(), tier: "GOLD".into(), division: "II".into(), lp: 0, delta: 20, win: true }
    }

    #[test]
    fn session_starts_after_the_last_long_pause_and_ends_after_one() {
        let h = 60 * 60 * 1000;
        let all = [at(0), at(10 * h), at(11 * h), at(12 * h)];
        assert_eq!(session(&all, 12 * h + 1).len(), 3);
        assert!(session(&all, 16 * h).is_empty());
    }
}
