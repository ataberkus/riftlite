//! Champion and item names from Riot's official Data Dragon CDN, cached per
//! patch under `%LOCALAPPDATA%\riftlite\ddragon`. Only the name maps, the few
//! item stats used to classify builds and the image paths the UI shows are kept
//! in memory.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use serde_json::Value;

const CDN: &str = "https://ddragon.leagueoflegends.com";
/// Cheapest legendary item price; tier-2/3 boots (≤ 1250) and support items (400) are below it.
const COMPLETED_GOLD: i64 = 2000;

#[derive(Clone, Default)]
pub struct Champion {
    /// Data Dragon id, also the folder name under `Config/Champions` (e.g. `MonkeyKing`).
    pub id: String,
    pub name: String,
    /// Riot's class tags (`Mage`, `Tank`, …).
    pub tags: Vec<String>,
    /// Riot's 0–10 ratings `info.attack` and `info.magic`: how physical or magic its damage is.
    pub attack: i64,
    pub magic: i64,
}

/// What an item gives, from Data Dragon's `stats` and `tags`; used to tell an AP
/// build from an AD, crit, on-hit or tank one.
#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct ItemStats {
    /// `FlatMagicDamageMod`
    pub ap: f64,
    /// `FlatPhysicalDamageMod`
    pub ad: f64,
    /// `CriticalStrike` tag
    pub crit: bool,
    /// `OnHit` tag
    pub on_hit: bool,
    /// `Health`, `Armor` or `SpellBlock` tag
    pub defensive: bool,
    /// `Boots` tag
    pub boots: bool,
}

#[derive(Default, Clone)]
pub struct StaticData {
    pub version: String,
    pub champions: HashMap<i64, Champion>,
    pub items: HashMap<i64, String>,
    pub item_stats: HashMap<i64, ItemStats>,
    /// Finished (legendary) items, for enemy item callouts.
    pub completed_items: HashSet<i64>,
    /// Summoner spells by numeric key.
    pub spells: HashMap<i64, String>,
    /// Summoner spell Data Dragon ids (`SummonerFlash`) by numeric key, for spell images.
    pub spell_keys: HashMap<i64, String>,
    /// Runes and rune paths (styles) by id.
    pub perks: HashMap<i64, String>,
    /// Rune and rune path image paths under `cdn/img/` by id.
    pub perk_icons: HashMap<i64, String>,
    /// The League client's local asset names have been merged in.
    pub has_client_assets: bool,
}

impl StaticData {
    pub fn champion_name(&self, id: i64) -> String {
        match self.champions.get(&id) {
            Some(c) => c.name.clone(),
            None if id <= 0 => "—".into(),
            None => format!("#{id}"),
        }
    }

    /// Numeric id for a Data Dragon champion id (`MissFortune`).
    pub fn champion_by_key(&self, key: &str) -> Option<i64> {
        self.champions.iter().find(|(_, c)| c.id.eq_ignore_ascii_case(key)).map(|(&id, _)| id)
    }

    pub fn item_name(&self, id: i64) -> String {
        self.items.get(&id).cloned().unwrap_or_else(|| format!("#{id}"))
    }

    pub fn spell_name(&self, id: i64) -> String {
        self.spells.get(&id).cloned().unwrap_or_else(|| format!("#{id}"))
    }

    /// Numeric id of a summoner spell's Data Dragon id (`SummonerFlash`).
    pub fn spell_by_key(&self, key: &str) -> Option<i64> {
        self.spell_keys.iter().find(|(_, k)| k.as_str() == key).map(|(&id, _)| id)
    }

    pub fn perk_name(&self, id: i64) -> String {
        self.perks.get(&id).cloned().unwrap_or_else(|| format!("#{id}"))
    }

    /// Image paths relative to the CDN root, for [`fetch_image`].
    pub fn champion_icon(&self, id: i64) -> Option<String> {
        self.champion_icon_by_key(&self.champions.get(&id)?.id)
    }

    pub fn champion_icon_by_key(&self, key: &str) -> Option<String> {
        (!self.version.is_empty() && !key.is_empty()).then(|| format!("cdn/{}/img/champion/{key}.png", self.version))
    }

    pub fn champion_splash(&self, id: i64) -> Option<String> {
        Some(format!("cdn/img/champion/splash/{}_0.jpg", self.champions.get(&id)?.id))
    }

