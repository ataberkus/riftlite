//! Aggregated champion builds from OP.GG.
//!
//! Source: `https://lol-api-champion.op.gg/api/global/champions/{mode}/{id}/{lane}`
//! (`arena/{id}` for Arena), the JSON endpoint behind op.gg's champion pages. It
//! lists up to 15 core builds per champion with games, wins and pick rate, plus
//! the most played starting items, boots, rune pages, summoner spells, skill
//! orders and lane matchups, and the champion's own win, pick and ban rates, KDA,
//! win rate per recent patch and per game length. OP.GG's public MCP server (`mcp-api.op.gg`) returns
//! only the single most played core build, so it cannot offer several builds to
//! pick from. OP.GG permits use of its data with the source cited and without
//! excessive requests: riftlite sends one request per picked champion, containing
//! only the champion id, mode and lane, and labels everything it writes "OP.GG".
//!
//! Builds are classified from Data Dragon item data (`ddragon::ItemStats`), never
//! from item names: AP or AD by which damage stat the core items mostly give, and
//! Crit / On-hit / Tank by the item tags.
//!
//! Arena (Riot policy): no win rates or placements for items, builds, skill orders or the
//! champion; options are shown with pick rate and games, and ranked by games.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::ddragon::StaticData;

const BASE: &str = "https://lol-api-champion.op.gg/api/global/champions";
/// op.gg's default rank filter for ranked tables.
const RANKED_TIER: &str = "emerald_plus";
/// Below this many games OP.GG still lists an option, but its win rate is noise.
pub const MIN_GAMES: u64 = 100;
/// A win rate whose 95 % margin is at least this many percentage points is flagged "few games".
pub const FEW_GAMES_MARGIN: f64 = 2.5;
/// z for a 95 % interval.
const Z: f64 = 1.96;

/// What to ask OP.GG for: its game mode and lane names.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Query {
    pub mode: &'static str,
    pub position: &'static str,
}

impl Query {
    pub fn label(&self) -> String {
        match self.position {
            "NONE" => self.mode.replace('_', " "),
            p => format!("{} {}", self.mode, p.to_lowercase()),
        }
    }

    fn url(&self, champion_id: i64) -> String {
        match self.mode {
            // Arena has no lanes; OP.GG rejects any position for it.
            "arena" => format!("{BASE}/arena/{champion_id}"),
            // Only ranked tables have rank tiers; elsewhere a tier filter only shrinks the sample.
            "ranked" | "flex" => format!("{BASE}/{}/{champion_id}/{}?tier={RANKED_TIER}", self.mode, self.position),
            _ => format!("{BASE}/{}/{champion_id}/{}", self.mode, self.position),
        }
    }
}

/// OP.GG mode for a queue. `map_id` 0 (unknown, as in match history) counts as Summoner's Rift.
pub fn mode_for(queue_id: i64, map_id: i64) -> Result<&'static str, String> {
    Ok(match queue_id {
        // ARAM Mayhem has no OP.GG table of its own; its item pool is ARAM's.
        450 | 2400 => "aram",
        900 | 1900 | 1010 => "urf",
        1300 => "nexus_blitz",
        440 => "flex",
        1700 | 1710 => "arena",
        1810..=1840 => return Err("OP.GG has no builds for this mode".into()),
        _ if map_id == 12 => "aram",
        _ if map_id == 30 => "arena",
        _ if map_id == 11 || map_id == 0 => "ranked",
        _ => return Err("OP.GG has no builds for this map".into()),
    })
}

/// OP.GG mode for a queue, and its lane from the champ-select position
/// (`TOP`/`JUNGLE`/`MIDDLE`/`BOTTOM`/`UTILITY`/`NONE`, see `lcu::perk_position`).
pub fn query_for(queue_id: i64, map_id: i64, position: &str) -> Result<Query, String> {
    let mode = mode_for(queue_id, map_id)?;
    let position = match mode {
        "aram" | "urf" | "nexus_blitz" | "arena" => "NONE",
        _ => match position {
            "TOP" => "TOP",
            "JUNGLE" => "JUNGLE",
            "MIDDLE" => "MID",
            "BOTTOM" => "ADC",
            "UTILITY" => "SUPPORT",
            _ => return Err("no lane known for this pick".into()),
        },
    };
    Ok(Query { mode, position })
}

fn rate(wins: u64, games: u64) -> f64 {
    if games == 0 { 0.0 } else { wins as f64 / games as f64 }
}

/// 95 % margin of a win rate, in percentage points.
pub(crate) fn margin(wins: u64, games: u64) -> f64 {
    if games == 0 {
        return 100.0;
    }
    let p = rate(wins, games);
    Z * (p * (1.0 - p) / games as f64).sqrt() * 100.0
}

/// Lower end of the 95 % Wilson interval: a win rate that small samples cannot inflate.
pub(crate) fn wilson_lower(wins: u64, games: u64) -> f64 {
    if games == 0 {
        return 0.0;
    }
    let (n, p) = (games as f64, rate(wins, games));
    let z2 = Z * Z;
    (p + z2 / (2.0 * n) - Z * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt()) / (1.0 + z2 / n)
}

/// Upper end of the 95 % Wilson interval: a win rate that small samples cannot deflate, so a
/// low value means the record is bad even allowing for few games.
pub(crate) fn wilson_upper(wins: u64, games: u64) -> f64 {
    if games == 0 {
        return 1.0;
    }
    let (n, p) = (games as f64, rate(wins, games));
    let z2 = Z * Z;
    (p + z2 / (2.0 * n) + Z * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt()) / (1.0 + z2 / n)
}

/// "53.2% ±0.3 WR · 39.0% pick · 92,134 games"; for Arena "39.0% pick · 92,134 games" (Riot
/// policy: no win rates or placements for Arena items, builds or skill orders).
fn stats_line(wins: u64, games: u64, pick_rate: Option<f64>, arena: bool) -> String {
    let mut parts = Vec::new();
    if !arena {
        parts.push(format!("{:.1}% ±{:.1} WR", rate(wins, games) * 100.0, margin(wins, games)));
    }
    if let Some(p) = pick_rate {
        parts.push(format!("{:.1}% pick", p * 100.0));
    }
    parts.push(format!("{} games", thousands(games)));
    parts.join(" · ")
}

/// The win rate's 95 % margin is at least `FEW_GAMES_MARGIN` points. Arena: the margin a 50 %
/// win rate would have with this many games (the worst case, under 1,537 games), so the flag
/// depends on the sample size only and never on an Arena win rate.
fn few_games(wins: u64, games: u64, arena: bool) -> bool {
    let wins = if arena { games / 2 } else { wins };
    margin(wins, games) >= FEW_GAMES_MARGIN
}

