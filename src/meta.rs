//! Aggregated champion builds from OP.GG.
//!
//! Source: `https://lol-api-champion.op.gg/api/global/champions/{mode}/{id}/{lane}`
//! (`arena/{id}` for Arena), the JSON endpoint behind op.gg's champion pages. It
//! lists up to 15 core builds per champion with games, wins and pick rate, plus
//! the most played starting items, boots, rune pages, summoner spells, skill
//! orders and lane matchups. OP.GG's public MCP server (`mcp-api.op.gg`) returns
//! only the single most played core build, so it cannot offer several builds to
//! pick from. OP.GG permits use of its data with the source cited and without
//! excessive requests: riftlite sends one request per picked champion, containing
//! only the champion id, mode and lane, and labels everything it writes "OP.GG".
//!
//! Builds are classified from Data Dragon item data (`ddragon::ItemStats`), never
//! from item names: AP or AD by which damage stat the core items mostly give, and
//! Crit / On-hit / Tank by the item tags.

use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;

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
fn margin(wins: u64, games: u64) -> f64 {
    if games == 0 {
        return 100.0;
    }
    let p = rate(wins, games);
    Z * (p * (1.0 - p) / games as f64).sqrt() * 100.0
}

/// Lower end of the 95 % Wilson interval: a win rate that small samples cannot inflate.
fn wilson_lower(wins: u64, games: u64) -> f64 {
    if games == 0 {
        return 0.0;
    }
    let (n, p) = (games as f64, rate(wins, games));
    let z2 = Z * Z;
    (p + z2 / (2.0 * n) - Z * (p * (1.0 - p) / n + z2 / (4.0 * n * n)).sqrt()) / (1.0 + z2 / n)
}

/// "53.2% ±0.3 WR · 39.0% pick · 92,134 games" (avg place appended for Arena).
fn stats_line(wins: u64, games: u64, pick_rate: Option<f64>, total_place: u64) -> String {
    let mut s = format!("{:.1}% ±{:.1} WR", rate(wins, games) * 100.0, margin(wins, games));
    if let Some(p) = pick_rate {
        s += &format!(" · {:.1}% pick", p * 100.0);
    }
    s += &format!(" · {} games", thousands(games));
    if total_place > 0 && games > 0 {
        s += &format!(" · avg place {:.1}", total_place as f64 / games as f64);
    }
    s
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
    /// Arena only: sum of final placements (1–8).
    pub total_place: u64,
}

impl Choice {
    pub fn win_rate(&self) -> f64 {
        rate(self.wins, self.games)
    }

    /// The win rate's 95 % margin is at least `FEW_GAMES_MARGIN` points.
    pub fn few_games(&self) -> bool {
        margin(self.wins, self.games) >= FEW_GAMES_MARGIN
    }

    /// Ranking key: the low end of the win rate's 95 % Wilson interval, so a few lucky games
    /// cannot outrank a much-played build.
    fn score(&self) -> f64 {
        wilson_lower(self.wins, self.games)
    }

    pub fn stats(&self) -> String {
        stats_line(self.wins, self.games, Some(self.pick_rate), self.total_place)
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
}

impl Skills {
    pub fn few_games(&self) -> bool {
        margin(self.wins, self.games) >= FEW_GAMES_MARGIN
    }

    pub fn stats(&self) -> String {
        stats_line(self.wins, self.games, Some(self.pick_rate), 0)
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
}

impl Runes {
    /// Keystone, 3 primary, 2 secondary, 3 shards, like the client's pages; `None` if incomplete.
    pub fn perks(&self) -> Option<Vec<i64>> {
        let p: Vec<i64> = self.primary_rune_ids.iter().chain(&self.secondary_rune_ids).chain(&self.stat_mod_ids).copied().collect();
        (self.primary_rune_ids.len() == 4 && self.secondary_rune_ids.len() == 2 && self.stat_mod_ids.len() == 3).then_some(p)
    }

    pub fn few_games(&self) -> bool {
        margin(self.wins, self.games) >= FEW_GAMES_MARGIN
    }

