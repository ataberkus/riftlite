//! Personal per-role goals (CS per minute, deaths, vision) and their streaks, kept in
//! `%LOCALAPPDATA%\riftlite\goals.json` next to the settings; nothing leaves the machine.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Game results kept; the oldest are dropped.
const MAX_RESULTS: usize = 500;
/// A per-minute rate this close to its target (fraction of the target) is on track.
const ON_TRACK: f64 = 0.05;

/// Targets for one role; `None` = no goal.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug, Default)]
#[serde(default)]
pub struct Goals {
    pub cs_per_min: Option<f64>,
    /// Most deaths allowed.
    pub max_deaths: Option<i64>,
    /// Vision score per minute.
    pub vision_per_min: Option<f64>,
    /// Control wards bought.
    pub control_wards: Option<i64>,
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
pub enum GoalKind {
    CsPerMin,
    MaxDeaths,
    VisionPerMin,
    ControlWards,
}

impl GoalKind {
    pub fn label(self) -> &'static str {
        match self {
            GoalKind::CsPerMin => "CS/min",
            GoalKind::MaxDeaths => "Deaths",
            GoalKind::VisionPerMin => "Vision/min",
            GoalKind::ControlWards => "Control wards",
        }
    }
}

/// One goal checked against a finished game.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Check {
    pub kind: GoalKind,
    pub target: f64,
    pub actual: f64,
    pub met: bool,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct GameResult {
    pub game_id: u64,
    /// When the game ended (Unix ms).
    pub at_ms: i64,
    /// Canonical position.
    pub role: String,
    pub checks: Vec<Check>,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(default)]
pub struct Store {
    /// Canonical position → goals.
    pub roles: BTreeMap<String, Goals>,
    /// Oldest first, capped.
    pub results: Vec<GameResult>,
}

/// Your numbers in a game, finished or so far.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Stats {
    pub cs: i64,
    pub deaths: i64,
    pub vision: f64,
    /// `None` live: the Live Client API does not report purchases.
    pub control_wards: Option<i64>,
    pub minutes: f64,
}

impl Goals {
    pub fn is_empty(&self) -> bool {
        *self == Goals::default()
    }

    /// Every set goal against a finished game; control wards are skipped when unknown, and
    /// rates use at least one minute.
    pub fn evaluate(&self, s: &Stats) -> Vec<Check> {
        let minutes = s.minutes.max(1.0);
        let check = |kind, target: f64, actual: f64, met: bool| Check { kind, target, actual, met };
        let mut out = Vec::new();
        if let Some(t) = self.cs_per_min {
            let actual = s.cs as f64 / minutes;
            out.push(check(GoalKind::CsPerMin, t, actual, actual >= t));
        }
        if let Some(max) = self.max_deaths {
            out.push(check(GoalKind::MaxDeaths, max as f64, s.deaths as f64, s.deaths <= max));
        }
        if let Some(t) = self.vision_per_min {
            let actual = s.vision / minutes;
            out.push(check(GoalKind::VisionPerMin, t, actual, actual >= t));
        }
        if let (Some(t), Some(n)) = (self.control_wards, s.control_wards) {
            out.push(check(GoalKind::ControlWards, t as f64, n as f64, n >= t));
        }
        out
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Pace {
    Ahead,
    OnTrack,
    Behind,
    /// Cannot be met any more this game.
    Failed,
}

/// Each set goal against the game so far: per-minute goals compare the rate so far (on track
/// within 5 % of the target); deaths are on track below the maximum, behind at it and failed
/// above it; a control ward count has no pace, so it is ahead once met and on track until then
/// (skipped when unknown).
pub fn live_pace(goals: &Goals, s: &Stats) -> Vec<(GoalKind, f64, f64, Pace)> {
    let minutes = s.minutes.max(1.0);
    let rate = |target: f64, actual: f64| {
        if actual > target * (1.0 + ON_TRACK) {
            Pace::Ahead
        } else if actual < target * (1.0 - ON_TRACK) {
            Pace::Behind
        } else {
            Pace::OnTrack
        }
    };
    let mut out = Vec::new();
    if let Some(t) = goals.cs_per_min {
        let actual = s.cs as f64 / minutes;
        out.push((GoalKind::CsPerMin, t, actual, rate(t, actual)));
    }
    if let Some(max) = goals.max_deaths {
        let pace = match s.deaths.cmp(&max) {
            std::cmp::Ordering::Less => Pace::OnTrack,
            std::cmp::Ordering::Equal => Pace::Behind,
            std::cmp::Ordering::Greater => Pace::Failed,
        };
        out.push((GoalKind::MaxDeaths, max as f64, s.deaths as f64, pace));
    }
    if let Some(t) = goals.vision_per_min {
        let actual = s.vision / minutes;
        out.push((GoalKind::VisionPerMin, t, actual, rate(t, actual)));
    }
    if let (Some(t), Some(n)) = (goals.control_wards, s.control_wards) {
        out.push((GoalKind::ControlWards, t as f64, n as f64, if n >= t { Pace::Ahead } else { Pace::OnTrack }));
    }
    out
}

fn path() -> PathBuf {
    crate::settings::local_dir().join("goals.json")
}

impl Store {
    pub fn load() -> Store {
        std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let p = path();
        std::fs::create_dir_all(p.parent().expect("goals path has a parent")).map_err(|e| format!("goals: {e}"))?;
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).expect("goals serialize"))
            .and_then(|_| std::fs::rename(&tmp, &p))
            .map_err(|e| format!("goals: {e}"))
    }

    /// The goals of a canonical role, when any is set.
    pub fn goals(&self, role: &str) -> Option<&Goals> {
        self.roles.get(role).filter(|g| !g.is_empty())
    }