    pub fn item_icon(&self, id: i64) -> Option<String> {
        (id > 0 && !self.version.is_empty()).then(|| format!("cdn/{}/img/item/{id}.png", self.version))
    }

    pub fn spell_icon(&self, id: i64) -> Option<String> {
        Some(format!("cdn/{}/img/spell/{}.png", self.version, self.spell_keys.get(&id)?))
    }

    pub fn perk_icon(&self, id: i64) -> Option<String> {
        Some(format!("cdn/img/{}", self.perk_icons.get(&id)?))
    }

    pub fn profile_icon(&self, id: i64) -> Option<String> {
        (id > 0 && !self.version.is_empty()).then(|| format!("cdn/{}/img/profileicon/{id}.png", self.version))
    }

    /// Fills ids Data Dragon lacks (mode-only items such as ARAM Mayhem's, stat
    /// shards) from the client's own `/lol-game-data` assets. Data Dragon names win.
    pub fn with_client_assets(&self, items: &Value, perks: &Value, styles: &Value, spells: &Value) -> StaticData {
        let mut d = self.clone();
        let pairs = |v: &Value| -> Vec<(i64, String)> {
            v.as_array()
                .into_iter()
                .flatten()
                .filter_map(|x| Some((x["id"].as_i64()?, x["name"].as_str().filter(|n| !n.is_empty())?.to_string())))
                .collect()
        };
        for (id, n) in pairs(items) {
            d.items.entry(id).or_insert(n);
        }
        for (id, n) in pairs(perks).into_iter().chain(pairs(&styles["styles"])) {
            d.perks.entry(id).or_insert(n);
        }
        for (id, n) in pairs(spells) {
            d.spells.entry(id).or_insert(n);
        }
        d.has_client_assets = true;
        d
    }
}

fn cache_dir() -> PathBuf {
    crate::settings::local_dir().join("ddragon")
}

fn fetch_json(agent: &ureq::Agent, url: &str) -> Result<Value, String> {
    agent.get(url).call().map_err(|e| e.to_string())?.into_json().map_err(|e| e.to_string())
}

fn cached_or_fetch(agent: &ureq::Agent, version: &str, file: &str) -> Result<Value, String> {
    let path = cache_dir().join(version).join(file);
    if let Some(v) = std::fs::read(&path).ok().and_then(|b| serde_json::from_slice(&b).ok()) {
        return Ok(v);
    }
    let v = fetch_json(agent, &format!("{CDN}/cdn/{version}/data/en_US/{file}"))?;
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(&path, v.to_string());
    }
    Ok(v)
}

fn newest_cached_version() -> Option<String> {
    std::fs::read_dir(cache_dir())
        .ok()?
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .max_by(|a, b| version_key(a).cmp(&version_key(b)))
}

fn version_key(v: &str) -> Vec<u32> {
    v.split('.').filter_map(|p| p.parse().ok()).collect()
}

/// Removes caches of older patches so the cache never grows past one patch.
fn prune(keep: &str) {
    if let Ok(rd) = std::fs::read_dir(cache_dir()) {
        for e in rd.flatten() {
            if e.file_name() != keep {
                let _ = std::fs::remove_dir_all(e.path());
            }
        }
    }
}

