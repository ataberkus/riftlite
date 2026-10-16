//! Aggregated champion builds from OP.GG.
//!
//! Source: `https://lol-api-champion.op.gg/api/global/champions/{mode}/{id}/{lane}`,
//! the JSON endpoint behind op.gg's champion pages. It lists up to 15 core
//! builds per champion with games, wins and pick rate. OP.GG's public MCP server
//! (`mcp-api.op.gg`) returns only the single most played core build, so it cannot
//! offer several builds to pick from. OP.GG permits use of its data with the
//! source cited and without excessive requests: riftlite sends one request per
//! picked champion, containing only the champion id, mode and lane, and labels
//! everything it writes "OP.GG".
//!
//! Builds are classified from Data Dragon item data (`ddragon::ItemStats`), never
//! from item names: AP or AD by which damage stat the core items mostly give, and
//! Crit / On-hit / Tank by the item tags.

use serde::Deserialize;

use crate::ddragon::StaticData;

const BASE: &str = "https://lol-api-champion.op.gg/api/global/champions";
/// op.gg's default rank filter for ranked tables.
const RANKED_TIER: &str = "emerald_plus";
/// Below this many games OP.GG still lists a build, but its win rate is noise.
pub const MIN_GAMES: u64 = 100;

/// What to ask OP.GG for: its game mode and lane names.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
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
        let base = format!("{BASE}/{}/{champion_id}/{}", self.mode, self.position);
        // ARAM, URF and Nexus Blitz have no rank tiers; a tier filter only shrinks the sample.
        if matches!(self.mode, "ranked" | "flex") { format!("{base}?tier={RANKED_TIER}") } else { base }
    }
}

