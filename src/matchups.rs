//! Your own record against each enemy champion, built from match history and kept in
//! `%LOCALAPPDATA%\riftlite\matchups.json`; nothing leaves the machine.

use std::collections::{BTreeSet, HashMap};
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::lcu::{LANE_QUEUES, MatchDetail};
use crate::meta::wilson_upper;

/// Entries kept (about 1000 games); the oldest are dropped.
const MAX_ENTRIES: usize = 5000;
/// Game ids remembered as already handled; the smallest (oldest) are dropped.
const MAX_SEEN: usize = 20_000;
/// Shorter games are remakes.
const REMAKE_S: i64 = 300;
/// Personal records below this many games are flagged as few games.
const FEW_GAMES: u64 = 5;

/// One enemy in one of your games.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Entry {
    pub game_id: u64,
    pub created_ms: i64,
    pub queue_id: i64,
    pub my_champion: i64,
    pub enemy: i64,
    pub win: bool,
    /// The enemy played your position (both known).
    pub lane: bool,
    /// Your canonical position, `""` when unknown.
    pub position: String,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug, Default)]
#[serde(default)]
pub struct Store {
    /// Oldest first; one entry per (game id, enemy).
    entries: Vec<Entry>,
    /// Every game handled, with or without entries, so it is not fetched again.
    seen: BTreeSet<u64>,
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Record {
    pub games: u64,
    pub wins: u64,
}

impl Record {
    pub fn losses(&self) -> u64 {
        self.games - self.wins
    }

    /// 0..=1; 0 without games.
    pub fn win_rate(&self) -> f64 {
        if self.games == 0 { 0.0 } else { self.wins as f64 / self.games as f64 }
    }

    /// Under 5 games. Personal samples are always small, so OP.GG's margin rule would flag
    /// everything.
    pub fn few_games(&self) -> bool {
        self.games < FEW_GAMES
    }

    fn add(&mut self, win: bool) {
        self.games += 1;
        self.wins += win as u64;
    }
}

fn path() -> PathBuf {
    crate::settings::local_dir().join("matchups.json")
}

impl Store {
    pub fn load() -> Store {
        std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let p = path();
        std::fs::create_dir_all(p.parent().expect("matchups path has a parent")).map_err(|e| format!("matchups: {e}"))?;
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(self).expect("matchups serialize"))
            .and_then(|_| std::fs::rename(&tmp, &p))
            .map_err(|e| format!("matchups: {e}"))
    }

    pub fn has_game(&self, game_id: u64) -> bool {
        self.seen.contains(&game_id)
    }

    /// Remembers a game with no usable data (other mode, remake, fetch failed for good) so it is
    /// not fetched again.
    pub fn mark_seen(&mut self, game_id: u64) {
        self.seen.insert(game_id);
        while self.seen.len() > MAX_SEEN {
            self.seen.pop_first();
        }
    }

    /// One entry per enemy-team player of a Summoner's Rift lane-queue game; other queues and
    /// remakes are only marked seen. Needs the viewer (`d.me()`). Returns whether anything changed.
    pub fn add_game(&mut self, d: &MatchDetail) -> bool {
        if self.has_game(d.game_id) {
            return false;
        }
        let Some(me) = d.me() else { return false };
        if LANE_QUEUES.contains(&d.queue_id) && d.duration_s >= REMAKE_S {
            self.entries.extend(d.players.iter().filter(|p| p.team != me.team).map(|p| Entry {
                game_id: d.game_id,
                created_ms: d.created_ms,
                queue_id: d.queue_id,
                my_champion: me.champion_id,
                enemy: p.champion_id,
                win: me.win,
                lane: !me.position.is_empty() && me.position == p.position,
                position: me.position.to_string(),
            }));
            self.entries.sort_by_key(|e| (e.created_ms, e.game_id, e.enemy));
            let excess = self.entries.len().saturating_sub(MAX_ENTRIES);
            self.entries.drain(..excess);
        }
        self.mark_seen(d.game_id);
        true
    }

    /// Your games against `enemy`, optionally only on one champion and only when it was your
    /// lane opponent.
    pub fn record_vs(&self, enemy: i64, my_champion: Option<i64>, lane_only: bool) -> Record {
        let mut r = Record::default();
        for e in self.entries.iter().filter(|e| e.enemy == enemy && my_champion.is_none_or(|c| e.my_champion == c) && (e.lane || !lane_only)) {
            r.add(e.win);
        }
        r
    }

