//! In-game builds per playstyle (AP, Bruiser, Crit / On-Hit, …) from Blitz, laid out like
//! blitz.gg's champion pages: starting items, build order, completed and situational items,
//! summoner spells and the skill order, plus (ARAM Mayhem) the champion's augment tiers and
//! Mayhem-only items.
//!
//! Sources (undocumented, may change):
//! - `data.v2.iesdev.com/api/v1/query_objects/prod/lol/champion_builds_tags`, the JSON behind
//!   blitz.gg's champion build pages. For one champion and queue it lists one build per
//!   playstyle tag set, each with games, win rate, starting items, core builds, component
//!   orders per item, items bought after a core, situational items, summoner spells and skill
//!   orders. Blitz has tables for ranked solo/duo (per lane; also used for other Summoner's
//!   Rift games), ARAM (also used for ARAM Mayhem, as blitz.gg's Mayhem pages do) and URF.
//! - `…/aram_mayhem_champion?champion_id=`, blitz.gg's champion ARAM Mayhem page: augment and
//!   item tiers on the champion. Augment names, rarities and icon paths come from the League
//!   client's `/lol-game-data/assets/v1/cherry-augments.json`; the icons from CommunityDragon.
//!
//! Only the champion id, the queue and the lane are sent to Blitz.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use serde_json::Value;

use crate::augments::{self, Tier};
use crate::ddragon::StaticData;
use crate::lcu::{Creds, Lcu};
use crate::net;
use crate::worker::{Shared, set};

const URL: &str = "https://data.v2.iesdev.com/api/v1/query_objects/prod/lol/champion_builds_tags";
/// CommunityDragon's mirror of the client's `/lol-game-data/assets/` files.
const CDRAGON: &str = "https://raw.communitydragon.org/latest/plugins/rcp-be-lol-game-data/global/default/";
/// Blitz aggregates about daily.
const TTL: Duration = Duration::from_secs(6 * 3600);
const RETRY: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_secs(1);
/// Blitz's item tables (blitz-desktop `CfOGcc_p.js`, build 3.0.0-beta.1787885272).
mod blitz_items {
    /// Boots, every tier and mode variant.
    pub const BOOTS: &[i64] = &[
        1001, 3158, 3006, 3008, 3009, 3020, 3047, 3111, 3117, 2422, 3005, 3010, 3013, 223005, 223111, 223047, 223006, 223008, 223009, 223020,
        223158, 773006, 773009, 771001, 773020, 773047, 773111, 773117, 773158,
    ];
    pub const BASIC_BOOTS: i64 = 1001;
    /// Potions and elixirs a start may hold.
    pub const CONSUMABLES: &[i64] = &[2003, 2031, 2139, 2138, 2140];
    /// Starter items (Doran's, Dark Seal, Cull, Tear-free starters, Arena variants).
    pub const STARTERS: &[i64] = &[1054, 1055, 1056, 1086, 1120, 2049, 2050, 222051, 223112, 223177, 223184, 223185];
    pub const TEAR: i64 = 3070;
    /// Tear and what it becomes.
    pub const TEAR_FAMILY: &[i64] = &[3070, 3004, 3043, 3003, 3040, 3119, 3121, 2526];
    /// Transformed items never listed from situational stats.
    pub const NOT_SITUATIONAL: &[i64] = &[6657, 3003, 3040, 3119, 3121, 3004, 3043];
    pub const SUPPORT_QUEST: &[i64] = &[3869, 3870, 3871, 3876, 3877];
    pub const JUNGLE_PETS: &[i64] = &[1101, 1102, 1103];
    pub const GUSTWALKER: i64 = 1102;
    pub const HEALTH_POTION: i64 = 2003;
    pub const HEARTSTEEL: i64 = 3084;
    /// Skipped in build orders.
    pub const NOT_IN_ORDER: i64 = 3867;
    /// Items sharing a unique passive: a later one is dropped when an earlier one has it.
    pub const GROUPS: &[&[i64]] = &[
        &[3003, 3004, 3042, 3040, 3119, 3121, 2526, 2530],
        &[6673, 3053, 3156, 3155, 3040, 2525],
        &[6694, 3036, 3035, 3033],
        &[6035, 3139],
        &[3748, 3074, 6698, 6631],
        &[3100, 6632, 3078, 6662, 2510, 3508],
        &[6694, 3071, 3036, 3033, 3302],
        &[3135, 3137],
        &[3068, 6664],
    ];
}
use blitz_items as bi;
/// Situational items Blitz lists after the final build.
const SITUATIONAL_LIMIT: usize = 7;
/// Blitz's playstyle tags with an English title; other tags are not shown.
const TAGS: &[&str] = &[
    "Tank", "Crit", "On-Hit", "Mobility", "Utility", "Scaling", "Bruiser", "Fighter", "Burst", "Lethality", "Heal & Shield", "CC", "AP", "AD", "Hybrid",
    "Sustain", "Burn", "Poke", "Defensive",
];
const JACK_OF_ALL_TRADES: i64 = 8316;
/// Augments listed per rarity (blitz.gg's page shows six before "Show all").
pub const AUGMENTS_PER_RARITY: usize = 6;
pub const KEYS: [char; 4] = ['Q', 'W', 'E', 'R'];

/// What to ask Blitz for.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Query {
    /// Blitz's queue name.
    pub queue: &'static str,
    /// `TOP`/`JUNGLE`/`MIDDLE`/`BOTTOM`/`UTILITY`; `None` for all lanes.
    pub position: Option<&'static str>,
    /// ARAM Mayhem: also the champion's augment and item tiers.
    pub mayhem: bool,
}

impl Query {
    /// The Blitz table for a live game (`gameMode`, map id, your position), `None` when Blitz has none.
    pub fn for_game(mode: &str, map: i64, position: &str) -> Option<Query> {
        let queue = match (mode, map) {
            ("URF" | "ARURF", _) => "SUMMONERS_RIFT_URF",
            (_, 12) => "HOWLING_ABYSS_ARAM",
            ("CLASSIC" | "SWIFTPLAY", 11) => "RANKED_SOLO_5X5",
            _ => return None,
        };
        let position = match position {
            _ if queue != "RANKED_SOLO_5X5" => None,
            "TOP" => Some("TOP"),
            "JUNGLE" => Some("JUNGLE"),
            "MIDDLE" => Some("MIDDLE"),
            "BOTTOM" => Some("BOTTOM"),
            "UTILITY" => Some("UTILITY"),
            _ => None,
        };
        Some(Query { queue, position, mayhem: mode == "KIWI" })
    }