    pub fn stats(&self) -> String {
        stats_line(self.wins, self.games, Some(self.pick_rate), 0)
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
        stats_line(self.wins, self.games, None, 0)
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
    fn fits(&self, s: &crate::ddragon::ItemStats) -> bool {
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

#[derive(Deserialize, Default)]
#[serde(default)]
struct Summary {
    id: i64,
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
    /// Lane matchups with at least `MIN_GAMES` games (none outside ranked and flex).
    pub counters: Vec<Matchup>,
    masteries: Vec<Mastery>,
}

impl MetaBuild {
    /// Ranks `cores` by `Choice::score`, then games.
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

    /// Indices into `cores` of a group's builds, best first.
    pub fn in_group(&self, g: Group) -> Vec<usize> {
        (0..self.cores.len()).filter(|&i| self.cores[i].in_group(g)).collect()
    }

    /// Index of the core build with exactly these items.
    pub fn index_of(&self, ids: &[i64]) -> Option<usize> {
        self.cores.iter().position(|c| c.items.ids == ids)
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
    let most = raw.core_items.iter().map(|c| c.games).max().unwrap_or(0);
    let cores: Vec<CoreBuild> = raw
        .core_items
        .into_iter()
        .filter(|c| !c.ids.is_empty() && c.games >= MIN_GAMES)
        .map(|c| CoreBuild::classify(c, data))
        .collect();
    if cores.is_empty() {
        return Err(match most {
            0 => "OP.GG has no build for this champion and mode".into(),
            1 => format!("OP.GG has only 1 game of this build ({}); too few to trust", query.label()),
            n => format!("OP.GG has only {n} games of this build ({}); too few to trust", query.label()),
        });
    }
    let items = |v: Vec<Choice>| trusted(v.into_iter().filter(|c| !c.ids.is_empty()).collect(), |c| c.games);
    let mut build = MetaBuild {
        champion_id,
        query,
        patch: env.meta.version,
        cores,
        starters: items(raw.starter_items),
        boots: items(raw.boots),
        skills: trusted(raw.skills.into_iter().filter(|s| !s.order.is_empty()).collect(), |s| s.games),
        runes: trusted(raw.runes.into_iter().filter(|r| r.perks().is_some()).collect(), |r| r.games),
        spells: trusted(raw.summoner_spells.into_iter().filter(|c| c.ids.len() == 2).collect(), |c| c.games),
        last_items: raw.last_items,
        counters: raw.counters.into_iter().filter(|m| m.games >= MIN_GAMES).collect(),
        masteries: raw.skill_masteries,
    };
    build.sort();
    Ok(build)
}

fn choices_path() -> PathBuf {
    crate::settings::local_dir().join("builds.json")
}

/// `"<championId>:<mode>"` → core item ids of the OP.GG build picked last.
fn load_choices() -> HashMap<String, Vec<i64>> {
    std::fs::read(choices_path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

/// Core items of the build last picked for this champion and OP.GG mode.
pub fn saved_build(champion_id: i64, mode: &str) -> Option<Vec<i64>> {
    load_choices().remove(&format!("{champion_id}:{mode}"))
}

/// Remembers a picked build, in `%LOCALAPPDATA%\riftlite\builds.json`; nothing leaves the machine.
pub fn save_build(champion_id: i64, mode: &str, ids: &[i64]) -> Result<(), String> {
    let mut all = load_choices();
    all.insert(format!("{champion_id}:{mode}"), ids.to_vec());
    let p = choices_path();
    std::fs::create_dir_all(p.parent().expect("builds path has a parent")).map_err(|e| e.to_string())?;
    let tmp = p.with_extension("json.tmp");
    std::fs::write(&tmp, serde_json::to_vec_pretty(&all).expect("builds serialize"))
        .and_then(|_| std::fs::rename(&tmp, &p))
        .map_err(|e| e.to_string())
}

fn thousands(n: u64) -> String {
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
        ItemStats { ap, ad, crit, on_hit, defensive, boots }
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
        assert!(b.cores[1].items.win_rate() > b.cores[0].items.win_rate(), "65% beats 60% on raw win rate");
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
}