/// One OP.GG option: items (or spells) with how often it was played and won.
#[derive(Clone, PartialEq, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Choice {
    pub ids: Vec<i64>,
    #[serde(rename = "play")]
    pub games: u64,
    #[serde(rename = "win")]
    pub wins: u64,
    /// Share of the champion's games in this mode and lane that used it (0–1).
    pub pick_rate: f64,
    /// Arena build: win rates are neither shown nor used for ranking.
    #[serde(skip)]
    pub arena: bool,
}

impl Choice {
    pub fn few_games(&self) -> bool {
        few_games(self.wins, self.games, self.arena)
    }

    /// Ranking key: the low end of the win rate's 95 % Wilson interval, so a few lucky games
    /// cannot outrank a much-played build. Arena: games played.
    fn score(&self) -> f64 {
        if self.arena { self.games as f64 } else { wilson_lower(self.wins, self.games) }
    }

    pub fn stats(&self) -> String {
        stats_line(self.wins, self.games, Some(self.pick_rate), self.arena)
    }
}

#[derive(Clone, PartialEq, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Skills {
    /// Ability per level, e.g. `["Q","E","W","Q",…]`.
    pub order: Vec<String>,
    #[serde(rename = "play")]
    pub games: u64,
    #[serde(rename = "win")]
    pub wins: u64,
    pub pick_rate: f64,
    /// Arena skill order: no win rate shown.
    #[serde(skip)]
    pub arena: bool,
}

impl Skills {
    pub fn few_games(&self) -> bool {
        few_games(self.wins, self.games, self.arena)
    }

    pub fn stats(&self) -> String {
        stats_line(self.wins, self.games, Some(self.pick_rate), self.arena)
    }
}

/// OP.GG's max order (`ids`, e.g. Q > E > W) and the level orders that follow it.
#[derive(Clone, PartialEq, Debug, Default, Deserialize)]
#[serde(default)]
struct Mastery {
    ids: Vec<String>,
    builds: Vec<Skills>,
}

#[derive(Clone, PartialEq, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Runes {
    pub primary_page_id: i64,
    pub primary_rune_ids: Vec<i64>,
    pub secondary_page_id: i64,
    pub secondary_rune_ids: Vec<i64>,
    pub stat_mod_ids: Vec<i64>,
    #[serde(rename = "play")]
    pub games: u64,
    #[serde(rename = "win")]
    pub wins: u64,
    pub pick_rate: f64,
    /// Arena page: no win rate shown.
    #[serde(skip)]
    pub arena: bool,
}

impl Runes {
    /// Keystone, 3 primary, 2 secondary, 3 shards, like the client's pages; `None` if incomplete.
    pub fn perks(&self) -> Option<Vec<i64>> {
        let p: Vec<i64> = self.primary_rune_ids.iter().chain(&self.secondary_rune_ids).chain(&self.stat_mod_ids).copied().collect();
        (self.primary_rune_ids.len() == 4 && self.secondary_rune_ids.len() == 2 && self.stat_mod_ids.len() == 3).then_some(p)
    }

    pub fn few_games(&self) -> bool {
        few_games(self.wins, self.games, self.arena)
    }

    pub fn stats(&self) -> String {
        stats_line(self.wins, self.games, Some(self.pick_rate), self.arena)
    }
}

/// The picked champion against another one in the same lane, from the picked champion's side.
#[derive(Clone, PartialEq, Debug, Default, Deserialize)]
#[serde(default)]
pub struct Matchup {
    pub champion_id: i64,
    #[serde(rename = "play")]
    pub games: u64,
    #[serde(rename = "win")]
    pub wins: u64,
}

impl Matchup {
    pub fn stats(&self) -> String {
        stats_line(self.wins, self.games, None, false)
    }
}

/// Main damage of a core build.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Damage {
    Ap,
    Ad,
    /// Core items give no attack or ability power (tank and support builds).
    None,
}

/// Ways to group builds for picking; a build can be in several groups, and every build is
/// in at least one (AP, AD, Tank or Utility).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Group {
    Ap,
    Ad,
    OnHit,
    Crit,
    Tank,
    /// Neither attack nor ability power, and not a tank build (e.g. enchanter supports).
    Utility,
}

impl Group {
    pub const ORDER: [Group; 6] = [Group::Ap, Group::Ad, Group::OnHit, Group::Crit, Group::Tank, Group::Utility];

    pub fn label(self) -> &'static str {
        match self {
            Group::Ap => "AP",
            Group::Ad => "AD",
            Group::OnHit => "On-hit",
            Group::Crit => "Crit",
            Group::Tank => "Tank",
            Group::Utility => "Utility",
        }
    }
}

/// A core build (OP.GG's first three completed items) and what kind of build it is.
#[derive(Clone, PartialEq, Debug)]
pub struct CoreBuild {
    pub items: Choice,
    pub damage: Damage,
    /// At least two core items carry the tag.
    pub crit: bool,
    pub on_hit: bool,
    /// At least two core items are defensive and give no attack or ability power.
    pub tank: bool,
}

impl CoreBuild {
    /// Classifies from Data Dragon stats: an item counts as AP (AD) when it gives more
    /// ability (attack) power than the other; ties between AP and AD items are broken
    /// by the summed stat. Items Data Dragon does not know count for nothing.
    pub fn classify(items: Choice, data: &StaticData) -> CoreBuild {
        let stats: Vec<_> = items.ids.iter().filter_map(|id| data.item_stats.get(id)).collect();
        let ap_items = stats.iter().filter(|s| s.ap > s.ad).count();
        let ad_items = stats.iter().filter(|s| s.ad > s.ap).count();
        let (ap, ad): (f64, f64) = stats.iter().fold((0.0, 0.0), |(p, d), s| (p + s.ap, d + s.ad));
        let damage = match ap_items.cmp(&ad_items) {
            std::cmp::Ordering::Greater => Damage::Ap,
            std::cmp::Ordering::Less => Damage::Ad,
            std::cmp::Ordering::Equal if ap > ad => Damage::Ap,
            std::cmp::Ordering::Equal if ad > ap => Damage::Ad,
            std::cmp::Ordering::Equal => Damage::None,
        };
        let count = |f: &dyn Fn(&crate::ddragon::ItemStats) -> bool| stats.iter().filter(|s| f(s)).count();
        CoreBuild {
            crit: count(&|s| s.crit) >= 2,
            on_hit: count(&|s| s.on_hit) >= 2,
            tank: count(&|s| s.defensive && s.ap == 0.0 && s.ad == 0.0) >= 2,
            damage,
            items,
        }
    }

    pub fn in_group(&self, g: Group) -> bool {
        match g {
            Group::Ap => self.damage == Damage::Ap,
            Group::Ad => self.damage == Damage::Ad,
            Group::OnHit => self.on_hit,
            Group::Crit => self.crit,
            Group::Tank => self.tank,
            Group::Utility => self.damage == Damage::None && !self.tank,
        }
    }