    /// The Blitz table for champ select (queue id, map id, your assigned position), `None`
    /// when Blitz has none. Matches `for_game` for the game that follows.
    pub fn for_queue(queue_id: i64, map: i64, position: &str) -> Option<Query> {
        let mode = match (queue_id, map) {
            (2400, _) => "KIWI",
            (900 | 1900 | 1010, _) => "URF",
            (450, _) | (_, 12) => "ARAM",
            (1300 | 1700 | 1710 | 1810..=1840, _) => return None,
            (_, 11) => "CLASSIC",
            _ => return None,
        };
        Query::for_game(mode, if mode == "KIWI" || mode == "ARAM" { 12 } else { 11 }, &position.to_uppercase())
    }

    pub fn label(&self) -> String {
        let mode = match self.queue {
            _ if self.mayhem => "ARAM Mayhem",
            "HOWLING_ABYSS_ARAM" => "ARAM",
            "SUMMONERS_RIFT_URF" => "URF",
            _ => "ranked",
        };
        match self.position {
            Some(p) => format!("{mode} {}", p.to_lowercase()),
            None => mode.to_string(),
        }
    }

    fn url(&self, champion_id: i64) -> String {
        // Blitz's own pages add the build tier only to ranked queries; ARAM rejects it.
        let mut url = format!("{URL}?champion_id={champion_id}&queue_id={}", self.queue);
        if self.queue == "RANKED_SOLO_5X5" {
            url.push_str("&build_tier=Standard");
        }
        if let Some(p) = self.position {
            url.push_str(&format!("&individual_position={p}"));
        }
        url
    }
}

/// Games and win rate of one option.
#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct Record {
    pub games: u64,
    pub win_rate: Option<f64>,
}

impl Record {
    fn of(v: &Value) -> Record {
        Record { games: games(v), win_rate: v["win_rate"].as_f64().filter(|w| (0.0..=1.0).contains(w)) }
    }

    /// "48% · 1,247 games"
    pub fn text(&self) -> String {
        match self.win_rate {
            Some(w) => format!("{:.0}% · {} games", w * 100.0, thousands(self.games)),
            None => format!("{} games", thousands(self.games)),
        }
    }
}

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

/// One skill order: the ability (0 Q … 3 R) levelled at each champion level.
#[derive(Clone, PartialEq, Debug)]
pub struct SkillOrder {
    pub order: Vec<usize>,
    pub record: Record,
}

impl SkillOrder {
    /// Q, W and E in the order they are maxed (fifth point first; never maxed: most points, then first taken).
    pub fn max_order(&self) -> Vec<char> {
        let mut keys: Vec<(usize, usize, usize, usize)> = (0..3)
            .map(|k| {
                let fifth = self.order.iter().enumerate().filter(|&(_, &a)| a == k).nth(4).map_or(usize::MAX, |(i, _)| i);
                let points = self.order.iter().filter(|&&a| a == k).count();
                let first = self.order.iter().position(|&a| a == k).unwrap_or(usize::MAX);
                (k, fifth, points, first)
            })
            .collect();
        keys.sort_by(|a, b| a.1.cmp(&b.1).then(b.2.cmp(&a.2)).then(a.3.cmp(&b.3)));
        keys.into_iter().map(|(k, ..)| KEYS[k]).collect()
    }

    /// The ability (0 Q … 3 R) to level next: the first point of this order you have not
    /// taken that you can take now. `None` when no point is unspent.
    pub fn next(&self, levels: [i64; 4], champion_level: i64) -> Option<usize> {
        self.next_step(levels, champion_level).map(|i| self.order[i])
    }

    /// Index in `order` of the point `next` suggests.
    pub fn next_step(&self, levels: [i64; 4], champion_level: i64) -> Option<usize> {
        if levels.iter().sum::<i64>() >= champion_level {
            return None;
        }
        let mut counts = [0i64; 4];
        for (i, &k) in self.order.iter().enumerate() {
            counts[k] += 1;
            if counts[k] > levels[k] && can_level(k, levels, champion_level) {
                return Some(i);
            }
        }
        None
    }
}

/// League's levelling rules: basic abilities up to 5 and at most half the champion level
/// (rounded up), the ultimate at 6, 11 and 16.
fn can_level(k: usize, levels: [i64; 4], champion_level: i64) -> bool {
    if k == 3 {
        let allowed = match champion_level {
            16.. => 3,
            11.. => 2,
            6.. => 1,
            _ => 0,
        };
        levels[3] < allowed
    } else {
        levels[k] < 5 && levels[k] < (champion_level + 1) / 2
    }
}

/// One playstyle's build, assembled the way the Blitz app assembles it.
#[derive(Clone, PartialEq, Debug)]
pub struct Style {
    /// Blitz's title: its tags joined with " / " (e.g. "Crit / On-Hit"), "Jack of All Trades" or
    /// "Well Rounded" for uncategorized builds, "Blitz Build" without tags.
    pub title: String,
    pub record: Record,
    /// Blitz's start: the most picked starting set made of potions, starters and core components.
    pub starting: Vec<i64>,
    /// The first three items of the final build, each after its most picked component order.
    pub build_order: Vec<i64>,
    /// The most played core (picked by more than 5% of games), the items bought after it, then
    /// the most bought items; six (seven for bot lane), boots included.
    pub completed: Vec<i64>,
    /// The next most bought items (up to seven).
    pub situational: Vec<i64>,
    /// Boots in core builds and boots bought in more than 10% of games.
    pub boots: Vec<i64>,
    /// Most played summoner spell pair.
    pub spells: Option<(Vec<i64>, Record)>,
    /// Most played skill order.
    pub skills: Option<SkillOrder>,
}

impl Style {
    pub fn label(&self) -> String {
        self.title.clone()
    }