    /// Enemies you lose to (any lane, any champion of yours): at least `min_games` games and
    /// more losses than wins, worst first by the Wilson upper bound (bad even allowing for few
    /// games), then most games; at most `n`, none from `exclude`.
    pub fn ban_hints(&self, min_games: u64, n: usize, exclude: &[i64]) -> Vec<(i64, Record)> {
        let mut by_enemy: HashMap<i64, Record> = HashMap::new();
        for e in self.entries.iter().filter(|e| !exclude.contains(&e.enemy)) {
            by_enemy.entry(e.enemy).or_default().add(e.win);
        }
        let mut out: Vec<(i64, Record)> = by_enemy.into_iter().filter(|(_, r)| r.games >= min_games && r.wins < r.losses()).collect();
        out.sort_by(|a, b| {
            wilson_upper(a.1.wins, a.1.games)
                .total_cmp(&wilson_upper(b.1.wins, b.1.games))
                .then(b.1.games.cmp(&a.1.games))
                .then(a.0.cmp(&b.0))
        });
        out.truncate(n);
        out
    }

    /// Stored entries (one per enemy per game).
    pub fn len(&self) -> usize {
        self.entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lcu::MatchPlayer;

    fn player(champion_id: i64, team: i64, position: &'static str, is_me: bool) -> MatchPlayer {
        MatchPlayer { champion_id, team, position, is_me, win: team == 100, ..Default::default() }
    }

    /// You (champion 1, mid, blue) against red champions `enemies`, the first one mid.
    fn game(game_id: u64, queue_id: i64, my_win: bool, enemies: &[i64]) -> MatchDetail {
        let mut players = vec![player(1, 100, "MIDDLE", true), player(2, 100, "TOP", false)];
        players[0].win = my_win;
        players.extend(enemies.iter().enumerate().map(|(i, &c)| player(c, 200, if i == 0 { "MIDDLE" } else { "" }, false)));
        MatchDetail { game_id, created_ms: game_id as i64 * 1000, duration_s: 1800, queue_id, players, ..Default::default() }
    }

    #[test]
    fn add_game_dedupes_flags_lane_and_skips_other_modes() {
        let mut s = Store::default();
        assert!(s.add_game(&game(10, 420, true, &[50, 60])));
        assert_eq!(s.len(), 2, "one entry per enemy, none for the teammate");
        assert!(!s.add_game(&game(10, 420, true, &[50, 60])), "same game twice adds nothing");
        assert_eq!(s.len(), 2);
        // The enemy mid was the lane opponent; the one with an unknown position was not.
        assert_eq!(s.record_vs(50, None, true), Record { games: 1, wins: 1 });
        assert_eq!(s.record_vs(60, None, true), Record::default());
        assert_eq!(s.record_vs(60, Some(1), false).games, 1);
        assert_eq!(s.record_vs(60, Some(7), false).games, 0, "other champion of yours");
        // ARAM and a remake: marked seen so they are not fetched again, but no entries.
        assert!(s.add_game(&game(11, 450, false, &[50])));
        let mut remake = game(12, 420, false, &[50]);
        remake.duration_s = 200;
        assert!(s.add_game(&remake));
        assert!(s.has_game(11) && s.has_game(12));
        assert_eq!(s.len(), 2);
        // Without the viewer nothing is known about whose side is whose.
        let mut anonymous = game(13, 420, true, &[50]);
        anonymous.players.iter_mut().for_each(|p| p.is_me = false);
        assert!(!s.add_game(&anonymous));
        assert!(!s.has_game(13), "retried once the viewer is known");
    }

    #[test]
    fn ban_hints_rank_by_wilson_upper_with_min_games_and_exclusions() {
        let mut s = Store::default();
        let mut id = 0;
        let mut play = |s: &mut Store, enemy: i64, wins: usize, losses: usize| {
            for k in 0..wins + losses {
                id += 1;
                s.add_game(&game(id, 420, k < wins, &[enemy]));
            }
        };
        play(&mut s, 50, 0, 3); // 0-3: bad, but few games
        play(&mut s, 60, 2, 8); // 2-8: clearly bad
        play(&mut s, 70, 1, 2); // 1-2
        play(&mut s, 80, 5, 5); // even: never a hint
        play(&mut s, 90, 0, 2); // 0-2: below 3 games
        let hints = s.ban_hints(3, 5, &[]);
        let order: Vec<i64> = hints.iter().map(|h| h.0).collect();
        // 2-8 has the lowest upper bound (wilson_upper ≈ 0.51 vs 0.56 for 0-3 and 0.79 for 1-2).
        assert_eq!(order, [60, 50, 70]);
        assert_eq!(hints[0].1, Record { games: 10, wins: 2 });
        assert_eq!(s.ban_hints(3, 1, &[]).len(), 1);
        assert_eq!(s.ban_hints(3, 5, &[60]).iter().map(|h| h.0).collect::<Vec<_>>(), [50, 70], "banned or picked champions are skipped");
        assert_eq!(s.ban_hints(2, 5, &[]).iter().map(|h| h.0).collect::<Vec<_>>(), [60, 50, 90, 70], "0-2 sits at ≈ 0.66");
    }
}