    /// "AD · On-hit", "AP", "Tank"
    pub fn label(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        match self.damage {
            Damage::Ap => parts.push("AP"),
            Damage::Ad => parts.push("AD"),
            Damage::None if !self.tank => parts.push("Utility"),
            Damage::None => {}
        }
        for (on, name) in [(self.on_hit, "On-hit"), (self.crit, "Crit"), (self.tank, "Tank")] {
            if on {
                parts.push(name);
            }
        }
        parts.join(" · ")
    }

    /// Whether a later item fits this build's damage. An AP build skips AD items and crit
    /// items without ability power (crit scales attacks); an AD build skips AP items.
    pub(crate) fn fits(&self, s: &crate::ddragon::ItemStats) -> bool {
        match self.damage {
            Damage::Ap => s.ap > s.ad || (s.ap == s.ad && (s.ap > 0.0 || !s.crit)),
            Damage::Ad => s.ap <= s.ad,
            Damage::None => s.ap == 0.0 && s.ad == 0.0,
        }
    }
}

/// What the player picked among a build's options; indices into the `MetaBuild` lists.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Picks {
    pub build: usize,
    pub runes: usize,
    pub spells: usize,
    pub skills: usize,
}

/// One selection the player makes in the OP.GG panel.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Pick {
    Build(usize),
    Runes(usize),
    Spells(usize),
    Skills(usize),
}

