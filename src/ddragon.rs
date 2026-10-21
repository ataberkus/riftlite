//! Champion and item names from Riot's official Data Dragon CDN, cached per
//! patch under `%LOCALAPPDATA%\riftlite\ddragon`. Only the name maps, the item
//! stats and shop facts used to classify builds and suggest counter items and the
//! image paths the UI shows are kept in memory.

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

/// What an item gives, from Data Dragon's `stats`, `tags` and description markup; used to
/// tell an AP build from an AD, crit, on-hit or tank one and to find counter items.
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
    /// `FlatArmorMod`
    pub armor: f64,
    /// `FlatSpellBlockMod` (magic resist)
    pub mr: f64,
    /// `FlatHPPoolMod`
    pub health: f64,
    /// `PercentAttackSpeedMod`, 0.25 = 25 %
    pub attack_speed: f64,
    /// Heals its holder or allies: `LifeSteal`/`SpellVamp` tag, `<lifeSteal>`/`<omnivamp>`
    /// markup, or `<healing>` markup worded as a heal ("restore", "heals").
    pub heals: bool,
    /// `<shield>` markup.
    pub shields: bool,
    /// Applies Grievous Wounds (`Wounds` keyword).
    pub anti_heal: bool,
    /// "reduce(s) Shields" (Serpent's Fang).
    pub anti_shield: bool,
    /// `ArmorPenetration` tag (lethality or % penetration).
    pub armor_pen: bool,
    /// `MagicPenetration` tag.
    pub magic_pen: bool,
    /// "% Armor Penetration", or "reduces the target's Armor" (Black Cleaver's shred).
    pub percent_armor_pen: bool,
    /// "% Magic Penetration".
    pub percent_magic_pen: bool,
    /// "less damage from Critical Strikes" (Randuin's Omen).
    pub anti_crit: bool,
    /// Lowers the "Attack Speed of nearby" enemies (Frozen Heart).
    pub anti_attack_speed: bool,
}

/// Where and whether an item can be bought, from Data Dragon's `gold`, `maps`, `from` and flags.
#[derive(Clone, Default, PartialEq, Debug)]
pub struct ShopItem {
    /// `gold.total`
    pub gold: i64,
    /// Map ids with `maps[id] == true` (11 Summoner's Rift, 12/453 ARAM, 30 Arena).
    pub maps: Vec<i64>,
    /// `gold.purchasable`, `inStore` not false, not `hideFromAll`, no `requiredChampion`/`requiredAlly`.
    pub purchasable: bool,
    /// Boots upgrade level: 1 basic boots, 2 built from them (Plated Steelcaps), 3 their
    /// upgrades; 0 for other items.
    pub boots_tier: u8,
    /// Ids of the items it is built from (`from`), in Data Dragon's order.
    pub from: Vec<i64>,
}

impl ShopItem {
    /// Can be bought on `map`.
    pub fn buyable_on(&self, map: i64) -> bool {
        self.purchasable && self.maps.contains(&map)
    }
}

#[derive(Default, Clone)]
pub struct StaticData {
    pub version: String,
    pub champions: HashMap<i64, Champion>,
    pub items: HashMap<i64, String>,
    pub item_stats: HashMap<i64, ItemStats>,
    /// Finished (legendary) items, for enemy item callouts.
    pub completed_items: HashSet<i64>,
    /// Price, maps and purchasability of every Data Dragon item. Mode variants share the
    /// base item's name under other ids: 32xxxx Summoner's Rift support upgrades, 77xxxx
    /// ARAM (maps 12/453), 22xxxx Arena (map 30).
    pub shop: HashMap<i64, ShopItem>,
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

    /// The minimap of a map (`11` Summoner's Rift, `12` Howling Abyss).
    pub fn map_image(&self, map_id: i64) -> Option<String> {
        (map_id > 0 && !self.version.is_empty()).then(|| format!("cdn/{}/img/map/map{map_id}.png", self.version))
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
    let item_stats = item_data.iter().map(|&(id, i)| (id, item_stats(i))).collect();
    let by_id: HashMap<i64, &Value> = item_data.iter().copied().collect();
    let shop = item_data.iter().map(|&(id, i)| (id, shop_item(id, i, &by_id))).collect();
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
        shop,
        spells,
        spell_keys,
        perks,
        perk_icons,
        has_client_assets: false,
    })
}