pub fn load(agent: &ureq::Agent) -> Result<StaticData, String> {
    let version = fetch_json(agent, &format!("{CDN}/api/versions.json"))
        .ok()
        .and_then(|v| v.get(0)?.as_str().map(str::to_string))
        .or_else(newest_cached_version)
        .ok_or("Data Dragon unreachable and no cache")?;
    let champs = cached_or_fetch(agent, &version, "champion.json")?;
    let items = cached_or_fetch(agent, &version, "item.json")?;
    let spells = cached_or_fetch(agent, &version, "summoner.json")?;
    let runes = cached_or_fetch(agent, &version, "runesReforged.json")?;
    prune(&version);
    let champions = champs["data"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(_, c)| {
            let key = c["key"].as_str()?.parse().ok()?;
            let champion = Champion {
                id: c["id"].as_str()?.to_string(),
                name: c["name"].as_str()?.to_string(),
                tags: c["tags"].as_array().into_iter().flatten().filter_map(|t| Some(t.as_str()?.to_string())).collect(),
                attack: c["info"]["attack"].as_i64().unwrap_or(0),
                magic: c["info"]["magic"].as_i64().unwrap_or(0),
            };
            Some((key, champion))
        })
        .collect();
    let item_data: Vec<(i64, &Value)> =
        items["data"].as_object().into_iter().flatten().filter_map(|(k, i)| Some((k.parse().ok()?, i))).collect();
    let item_stats = item_data
        .iter()
        .map(|&(id, i)| {
            let tag = |t: &str| i["tags"].as_array().is_some_and(|a| a.iter().any(|x| x == t));
            let stats = ItemStats {
                ap: i["stats"]["FlatMagicDamageMod"].as_f64().unwrap_or(0.0),
                ad: i["stats"]["FlatPhysicalDamageMod"].as_f64().unwrap_or(0.0),
                crit: tag("CriticalStrike"),
                on_hit: tag("OnHit"),
                defensive: tag("Health") || tag("Armor") || tag("SpellBlock"),
                boots: tag("Boots"),
            };
            (id, stats)
        })
        .collect();
    let items = item_data.iter().filter_map(|&(id, i)| Some((id, i["name"].as_str()?.to_string()))).collect();
    // Built from components, builds into nothing and costs a legendary's price: boots and
    // support quest items stay out.
    let completed_items = item_data
        .iter()
        .filter(|(_, i)| {
            i["from"].as_array().is_some_and(|a| !a.is_empty())
                && i["into"].as_array().is_none_or(|a| a.is_empty())
                && i["gold"]["total"].as_i64().unwrap_or(0) >= COMPLETED_GOLD
        })
        .map(|&(id, _)| id)
        .collect();
    let spell_data: Vec<(i64, &Value)> = spells["data"]
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(_, s)| Some((s["key"].as_str()?.parse().ok()?, s)))
        .collect();
    let spells = spell_data.iter().filter_map(|&(k, s)| Some((k, s["name"].as_str()?.to_string()))).collect();
    let spell_keys = spell_data.iter().filter_map(|&(k, s)| Some((k, s["id"].as_str()?.to_string()))).collect();
    let mut perks = HashMap::new();
    let mut perk_icons = HashMap::new();
    let styles = runes.as_array().into_iter().flatten();
    for p in styles.flat_map(|style| {
        std::iter::once(style).chain(style["slots"].as_array().into_iter().flatten().flat_map(|s| s["runes"].as_array().into_iter().flatten()))
    }) {
        let Some(id) = p["id"].as_i64() else { continue };
        if let Some(name) = p["name"].as_str() {
            perks.insert(id, name.to_string());
        }
        if let Some(icon) = p["icon"].as_str() {
            perk_icons.insert(id, icon.to_string());
        }
    }
    Ok(StaticData {
        version,
        champions,
        items,
        item_stats,
        completed_items,
        spells,
        spell_keys,
        perks,
        perk_icons,
        has_client_assets: false,
    })
}

/// Bytes of one Data Dragon image (`path` from the `StaticData` image helpers),
/// from the patch cache or the CDN. Cached next to the patch's JSON, so pruning
/// an old patch drops its images too.
pub fn fetch_image(agent: &ureq::Agent, version: &str, path: &str) -> Result<Vec<u8>, String> {
    use std::io::Read;
    const MAX: u64 = 4 << 20;
    let file = cache_dir().join(version).join("img").join(path.replace('/', "_"));
    if let Ok(b) = std::fs::read(&file) {
        return Ok(b);
    }
    let resp = agent.get(&format!("{CDN}/{path}")).call().map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    resp.into_reader().take(MAX).read_to_end(&mut bytes).map_err(|e| e.to_string())?;
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(&file, &bytes);
    }
    Ok(bytes)
}

/// `TFT13_Ahri` → `Ahri`, `TFT13_Sniper` → `Sniper`. Character/trait ids are
/// set-prefixed API names; stripping the prefix gives readable labels without
/// downloading the per-set TFT data files.
pub fn tft_label(api_name: &str) -> &str {
    match api_name.split_once('_') {
        Some((prefix, rest)) if prefix.to_ascii_uppercase().starts_with("TFT") => rest,
        _ => api_name,
    }
}
