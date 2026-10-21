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
/// Completed items listed (blitz.gg shows six).
const COMPLETED: usize = 6;
/// Situational items: bought in at least this share of the style's games.
const SITUATIONAL_MIN_PICK_RATE: f64 = 0.03;
const SITUATIONAL_LIMIT: usize = 7;
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

/// One playstyle's build.
#[derive(Clone, PartialEq, Debug)]
pub struct Style {
    /// Blitz's tags, e.g. `["Crit", "On-Hit"]`.
    pub tags: Vec<String>,
    pub record: Record,
    /// Most played starting items.
    pub starting: Vec<i64>,
    /// The most played core with each item's most played components before it, in buy order.
    pub build_order: Vec<i64>,
    /// The most played core, the items bought after it, then the most bought other finished items.
    pub completed: Vec<i64>,
    /// Other finished items bought in this style, most bought first.
    pub situational: Vec<i64>,
    /// Most played summoner spell pair.
    pub spells: Option<(Vec<i64>, Record)>,
    /// Most played skill order.
    pub skills: Option<SkillOrder>,
}

impl Style {
    pub fn label(&self) -> String {
        self.tags.join(" / ")
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

fn run(shared: &Shared, ctx: &egui::Context) {
    let public = net::public_agent();
    let local = net::riot_local_agent();
    let mut last: Option<((i64, Query), Instant, bool)> = None;
    loop {
        let want = {
            let s = shared.snap.lock();
            let on = shared.settings.lock().style_builds;
            s.live.as_ref().filter(|_| on).and_then(|g| {
                let me = g.me()?;
                let key = (s.data.champion_by_key(&me.champion_key)?, Query::for_game(&g.mode, g.map, &me.position)?);
                Some((key, s.data.clone(), s.creds.clone()))
            })
        };
        let Some((key, data, creds)) = want else {
            last = None;
            publish(shared, ctx, None);
            std::thread::sleep(POLL);
            continue;
        };
        let due = last.is_none_or(|(k, at, ok)| k != key || at.elapsed() > if ok { TTL } else { RETRY });
        if due {
            let r = fetch(&public, &local, creds.as_ref(), key.0, key.1, &data);
            // A Mayhem extra that failed is fetched again sooner, like a failed build.
            let ok = r.as_ref().is_ok_and(|b| b.mayhem.as_ref().is_none_or(|m| m.augments.is_ok()));
            last = Some((key, Instant::now(), ok));
            publish(shared, ctx, Some(r));
        }
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
    let mut styles: Vec<Style> = rows.iter().filter_map(|r| style(r, data)).collect();
    if styles.is_empty() && !rows.is_empty() {
        return Err("Blitz builds: the data format changed".into());
    }
    styles.sort_by(|a, b| b.record.games.cmp(&a.record.games));
    let patch = rows.first().and_then(|r| r["patch"].as_str()).unwrap_or("").to_string();
    Ok(StyleBuilds { champion_id, query, patch, styles, mayhem: None })
}

fn style(r: &Value, data: &StaticData) -> Option<Style> {
    let tags: Vec<String> = r["name_tags"].as_array()?.iter().filter_map(|t| Some(t.as_str()?.to_string())).collect();
    let core = ids(&most_played(&r["coreItems"])?["itemIds"]);
    if tags.is_empty() || core.is_empty() {
        return None;
    }
    let starting = most_played(&r["startingItems"]).map(|s| ids(&s["itemIds"])).unwrap_or_default();

    // Each core item after its most played components (those not already bought).
    let mut build_order: Vec<i64> = Vec::new();
    for &item in &core {
        let components = most_played(&r["itemOrders"][item.to_string().as_str()]).map(|o| ids(&o["items_order"])).unwrap_or_default();
        for c in components {
            if !starting.contains(&c) && !build_order.contains(&c) {
                build_order.push(c);
            }
        }
        build_order.push(item);
    }

    // After the core: Blitz's items-after-core (keyed by the core's ids in ascending order,
    // then by slot "1", "2", …), then the most bought other finished items.
    let mut key_ids = core.clone();
    key_ids.sort_unstable();
    let key = key_ids.iter().map(i64::to_string).collect::<Vec<_>>().join(",");
    let mut slots: Vec<(u32, i64)> = r["itemsAfterCore"][key.as_str()]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(slot, options)| Some((slot.parse().ok()?, *ids(&most_played(options)?["itemIds"]).first()?)))
        .collect();
    slots.sort_unstable();
    let mut completed = core.clone();
    for (_, id) in slots {
        if !completed.contains(&id) {
            completed.push(id);
        }
    }
    let mut others: Vec<&Value> = r["situationalItems"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|s| s["pick_rate"].as_f64().is_some_and(|p| p >= SITUATIONAL_MIN_PICK_RATE))
        .collect();
    others.sort_by_key(|s| std::cmp::Reverse(games(s)));
    let boots = |id: i64| data.item_stats.get(&id).is_some_and(|s| s.boots);
    let finished = |id: i64| data.completed_items.is_empty() || data.completed_items.contains(&id) || boots(id);
    let mut situational = Vec::new();
    for id in others.into_iter().filter_map(|s| s["itemId"].as_i64()) {
        let has_boots = completed.iter().any(|&c| boots(c));
        if completed.contains(&id) || situational.contains(&id) || !finished(id) || (boots(id) && has_boots) {
            continue;
        }
        if completed.len() < COMPLETED {
            completed.push(id);
        } else if situational.len() < SITUATIONAL_LIMIT {
            situational.push(id);
        }
    }

    let spells = most_played(&r["summonerSpells"]).map(|s| (ids(&s["summonerSpellIds"]), Record::of(s))).filter(|(ids, _)| !ids.is_empty());
    let skills = most_played(&r["skillOrders"]).and_then(|s| {
        let order: Vec<usize> = s["skillOrder"].as_array()?.iter().filter_map(|k| Some(k.as_u64()?.checked_sub(1)? as usize)).filter(|&k| k < 4).collect();
        (!order.is_empty()).then(|| SkillOrder { order, record: Record::of(s) })
    });
    Some(Style { tags, record: Record::of(r), starting, build_order, completed, situational, spells, skills })
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
        d.completed_items = [3152, 3020, 3157, 4645, 4646, 3089, 3118, 3135, 3158, 3075].into_iter().collect();
        for boots in [1001, 3020, 3158] {
            d.item_stats.insert(boots, crate::ddragon::ItemStats { boots: true, ..Default::default() });
        }
        d
    }

    #[test]
    fn a_style_reads_like_blitzs_page() {
        let q = Query::for_game("KIWI", 12, "").unwrap();
        let b = parse(85, q, &kennen(), &data()).unwrap();
        assert_eq!(b.styles.iter().map(Style::label).collect::<Vec<_>>(), ["AP", "Bruiser"]);
        let s = &b.styles[0];
        assert_eq!(s.starting, [1001, 3145]);
        // Components before each core item; the starting items are not bought again.
        assert_eq!(s.build_order, [3067, 1028, 3152, 3020, 2420, 1058, 3157]);
        // The core, then the most bought finished items up to six.
        assert_eq!(s.completed, [3152, 3020, 3157, 4645, 4646, 3089]);
        // Then the rest, most bought first: no second pair of boots, no components, nothing rare.
        assert_eq!(s.situational, [3118, 3135]);
        assert_eq!(s.spells.as_ref().map(|(ids, r)| (ids.clone(), r.games)), Some((vec![4, 32], 1121)));
        assert_eq!(s.record.text(), "48% · 1,268 games");
        let skills = s.skills.as_ref().unwrap();
        assert_eq!(skills.max_order(), ['Q', 'W', 'E']);
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