/// `summary.average_stats` and `summary.positions` stay raw `Value`s, like `trends` and
/// `game_lengths`: they are read field by field, so a reshaped part empties only the summary.
#[derive(Deserialize, Default)]
#[serde(default)]
struct Summary {
    id: i64,
    average_stats: Value,
    positions: Value,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct RawData {
    summary: Summary,
    starter_items: Vec<Choice>,
    core_items: Vec<Choice>,
    boots: Vec<Choice>,
    last_items: Vec<Choice>,
    skills: Vec<Skills>,
    skill_masteries: Vec<Mastery>,
    runes: Vec<Runes>,
    summoner_spells: Vec<Choice>,
    counters: Vec<Matchup>,
    trends: Value,
    game_lengths: Value,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Info {
    version: String,
}

#[derive(Deserialize)]
struct Envelope {
    data: RawData,
    #[serde(default)]
    meta: Info,
}

/// OP.GG's numbers for the champion itself in the query's mode (and lane, when it has one).
#[derive(Clone, PartialEq, Debug, Default)]
pub struct ChampSummary {
    /// Games in the lane (or mode) OP.GG counted.
    pub games: u64,
    /// 0–1; `None` for Arena (Riot policy) or when OP.GG gave none.
    pub win_rate: Option<f64>,
    /// 0–1.
    pub pick_rate: Option<f64>,
    /// 0–1; ARAM has no bans.
    pub ban_rate: Option<f64>,
    pub kda: Option<f64>,
    /// Share of the champion's games played in the query's lane (0–1); `None` without a lane.
    pub lane_share: Option<f64>,
    /// (patch, win rate 0–1) for recent patches, oldest first; empty for Arena.
    pub trend: Vec<(String, f64)>,
    /// (game length in minutes from, win rate 0–1), shortest first: OP.GG's buckets start at
    /// 0, 25, 30, 35 and 40 minutes. Empty for Arena.
    pub game_lengths: Vec<(u32, f64)>,
}

impl ChampSummary {
    /// "Ahri mid · 51.2% WR · 9.4% pick · 3.0% ban · KDA 2.55 · 95% of Ahri's games are mid";
    /// fields OP.GG did not give are left out.
    pub fn line(&self, champion: &str, query: Query) -> String {
        let lane = (query.position != "NONE").then(|| query.position.to_lowercase());
        let mut parts = vec![match &lane {
            Some(l) => format!("{champion} {l}"),
            None => champion.to_string(),
        }];
        let pct = |v: Option<f64>, what: &str| v.map(|v| format!("{:.1}% {what}", v * 100.0));
        parts.extend(pct(self.win_rate, "WR"));
        parts.extend(pct(self.pick_rate, "pick"));
        parts.extend(pct(self.ban_rate, "ban"));
        parts.extend(self.kda.map(|k| format!("KDA {k:.2}")));
        parts.push(format!("{} games", thousands(self.games)));
        if let (Some(share), Some(l)) = (self.lane_share, &lane) {
            parts.push(format!("{:.0}% of {champion}'s games are {l}", share * 100.0));
        }
        parts.join(" · ")
    }
}

/// A finite number field.
fn num(v: &Value, key: &str) -> Option<f64> {
    v.get(key)?.as_f64().filter(|x| x.is_finite())
}

/// A rate field in 0–1.
fn share(v: &Value, key: &str) -> Option<f64> {
    num(v, key).filter(|x| (0.0..=1.0).contains(x))
}

/// The champion summary from OP.GG's `summary`, `trends` and `game_lengths`. Lane numbers
/// (`summary.positions[name == query.position].stats`) win over champion-wide ones
/// (`summary.average_stats`). Arena keeps games, pick rate, ban rate and KDA (from its kill,
/// death and assist totals) only. `None` when OP.GG counted no games.
fn parse_summary(summary: &Summary, trends: &Value, game_lengths: &Value, query: Query) -> Option<ChampSummary> {
    let arena = query.mode == "arena";
    let lane = (query.position != "NONE")
        .then(|| summary.positions.as_array()?.iter().find(|p| p.get("name").and_then(Value::as_str) == Some(query.position)))
        .flatten();
    let stats = lane.and_then(|p| p.get("stats")).filter(|s| s.is_object()).unwrap_or(&summary.average_stats);
    let games = stats.get("play").and_then(Value::as_u64).filter(|&n| n > 0)?;
    let kda = if arena {
        let total = |k: &str| num(stats, k).filter(|x| *x >= 0.0);
        match (total("kills"), total("deaths"), total("assists")) {
            (Some(k), Some(d), Some(a)) if d > 0.0 => Some((k + a) / d),
            _ => None,
        }
    } else {
        num(stats, "kda").filter(|k| *k >= 0.0)
    };
    let mut trend: Vec<(String, f64)> = Vec::new();
    let mut lengths: Vec<(u32, f64)> = Vec::new();
    if !arena {
        // Newest patch first in OP.GG's answer.
        for t in trends.get("win").and_then(Value::as_array).into_iter().flatten().rev() {
            if let (Some(v), Some(r)) = (t.get("version").and_then(Value::as_str).filter(|v| !v.is_empty()), share(t, "rate")) {
                trend.push((v.to_string(), r));
            }
        }
        for g in game_lengths.as_array().into_iter().flatten() {
            let from = g.get("game_length").and_then(Value::as_u64).and_then(|m| u32::try_from(m).ok());
            if let (Some(m), Some(r)) = (from, share(g, "rate")) {
                lengths.push((m, r));
            }
        }
        lengths.sort_by_key(|&(m, _)| m);
        lengths.dedup_by_key(|&mut (m, _)| m);
    }
    Some(ChampSummary {
        games,
        win_rate: if arena { None } else { share(stats, "win_rate") },
        pick_rate: share(stats, "pick_rate"),
        ban_rate: share(stats, "ban_rate"),
        kda,
        lane_share: lane.and_then(|p| share(p.get("stats")?, "role_rate")),
        trend,
        game_lengths: lengths,
    })
}

/// A champion's OP.GG builds for one mode and lane.
#[derive(Clone, PartialEq, Debug)]
pub struct MetaBuild {
    pub champion_id: i64,
    pub query: Query,
    /// Patch OP.GG aggregated, e.g. `16.20`; empty when it did not say.
    pub patch: String,
    /// Core builds with at least `MIN_GAMES` games, best `Choice::score` first. Never empty.
    pub cores: Vec<CoreBuild>,
    /// Options below, most played first: the first one always, others with `MIN_GAMES`.
    pub starters: Vec<Choice>,
    pub boots: Vec<Choice>,
    pub skills: Vec<Skills>,
    /// Complete pages only.
    pub runes: Vec<Runes>,
    /// Two-spell pairs only.
    pub spells: Vec<Choice>,
    /// Single items players finished with, most played first.
    pub last_items: Vec<Choice>,
    /// Lane matchups with at least `MIN_GAMES` games (none outside ranked and flex; never for Arena).
    pub counters: Vec<Matchup>,
    /// The champion's own OP.GG numbers; `None` when OP.GG gave none.
    pub summary: Option<ChampSummary>,
    masteries: Vec<Mastery>,
}

/// Most builds listed per group.
pub const GROUP_LIMIT: usize = 3;

impl MetaBuild {
    /// Ranks `cores` by `Choice::score` (Arena: games), then games.
    fn sort(&mut self) {
        self.cores.sort_by(|a, b| b.items.score().total_cmp(&a.items.score()).then(b.items.games.cmp(&a.items.games)));
    }

    /// Groups that have at least one build, ordered by their top build (same key as `cores`),
    /// then games, then `Group::ORDER`.
    pub fn groups(&self) -> Vec<Group> {
        // `cores` is already ranked, so a group's first member is its top build.
        let mut groups: Vec<(Group, &Choice)> =
            Group::ORDER.into_iter().filter_map(|g| Some((g, &self.cores.iter().find(|c| c.in_group(g))?.items))).collect();
        // Stable sort: equal scores and games keep `Group::ORDER`.
        groups.sort_by(|(_, a), (_, b)| b.score().total_cmp(&a.score()).then(b.games.cmp(&a.games)));
        groups.into_iter().map(|(g, _)| g).collect()
    }

    /// Indices into `cores` of a group's best `GROUP_LIMIT` builds, best first.
    pub fn in_group(&self, g: Group) -> Vec<usize> {
        (0..self.cores.len()).filter(|&i| self.cores[i].in_group(g)).take(GROUP_LIMIT).collect()
    }

    /// Index of the core build with exactly these items.
    pub fn index_of(&self, ids: &[i64]) -> Option<usize> {
        self.cores.iter().position(|c| c.items.ids == ids)
    }

    /// Indices of remembered picks in this build's lists; one no longer offered falls back to
    /// the default (first) option.
    pub fn picks_of(&self, saved: &SavedPicks) -> Picks {
        let find = |found: Option<usize>| found.unwrap_or(0);
        Picks {
            build: find(self.index_of(&saved.build)),
            runes: find(self.runes.iter().position(|r| r.perks().as_deref() == Some(saved.runes.as_slice()))),
            spells: find(self.spells.iter().position(|s| s.ids == saved.spells)),
            skills: find(self.skills.iter().position(|s| s.order == saved.skills)),
        }
    }

    /// The content of `picks`, to remember.
    pub fn saved(&self, picks: Picks) -> SavedPicks {
        SavedPicks {
            build: self.cores.get(picks.build).map(|c| c.items.ids.clone()).unwrap_or_default(),
            runes: self.runes.get(picks.runes).and_then(Runes::perks).unwrap_or_default(),
            spells: self.spells.get(picks.spells).map(|s| s.ids.clone()).unwrap_or_default(),
            skills: self.skills.get(picks.skills).map(|s| s.order.clone()).unwrap_or_default(),
        }
    }

    /// Most played boots.
    pub fn boots(&self) -> Option<&Choice> {
        self.boots.first()
    }

    /// The core, boots, then the most played finishing items that fit its damage: six items.
    pub fn full_build(&self, core: usize, data: &StaticData) -> Vec<i64> {
        let c = &self.cores[core];
        let mut out: Vec<i64> = c.items.ids.clone();
        out.extend(self.boots().iter().flat_map(|b| b.ids.iter().copied()).filter(|id| !c.items.ids.contains(id)));
        for id in self.last_items.iter().flat_map(|l| l.ids.iter().copied()) {
            if out.len() >= 6 {
                break;
            }
            let fits = data.item_stats.get(&id).is_some_and(|s| !s.boots && c.fits(s));
            if fits && !out.contains(&id) {
                out.push(id);
            }
        }
        out
    }

    /// Other often finished items that fit the build, beyond the full build.
    pub fn situational(&self, core: usize, data: &StaticData) -> Vec<i64> {
        let full = self.full_build(core, data);
        let c = &self.cores[core];
        self.last_items
            .iter()
            .flat_map(|l| l.ids.iter().copied())
            .filter(|id| !full.contains(id) && data.item_stats.get(id).is_some_and(|s| !s.boots && c.fits(s)))
            .take(6)
            .collect()
    }

    /// OP.GG's max order (e.g. Q > E > W) for skill order `i`; empty when OP.GG lists none.
    pub fn skill_priority(&self, i: usize) -> Vec<String> {
        let Some(order) = self.skills.get(i).map(|s| &s.order) else { return Vec::new() };
        self.masteries.iter().find(|m| m.builds.iter().any(|b| &b.order == order)).map(|m| m.ids.clone()).unwrap_or_default()
    }

    pub fn matchup(&self, champion_id: i64) -> Option<&Matchup> {
        self.counters.iter().find(|m| m.champion_id == champion_id)
    }

    /// OP.GG's patch is the Data Dragon patch (`16.20` for `16.20.1`); unknown counts as matching.
    pub fn patch_matches(&self, ddragon_version: &str) -> bool {
        if self.patch.is_empty() || ddragon_version.is_empty() {
            return true;
        }
        ddragon_version == self.patch || ddragon_version.strip_prefix(self.patch.as_str()).is_some_and(|rest| rest.starts_with('.'))
    }
}

/// Fetches, validates and classifies the builds. Blocking (about a second); call it off the
/// poll thread, and only once `data` holds Data Dragon's items (classification needs them).
pub fn fetch(agent: &ureq::Agent, data: &StaticData, champion_id: i64, query: Query) -> Result<MetaBuild, String> {
    let resp = match agent.get(&query.url(champion_id)).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(404, _)) => return Err("OP.GG does not know this champion".into()),
        Err(ureq::Error::Status(code, _)) => return Err(format!("OP.GG HTTP {code}")),
        Err(e) => return Err(format!("OP.GG unreachable: {e}")),
    };
    let text = resp.into_string().map_err(|e| format!("OP.GG read: {e}"))?;
    parse_build(champion_id, query, &text, data)
}