    /// Items worth comparing with what you hold: build order, completed and situational.
    fn items(&self) -> impl Iterator<Item = &i64> {
        self.build_order.iter().chain(&self.completed).chain(&self.situational)
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Rarity {
    Prismatic,
    Gold,
    Silver,
}

impl Rarity {
    pub const ORDER: [Rarity; 3] = [Rarity::Prismatic, Rarity::Gold, Rarity::Silver];

    pub fn label(self) -> &'static str {
        match self {
            Rarity::Prismatic => "Prismatic",
            Rarity::Gold => "Gold",
            Rarity::Silver => "Silver",
        }
    }

    fn from_client(s: &str) -> Option<Rarity> {
        Some(match s {
            "kPrismatic" => Rarity::Prismatic,
            "kGold" => Rarity::Gold,
            "kSilver" => Rarity::Silver,
            _ => return None,
        })
    }
}

/// An augment Blitz rates on the champion.
#[derive(Clone, PartialEq, Debug)]
pub struct Augment {
    pub id: i64,
    pub name: String,
    pub rarity: Rarity,
    pub tier: Tier,
    /// CommunityDragon URL of the client's icon.
    pub icon: Option<String>,
}

/// ARAM Mayhem extras for the champion.
#[derive(Clone, PartialEq, Debug)]
pub struct Mayhem {
    /// Best tier first, then by name; an error when the champion page or the client's names failed.
    pub augments: Result<Vec<Augment>, String>,
    /// Mayhem-only items Blitz rates on the champion (best tier first).
    pub items: Vec<(i64, Tier)>,
}

impl Mayhem {
    /// The best `AUGMENTS_PER_RARITY` augments of one rarity.
    pub fn best(&self, rarity: Rarity) -> Vec<&Augment> {
        let Ok(all) = &self.augments else { return Vec::new() };
        all.iter().filter(|a| a.rarity == rarity).take(AUGMENTS_PER_RARITY).collect()
    }
}

/// A champion's Blitz builds for one queue, one per playstyle, most played first.
#[derive(Clone, PartialEq, Debug)]
pub struct StyleBuilds {
    pub champion_id: i64,
    pub query: Query,
    /// Patch Blitz aggregated; empty when it did not say.
    pub patch: String,
    /// Empty when Blitz has no builds for the champion here.
    pub styles: Vec<Style>,
    pub mayhem: Option<Mayhem>,
}

impl StyleBuilds {
    /// The style to show: the one picked by `label`, else the one sharing the most items with
    /// `owned` (the most played on ties and before you buy anything). `None` when there is none.
    pub fn active(&self, label: Option<&str>, owned: &[i64]) -> Option<usize> {
        if let Some(i) = label.and_then(|l| self.styles.iter().position(|s| s.label() == l)) {
            return Some(i);
        }
        let overlap = |s: &Style| owned.iter().filter(|id| s.items().any(|x| x == *id)).count();
        let best = self.styles.iter().map(overlap).max()?;
        self.styles.iter().position(|s| overlap(s) == best)
    }
}

pub fn spawn(shared: Arc<Shared>, ctx: egui::Context) {
    std::thread::Builder::new()
        .name("riftlite-styles".into())
        .spawn(move || run(&shared, &ctx))
        .expect("spawn styles thread");
}

/// What the styles thread fetches for: your champion and Blitz table in a live game, else
/// in champ select once your champion is picked.
fn wanted(shared: &Shared) -> Option<((i64, Query), Arc<StaticData>, Option<Creds>)> {
    if !shared.settings.lock().style_builds {
        return None;
    }
    let s = shared.snap.lock();
    let key = if let Some(g) = &s.live {
        let me = g.me()?;
        (s.data.champion_by_key(&me.champion_key)?, Query::for_game(&g.mode, g.map, &me.position)?)
    } else {
        let cs = s.champ_select.as_ref()?;
        let map = s.session.as_ref().map_or(0, |g| g.map_id);
        (cs.my_champion()?, Query::for_queue(cs.queue_id, map, &cs.me()?.position)?)
    };
    Some((key, s.data.clone(), s.creds.clone()))
}

fn run(shared: &Shared, ctx: &egui::Context) {
    let public = net::public_agent();
    let local = net::riot_local_agent();
    // The last answer, kept across the gap between champ select and the game.
    let mut last: Option<((i64, Query), Instant, bool, Result<StyleBuilds, String>)> = None;
    loop {
        let Some((key, data, creds)) = wanted(shared) else {
            publish(shared, ctx, None);
            std::thread::sleep(POLL);
            continue;
        };
        let due = last.as_ref().is_none_or(|(k, at, ok, _)| *k != key || at.elapsed() > if *ok { TTL } else { RETRY });
        if due {
            let r = fetch(&public, &local, creds.as_ref(), key.0, key.1, &data);
            // A Mayhem extra that failed is fetched again sooner, like a failed build.
            let ok = r.as_ref().is_ok_and(|b| b.mayhem.as_ref().is_none_or(|m| m.augments.is_ok()));
            last = Some((key, Instant::now(), ok, r));
        }
        publish(shared, ctx, last.as_ref().map(|(.., r)| r.clone()));
        std::thread::sleep(POLL);
    }
}

fn publish(shared: &Shared, ctx: &egui::Context, v: Option<Result<StyleBuilds, String>>) {
    if set(&mut shared.snap.lock().styles, v) {
        ctx.request_repaint();
        ctx.request_repaint_of(crate::ui::overlay_id());
    }
}

fn fetch(
    public: &ureq::Agent,
    local: &ureq::Agent,
    creds: Option<&Creds>,
    champion_id: i64,
    query: Query,
    data: &StaticData,
) -> Result<StyleBuilds, String> {
    let v = augments::get_json(public, &query.url(champion_id)).map_err(|e| format!("Blitz builds: {e}"))?;
    let mut b = parse(champion_id, query, &v, data)?;
    if query.mayhem {
        b.mayhem = Some(fetch_mayhem(public, local, creds, champion_id, data));
    }
    Ok(b)
}

fn fetch_mayhem(public: &ureq::Agent, local: &ureq::Agent, creds: Option<&Creds>, champion_id: i64, data: &StaticData) -> Mayhem {
    let page = augments::get_json(public, &format!("{}?champion_id={champion_id}", augments::CHAMPION_TIERS_URL))
        .map_err(|e| format!("Blitz champion tiers: {e}"));
    let items = page.as_ref().map(|v| mayhem_items(v, data)).unwrap_or_default();
    let augments = page.and_then(|v| {
        let tiers = augments::parse_champion_tiers(&v)?;
        let creds = creds.ok_or("League client not connected: augment names unavailable")?;
        let assets = Lcu::new(local.clone(), creds)
            .get("/lol-game-data/assets/v1/cherry-augments.json")
            .map_err(|e| format!("Augment names: {e}"))?;
        Ok(champion_augments(&tiers, &assets))
    });
    Mayhem { augments, items }
}

/// The augments in `tiers` that the client names, best tier first, then by name.
fn champion_augments(tiers: &HashMap<i64, Tier>, assets: &Value) -> Vec<Augment> {
    let mut out: Vec<Augment> = assets
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            let id = a["id"].as_i64()?;
            Some(Augment {
                id,
                name: a["nameTRA"].as_str().filter(|n| !n.is_empty())?.to_string(),
                rarity: Rarity::from_client(a["rarity"].as_str()?)?,
                tier: *tiers.get(&id)?,
                icon: a["augmentSmallIconPath"].as_str().and_then(cdragon_url),
            })
        })
        .collect();
    out.sort_by(|a, b| (a.tier as u8).cmp(&(b.tier as u8)).then_with(|| a.name.cmp(&b.name)));
    out
}