fn has_tag(i: &Value, t: &str) -> bool {
    i["tags"].as_array().is_some_and(|a| a.iter().any(|x| x == t))
}

/// Description text with markup tags replaced by spaces and whitespace collapsed.
fn plain_text(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                out.push(' ');
            }
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// A `<healing>` span that is worded as a heal: its text or the two words before it say
/// "heal(s/ing/ed)" or "restore…". Data Dragon also colors plain Health stats with it
/// (Overlord's Bloodmail's "<healing>bonus Health</healing>"), which is no healing.
fn heal_markup(desc: &str) -> bool {
    let is_heal = |w: &str| {
        let w = w.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
        w.starts_with("restor") || matches!(w.as_str(), "heal" | "heals" | "healing" | "healed")
    };
    let parts: Vec<&str> = desc.split("<healing>").collect();
    parts.windows(2).any(|w| {
        let inner = plain_text(w[1].split("</healing>").next().unwrap_or(""));
        let before = plain_text(w[0]);
        let tail: Vec<&str> = before.split_whitespace().rev().take(2).collect();
        inner.split_whitespace().chain(tail).any(|w| is_heal(w))
    })
}

/// Stats, tags and description signals of one Data Dragon item.
fn item_stats(i: &Value) -> ItemStats {
    let tag = |t: &str| has_tag(i, t);
    let stat = |k: &str| i["stats"][k].as_f64().unwrap_or(0.0);
    let desc = i["description"].as_str().unwrap_or("");
    let text = plain_text(desc);
    let lower = text.to_lowercase();
    ItemStats {
        ap: stat("FlatMagicDamageMod"),
        ad: stat("FlatPhysicalDamageMod"),
        crit: tag("CriticalStrike"),
        on_hit: tag("OnHit"),
        defensive: tag("Health") || tag("Armor") || tag("SpellBlock"),
        boots: tag("Boots"),
        armor: stat("FlatArmorMod"),
        mr: stat("FlatSpellBlockMod"),
        health: stat("FlatHPPoolMod"),
        attack_speed: stat("PercentAttackSpeedMod"),
        heals: tag("LifeSteal") || tag("SpellVamp") || desc.contains("<lifeSteal>") || desc.contains("<omnivamp>") || heal_markup(desc),
        shields: desc.contains("<shield>"),
        anti_heal: desc.contains("Wounds</keyword>"),
        anti_shield: lower.contains("reduce shields") || lower.contains("reduces shields"),
        armor_pen: tag("ArmorPenetration"),
        magic_pen: tag("MagicPenetration"),
        percent_armor_pen: text.contains("% Armor Penetration") || lower.contains("reduces the target's armor"),
        percent_magic_pen: text.contains("% Magic Penetration"),
        anti_crit: lower.contains("less damage from critical strikes"),
        anti_attack_speed: lower.contains("attack speed of nearby"),
    }
}

/// Boots upgrade level of `id` (see [`ShopItem::boots_tier`]): one more than the best boots it
/// is built from.
fn boots_tier(id: i64, items: &HashMap<i64, &Value>) -> u8 {
    let Some(i) = items.get(&id).filter(|i| has_tag(i, "Boots")) else { return 0 };
    let from = i["from"].as_array().into_iter().flatten().filter_map(|f| f.as_str()?.parse().ok());
    1 + from.map(|f| boots_tier(f, items)).max().unwrap_or(0)
}

fn shop_item(id: i64, i: &Value, items: &HashMap<i64, &Value>) -> ShopItem {
    let unset = |k: &str| i[k].as_str().is_none_or(str::is_empty);
    ShopItem {
        gold: i["gold"]["total"].as_i64().unwrap_or(0),
        maps: i["maps"]
            .as_object()
            .into_iter()
            .flatten()
            .filter(|(_, on)| on.as_bool() == Some(true))
            .filter_map(|(m, _)| m.parse().ok())
            .collect(),
        purchasable: i["gold"]["purchasable"].as_bool() == Some(true)
            && i["inStore"].as_bool() != Some(false)
            && i["hideFromAll"].as_bool() != Some(true)
            && unset("requiredChampion")
            && unset("requiredAlly"),
        boots_tier: boots_tier(id, items),
        from: i["from"].as_array().into_iter().flatten().filter_map(|f| f.as_str()?.parse().ok()).collect(),
    }
}