/// Most played first; the most played option always stays, others need `MIN_GAMES`.
fn trusted<T>(mut v: Vec<T>, games: impl Fn(&T) -> u64) -> Vec<T> {
    v.sort_by_key(|x| std::cmp::Reverse(games(x)));
    let mut first = true;
    v.retain(|x| std::mem::take(&mut first) || games(x) >= MIN_GAMES);
    v
}

/// Turns OP.GG's JSON into classified builds, rejecting other champions and tiny samples.
pub fn parse_build(champion_id: i64, query: Query, json: &str, data: &StaticData) -> Result<MetaBuild, String> {
    let changed = |why: String| format!("OP.GG's data format changed ({why}); riftlite needs an update to read it");
    let env = serde_json::from_str::<Envelope>(json).map_err(|e| changed(e.to_string()))?;
    let raw = env.data;
    // Every field is optional, so a reshaped answer parses into defaults; no champion id gives it away.
    if raw.summary.id == 0 {
        return Err(changed("no champion id".into()));
    }
    if raw.summary.id != champion_id {
        return Err(format!("OP.GG answered for champion {} instead of {champion_id}", raw.summary.id));
    }
    let arena = query.mode == "arena";
    let summary = parse_summary(&raw.summary, &raw.trends, &raw.game_lengths, query);
    let most = raw.core_items.iter().map(|c| c.games).max().unwrap_or(0);
    let flag = |c: Choice| Choice { arena, ..c };
    let cores: Vec<CoreBuild> = raw
        .core_items
        .into_iter()
        .filter(|c| !c.ids.is_empty() && c.games >= MIN_GAMES)
        .map(|c| CoreBuild::classify(flag(c), data))
        .collect();
    if cores.is_empty() {
        return Err(match most {
            0 => "OP.GG has no build for this champion and mode".into(),
            1 => format!("OP.GG has only 1 game of this build ({}); too few to trust", query.label()),
            n => format!("OP.GG has only {n} games of this build ({}); too few to trust", query.label()),
        });
    }
    let items = |v: Vec<Choice>| trusted(v.into_iter().filter(|c| !c.ids.is_empty()).map(flag).collect(), |c| c.games);
    let skills = raw.skills.into_iter().filter(|s| !s.order.is_empty()).map(|s| Skills { arena, ..s });
    let runes = raw.runes.into_iter().filter(|r| r.perks().is_some()).map(|r| Runes { arena, ..r });
    let mut build = MetaBuild {
        champion_id,
        query,
        patch: env.meta.version,
        cores,
        starters: items(raw.starter_items),
        boots: items(raw.boots),
        skills: trusted(skills.collect(), |s| s.games),
        runes: trusted(runes.collect(), |r| r.games),
        spells: trusted(raw.summoner_spells.into_iter().filter(|c| c.ids.len() == 2).map(flag).collect(), |c| c.games),
        last_items: raw.last_items.into_iter().map(flag).collect(),
        // Matchups are win rates; Arena has no lanes, and Riot policy rules out its win rates.
        counters: if arena { Vec::new() } else { raw.counters.into_iter().filter(|m| m.games >= MIN_GAMES).collect() },
        summary,
        masteries: raw.skill_masteries,
    };
    build.sort();
    Ok(build)
}

fn choices_path() -> PathBuf {
    crate::settings::local_dir().join("builds.json")
}

/// The OP.GG options last picked for a champion and mode, by content so they survive OP.GG
/// reordering its lists. Empty fields mean "not picked": the default option stays.
#[derive(Clone, PartialEq, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SavedPicks {
    /// Core item ids.
    pub build: Vec<i64>,
    /// Keystone, runes and shards (`Runes::perks`).
    pub runes: Vec<i64>,
    pub spells: Vec<i64>,
    pub skills: Vec<String>,
}

/// A `builds.json` entry: older versions kept only the core items.
#[derive(Deserialize)]
#[serde(untagged)]
enum SavedEntry {
    Picks(SavedPicks),
    Build(Vec<i64>),
}

/// `"<championId>:<mode>"` → the picks made last.
fn load_choices() -> HashMap<String, SavedPicks> {
    let all: HashMap<String, SavedEntry> = std::fs::read(choices_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    all.into_iter()
        .map(|(k, e)| {
            let picks = match e {
                SavedEntry::Picks(p) => p,
                SavedEntry::Build(build) => SavedPicks { build, ..SavedPicks::default() },
            };
            (k, picks)
        })
        .collect()
}

/// The picks made last for this champion and OP.GG mode.
pub fn saved_picks(champion_id: i64, mode: &str) -> Option<SavedPicks> {
    load_choices().remove(&format!("{champion_id}:{mode}"))
}

/// Remembers the picks, in `%LOCALAPPDATA%\riftlite\builds.json`; nothing leaves the machine.
pub fn save_picks(champion_id: i64, mode: &str, picks: SavedPicks) -> Result<(), String> {
    let mut all = load_choices();
    all.insert(format!("{champion_id}:{mode}"), picks);
    let p = choices_path();
    std::fs::create_dir_all(p.parent().expect("builds path has a parent")).map_err(|e| e.to_string())?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&all).expect("builds serialize"))
        .and_then(|_| std::fs::rename(&tmp, &p))
        .map_err(|e| e.to_string())
}