/// `/lol-game-data/assets/ASSETS/UX/…/x_small.png` → its CommunityDragon URL (lowercased, as
/// CommunityDragon stores paths).
fn cdragon_url(path: &str) -> Option<String> {
    let rest = path.strip_prefix("/lol-game-data/assets/")?;
    Some(format!("{CDRAGON}{}", rest.to_lowercase()))
}

/// Mayhem-only item variants (ids from 100000) Blitz rates on the champion that can be bought
/// on Howling Abyss, best tier first.
fn mayhem_items(v: &Value, data: &StaticData) -> Vec<(i64, Tier)> {
    let mut out: Vec<(i64, Tier)> = v["data"][0]["data"]["items"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(id, x)| Some((id.parse::<i64>().ok()?, Tier::from_blitz(x["tier"].as_i64()?)?)))
        .filter(|&(id, _)| id >= 100_000 && data.shop.get(&id).is_none_or(|s| s.buyable_on(12)))
        .collect();
    out.sort_by_key(|&(id, t)| (t as u8, id));
    out
}

/// Item ids from `"3078,3111,6610"`, `["1052", "2003"]` or `[4645]`.
fn ids(v: &Value) -> Vec<i64> {
    match v {
        Value::String(s) => s.split(',').filter_map(|x| x.trim().parse().ok()).collect(),
        Value::Array(a) => a.iter().filter_map(|x| x.as_i64().or_else(|| x.as_str()?.parse().ok())).collect(),
        _ => Vec::new(),
    }
}

fn games(v: &Value) -> u64 {
    v["games"].as_u64().unwrap_or(0)
}

/// The most played entry of a list.
fn most_played(v: &Value) -> Option<&Value> {
    v.as_array()?.iter().max_by_key(|x| games(x))
}

fn parse(champion_id: i64, query: Query, v: &Value, data: &StaticData) -> Result<StyleBuilds, String> {
    let rows = v["data"].as_array().ok_or("Blitz builds: the data format changed")?;
    let mut styles: Vec<Style> = rows.iter().filter_map(|r| style(r, query, data)).collect();
    if styles.is_empty() && !rows.is_empty() {
        return Err("Blitz builds: the data format changed".into());
    }
    styles.sort_by(|a, b| b.record.games.cmp(&a.record.games));
    let patch = rows.first().and_then(|r| r["patch"].as_str()).unwrap_or("").to_string();
    Ok(StyleBuilds { champion_id, query, patch, styles, mayhem: None })
}

/// Blitz's playstyle title for a row (`name_tags`, runes).
fn title(r: &Value) -> String {
    let tags: Vec<&str> = r["name_tags"].as_array().into_iter().flatten().filter_map(Value::as_str).collect();
    if tags.is_empty() {
        return "Blitz Build".into();
    }
    if tags.contains(&"Uncategorized") {
        let jack = r["runes"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|x| x["pick_rate"].as_f64().is_some_and(|p| p >= 0.2) && x["runeId"].as_i64() == Some(JACK_OF_ALL_TRADES));
        return if jack { "Jack of All Trades" } else { "Well Rounded" }.into();
    }
    tags.into_iter().filter(|t| TAGS.contains(t)).collect::<Vec<_>>().join(" / ")
}

fn pick_rate(v: &Value) -> f64 {
    v["pick_rate"].as_f64().unwrap_or(0.0)
}

fn boots(id: i64) -> bool {
    bi::BOOTS.contains(&id)
}

/// Blitz's core: the most played core build picked in more than 5% of games, else the first listed.
fn core(rows: &Value) -> Vec<i64> {
    let list = rows.as_array().map(Vec::as_slice).unwrap_or_default();
    let mut best: Option<&Value> = None;
    for c in list.iter().filter(|c| pick_rate(c) > 0.05) {
        if best.is_none_or(|b| games(c) > games(b)) {
            best = Some(c);
        }
    }
    best.or(list.first()).map(|c| ids(&c["itemIds"])).unwrap_or_default()
}

/// Situational stats, most bought first: items in ≥ 2.5% of games (`primary`), in ≥ 0.2%
/// (`secondary`), boots in > 10%, the first support quest item and each item's average slot.
struct Situational {
    average_index: HashMap<i64, f64>,
    primary: Vec<i64>,
    secondary: Vec<i64>,
    boots: Vec<i64>,
    support_item: Option<i64>,
}

fn situational(rows: &Value) -> Situational {
    let mut list: Vec<&Value> = rows.as_array().into_iter().flatten().collect();
    list.sort_by_key(|x| std::cmp::Reverse(games(x)));
    let mut s = Situational { average_index: HashMap::new(), primary: Vec::new(), secondary: Vec::new(), boots: Vec::new(), support_item: None };
    for x in list {
        let Some(id) = x["itemId"].as_i64() else { continue };
        let avg = x["averageIndex"].as_f64().filter(|&a| a != 0.0).unwrap_or(99.0);
        s.average_index.insert(id, avg);
        let p = pick_rate(x);
        if boots(id) {
            if p > 0.1 {
                s.boots.push(id);
            }
            continue;
        }
        if bi::SUPPORT_QUEST.contains(&id) && s.support_item.is_none() {
            s.support_item = Some(id);
        }
        if !bi::NOT_SITUATIONAL.contains(&id) && p >= 0.002 {
            s.secondary.push(id);
            if p >= 0.025 {
                s.primary.push(id);
            }
        }
    }
    s
}