/// Bytes of one Data Dragon image (`path` from the `StaticData` image helpers, or an absolute
/// `https://` URL such as a CommunityDragon icon), from the patch cache or the network.
/// Cached next to the patch's JSON, so pruning an old patch drops its images too.
pub fn fetch_image(agent: &ureq::Agent, version: &str, path: &str) -> Result<Vec<u8>, String> {
    use std::io::Read;
    const MAX: u64 = 4 << 20;
    let name: String = path.trim_start_matches("https://").chars().map(|c| if c.is_ascii_alphanumeric() || c == '.' { c } else { '_' }).collect();
    let file = cache_dir().join(version).join("img").join(name);
    if let Ok(b) = std::fs::read(&file) {
        return Ok(b);
    }
    let url = if path.starts_with("https://") { path.to_string() } else { format!("{CDN}/{path}") };
    let resp = agent.get(&url).call().map_err(|e| e.to_string())?;
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Descriptions, tags and stats as Data Dragon 16.20.1 has them.
    #[test]
    fn item_signals_from_real_descriptions() {
        let bloodthirster = item_stats(&json!({
            "description": "<mainText><stats><attention>80</attention> Attack Damage<br><attention>15%</attention> Life Steal</stats><br><br><passive>Ichorshield</passive><br>Convert excess healing from your Lifesteal to a <shield>Shield</shield>.</mainText>",
            "tags": ["Damage", "LifeSteal"], "stats": {"FlatPhysicalDamageMod": 80, "PercentLifeStealMod": 0.15}}));
        assert!(bloodthirster.heals && bloodthirster.shields && !bloodthirster.anti_heal);
        assert_eq!(bloodthirster.ad, 80.0);
        // `<healing>` only colors a Health stat here: not a heal.
        let bloodmail = item_stats(&json!({
            "description": "<mainText><stats><attention>30</attention> Attack Damage<br><attention>550</attention> Health</stats><br><br><passive>Tyranny</passive><br>Gain 2.5% of your <healing>bonus Health</healing> as <scaleAD>Attack Damage</scaleAD>. <br><br><passive>Retribution</passive><br>Gain up to 12% increased <scaleAD>Attack Damage</scaleAD> based on your percent missing Health. </mainText>",
            "tags": ["Health", "Damage"], "stats": {"FlatHPPoolMod": 550, "FlatPhysicalDamageMod": 30}}));
        assert!(!bloodmail.heals);
        assert_eq!(bloodmail.health, 550.0);
        // Worded as a heal ("Restore" right before the span): a heal.
        let redemption = item_stats(&json!({
            "description": "<mainText><stats><attention>30</attention> Ability Power</stats><br><br><active>Intervention</active><br>Restore <healing>150 - 350 Health</healing> to allied units.</mainText>",
            "tags": ["SpellDamage"], "stats": {"FlatMagicDamageMod": 30}}));
        assert!(redemption.heals);
        // Armor shred counts as % armor penetration; the tag alone (lethality) does not.
        let cleaver = item_stats(&json!({
            "description": "<mainText><stats><attention>45</attention> Attack Damage<br><attention>400</attention> Health</stats><br><br><passive>Carve</passive><br>Dealing <physicalDamage>physical damage</physicalDamage> to champions reduces the target's <scaleArmor>Armor by 6%</scaleArmor> for 6 seconds.</mainText>",
            "tags": ["Health", "Damage", "OnHit", "ArmorPenetration"], "stats": {"FlatHPPoolMod": 400, "FlatPhysicalDamageMod": 45}}));
        assert!(cleaver.percent_armor_pen && cleaver.armor_pen);
        let fang = item_stats(&json!({
            "description": "<mainText><stats><attention>55</attention> Attack Damage<br><attention>15</attention> Lethality</stats><br><br><passive>Shield Reaver</passive><br>Damaging an enemy champion reduces Shields they gain by % for 3 seconds. <br>If they were not already affected by <passive>Shield Reaver</passive>, reduce Shields on them by %.</mainText>",
            "tags": ["Damage", "ArmorPenetration"], "stats": {"FlatPhysicalDamageMod": 55}}));
        assert!(fang.anti_shield && fang.armor_pen && !fang.percent_armor_pen && !fang.shields);
        let thornmail = item_stats(&json!({
            "description": "<mainText><stats><attention>150</attention> Health<br><attention>75</attention> Armor</stats><br><br><passive>Thorns</passive><br>When struck by an Attack, deal <magicDamage>magic damage</magicDamage> to the attacker and apply 40% <keyword>Wounds</keyword> for 3 seconds if they are a champion.</mainText>",
            "tags": ["Health", "Armor"], "stats": {"FlatArmorMod": 75, "FlatHPPoolMod": 150}}));
        assert!(thornmail.anti_heal && !thornmail.anti_crit && !thornmail.heals);
        assert_eq!((thornmail.armor, thornmail.health), (75.0, 150.0));
        let randuin = item_stats(&json!({
            "description": "<mainText><stats><attention>350</attention> Health<br><attention>75</attention> Armor</stats><br><br><passive>Resilience</passive><br>Receive 30% less damage from Critical Strikes.<br><active>Humility</active><br><keyword>Slow</keyword> nearby enemies by 70% for 2 seconds.</mainText>",
            "tags": ["Health", "Armor", "Active", "Slow"], "stats": {"FlatArmorMod": 75, "FlatHPPoolMod": 350}}));
        assert!(randuin.anti_crit && !randuin.anti_attack_speed);
        let frozen = item_stats(&json!({
            "description": "<mainText><stats><attention>75</attention> Armor</stats><br><br><passive>Winter's Caress</passive><br>Reduce the <attackSpeed>Attack Speed</attackSpeed> of nearby champions by <attackSpeed>20%</attackSpeed>.</mainText>",
            "tags": ["Armor", "Mana"], "stats": {"FlatArmorMod": 75}}));
        assert!(frozen.anti_attack_speed);
        let void = item_stats(&json!({
            "description": "<mainText><stats><attention>95</attention> Ability Power<br><attention>40%</attention> Magic Penetration</stats><br><br></mainText>",
            "tags": ["MagicPenetration", "SpellDamage"], "stats": {"FlatMagicDamageMod": 95}}));
        assert!(void.percent_magic_pen && void.magic_pen && !void.percent_armor_pen);
    }

    #[test]
    fn shop_maps_purchasability_and_boots_tiers() {
        let items = json!({
            "1001": {"tags": ["Boots"], "gold": {"total": 300, "purchasable": true}, "maps": {"11": true, "12": true, "30": false}},
            "3047": {"tags": ["Armor", "Boots"], "from": ["1001", "1029"], "gold": {"total": 1200, "purchasable": true}, "maps": {"11": true}},
            "3174": {"tags": ["Armor", "Boots"], "from": ["3047"], "gold": {"total": 1200, "purchasable": true}, "maps": {"11": true}},
            "1029": {"tags": ["Armor"], "gold": {"total": 300, "purchasable": true}, "maps": {"11": true}},
            "7001": {"tags": [], "gold": {"total": 2500, "purchasable": true}, "maps": {"11": true}, "requiredAlly": "Ornn"},
            "7002": {"tags": [], "gold": {"total": 2500, "purchasable": true}, "maps": {"11": true}, "inStore": false},
            "7003": {"tags": [], "gold": {"total": 0, "purchasable": false}, "maps": {"11": true}},
            "7004": {"tags": [], "gold": {"total": 2500, "purchasable": true}, "maps": {"11": true}, "requiredChampion": ""}
        });
        let by_id: HashMap<i64, &Value> = items.as_object().unwrap().iter().map(|(k, v)| (k.parse().unwrap(), v)).collect();
        let shop = |id: i64| shop_item(id, by_id[&id], &by_id);
        assert_eq!([1001, 3047, 3174, 1029].map(|id| shop(id).boots_tier), [1, 2, 3, 0]);
        let mut maps = shop(1001).maps;
        maps.sort();
        assert_eq!(maps, [11, 12], "false map entries are not maps the item is on");
        assert!(shop(3047).buyable_on(11) && !shop(3047).buyable_on(12));
        // Ornn upgrades, hidden and unbuyable items are out; an empty requirement is none.
        assert_eq!([7001, 7002, 7003, 7004].map(|id| shop(id).purchasable), [false, false, false, true]);
    }
}