/// `12345` → `12,345`.
pub fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ddragon::ItemStats;

    fn item(ap: f64, ad: f64, crit: bool, on_hit: bool, defensive: bool, boots: bool) -> ItemStats {
        ItemStats { ap, ad, crit, on_hit, defensive, boots, ..ItemStats::default() }
    }

    /// Data Dragon 16.20 stats/tags of the items in the Kai'Sa answer below.
    fn data() -> StaticData {
        let mut d = StaticData::default();
        for (id, s) in [
            (6672, item(0.0, 45.0, false, true, false, false)),  // Kraken Slayer
            (3124, item(30.0, 30.0, false, true, false, false)), // Guinsoo's Rageblade
            (3302, item(0.0, 30.0, false, true, false, false)),  // Terminus
            (3115, item(80.0, 0.0, false, true, false, false)),  // Nashor's Tooth
            (3087, item(45.0, 45.0, false, true, false, false)), // Statikk Shiv
            (3032, item(0.0, 50.0, true, false, false, false)),  // Yun Tal Wildarrows
            (3031, item(0.0, 75.0, true, false, false, false)),  // Infinity Edge
            (3046, item(0.0, 0.0, true, false, false, false)),   // Phantom Dancer
            (3085, item(0.0, 0.0, true, true, false, false)),    // Runaan's Hurricane
            (3089, item(130.0, 0.0, false, false, false, false)), // Rabadon's Deathcap
            (3006, item(0.0, 0.0, false, false, false, true)),   // Berserker's Greaves
            (3020, item(0.0, 0.0, false, false, false, true)),   // Sorcerer's Shoes
            (3068, item(0.0, 0.0, false, false, true, false)),   // Sunfire Aegis
            (3075, item(0.0, 0.0, false, false, true, false)),   // Thornmail
            (3047, item(0.0, 0.0, false, false, true, true)),    // Plated Steelcaps
        ] {
            d.item_stats.insert(id, s);
        }
        d
    }

    /// Trimmed from a real answer for Kai'Sa, ranked ADC, emerald+.
    const KAISA: &str = r#"{"meta":{"version":"16.20"},"data":{"summary":{"id":145},
        "core_items":[
          {"ids":[6672,3124,3302],"play":92134,"win":49015,"pick_rate":0.3896},
          {"ids":[3087,3124,3115],"play":23020,"win":12707,"pick_rate":0.0973},
          {"ids":[3032,3031,3085],"play":1176,"win":707,"pick_rate":0.005},
          {"ids":[3068,3075,3047],"play":40,"win":30,"pick_rate":0.0002}],
        "starter_items":[{"ids":[1055,2003],"play":23399,"win":11406},{"ids":[1086,2003,2003],"play":255880,"win":126541},{"ids":[1083,2003],"play":40,"win":20}],
        "boots":[{"ids":[3006],"play":90000,"win":47000,"pick_rate":0.8},{"ids":[3020],"play":9000,"win":4800,"pick_rate":0.08}],
        "last_items":[{"ids":[3006],"play":50000,"win":1},{"ids":[3115],"play":30000,"win":1},{"ids":[3031],"play":20000,"win":1},{"ids":[3046],"play":15000,"win":1},{"ids":[3089],"play":9000,"win":1}],
        "skills":[{"order":["Q","W","E","Q","Q","R","Q","E"],"play":70297,"win":41378},{"order":["Q","W","E","W","W","R","W","Q"],"play":900,"win":400}],
        "skill_masteries":[{"ids":["Q","E","W"],"builds":[{"order":["Q","W","E","Q","Q","R","Q","E"]}]},{"ids":["W","Q","E"],"builds":[{"order":["Q","W","E","W","W","R","W","Q"]}]}],
        "runes":[{"primary_page_id":8000,"primary_rune_ids":[8008,9111,9104,8017],"secondary_page_id":8100,"secondary_rune_ids":[8139,8135],"stat_mod_ids":[5005,5008,5011],"play":100,"win":50},
                 {"primary_page_id":8000,"primary_rune_ids":[8008],"play":5000,"win":2500}],
        "summoner_spells":[{"ids":[4,7],"play":100,"win":50,"pick_rate":0.9},{"ids":[4],"play":999,"win":1}],
        "counters":[{"champion_id":202,"play":26943,"win":13935},{"champion_id":51,"play":60,"win":40}]}}"#;

    const ADC: Query = Query { mode: "ranked", position: "ADC" };

    #[test]
    fn kaisa_builds_split_into_ap_ad_on_hit_and_crit_ranked_by_score() {
        let b = parse_build(145, ADC, KAISA, &data()).expect("parses");
        // The 40-game tank line is under MIN_GAMES; the rest is best score first.
        let order: Vec<_> = b.cores.iter().map(|c| (c.items.ids.clone(), c.label())).collect();
        assert_eq!(
            order,
            [
                (vec![3032, 3031, 3085], "AD · Crit".to_string()),
                (vec![3087, 3124, 3115], "AP · On-hit".to_string()),
                (vec![6672, 3124, 3302], "AD · On-hit".to_string()),
            ]
        );
        // AD and Crit share the top build, as do AP and On-hit the second: ties keep `Group::ORDER`.
        assert_eq!(b.groups(), [Group::Ad, Group::Crit, Group::Ap, Group::OnHit]);
        assert_eq!(b.in_group(Group::Ad), [0, 2]);
        assert_eq!(b.in_group(Group::Ap), [1]);
        // An AP build keeps AP finishing items and drops Infinity Edge and Phantom Dancer
        // (crit without ability power); boots are never doubled.
        assert_eq!(b.full_build(1, &data()), [3087, 3124, 3115, 3006, 3089]);
        // Nashor's Tooth (AP) stays out of the AD build; Phantom Dancer fits it.
        assert_eq!(b.full_build(2, &data()), [6672, 3124, 3302, 3006, 3031, 3046]);
    }

    #[test]
    fn groups_list_at_most_three_best_builds() {
        let extra = r#"{"ids":[3124,6672,3302],"play":9000,"win":4700},{"ids":[3302,3124,6672],"play":8000,"win":4300},{"ids":[3031,3032,3085],"play":7000,"win":3600},"#;
        let json = KAISA.replacen(r#""core_items":["#, &format!(r#""core_items":[{extra}"#), 1);
        let b = parse_build(145, ADC, &json, &data()).expect("parses");
        let ad = b.in_group(Group::Ad);
        assert_eq!(b.cores.iter().filter(|c| c.in_group(Group::Ad)).count(), 5);
        assert_eq!(ad.len(), GROUP_LIMIT);
        // The best three, in rank order.
        assert!(ad.windows(2).all(|w| w[0] < w[1]) && ad[0] == 0);
    }

    #[test]
    fn options_keep_the_most_played_and_trusted_alternatives() {
        let b = parse_build(145, ADC, KAISA, &data()).expect("parses");
        // Most played first; the 40-game start is dropped, the incomplete rune page and
        // the one-spell entry are skipped, the 60-game matchup is too small.
        assert_eq!(b.starters.iter().map(|s| s.ids.clone()).collect::<Vec<_>>(), [vec![1086, 2003, 2003], vec![1055, 2003]]);
        assert_eq!((b.runes.len(), b.spells.len()), (1, 1));
        assert_eq!(b.counters.iter().map(|m| m.champion_id).collect::<Vec<_>>(), [202]);
        // Each level order maps to the max order OP.GG lists it under.
        assert_eq!(b.skill_priority(0), ["Q", "E", "W"]);
        assert_eq!(b.skill_priority(1), ["W", "Q", "E"]);
        assert!(b.skill_priority(9).is_empty());
    }

    #[test]
    fn ranking_discounts_small_samples() {
        let json = r#"{"data":{"summary":{"id":145},"core_items":[
            {"ids":[3087,3124,3115],"play":100,"win":65},
            {"ids":[6672,3124,3302],"play":10000,"win":6000}]}}"#;
        let b = parse_build(145, ADC, json, &data()).expect("parses");
        let raw = |c: &Choice| rate(c.wins, c.games);
        assert!(raw(&b.cores[1].items) > raw(&b.cores[0].items), "65% beats 60% on raw win rate");
        assert!(b.cores[1].items.few_games() && !b.cores[0].items.few_games());
        // 65 of 100 has a lower bound near 55%; 6,000 of 10,000 stays near 59%.
        assert_eq!(b.cores[0].items.ids, [6672, 3124, 3302]);
        assert_eq!(b.groups()[0], Group::Ad);
    }

    #[test]
    fn tank_builds_and_rejections() {
        let d = data();
        let tank = CoreBuild::classify(Choice { ids: vec![3068, 3075, 3047], ..Choice::default() }, &d);
        assert_eq!((tank.damage, tank.tank, tank.label().as_str()), (Damage::None, true, "Tank"));
        assert!(tank.in_group(Group::Tank) && !tank.in_group(Group::Utility));
        // No damage and not tank (Runaan's carries only tags): still listed, under Utility.
        let utility = CoreBuild::classify(Choice { ids: vec![3085, 3006], ..Choice::default() }, &d);
        assert!(Group::ORDER.iter().filter(|&&g| utility.in_group(g)).eq([&Group::Utility]));
        assert!(parse_build(62, ADC, KAISA, &d).unwrap_err().contains("instead of 62"));
        let sona = r#"{"data":{"summary":{"id":37},"core_items":[{"ids":[3070,3100,3040],"play":1,"win":1}]}}"#;
        assert!(parse_build(37, Query { mode: "ranked", position: "JUNGLE" }, sona, &d).unwrap_err().contains("only 1 game "));
        let none = r#"{"data":{"summary":{"id":37},"core_items":[]}}"#;
        assert!(parse_build(37, ADC, none, &d).unwrap_err().contains("no build"));
        assert!(parse_build(37, ADC, r#"{"data":[]}"#, &d).unwrap_err().contains("format changed"));
        assert!(parse_build(37, ADC, r#"{"data":{"champion":{"id":37}}}"#, &d).unwrap_err().contains("format changed"));
        assert!(parse_build(37, ADC, "<html>", &d).unwrap_err().contains("format changed"));
    }

    #[test]
    fn patch_check_compares_whole_version_parts() {
        let b = parse_build(145, ADC, KAISA, &data()).expect("parses");
        assert!(b.patch_matches("16.20.1"));
        assert!(!b.patch_matches("16.21.1"));
        assert!(!b.patch_matches("16.2.1"));
        let older = MetaBuild { patch: "16.2".into(), ..b };
        assert!(!older.patch_matches("16.20.1"), "16.2 is not a prefix match for 16.20");
    }

    #[test]
    fn builds_json_reads_old_build_only_and_new_entries() {
        let all: HashMap<String, SavedEntry> =
            serde_json::from_str(r#"{"1:ranked": [3089, 3157], "2:aram": {"build": [6655], "spells": [4, 32], "skills": ["Q", "E"]}}"#).unwrap();
        let picks = |k: &str| match &all[k] {
            SavedEntry::Picks(p) => p.clone(),
            SavedEntry::Build(b) => SavedPicks { build: b.clone(), ..SavedPicks::default() },
        };
        assert_eq!(picks("1:ranked"), SavedPicks { build: vec![3089, 3157], ..SavedPicks::default() });
        assert_eq!(picks("2:aram").spells, [4, 32]);
        assert!(picks("2:aram").runes.is_empty());
    }

    #[test]
    fn queues_map_to_opgg_modes_and_lanes() {
        assert_eq!(query_for(420, 11, "MIDDLE"), Ok(Query { mode: "ranked", position: "MID" }));
        assert_eq!(query_for(440, 11, "UTILITY"), Ok(Query { mode: "flex", position: "SUPPORT" }));
        assert_eq!(query_for(2400, 12, "NONE"), Ok(Query { mode: "aram", position: "NONE" }));
        assert_eq!(query_for(1900, 11, "TOP"), Ok(Query { mode: "urf", position: "NONE" }));
        assert_eq!(query_for(1700, 30, "NONE"), Ok(Query { mode: "arena", position: "NONE" }));
        assert!(query_for(430, 11, "NONE").is_err(), "blind pick without a lane");
        assert!(query_for(1830, 33, "NONE").is_err(), "Swarm");
        // Match history has no map: its queues still map to the item pool's mode.
        assert_eq!((mode_for(450, 0), mode_for(400, 0)), (Ok("aram"), Ok("ranked")));
        assert_eq!(Query { mode: "aram", position: "NONE" }.url(115), format!("{BASE}/aram/115/NONE"));
        assert_eq!(Query { mode: "arena", position: "NONE" }.url(145), format!("{BASE}/arena/145"));
        assert_eq!(Query { mode: "ranked", position: "ADC" }.url(145), format!("{BASE}/ranked/145/ADC?tier=emerald_plus"));
    }

    /// Trimmed from a real answer for Ahri, ranked MID, emerald+ (2026-10-19), with a few
    /// entries broken on purpose.
    const AHRI_MID: &str = r#"{"data":{"summary":{"id":103,
        "average_stats":{"play":222269,"win_rate":0.512015,"pick_rate":0.0988588,"ban_rate":0.0296825,"kda":2.529199,"tier":2,"rank":8},
        "positions":[{"name":"TOP","stats":{"play":5000,"win_rate":0.45,"pick_rate":0.002,"role_rate":0.02,"ban_rate":0.0296,"kda":2.0}},
                     {"name":"MID","stats":{"play":211449,"win_rate":0.512228,"pick_rate":0.0935197,"role_rate":0.95132,"ban_rate":0.0296678,"kda":2.547884}}]},
        "core_items":[{"ids":[3115,3089,3087],"play":500,"win":260}],
        "trends":{"total_rank":250,"win":[{"version":"16.20","rate":0.5122},{"version":"16.19","rate":0.5077},{"version":"16.18","rate":null},{"version":"16.17","rate":0.5091}]},
        "game_lengths":[{"game_length":25,"rate":0.522977,"average":0.5},{"game_length":0,"rate":0.499609,"average":0.5},{"game_length":30,"rate":"n/a"},
                        {"game_length":35,"rate":0.506122},{"game_length":40,"rate":0.500292}]}}"#;

    const MID: Query = Query { mode: "ranked", position: "MID" };

    #[test]
    fn summary_prefers_lane_numbers_and_orders_trend_and_game_lengths() {
        let s = parse_build(103, MID, AHRI_MID, &data()).expect("parses").summary.expect("has a summary");
        // The MID entry wins over the champion-wide numbers (222,269 games, 51.2015%).
        assert_eq!((s.games, s.win_rate, s.lane_share), (211449, Some(0.512228), Some(0.95132)));
        // OP.GG lists patches newest first; the chart wants oldest first. The null rate is skipped.
        let patches: Vec<_> = s.trend.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(patches, ["16.17", "16.19", "16.20"]);
        // Buckets sorted by their first minute; the bucket with a text rate is dropped.
        assert_eq!(s.game_lengths, [(0, 0.499609), (25, 0.522977), (35, 0.506122), (40, 0.500292)]);
        assert_eq!(
            s.line("Ahri", MID),
            "Ahri mid · 51.2% WR · 9.4% pick · 3.0% ban · KDA 2.55 · 211,449 games · 95% of Ahri's games are mid"
        );
        // A lane OP.GG does not list falls back to the champion-wide numbers, without a lane share.
        let jungle = Query { mode: "ranked", position: "JUNGLE" };
        let s = parse_build(103, jungle, AHRI_MID, &data()).expect("parses").summary.expect("has a summary");
        assert_eq!((s.games, s.win_rate, s.lane_share), (222269, Some(0.512015), None));
    }

    #[test]
    fn summary_is_absent_safe() {
        let aram = Query { mode: "aram", position: "NONE" };
        // ARAM: no positions, no bans, no game lengths.
        let json = r#"{"data":{"summary":{"id":103,"average_stats":{"play":50000,"win_rate":0.53,"pick_rate":0.2,"ban_rate":null,"kda":3.1},"positions":null},
            "core_items":[{"ids":[3115,3089,3087],"play":500,"win":260}],"trends":{"win":[{"version":"16.20","rate":0.53}]},"game_lengths":null}}"#;
        let s = parse_build(103, aram, json, &data()).expect("parses").summary.expect("has a summary");
        assert_eq!((s.ban_rate, s.lane_share, s.game_lengths.len(), s.trend.len()), (None, None, 0, 1));
        assert_eq!(s.line("Ahri", aram), "Ahri · 53.0% WR · 20.0% pick · KDA 3.10 · 50,000 games");
        // A reshaped summary leaves only the summary out; the builds still load.
        let odd = r#"{"data":{"summary":{"id":103,"average_stats":"gone","positions":{"MID":1}},
            "core_items":[{"ids":[3115,3089,3087],"play":500,"win":260}],"trends":[1,2],"game_lengths":{"0":0.5}}}"#;
        let b = parse_build(103, MID, odd, &data()).expect("builds still parse");
        assert_eq!((b.summary, b.cores.len()), (None, 1));
        // No games counted: no summary.
        let none = r#"{"data":{"summary":{"id":103},"core_items":[{"ids":[3115,3089,3087],"play":500,"win":260}]}}"#;
        assert!(parse_build(103, MID, none, &data()).expect("parses").summary.is_none());
    }

    /// Shaped like OP.GG's Arena answer for Ahri (2026-10-19): wins, placements, no win rates.
    /// Trends, game lengths and counters are added to show they are ignored for Arena.
    const ARENA_JSON: &str = r#"{"data":{"summary":{"id":103,
        "average_stats":{"win":46099,"play":89223,"total_place":306630,"first_place":15544,"pick_rate":0.131839,"ban_rate":0.0517615,"kills":660466,"assists":945544,"deaths":625150,"tier":2,"rank":48}},
        "core_items":[{"ids":[3115,3089,3087],"play":500,"win":400,"total_place":1200,"pick_rate":0.01},
                      {"ids":[6672,3124,3302],"play":5000,"win":2000,"total_place":24000,"pick_rate":0.1},
                      {"ids":[3032,3031,3085],"play":1000,"win":950,"total_place":1500,"pick_rate":0.02}],
        "boots":[{"ids":[3006],"win":15201,"play":35772,"total_place":134794,"first_place":4435,"pick_rate":0.8053}],
        "skills":[{"order":["Q","W","E","Q"],"play":25729,"win":16955,"total_place":73152,"pick_rate":0.4186}],
        "trends":{"win":[{"version":"16.20","rate":0.51}]},"game_lengths":[{"game_length":0,"rate":0.5}],
        "counters":[{"champion_id":202,"play":26943,"win":13935}]}}"#;

    #[test]
    fn arena_shows_no_win_rates_and_ranks_by_games() {
        let arena = Query { mode: "arena", position: "NONE" };
        let b = parse_build(103, arena, ARENA_JSON, &data()).expect("parses");
        // Most played first, although the 95%-win 1,000-game build would top a Wilson ranking.
        let games: Vec<u64> = b.cores.iter().map(|c| c.items.games).collect();
        assert_eq!(games, [5000, 1000, 500]);
        assert_eq!(b.groups(), [Group::Ad, Group::OnHit, Group::Crit, Group::Ap]);
        assert_eq!(b.boots[0].stats(), "80.5% pick · 35,772 games");
        assert_eq!(b.skills[0].stats(), "41.9% pick · 25,729 games");
        let lines: Vec<String> = b.cores.iter().map(|c| c.items.stats()).chain([b.boots[0].stats(), b.skills[0].stats()]).collect();
        assert!(lines.iter().all(|l| !l.contains("WR") && !l.contains("avg place")), "{lines:?}");
        // "Few games" follows the sample size only: 1,000 games is few for Arena at any win rate,
        // while the same record elsewhere has a narrow margin.
        assert!(b.cores[1].items.few_games() && !b.cores[0].items.few_games());
        assert!(!Choice { games: 1000, wins: 950, ..Choice::default() }.few_games());
        assert!(b.counters.is_empty());
        let s = b.summary.expect("has a summary");
        assert_eq!((s.games, s.win_rate, s.pick_rate, s.ban_rate), (89223, None, Some(0.131839), Some(0.0517615)));
        assert!(s.trend.is_empty() && s.game_lengths.is_empty());
        // KDA from the kill, death and assist totals: (660,466 + 945,544) / 625,150.
        assert_eq!(s.line("Ahri", arena), "Ahri · 13.2% pick · 5.2% ban · KDA 2.57 · 89,223 games");
    }
}