/// Blitz's items after the core: the after-core table keyed by exactly the core's items (the
/// most played one), else by the smallest superset; its single most played option, boots
/// dropped when the core has some, ordered by average slot.
fn after_core(table: &Value, core: &[i64], average_index: &HashMap<i64, f64>, core_has_boots: bool) -> Vec<i64> {
    use std::collections::BTreeSet;
    let want: BTreeSet<i64> = core.iter().copied().collect();
    if want.is_empty() {
        return Vec::new();
    }
    fn options(v: &Value) -> Vec<&Value> {
        v.as_object().into_iter().flatten().flat_map(|(_, o)| o.as_array().into_iter().flatten()).collect()
    }
    let max_games = |v: &Value| options(v).iter().map(|o| games(o)).max().unwrap_or(0);
    let mut exact: Vec<&Value> = Vec::new();
    let mut supersets: Vec<(usize, &Value)> = Vec::new();
    for (key, v) in table.as_object().into_iter().flatten() {
        let set: BTreeSet<i64> = key.split(',').filter_map(|x| x.parse().ok()).collect();
        if set == want {
            exact.push(v);
        } else if want.is_subset(&set) {
            supersets.push((set.len(), v));
        }
    }
    let chosen = if !exact.is_empty() {
        exact.into_iter().max_by_key(|v| max_games(v))
    } else {
        supersets.sort_by(|a, b| a.0.cmp(&b.0).then(max_games(b.1).cmp(&max_games(a.1))));
        supersets.first().map(|(_, v)| *v)
    };
    let Some(best) = chosen.and_then(|v| options(v).into_iter().reduce(|a, b| if games(b) > games(a) { b } else { a })) else { return Vec::new() };
    let mut items: Vec<i64> = ids(&best["itemIds"]).into_iter().filter(|&i| !(core_has_boots && boots(i))).collect();
    let slot = |i: &i64| average_index.get(i).copied().unwrap_or(99.0);
    items.sort_by(|a, b| slot(a).total_cmp(&slot(b)));
    items
}

/// Drops each item sharing a unique-passive group with an earlier item.
fn drop_group_conflicts(items: Vec<i64>) -> Vec<i64> {
    let mut seen: Vec<usize> = Vec::new();
    let mut dropped: Vec<i64> = Vec::new();
    for &id in &items {
        let mut conflict = false;
        for (g, members) in bi::GROUPS.iter().enumerate() {
            if members.contains(&id) {
                if seen.contains(&g) {
                    conflict = true;
                } else {
                    seen.push(g);
                }
            }
        }
        if conflict {
            dropped.push(id);
        }
    }
    items.into_iter().filter(|i| !dropped.contains(i)).collect()
}

/// Blitz's start: the most picked starting set made only of potions, starters and core
/// components (no Tear outside Tear builds), highest id first; junglers get a pet.
fn starting(rows: &Value, core: &[i64], tear_build: bool, support: bool, jungle: bool, data: &StaticData) -> Vec<i64> {
    let components: Vec<i64> = core.iter().filter_map(|c| data.shop.get(c)).flat_map(|s| s.from.iter().copied()).collect();
    let allowed = |i: &i64| bi::CONSUMABLES.contains(i) || bi::STARTERS.contains(i) || components.contains(i);
    let (mut best, mut best_pet, mut best_no_pet): (Option<&Value>, bool, Option<&Value>) = (None, false, None);
    for r in rows.as_array().into_iter().flatten() {
        let items = ids(&r["itemIds"]);
        if (!tear_build && items.contains(&bi::TEAR)) || !items.iter().all(allowed) {
            continue;
        }
        let pet = items.iter().any(|i| bi::JUNGLE_PETS.contains(i));
        if best.is_none_or(|b| pick_rate(r) > pick_rate(b)) {
            best = Some(r);
            best_pet = pet;
        }
        if !pet && best_no_pet.is_none_or(|b| pick_rate(r) > pick_rate(b)) {
            best_no_pet = Some(r);
        }
    }
    let sorted = |r: Option<&Value>| {
        let mut v = r.map(|r| ids(&r["itemIds"])).unwrap_or_default();
        v.sort_unstable_by(|a, b| b.cmp(a));
        v
    };
    if jungle && !best_pet {
        return vec![bi::HEALTH_POTION, bi::GUSTWALKER];
    }
    if !jungle && best_pet {
        let v = sorted(best_no_pet);
        return if !v.is_empty() {
            v
        } else if support {
            vec![bi::HEALTH_POTION, 3854]
        } else {
            vec![bi::HEALTH_POTION, 1054]
        };
    }
    sorted(best)
}

/// An item's components the way Blitz orders them without order stats: the dearer first.
fn components(id: i64, data: &StaticData) -> Vec<i64> {
    let from: Vec<i64> = data.shop.get(&id).map(|s| s.from.clone()).unwrap_or_default();
    let gold = |i: &i64| data.shop.get(i).map_or(0, |s| s.gold);
    if from.len() == 2 {
        let (a, b) = (from[0], from[1]);
        return match (data.shop.contains_key(&a), data.shop.contains_key(&b)) {
            (true, true) if gold(&a) < gold(&b) => vec![b, a],
            (ka, kb) => [(ka, a), (kb, b)].into_iter().filter(|(k, _)| *k).map(|(_, i)| i).collect(),
        };
    }
    let mut v: Vec<i64> = from.into_iter().filter(|i| data.shop.contains_key(i)).collect();
    v.sort_by_key(|i| std::cmp::Reverse(gold(i)));
    v
}

/// Blitz's build order: the first three items of the final build (Tear added in Tear builds),
/// each after its most picked component order (Data Dragon components without one), skipping
/// basic boots when the start has boots and starting items among the first two entries.
fn build_order(final_build: &[i64], orders: &Value, starting: &[i64], tear_build: bool, data: &StaticData) -> Vec<i64> {
    if data.shop.is_empty() {
        return Vec::new();
    }
    let mut first: Vec<i64> = final_build.iter().take(3).copied().filter(|i| data.shop.contains_key(i)).collect();
    let has_tear_item = first.iter().any(|i| bi::TEAR_FAMILY.contains(i));
    if tear_build && data.shop.contains_key(&bi::TEAR) {
        first.insert(if has_tear_item { 0 } else { 1.min(first.len()) }, bi::TEAR);
    }
    let mut steps: Vec<i64> = Vec::new();
    for &item in &first {
        let picked = orders[item.to_string().as_str()]
            .as_array()
            .and_then(|o| o.iter().min_by_key(|x| x["pick_rate_rank"].as_i64().unwrap_or(i64::MAX)))
            .map(|o| ids(&o["items_order"]));
        match picked {
            Some(order) => steps.extend(order),
            None => steps.extend(components(item, data)),
        }
        steps.push(item);
    }
    let start_boots = starting.iter().any(|&i| boots(i));
    let start_tear = starting.contains(&bi::TEAR);
    let mut tear_listed = false;
    let mut out = Vec::new();
    for (i, &id) in steps.iter().enumerate() {
        if start_boots && id == bi::BASIC_BOOTS {
            continue;
        }
        let tear = id == bi::TEAR;
        if tear && (tear_listed || start_tear) {
            continue;
        }
        if tear {
            tear_listed = true;
        } else if id == bi::NOT_IN_ORDER {
            continue;
        }
        if !(i < 2 && starting.contains(&id)) {
            out.push(id);
        }
    }
    out
}