/// OP.GG mode for a queue, and its lane from the champ-select position
/// (`TOP`/`JUNGLE`/`MIDDLE`/`BOTTOM`/`UTILITY`/`NONE`, see `lcu::perk_position`).
pub fn query_for(queue_id: i64, map_id: i64, position: &str) -> Result<Query, String> {
    let mode = match queue_id {
        // ARAM Mayhem has no OP.GG table of its own; its item pool is ARAM's.
        450 | 2400 => "aram",
        900 | 1900 | 1010 => "urf",
        1300 => "nexus_blitz",
        440 => "flex",
        1700 | 1710 | 1810..=1840 => return Err("OP.GG has no builds for this mode".into()),
        _ if map_id == 12 => "aram",
        _ if map_id == 11 || map_id == 0 => "ranked",
        _ => return Err("OP.GG has no builds for this map".into()),
    };
    let position = match mode {
        "aram" | "urf" | "nexus_blitz" => "NONE",
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
}

impl Choice {
    pub fn win_rate(&self) -> f64 {
        if self.games == 0 { 0.0 } else { self.wins as f64 / self.games as f64 }
    }

    /// "53% WR · 39% pick · 92,134 games"
    pub fn stats(&self) -> String {
        format!("{:.1}% WR · {:.1}% pick · {} games", self.win_rate() * 100.0, self.pick_rate * 100.0, thousands(self.games))
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
}

impl Runes {
    /// Keystone, 3 primary, 2 secondary, 3 shards, like the client's pages; `None` if incomplete.
    pub fn perks(&self) -> Option<Vec<i64>> {
        let p: Vec<i64> = self.primary_rune_ids.iter().chain(&self.secondary_rune_ids).chain(&self.stat_mod_ids).copied().collect();
        (self.primary_rune_ids.len() == 4 && self.secondary_rune_ids.len() == 2 && self.stat_mod_ids.len() == 3).then_some(p)
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

    /// Whether a later item fits this build's damage (an AP build skips AD items and vice versa).
    fn fits(&self, s: &crate::ddragon::ItemStats) -> bool {
        match self.damage {
            Damage::Ap => s.ad <= s.ap,
            Damage::Ad => s.ap <= s.ad,
            Damage::None => s.ap == 0.0 && s.ad == 0.0,
        }
    }
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Summary {
    id: i64,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Mastery {
    ids: Vec<String>,
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
}

#[derive(Deserialize)]
struct Envelope {
    data: RawData,
}

/// A champion's OP.GG builds for one mode and lane.
#[derive(Clone, PartialEq, Debug)]
pub struct MetaBuild {
    pub champion_id: i64,
    pub query: Query,
    /// Core builds with at least `MIN_GAMES` games, highest win rate first. Never empty.
    pub cores: Vec<CoreBuild>,
    pub starter: Option<Choice>,
    /// Boots options, most played first.
    pub boots: Vec<Choice>,
    /// Single items players finished with, most played first.
    pub last_items: Vec<Choice>,
    pub skills: Option<Skills>,
    /// Max order, e.g. `["Q","E","W"]`.
    pub skill_priority: Vec<String>,
    pub runes: Option<Runes>,
    pub spells: Option<Choice>,
}

impl MetaBuild {
    /// Groups that have at least one build, ordered by their top build: win rate, then
    /// games, then `Group::ORDER`.
    pub fn groups(&self) -> Vec<Group> {
        // `cores` is sorted by win rate then games, so a group's first member is its top build.
        let mut groups: Vec<(Group, &Choice)> =
            Group::ORDER.into_iter().filter_map(|g| Some((g, &self.cores.iter().find(|c| c.in_group(g))?.items))).collect();
        // Stable sort: equal win rate and games keep `Group::ORDER`.
        groups.sort_by(|(_, a), (_, b)| b.win_rate().total_cmp(&a.win_rate()).then(b.games.cmp(&a.games)));
        groups.into_iter().map(|(g, _)| g).collect()
    }

    /// Indices into `cores` of a group's builds, highest win rate first.
    pub fn in_group(&self, g: Group) -> Vec<usize> {
        (0..self.cores.len()).filter(|&i| self.cores[i].in_group(g)).collect()
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
}

/// Fetches, validates and classifies the builds. Blocking (about a second); call it off the poll thread.
pub fn fetch(agent: &ureq::Agent, data: &StaticData, champion_id: i64, query: Query) -> Result<MetaBuild, String> {
    if data.item_stats.is_empty() {
        return Err("Data Dragon item data is not loaded yet; builds cannot be classified".into());
    }
    let resp = match agent.get(&query.url(champion_id)).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(404, _)) => return Err("OP.GG does not know this champion".into()),
        Err(ureq::Error::Status(code, _)) => return Err(format!("OP.GG HTTP {code}")),
        Err(e) => return Err(format!("OP.GG unreachable: {e}")),
    };
    let text = resp.into_string().map_err(|e| format!("OP.GG read: {e}"))?;
    parse_build(champion_id, query, &text, data)
}

/// Turns OP.GG's JSON into classified builds, rejecting other champions and tiny samples.
pub fn parse_build(champion_id: i64, query: Query, json: &str, data: &StaticData) -> Result<MetaBuild, String> {
    let raw = serde_json::from_str::<Envelope>(json).map_err(|e| format!("OP.GG build: {e}"))?.data;
    if raw.summary.id != champion_id {
        return Err(format!("OP.GG answered for champion {} instead of {champion_id}", raw.summary.id));
    }
    let most = raw.core_items.iter().map(|c| c.games).max().unwrap_or(0);
    let mut cores: Vec<CoreBuild> = raw
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
    cores.sort_by(|a, b| b.items.win_rate().total_cmp(&a.items.win_rate()).then(b.items.games.cmp(&a.items.games)));
    let first = |v: Vec<Choice>| v.into_iter().find(|c| !c.ids.is_empty());
    Ok(MetaBuild {
        champion_id,
        query,
        cores,
        starter: first(raw.starter_items),
        boots: raw.boots.into_iter().filter(|b| !b.ids.is_empty()).collect(),
        last_items: raw.last_items,
        skills: raw.skills.into_iter().find(|s| !s.order.is_empty()),
        skill_priority: raw.skill_masteries.into_iter().next().map(|m| m.ids).unwrap_or_default(),
        runes: raw.runes.into_iter().next(),
        spells: first(raw.summoner_spells).filter(|c| c.ids.len() == 2),
    })
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
    const KAISA: &str = r#"{"data":{"summary":{"id":145},
        "core_items":[
          {"ids":[6672,3124,3302],"play":92134,"win":49015,"pick_rate":0.3896},
          {"ids":[3087,3124,3115],"play":23020,"win":12707,"pick_rate":0.0973},
          {"ids":[3032,3031,3085],"play":1176,"win":707,"pick_rate":0.005},
          {"ids":[3068,3075,3047],"play":40,"win":30,"pick_rate":0.0002}],
        "starter_items":[{"ids":[1055,2003],"play":9000,"win":4600,"pick_rate":0.9}],
        "boots":[{"ids":[3006],"play":90000,"win":47000,"pick_rate":0.8},{"ids":[3020],"play":9000,"win":4800,"pick_rate":0.08}],
        "last_items":[{"ids":[3006],"play":50000,"win":1},{"ids":[3115],"play":30000,"win":1},{"ids":[3031],"play":20000,"win":1},{"ids":[3089],"play":9000,"win":1}],
        "skills":[{"order":["Q","E","W","Q"],"play":10,"win":5}],
        "skill_masteries":[{"ids":["Q","E","W"],"play":10,"win":5}],
        "runes":[{"primary_page_id":8000,"primary_rune_ids":[8008,9111,9104,8017],"secondary_page_id":8100,"secondary_rune_ids":[8139,8135],"stat_mod_ids":[5005,5008,5011],"play":100,"win":50}],
        "summoner_spells":[{"ids":[4,7],"play":100,"win":50,"pick_rate":0.9}]}}"#;

    const ADC: Query = Query { mode: "ranked", position: "ADC" };

    #[test]
    fn kaisa_builds_split_into_ap_ad_on_hit_and_crit_sorted_by_win_rate() {
        let b = parse_build(145, ADC, KAISA, &data()).expect("parses");
        // The 40-game tank line is under MIN_GAMES; the rest is highest win rate first.
        let order: Vec<_> = b.cores.iter().map(|c| (c.items.ids.clone(), c.label())).collect();
        assert_eq!(
            order,
            [
                (vec![3032, 3031, 3085], "AD · Crit".to_string()),
                (vec![3087, 3124, 3115], "AP · On-hit".to_string()),
                (vec![6672, 3124, 3302], "AD · On-hit".to_string()),
            ]
        );
        // AD and Crit share the 60.1% top build, as do AP and On-hit at 55.2%: ties keep `Group::ORDER`.
        assert_eq!(b.groups(), [Group::Ad, Group::Crit, Group::Ap, Group::OnHit]);
        assert_eq!(b.in_group(Group::Ad), [0, 2]);
        assert_eq!(b.in_group(Group::Ap), [1]);
        // An AP build keeps AP finishing items and drops Infinity Edge; boots are never doubled.
        assert_eq!(b.full_build(1, &data()), [3087, 3124, 3115, 3006, 3089]);
        // Nashor's Tooth (AP) stays out of the AD build.
        assert_eq!(b.full_build(2, &data()), [6672, 3124, 3302, 3006, 3031]);
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
    }

    #[test]
    fn queues_map_to_opgg_modes_and_lanes() {
        assert_eq!(query_for(420, 11, "MIDDLE"), Ok(Query { mode: "ranked", position: "MID" }));
        assert_eq!(query_for(440, 11, "UTILITY"), Ok(Query { mode: "flex", position: "SUPPORT" }));
        assert_eq!(query_for(2400, 12, "NONE"), Ok(Query { mode: "aram", position: "NONE" }));
        assert_eq!(query_for(1900, 11, "TOP"), Ok(Query { mode: "urf", position: "NONE" }));
        assert!(query_for(430, 11, "NONE").is_err(), "blind pick without a lane");
        assert!(query_for(1700, 30, "NONE").is_err(), "Arena");
        assert_eq!(Query { mode: "aram", position: "NONE" }.url(115), format!("{BASE}/aram/115/NONE"));
        assert_eq!(Query { mode: "ranked", position: "ADC" }.url(145), format!("{BASE}/ranked/145/ADC?tier=emerald_plus"));
    }
}
