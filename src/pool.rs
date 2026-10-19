//! Champ select pick ideas from your own pool: champions you play, scored by your record,
//! whether Riot lists them in your position, what the team lacks and the likely lane opponent.
//! Several options with their reasons, never a single pick.

use std::collections::HashMap;

use crate::ddragon::StaticData;
use crate::meta::{Matchup, wilson_lower};

/// Ideas returned at most.
const MAX_IDEAS: usize = 6;
/// Games on a champion before it is an idea (unless it counters the lane opponent).
const MIN_POOL_GAMES: u64 = 2;
/// Your games against the opponent on a champion before that record is a reason.
const MIN_RECORD_GAMES: u64 = 2;
/// The opponent's OP.GG win rate against a champion below which that champion counters it.
const COUNTER_WIN_RATE: f64 = 0.49;
/// Score added per reason, on top of your Wilson lower bound (0..1) on the champion.
const ROLE_WEIGHT: f64 = 0.3;
const NEED_WEIGHT: f64 = 0.15;
const COUNTER_WEIGHT: f64 = 0.2;
/// Times (your win rate − 50 %) against the opponent on the champion.
const RECORD_WEIGHT: f64 = 0.4;

/// A champion from your recent games.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct PoolChamp {
    pub champion_id: i64,
    pub games: u64,
    pub wins: u64,
}

/// What your team lacks (from `ui::stats::Comp::warnings`).
#[derive(Clone, Copy, Default)]
pub struct Needs {
    pub magic: bool,
    pub physical: bool,
    pub tank: bool,
}

#[derive(Clone, PartialEq, Debug)]
pub enum Reason {
    /// Riot lists it for your position.
    Role,
    /// Magic damage the team lacks (Data Dragon `magic` ≥ `attack`).
    Magic,
    /// Physical damage the team lacks (Data Dragon `attack` ≥ `magic`).
    Physical,
    /// Tagged `Tank`, and the team has none.
    Tank,
    /// The likely lane opponent wins under 49 % against it on OP.GG.
    Counters { opponent: i64, games: u64, opponent_wins: u64 },
    /// Your own games on it against the likely lane opponent.
    Record { opponent: i64, games: u64, wins: u64 },
}

#[derive(Clone, PartialEq, Debug)]
pub struct Idea {
    pub champion_id: i64,
    /// Your recent games and wins on it.
    pub games: u64,
    pub wins: u64,
    pub reasons: Vec<Reason>,
}

pub struct Input<'a> {
    pub pool: &'a [PoolChamp],
    /// Empty = unknown, everything allowed.
    pub pickable: &'a [i64],
    /// Canonical position, `""` when unknown.
    pub position: &'a str,
    /// Riot's recommended positions per champion, canonical; empty when not loaded.
    pub positions: &'a HashMap<i64, Vec<String>>,
    pub needs: Needs,
    pub data: &'a StaticData,
    /// The probable lane opponent and its OP.GG lane matchups (wins are the opponent's).
    pub opponent: Option<(i64, &'a [Matchup])>,
    /// Your (games, wins) on a champion of yours against an enemy champion.
    pub record_vs: &'a dyn Fn(i64, i64) -> (u64, u64),
}

