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
/// Tiers with divisions, lowest first; Master, Grandmaster and Challenger follow them.
pub const TIERS: [&str; 7] = ["IRON", "BRONZE", "SILVER", "GOLD", "PLATINUM", "EMERALD", "DIAMOND"];
/// Divisions within a tier, lowest first.
pub const DIVISIONS: [&str; 4] = ["IV", "III", "II", "I"];
/// LP by which one game's LP before may differ from the previous game's LP after before the
/// chart marks a gap (unlogged games or LP changes).
const GAP_LP: i64 = 2;

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

/// Position on one LP scale: Iron IV 0 LP = 0, +100 per division, so Master 0 LP = 2800;
/// Master, Grandmaster and Challenger share that ladder by LP. `None` when unranked or unknown.
pub fn ladder(tier: &str, division: &str, lp: i64) -> Option<i64> {
    if matches!(tier, "MASTER" | "GRANDMASTER" | "CHALLENGER") {
        return Some((TIERS.len() * DIVISIONS.len()) as i64 * 100 + lp);
    }
    let t = TIERS.iter().position(|&x| x == tier)?;
    let d = DIVISIONS.iter().position(|&x| x == division)?;
    Some(((t * DIVISIONS.len() + d) * 100) as i64 + lp)
}

#[derive(Clone, PartialEq, Debug)]
pub struct ChartPoint {
    pub at_ms: i64,
    /// [`ladder`] value after the game.
    pub value: i64,
    pub win: bool,
    pub delta: i64,
    /// The LP before this game is not the previous entry's LP after it (unlogged games, decay,
    /// season reset, demotion to 75 LP).
    pub gap_before: bool,
}

/// One queue's logged games on the [`ladder`] scale, oldest first; entries without a known rank
/// are skipped.
pub fn chart(all: &[LpEntry], queue: &str) -> Vec<ChartPoint> {
    let mut out: Vec<ChartPoint> = Vec::new();
    for e in all.iter().filter(|e| e.queue == queue) {
        let Some(value) = ladder(&e.tier, &e.division, e.lp) else { continue };
        let gap_before = out.last().is_some_and(|prev| ((value - e.delta) - prev.value).abs() > GAP_LP);
        out.push(ChartPoint { at_ms: e.at_ms, value, win: e.win, delta: e.delta, gap_before });
    }
    out
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

    fn game(id: u64, queue: &str, tier: &str, division: &str, lp: i64, delta: i64) -> LpEntry {
        LpEntry { game_id: id, at_ms: id as i64, queue: queue.into(), tier: tier.into(), division: division.into(), lp, delta, win: delta > 0 }
    }

    #[test]
    fn ladder_counts_divisions_and_puts_apex_tiers_on_one_scale() {
        assert_eq!(ladder("IRON", "IV", 0), Some(0));
        assert_eq!(ladder("GOLD", "I", 90), Some(1590));
        assert_eq!(ladder("PLATINUM", "IV", 10), Some(1610));
        // Apex tiers have no divisions (the client reports "I" or nothing); LP alone orders them.
        assert_eq!(ladder("MASTER", "I", 0), Some(2800));
        assert_eq!(ladder("GRANDMASTER", "", 450), Some(3250));
        assert_eq!(ladder("", "", 0), None);
        assert_eq!(ladder("UNRANKED", "", 0), None);
        assert_eq!(ladder("GOLD", "NA", 50), None);
    }

    #[test]
    fn chart_keeps_promotions_continuous_and_flags_gaps() {
        const SOLO: &str = "RANKED_SOLO_5x5";
        let all = [
            game(1, SOLO, "GOLD", "I", 70, 20),
            game(2, SOLO, "GOLD", "I", 90, 20),
            // Promotion: Gold I 90 + 20 = Platinum IV 10, the same 1610 on the ladder.
            game(3, SOLO, "PLATINUM", "IV", 10, 20),
            game(4, "RANKED_FLEX_SR", "SILVER", "II", 40, -15),
            // Games without riftlite: 50 − 22 = 28 LP before this one, not the 10 logged last.
            game(5, SOLO, "PLATINUM", "IV", 50, 22),
            game(6, SOLO, "DIAMOND", "I", 95, 25),
            // Diamond I 95 + 25 = Master 20: continuous across the apex boundary.
            game(7, SOLO, "MASTER", "I", 20, 25),
            game(8, SOLO, "", "", 0, 0),
        ];
        let c = chart(&all, SOLO);
        assert_eq!(c.iter().map(|p| p.value).collect::<Vec<_>>(), [1570, 1590, 1610, 1650, 2795, 2820], "other queue and unranked entries skipped");
        assert_eq!(c.iter().map(|p| p.gap_before).collect::<Vec<_>>(), [false, false, false, true, true, false]);
        assert_eq!(chart(&all, "RANKED_FLEX_SR").len(), 1);
    }
}