/// One playstyle row, assembled like the Blitz app's item import (blitz-desktop `CxnY7q5R.js`
/// `Se`, "Common" sort: most played wins).
fn style(r: &Value, query: Query, data: &StaticData) -> Option<Style> {
    let core = core(&r["coreItems"]);
    if core.is_empty() {
        return None;
    }
    let (jungle, adc, support) = match query.position {
        Some("JUNGLE") => (true, false, false),
        Some("BOTTOM") => (false, true, false),
        Some("UTILITY") => (false, false, true),
        _ => (false, false, false),
    };
    let core_has_boots = core.iter().any(|&i| boots(i));
    let sit = situational(&r["situationalItems"]);
    let after = after_core(&r["itemsAfterCore"], &core, &sit.average_index, core_has_boots);
    let mut all: Vec<i64> = Vec::new();
    for id in core.iter().chain(&after).chain(&sit.primary).chain(&sit.secondary) {
        if !all.contains(id) {
            all.push(*id);
        }
    }
    let mut all: Vec<i64> = drop_group_conflicts(all).into_iter().filter(|i| !bi::SUPPORT_QUEST.contains(i)).collect();
    let tear_build = core.iter().filter_map(|c| data.shop.get(c)).any(|s| s.from.contains(&bi::TEAR));
    if !tear_build {
        all.retain(|i| !bi::TEAR_FAMILY.contains(i));
    }
    if !core.contains(&bi::HEARTSTEEL) {
        all.retain(|&i| i != bi::HEARTSTEEL);
    }
    let size = if adc { 7 } else { 6 };
    let mut final_build: Vec<i64> = all.iter().take(size).copied().collect();
    let mut rest: Vec<i64> = all.iter().skip(size).copied().collect();
    let start = starting(&r["startingItems"], &core, tear_build, support, jungle, data);
    if final_build.len() < size {
        let fill: Vec<i64> = rest.iter().copied().filter(|&i| !boots(i)).take(size - final_build.len()).collect();
        final_build.extend(&fill);
        rest.retain(|i| !fill.contains(i));
    }
    let mut boot_options: Vec<i64> = Vec::new();
    for c in r["coreItems"].as_array().into_iter().flatten() {
        if let Some(b) = ids(&c["itemIds"]).into_iter().find(|&i| boots(i)) {
            if !boot_options.contains(&b) {
                boot_options.push(b);
            }
        }
    }
    for &b in &sit.boots {
        if !boot_options.contains(&b) {
            boot_options.push(b);
        }
    }
    // Blitz splices boots in second and the support quest item third, each pushing the last
    // item out to the situational list.
    if !final_build.iter().any(|&i| boots(i)) {
        if let Some(&b) = boot_options.first() {
            final_build.insert(1.min(final_build.len()), b);
            let last = final_build.pop().expect("just inserted");
            rest.insert(0, last);
        }
    }
    if let Some(quest) = sit.support_item.filter(|_| support) {
        final_build.insert(2.min(final_build.len()), quest);
        let last = final_build.pop().expect("just inserted");
        rest.retain(|&i| i != quest);
        rest.insert(0, last);
    }
    let order = build_order(&final_build, &r["itemOrders"], &start, tear_build, data);
    let mut situational: Vec<i64> = rest.into_iter().take(SITUATIONAL_LIMIT).collect();
    situational.extend(ids(&r["additionalSituationalItems"]));

    let spells = most_played(&r["summonerSpells"]).map(|s| (ids(&s["summonerSpellIds"]), Record::of(s))).filter(|(ids, _)| !ids.is_empty());
    let skills = most_played(&r["skillOrders"]).and_then(|s| {
        let order: Vec<usize> = s["skillOrder"].as_array()?.iter().filter_map(|k| Some(k.as_u64()?.checked_sub(1)? as usize)).filter(|&k| k < 4).collect();
        (!order.is_empty()).then(|| SkillOrder { order, record: Record::of(s) })
    });
    Some(Style {
        title: title(r),
        record: Record::of(r),
        starting: start,
        build_order: order,
        completed: final_build,
        situational,
        boots: boot_options,
        spells,
        skills,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn games_map_to_blitz_tables() {
        let q = |mode, map, pos| Query::for_game(mode, map, pos);
        assert_eq!(q("CLASSIC", 11, "TOP"), Some(Query { queue: "RANKED_SOLO_5X5", position: Some("TOP"), mayhem: false }));
        assert_eq!(q("CLASSIC", 11, "NONE"), Some(Query { queue: "RANKED_SOLO_5X5", position: None, mayhem: false }));
        assert_eq!(q("KIWI", 12, ""), Some(Query { queue: "HOWLING_ABYSS_ARAM", position: None, mayhem: true }));
        assert_eq!(q("URF", 11, "MIDDLE"), Some(Query { queue: "SUMMONERS_RIFT_URF", position: None, mayhem: false }));
        assert_eq!(q("CHERRY", 30, ""), None);
        assert_eq!(q("NEXUSBLITZ", 21, ""), None);
        // ARAM rejects the build tier filter; ranked uses it like Blitz's pages.
        assert!(!q("ARAM", 12, "").unwrap().url(24).contains("build_tier"));
        assert!(q("CLASSIC", 11, "TOP").unwrap().url(24).ends_with("&build_tier=Standard&individual_position=TOP"));
    }

    fn kennen() -> Value {
        // Shaped like Blitz's Kennen ARAM answer (patch 16.21), trimmed.
        serde_json::json!({"data": [
            {"name_tags": ["Bruiser"], "games": 56, "win_rate": 0.49, "patch": "16.21",
             "coreItems": [{"games": 9, "itemIds": "6653,4633,8010"}], "startingItems": [], "situationalItems": [], "itemsAfterCore": {}},
            {"name_tags": ["AP"], "games": 1268, "win_rate": 0.4826, "patch": "16.21",
             "coreItems": [{"games": 109, "itemIds": "3152,3020,4646"}, {"games": 166, "itemIds": "3152,3020,3157"}],
             "startingItems": [{"games": 113, "itemIds": ["2031", "3802"]}, {"games": 486, "itemIds": ["1001", "3145"]}],
             "itemOrders": {
                 "3152": [{"games": 184, "items_order": [3145, 1028, 3067]}, {"games": 439, "items_order": [3145, 3067, 1028]}],
                 "3020": [{"games": 1085, "items_order": [1001]}],
                 "3157": [{"games": 645, "items_order": [2420, 1058]}]},
             "itemsAfterCore": {},
             "situationalItems": [
                 {"itemId": 3020, "games": 1034, "pick_rate": 0.9},
                 {"itemId": 4645, "games": 779, "pick_rate": 0.6},
                 {"itemId": 4646, "games": 769, "pick_rate": 0.6},
                 {"itemId": 3089, "games": 400, "pick_rate": 0.3},
                 {"itemId": 3118, "games": 392, "pick_rate": 0.3},
                 {"itemId": 3158, "games": 102, "pick_rate": 0.08},
                 {"itemId": 1058, "games": 90, "pick_rate": 0.07},
                 {"itemId": 3135, "games": 205, "pick_rate": 0.16},
                 {"itemId": 3075, "games": 2, "pick_rate": 0.001}],
             "summonerSpells": [{"games": 32, "summonerSpellIds": [4, 6], "win_rate": 0.4}, {"games": 1121, "summonerSpellIds": [4, 32], "win_rate": 0.47}],
             "skillOrders": [{"games": 316, "win_rate": 0.525, "skillOrder": [1, 2, 3, 1, 1, 4, 1, 2, 1, 2, 4, 2, 2, 3, 3, 4, 3, 3]}]}
        ]})
    }

    fn data() -> StaticData {
        let mut d = StaticData::default();
        // Gold and components (Data Dragon 16.21) of the items in the crit fixtures.
        for (id, gold, from) in [
            (6672, 3100, &[1043, 1042][..]),
            (3031, 3450, &[1038, 1037]),
            (3006, 1100, &[1001, 1042]),
            (3036, 3000, &[1037, 3035]),
            (1038, 1300, &[]),
            (1037, 875, &[]),
            (1043, 700, &[]),
            (1042, 300, &[]),
            (1001, 300, &[]),
            (3035, 1450, &[]),
            (3072, 3400, &[]),
            (3139, 3000, &[]),
            (3046, 2600, &[]),
            (3094, 2700, &[]),
            (3047, 1100, &[]),
            (1055, 450, &[]),
            (2003, 50, &[]),
            (1036, 350, &[]),
        ] {
            d.shop.insert(id, crate::ddragon::ShopItem { gold, from: from.to_vec(), ..Default::default() });
        }
        d
    }

    /// A crit row exercising Blitz's rules. Expected values are what the Blitz app's own
    /// build function (`Se`, 3.0.0-beta.1787885272) returns for the same row and items.
    fn crit() -> Value {
        serde_json::json!({"name_tags": ["Crit", "Uncategorized?"], "games": 1000, "win_rate": 0.51,
            "coreItems": [{"games": 900, "pick_rate": 0.04, "itemIds": "3031,3087,3094"}, {"games": 400, "pick_rate": 0.3, "itemIds": "6672,3006,3031"}],
            "startingItems": [{"itemIds": ["1055", "2003"], "pick_rate": 0.5, "games": 500}, {"itemIds": ["1036", "2003"], "pick_rate": 0.7, "games": 700}],
            "itemOrders": {
                "6672": [{"items_order": [1042, 1043], "pick_rate_rank": 2}, {"items_order": [1043, 1042], "pick_rate_rank": 1}],
                "3006": [{"items_order": [1001, 1042], "pick_rate_rank": 1}]},
            "itemsAfterCore": {"3006,3031,6672": {"1": [{"itemIds": "3036", "games": 50}, {"itemIds": "3072", "games": 80}], "2": [{"itemIds": "3139", "games": 120}]}},
            "situationalItems": [
                {"itemId": 3036, "games": 800, "pick_rate": 0.5, "averageIndex": 0.4},
                {"itemId": 3033, "games": 700, "pick_rate": 0.4, "averageIndex": 0.5},
                {"itemId": 3072, "games": 600, "pick_rate": 0.3, "averageIndex": 0.6},
                {"itemId": 3047, "games": 500, "pick_rate": 0.3, "averageIndex": 0.2},
                {"itemId": 3139, "games": 400, "pick_rate": 0.2, "averageIndex": 0.7},
                {"itemId": 6035, "games": 300, "pick_rate": 0.1, "averageIndex": 0.7},
                {"itemId": 3046, "games": 200, "pick_rate": 0.03, "averageIndex": 0.8},
                {"itemId": 3094, "games": 100, "pick_rate": 0.01, "averageIndex": 0.8},
                {"itemId": 3004, "games": 90, "pick_rate": 0.05, "averageIndex": 0.5}]})
    }

    #[test]
    fn a_style_is_assembled_like_the_blitz_app() {
        let aram = Query::for_game("ARAM", 12, "").unwrap();
        let s = style(&crit(), aram, &data()).unwrap();
        // Unknown tags are not shown.
        assert_eq!(s.label(), "Crit");
        // The 900-game core is picked by too few (4%); the start holding a non-component is skipped.
        assert_eq!(s.starting, [2003, 1055]);
        // Lowest pick-rate rank order first; Data Dragon components (dearer first) without one.
        assert_eq!(s.build_order, [1043, 1042, 6672, 1001, 1042, 3006, 1038, 1037, 3031]);
        // Core, the most played after-core option, then the most bought: Lord Dominik's (3033)
        // and Maw (6035) share a passive with earlier items; Manamune (3004) needs a Tear build.
        assert_eq!(s.completed, [6672, 3006, 3031, 3139, 3036, 3072]);
        assert_eq!(s.situational, [3046, 3094]);
        assert_eq!(s.boots, [3006, 3047]);

        // Bot lane gets seven; boots missing from the core go in second.
        let mut row = crit();
        row["coreItems"] = serde_json::json!([{"games": 400, "pick_rate": 0.3, "itemIds": "6672,3031,3036"}]);
        row["itemOrders"] = serde_json::json!({});
        row["itemsAfterCore"] = serde_json::json!({});
        row["startingItems"] = serde_json::json!([{"itemIds": ["1055", "2003"], "pick_rate": 0.5}]);
        row["situationalItems"] = serde_json::json!([
            {"itemId": 3072, "games": 600, "pick_rate": 0.3}, {"itemId": 3006, "games": 550, "pick_rate": 0.6},
            {"itemId": 3139, "games": 400, "pick_rate": 0.2}, {"itemId": 3046, "games": 200, "pick_rate": 0.03},
            {"itemId": 3094, "games": 100, "pick_rate": 0.01}]);
        let s = style(&row, Query::for_game("CLASSIC", 11, "BOTTOM").unwrap(), &data()).unwrap();
        assert_eq!(s.completed, [6672, 3006, 3031, 3036, 3072, 3139, 3046]);
        assert_eq!(s.situational, [3094]);
        assert_eq!(s.build_order, [1043, 1042, 6672, 1001, 1042, 3006, 1038, 1037, 3031]);
    }

    #[test]
    fn titles_follow_blitz() {
        let t = |v: Value| title(&v);
        assert_eq!(t(serde_json::json!({"name_tags": ["Crit", "On-Hit"]})), "Crit / On-Hit");
        assert_eq!(t(serde_json::json!({"name_tags": []})), "Blitz Build");
        assert_eq!(t(serde_json::json!({"name_tags": ["Uncategorized"], "runes": [{"runeId": 8316, "pick_rate": 0.3}]})), "Jack of All Trades");
        assert_eq!(t(serde_json::json!({"name_tags": ["Uncategorized"], "runes": [{"runeId": 8316, "pick_rate": 0.1}]})), "Well Rounded");
    }

    #[test]
    fn champ_select_queues_match_the_game_tables() {
        assert_eq!(Query::for_queue(2400, 12, ""), Query::for_game("KIWI", 12, ""));
        assert_eq!(Query::for_queue(450, 12, ""), Query::for_game("ARAM", 12, ""));
        assert_eq!(Query::for_queue(900, 11, ""), Query::for_game("URF", 11, ""));
        assert_eq!(Query::for_queue(420, 11, "middle"), Query::for_game("CLASSIC", 11, "MIDDLE"));
        assert_eq!(Query::for_queue(1700, 30, ""), None);
        assert_eq!(Query::for_queue(1300, 11, ""), None);
    }

    #[test]
    fn next_skill_follows_the_order_and_the_levelling_rules() {
        let s = SkillOrder { order: vec![0, 1, 2, 0, 0, 3, 0, 1, 0, 1, 3, 1, 1, 2, 2, 3, 2, 2], record: Record::default() };
        // Level 1, nothing taken: Q.
        assert_eq!(s.next([0, 0, 0, 0], 1), Some(0));
        // No unspent point.
        assert_eq!(s.next([1, 0, 0, 0], 1), None);
        // Level 6 on the order: R.
        assert_eq!(s.next([3, 1, 1, 0], 6), Some(3));
        // Took E instead of W at 2: the order's missing W comes next.
        assert_eq!(s.next([1, 0, 1, 0], 3), Some(1));
        // Q is capped at level 3 (two points): the order's next legal point instead.
        assert_eq!(s.next([2, 0, 0, 0], 3), Some(1));
    }

    #[test]
    fn the_active_style_is_the_picked_one_else_the_one_you_are_building() {
        let b = parse(85, Query::for_game("ARAM", 12, "").unwrap(), &kennen(), &data()).unwrap();
        assert_eq!(b.active(None, &[]), Some(0));
        assert_eq!(b.active(None, &[6653, 4633]), Some(1));
        assert_eq!(b.active(Some("Bruiser"), &[3152]), Some(1));
        assert_eq!(b.active(Some("Gone"), &[3152]), Some(0));
        let ap = &b.styles[0];
        assert_eq!(ap.spells.as_ref().map(|(ids, r)| (ids.clone(), r.games)), Some((vec![4, 32], 1121)));
        assert_eq!(ap.record.text(), "48% · 1,268 games");
        assert_eq!(ap.skills.as_ref().unwrap().max_order(), ['Q', 'W', 'E']);
    }

    #[test]
    fn mayhem_augments_and_items_come_from_the_champion_page_and_client_names() {
        let tiers = HashMap::from([(1004, Tier::A), (1030, Tier::S), (2032, Tier::S), (9999, Tier::S)]);
        let assets = serde_json::json!([
            {"id": 1004, "nameTRA": "Back To Basics", "rarity": "kPrismatic", "augmentSmallIconPath": "/lol-game-data/assets/ASSETS/UX/Cherry/Augments/Icons/BackToBasics_small.png"},
            {"id": 1030, "nameTRA": "Eureka", "rarity": "kPrismatic", "augmentSmallIconPath": ""},
            {"id": 2032, "nameTRA": "Shark Bait", "rarity": "kGold"},
            {"id": 1205, "nameTRA": "ADAPt", "rarity": "kSilver"}
        ]);
        let a = champion_augments(&tiers, &assets);
        assert_eq!(a.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(), ["Eureka", "Shark Bait", "Back To Basics"]);
        assert_eq!(
            a[2].icon.as_deref(),
            Some("https://raw.communitydragon.org/latest/plugins/rcp-be-lol-game-data/global/default/assets/ux/cherry/augments/icons/backtobasics_small.png")
        );
        let page = serde_json::json!({"data": [{"data": {"items": {"3152": {"tier": 1}, "228002": {"tier": 2}, "123430": {"tier": 2}, "126697": {"tier": 5}}}}]});
        assert_eq!(mayhem_items(&page, &StaticData::default()), [(123430, Tier::A), (228002, Tier::A), (126697, Tier::D)]);
    }

    #[test]
    fn no_rows_is_an_empty_answer_and_unreadable_rows_a_format_change() {
        let q = Query::for_game("URF", 11, "").unwrap();
        let d = StaticData::default();
        assert!(parse(1, q, &serde_json::json!({"data": []}), &d).unwrap().styles.is_empty());
        assert!(parse(1, q, &serde_json::json!({"data": [{"games": 5}]}), &d).is_err());
        assert!(parse(1, q, &serde_json::json!({"error": "x"}), &d).is_err());
    }
}