/// Up to six pool champions, best first: your Wilson lower bound on the champion plus a weight
/// per reason. With a known position and Riot's list loaded, champions Riot does not list for
/// that position come last.
pub fn ideas(i: &Input) -> Vec<Idea> {
    let role_known = !i.position.is_empty() && !i.positions.is_empty();
    let mut scored: Vec<(bool, f64, Idea)> = i
        .pool
        .iter()
        .filter(|p| i.pickable.is_empty() || i.pickable.contains(&p.champion_id))
        .filter_map(|p| {
            let mut reasons = Vec::new();
            let mut score = wilson_lower(p.wins, p.games);
            let listed = i.positions.get(&p.champion_id).is_some_and(|v| v.iter().any(|x| x == i.position));
            if role_known && listed {
                reasons.push(Reason::Role);
                score += ROLE_WEIGHT;
            }
            if let Some(ch) = i.data.champions.get(&p.champion_id) {
                let needs = [
                    (i.needs.magic && ch.magic >= ch.attack, Reason::Magic),
                    (i.needs.physical && ch.attack >= ch.magic, Reason::Physical),
                    (i.needs.tank && ch.tags.iter().any(|t| t == "Tank"), Reason::Tank),
                ];
                for (_, r) in needs.into_iter().filter(|n| n.0) {
                    reasons.push(r);
                    score += NEED_WEIGHT;
                }
            }
            let mut counters = false;
            if let Some((opponent, rows)) = i.opponent {
                let beats = |m: &&Matchup| m.champion_id == p.champion_id && m.games > 0 && (m.wins as f64 / m.games as f64) < COUNTER_WIN_RATE;
                if let Some(m) = rows.iter().find(beats) {
                    counters = true;
                    reasons.push(Reason::Counters { opponent, games: m.games, opponent_wins: m.wins });
                    score += COUNTER_WEIGHT;
                }
                let (games, wins) = (i.record_vs)(p.champion_id, opponent);
                if games >= MIN_RECORD_GAMES {
                    reasons.push(Reason::Record { opponent, games, wins });
                    score += RECORD_WEIGHT * (wins as f64 / games as f64 - 0.5);
                }
            }
            if p.games < MIN_POOL_GAMES && !(counters && p.games >= 1) {
                return None;
            }
            let idea = Idea { champion_id: p.champion_id, games: p.games, wins: p.wins, reasons };
            Some((role_known && !listed, score, idea))
        })
        .collect();
    scored.sort_by(|a, b| a.0.cmp(&b.0).then(b.1.total_cmp(&a.1)).then(b.2.games.cmp(&a.2.games)));
    scored.into_iter().take(MAX_IDEAS).map(|s| s.2).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ddragon::Champion;

    const AHRI: i64 = 103;
    const ZED: i64 = 238;
    const MALPHITE: i64 = 54;
    const YASUO: i64 = 157;
    const GAREN: i64 = 86;

    fn data() -> StaticData {
        let mut d = StaticData::default();
        for (id, name, attack, magic, tank) in [(AHRI, "Ahri", 3, 8, false), (ZED, "Zed", 9, 1, false), (MALPHITE, "Malphite", 5, 7, true), (YASUO, "Yasuo", 8, 4, false), (GAREN, "Garen", 7, 1, true)] {
            let tags = if tank { vec!["Tank".to_string()] } else { vec!["Fighter".to_string()] };
            d.champions.insert(id, Champion { name: name.into(), attack, magic, tags, ..Default::default() });
        }
        d
    }

    fn pool_champ(champion_id: i64, games: u64, wins: u64) -> PoolChamp {
        PoolChamp { champion_id, games, wins }
    }

    fn mid_positions() -> HashMap<i64, Vec<String>> {
        [(AHRI, vec!["MIDDLE"]), (ZED, vec!["MIDDLE"]), (YASUO, vec!["MIDDLE", "TOP"]), (MALPHITE, vec!["TOP"]), (GAREN, vec!["TOP"])]
            .into_iter()
            .map(|(id, p)| (id, p.into_iter().map(String::from).collect()))
            .collect()
    }

    fn no_record(_: i64, _: i64) -> (u64, u64) {
        (0, 0)
    }

    fn ids(v: &[Idea]) -> Vec<i64> {
        v.iter().map(|i| i.champion_id).collect()
    }

    #[test]
    fn reasons_for_role_needs_counter_and_record() {
        let data = data();
        let positions = mid_positions();
        let pool = [pool_champ(AHRI, 10, 6), pool_champ(ZED, 10, 6), pool_champ(MALPHITE, 1, 1)];
        // The opponent is Yasuo; on OP.GG it wins 47 % against Malphite and 52 % against Ahri.
        let yasuo_matchups = [
            Matchup { champion_id: MALPHITE, games: 1000, wins: 470 },
            Matchup { champion_id: AHRI, games: 1000, wins: 520 },
        ];
        let record = |mine: i64, enemy: i64| -> (u64, u64) { if (mine, enemy) == (ZED, YASUO) { (3, 1) } else { (0, 0) } };
        let input = Input {
            pool: &pool,
            pickable: &[],
            position: "MIDDLE",
            positions: &positions,
            needs: Needs { magic: true, physical: false, tank: true },
            data: &data,
            opponent: Some((YASUO, &yasuo_matchups[..])),
            record_vs: &record,
        };
        let out = ideas(&input);
        let reasons = |id: i64| out.iter().find(|i| i.champion_id == id).map(|i| i.reasons.clone());
        assert_eq!(reasons(AHRI), Some(vec![Reason::Role, Reason::Magic]));
        assert_eq!(reasons(ZED), Some(vec![Reason::Role, Reason::Record { opponent: YASUO, games: 3, wins: 1 }]));
        // One game is enough when it counters the opponent; Riot lists it top only, so it ranks
        // last despite three reasons.
        assert_eq!(reasons(MALPHITE), Some(vec![Reason::Magic, Reason::Tank, Reason::Counters { opponent: YASUO, games: 1000, opponent_wins: 470 }]));
        assert_eq!(ids(&out), [AHRI, ZED, MALPHITE]);
    }

    #[test]
    fn pickable_and_minimum_games_filter_candidates() {
        let data = data();
        let positions = mid_positions();
        let pool = [pool_champ(AHRI, 10, 6), pool_champ(ZED, 10, 6), pool_champ(YASUO, 1, 1)];
        let input = Input {
            pool: &pool,
            pickable: &[AHRI, YASUO, GAREN],
            position: "MIDDLE",
            positions: &positions,
            needs: Needs::default(),
            data: &data,
            opponent: None,
            record_vs: &no_record,
        };
        // Zed is not owned/banned; one Yasuo game is too few without a counter reason.
        assert_eq!(ids(&ideas(&input)), [AHRI]);
    }

    #[test]
    fn ranking_uses_record_then_reasons_and_unknown_position_never_penalises() {
        let data = data();
        let positions = mid_positions();
        // Garen: great record but Riot lists him top only. Zed: decent record. Ahri: weaker record.
        let pool = [pool_champ(GAREN, 20, 16), pool_champ(ZED, 20, 12), pool_champ(AHRI, 20, 10), pool_champ(YASUO, 2, 0), pool_champ(MALPHITE, 3, 2)];
        let mut input = Input {
            pool: &pool,
            pickable: &[],
            position: "MIDDLE",
            positions: &positions,
            needs: Needs::default(),
            data: &data,
            opponent: None,
            record_vs: &no_record,
        };
        assert_eq!(ids(&ideas(&input)), [ZED, AHRI, YASUO, GAREN, MALPHITE]);
        // Lacking magic damage lifts Ahri over Zed.
        input.needs.magic = true;
        assert_eq!(ids(&ideas(&input))[..2], [AHRI, ZED]);
        // Position unknown: no Role reasons, nobody sinks, the record decides.
        input.needs.magic = false;
        input.position = "";
        let out = ideas(&input);
        assert_eq!(ids(&out), [GAREN, ZED, AHRI, MALPHITE, YASUO]);
        assert!(out.iter().all(|i| i.reasons.is_empty()));
    }
}