    /// Adds a game's result once per game id; results without checks are ignored. Returns
    /// whether it was added.
    pub fn record(&mut self, r: GameResult) -> bool {
        if r.checks.is_empty() || self.results.iter().any(|x| x.game_id == r.game_id) {
            return false;
        }
        self.results.push(r);
        let excess = self.results.len().saturating_sub(MAX_RESULTS);
        self.results.drain(..excess);
        true
    }

    /// Newest games in a row (of `role`, or any) with every goal met.
    pub fn streak(&self, role: Option<&str>) -> usize {
        self.results.iter().rev().filter(|r| role.is_none_or(|x| r.role == x)).take_while(|r| r.checks.iter().all(|c| c.met)).count()
    }

    /// Newest games in a row that had this goal and met it; games without it are skipped.
    pub fn kind_streak(&self, kind: GoalKind) -> usize {
        self.results
            .iter()
            .rev()
            .filter_map(|r| r.checks.iter().find(|c| c.kind == kind))
            .take_while(|c| c.met)
            .count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(cs: i64, deaths: i64, vision: f64, control_wards: Option<i64>, minutes: f64) -> Stats {
        Stats { cs, deaths, vision, control_wards, minutes }
    }

    fn all_goals() -> Goals {
        Goals { cs_per_min: Some(7.0), max_deaths: Some(4), vision_per_min: Some(1.0), control_wards: Some(2) }
    }

    #[test]
    fn evaluate_uses_rates_per_minute_and_skips_unknown_control_wards() {
        // 30 minutes: 225 CS = 7.5/min, 4 deaths is exactly the maximum, 24 vision = 0.8/min.
        let checks = all_goals().evaluate(&stats(225, 4, 24.0, Some(1), 30.0));
        let got: Vec<(GoalKind, f64, bool)> = checks.iter().map(|c| (c.kind, c.actual, c.met)).collect();
        assert_eq!(
            got,
            [(GoalKind::CsPerMin, 7.5, true), (GoalKind::MaxDeaths, 4.0, true), (GoalKind::VisionPerMin, 0.8, false), (GoalKind::ControlWards, 1.0, false)]
        );
        let live = all_goals().evaluate(&stats(225, 5, 24.0, None, 30.0));
        assert_eq!(live.len(), 3, "control wards unknown: no check rather than a fail");
        assert!(!live[1].met, "5 deaths is over a maximum of 4");
        // A 20-second game does not turn 3 CS into 9 CS/min.
        assert_eq!(all_goals().evaluate(&stats(3, 0, 0.0, None, 1.0 / 3.0))[0].actual, 3.0);
        assert!(Goals::default().evaluate(&stats(100, 0, 10.0, Some(3), 20.0)).is_empty());
        assert!(Goals::default().is_empty() && !all_goals().is_empty());
    }

    #[test]
    fn live_pace_compares_rate_so_far_and_fails_deaths_over_the_max() {
        let pace = |s: Stats| live_pace(&all_goals(), &s).into_iter().map(|p| (p.0, p.3)).collect::<Vec<_>>();
        // 10 minutes: 68 CS is within 5 % of 70, 12 vision is ahead of 10, 4 deaths is at the max.
        assert_eq!(pace(stats(68, 4, 12.0, None, 10.0)), [(GoalKind::CsPerMin, Pace::OnTrack), (GoalKind::MaxDeaths, Pace::Behind), (GoalKind::VisionPerMin, Pace::Ahead)]);
        assert_eq!(pace(stats(60, 5, 9.0, None, 10.0)), [(GoalKind::CsPerMin, Pace::Behind), (GoalKind::MaxDeaths, Pace::Failed), (GoalKind::VisionPerMin, Pace::Behind)]);
        assert_eq!(pace(stats(80, 0, 10.0, Some(2), 10.0))[3], (GoalKind::ControlWards, Pace::Ahead));
        assert_eq!(pace(stats(80, 0, 10.0, Some(1), 10.0))[1], (GoalKind::MaxDeaths, Pace::OnTrack));
    }

    fn result(game_id: u64, role: &str, met: &[(GoalKind, bool)]) -> GameResult {
        let checks = met.iter().map(|&(kind, met)| Check { kind, target: 1.0, actual: if met { 1.0 } else { 0.0 }, met }).collect();
        GameResult { game_id, at_ms: game_id as i64, role: role.into(), checks }
    }

    #[test]
    fn streaks_follow_the_role_filter_and_stop_at_a_miss() {
        use GoalKind::*;
        let mut s = Store::default();
        assert!(s.record(result(1, "MIDDLE", &[(CsPerMin, false), (MaxDeaths, true)])));
        assert!(s.record(result(2, "MIDDLE", &[(CsPerMin, true), (MaxDeaths, true)])));
        assert!(s.record(result(3, "TOP", &[(CsPerMin, false), (MaxDeaths, true)])));
        assert!(s.record(result(4, "MIDDLE", &[(CsPerMin, true)])));
        assert!(s.record(result(5, "MIDDLE", &[(VisionPerMin, true)])));
        assert!(!s.record(result(5, "MIDDLE", &[(VisionPerMin, false)])), "once per game");
        assert!(!s.record(result(6, "MIDDLE", &[])), "a game without goals is no result");
        // The top game missed CS: it breaks the any-role streak but not the mid one.
        assert_eq!(s.streak(None), 2);
        assert_eq!(s.streak(Some("MIDDLE")), 3);
        assert_eq!(s.streak(Some("JUNGLE")), 0);
        // Game 5 had no CS goal, so it neither extends nor breaks the CS streak.
        assert_eq!(s.kind_streak(CsPerMin), 1);
        assert_eq!(s.kind_streak(MaxDeaths), 3);
        assert_eq!(s.kind_streak(ControlWards), 0);
    }
}
