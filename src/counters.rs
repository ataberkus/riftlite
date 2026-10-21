//! Live counter-item suggestions: what the enemies build (healing, shields, armor or
//! magic resist stacking, crit, attack speed) and shop items that answer it.
//!
//! Everything comes from the enemies' current items (Live Client Data API) and Data
//! Dragon's item stats and shop data. The result is a short list of options per threat
//! with the reason, never a build order.

use std::collections::{HashMap, HashSet};

use crate::ddragon::{Champion, ItemStats, StaticData};
use crate::live::{LiveGame, LivePlayer};
use crate::meta::{Choice, CoreBuild, Damage};

/// Arena: the Live Client API's teams do not identify duos, so the enemies are unknown.
const ARENA: i64 = 30;
/// Cheaper items (Doran's items, potions, most components) do not make an enemy a healer,
/// shielder and so on.
const THREAT_ITEM_GOLD: i64 = 1000;
/// Kills minus deaths at which one enemy alone makes a healing, shield, crit or attack
/// speed threat; otherwise it takes two enemies.
const FED_LEAD: i64 = 3;
/// Armor (magic resist) from items that makes an enemy a stacker; two stackers make a
/// threat, or one with `STACK_ALONE`.
const STACK: f64 = 100.0;
const STACK_ALONE: f64 = 150.0;
/// A team is physical (magic) heavy when that damage makes up this share of the weight of
/// the enemies whose damage type is known, from at least `HEAVY_MIN` enemies.
const HEAVY_SHARE: f64 = 0.7;
const HEAVY_MIN: usize = 3;
/// Summed attack speed from items that, like two on-hit items, makes an attack speed build.
const ATTACK_SPEED_BUILD: f64 = 0.5;
/// Unfinished items worth listing for their counter effect (Executioner's Calling,
/// Oblivion Orb, Bramble Vest cost 800).
const COMPONENT_GOLD: i64 = 700;
const MAX_THREATS: usize = 4;
const MAX_ITEMS: usize = 3;

/// What the enemies build that an item can answer.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Threat {
    Healing,
    Shields,
    Physical,
    Magic,
    Armor,
    MagicResist,
    Crit,
    AttackSpeed,
}

impl Threat {
    pub fn label(self) -> &'static str {
        match self {
            Threat::Healing => "Healing",
            Threat::Shields => "Shields",
            Threat::Physical => "Physical damage",
            Threat::Magic => "Magic damage",
            Threat::Armor => "Armor stacking",
            Threat::MagicResist => "Magic resist stacking",
            Threat::Crit => "Crit",
            Threat::AttackSpeed => "Attack speed / on-hit",
        }
    }

    /// How well an item answers it; 0 = not at all. Effects (Wounds, shield reduction,
    /// % penetration, Randuin's, Frozen Heart) count 2; armor and magic resist 1 per 40.
    fn answer(self, s: &ItemStats) -> f64 {
        let effect = |on: bool| if on { 2.0 } else { 0.0 };
        match self {
            Threat::Healing => effect(s.anti_heal),
            Threat::Shields => effect(s.anti_shield),
            Threat::Physical => s.armor / 40.0,
            Threat::Magic => s.mr / 40.0,
            Threat::Armor => effect(s.percent_armor_pen),
            Threat::MagicResist => effect(s.percent_magic_pen),
            Threat::Crit => effect(s.anti_crit),
            Threat::AttackSpeed => effect(s.anti_attack_speed),
        }
    }

    /// Answered by plain defensive stats, which tier-2 boots carry and components dilute.
    fn by_stats(self) -> bool {
        matches!(self, Threat::Physical | Threat::Magic)
    }
}

/// One threat with the items that answer it.
#[derive(Clone, PartialEq, Debug)]
pub struct CounterPick {
    pub threat: Threat,
    /// Names the counts or champions behind it ("3 enemies building healing (…)").
    pub reason: String,
    /// Options, best fit for your build first.
    pub items: Vec<CounterItem>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct CounterItem {
    pub id: i64,
    /// You hold an item of that name (any map or support variant).
    pub owned: bool,
}

/// One enemy as the threat rules see it.
struct Enemy<'a> {
    p: &'a LivePlayer,
    /// Non-consumable items: shop price of one unit, count, stats.
    items: Vec<(i64, i64, ItemStats)>,
    build: CoreBuild,
    /// The build's damage, else the champion's Data Dragon attack/magic ratings.
    damage: Damage,
    /// `1 + item gold / 3000 + kills / 5`: how much this enemy matters.
    weight: f64,
}

impl Enemy<'_> {
    fn fed(&self) -> bool {
        self.p.kills - self.p.deaths >= FED_LEAD
    }

    /// Holds an item of at least `THREAT_ITEM_GOLD` that does `f`.
    fn holds(&self, f: impl Fn(&ItemStats) -> bool) -> bool {
        self.items.iter().any(|(price, _, s)| *price >= THREAT_ITEM_GOLD && f(s))
    }

    fn sum(&self, f: impl Fn(&ItemStats) -> f64) -> f64 {
        self.items.iter().map(|(_, n, s)| f(s) * *n as f64).sum()
    }

    fn name(&self) -> &str {
        if self.p.champion.is_empty() { &self.p.champion_key } else { &self.p.champion }
    }
}

/// What the local player builds: decides which counter items fit.
struct Profile {
    build: CoreBuild,
    /// May take items without damage: tank, fighter or support champion, or a tank build.
    defensive: bool,
    /// Holds boots of tier 2 or more.
    has_boots: bool,
    /// Names of the items held, so map and support variants count as owned.
    owned: HashSet<String>,
}

fn champion<'a>(p: &LivePlayer, data: &'a StaticData) -> Option<&'a Champion> {
    data.champions.get(&data.champion_by_key(&p.champion_key)?)
}

fn rated_damage(c: &Champion) -> Damage {
    match c.attack.cmp(&c.magic) {
        std::cmp::Ordering::Greater => Damage::Ad,
        std::cmp::Ordering::Less => Damage::Ap,
        std::cmp::Ordering::Equal => Damage::None,
    }
}

fn held(p: &LivePlayer) -> impl Iterator<Item = &crate::live::LiveItem> {
    p.items.iter().filter(|i| !i.consumable && i.id > 0)
}

fn classify(p: &LivePlayer, data: &StaticData) -> CoreBuild {
    CoreBuild::classify(Choice { ids: held(p).map(|i| i.id).collect(), ..Default::default() }, data)
}

fn profile(me: &LivePlayer, data: &StaticData) -> Profile {
    let champ = champion(me, data);
    let mut build = classify(me, data);
    // No damage or tank items yet: the champion's class and ratings.
    if build.damage == Damage::None && !build.tank {
        if let Some(c) = champ {
            build.tank = c.tags.first().is_some_and(|t| t == "Tank");
            build.damage = if build.tank { Damage::None } else { rated_damage(c) };
            build.crit = c.tags.iter().any(|t| t == "Marksman");
        }
    }
    let tagged = |t: &str| champ.is_some_and(|c| c.tags.iter().any(|x| x == t));
    Profile {
        defensive: build.tank || tagged("Tank") || tagged("Fighter") || tagged("Support"),
        build,
        has_boots: held(me).any(|i| data.shop.get(&i.id).is_some_and(|s| s.boots_tier >= 2)),
        owned: held(me).filter_map(|i| data.items.get(&i.id).cloned()).collect(),
    }
}

/// Gives attack or ability power, crit or attack speed.
fn offensive(s: &ItemStats) -> bool {
    s.ap > 0.0 || s.ad > 0.0 || s.crit || s.attack_speed > 0.0
}

/// Damage items must fit your build's damage (`CoreBuild::fits`; an AP build also needs
/// ability power on them); items without damage only suit defensive players.
fn allowed(s: &ItemStats, p: &Profile) -> bool {
    if offensive(s) {
        p.build.damage != Damage::None && p.build.fits(s) && (p.build.damage != Damage::Ap || s.ap > 0.0)
    } else {
        p.defensive
    }
}

/// How well an item suits your build, added to the threat's answer: +1 for damage items
/// (−1 when its crit does not match yours, +0.5 for on-hit on on-hit builds), +1 for
/// items without damage on tank builds, +0.5 for health on defensive players.
fn fit(s: &ItemStats, p: &Profile) -> f64 {
    let mut f = 0.0;
    if offensive(s) {
        f += 1.0;
        if s.crit != p.build.crit {
            f -= 1.0;
        }
        if s.on_hit && p.build.on_hit {
            f += 0.5;
        }
    } else if p.build.tank {
        f += 1.0;
    }
    if s.health > 0.0 && p.defensive {
        f += 0.5;
    }
    f
}

/// Up to `MAX_ITEMS` items for `threat`, best first: buyable on `map`, finished items
/// (plus cheaper ones carrying the effect, and tier-2 boots for armor/magic resist while
/// you have none), one id per item name: the smallest valid on the map.
fn items_for(threat: Threat, p: &Profile, data: &StaticData, map: i64) -> Vec<CounterItem> {
    let mut by_name: HashMap<&str, i64> = HashMap::new();
    for (&id, shop) in &data.shop {
        let (Some(s), Some(name)) = (data.item_stats.get(&id), data.items.get(&id)) else { continue };
        if !shop.buyable_on(map) || threat.answer(s) <= 0.0 {
            continue;
        }
        let ok = if shop.boots_tier > 0 {
            shop.boots_tier == 2 && threat.by_stats() && !p.has_boots
        } else if data.completed_items.contains(&id) {
            allowed(s, p)
        } else {
            !threat.by_stats() && shop.gold >= COMPONENT_GOLD && allowed(s, p)
        };
        if ok {
            by_name.entry(name.as_str()).and_modify(|best| *best = (*best).min(id)).or_insert(id);
        }
    }
    let score = |id: i64| {
        let s = &data.item_stats[&id];
        let component = data.shop[&id].boots_tier == 0 && !data.completed_items.contains(&id);
        threat.answer(s) + fit(s, p) - if component { 1.5 } else { 0.0 }
    };
    let mut ids: Vec<i64> = by_name.into_values().collect();
    ids.sort_by(|&a, &b| score(b).total_cmp(&score(a)).then(data.shop[&a].gold.cmp(&data.shop[&b].gold)).then(a.cmp(&b)));
    ids.into_iter()
        .take(MAX_ITEMS)
        .map(|id| CounterItem { id, owned: data.items.get(&id).is_some_and(|n| p.owned.contains(n)) })
        .collect()
}

/// "Aatrox", "Aatrox and Vladimir", "Aatrox, Soraka and Vladimir".
fn names(es: &[&Enemy]) -> String {
    match es {
        [] => String::new(),
        [one] => one.name().to_string(),
        [rest @ .., last] => format!("{} and {}", rest.iter().map(|e| e.name()).collect::<Vec<_>>().join(", "), last.name()),
    }
}

fn who<'e, 'a>(enemies: &'e [Enemy<'a>], f: impl Fn(&Enemy) -> bool) -> Vec<&'e Enemy<'a>> {
    enemies.iter().filter(|e| f(*e)).collect()
}

/// A threat from enemies building `what`: two of them, or one who is fed.
fn group(threat: Threat, holders: Vec<&Enemy>, what: &str) -> Option<(Threat, String, f64)> {
    let weight: f64 = holders.iter().map(|e| e.weight).sum();
    let reason = match holders.as_slice() {
        [] => return None,
        [e] if e.fed() => format!("{} is fed ({}/{}) and builds {what}", e.name(), e.p.kills, e.p.deaths),
        [_] => return None,
        many => format!("{} enemies building {what} ({})", many.len(), names(many)),
    };
    Some((threat, reason, weight))
}

/// Enemies stacking armor (magic resist) from items.
fn stacking(threat: Threat, enemies: &[Enemy], stat: impl Fn(&ItemStats) -> f64, what: &str) -> Option<(Threat, String, f64)> {
    let stackers: Vec<(&Enemy, f64)> = enemies.iter().map(|e| (e, e.sum(&stat))).filter(|&(_, v)| v >= STACK).collect();
    let weight: f64 = stackers.iter().map(|(e, v)| e.weight * v / STACK).sum();
    let reason = match stackers.as_slice() {
        [] => return None,
        [(e, v)] if *v >= STACK_ALONE => format!("{} stacking {what} ({v:.0} from items)", e.name()),
        [_] => return None,
        many => format!("{} stacking {what}", names(&many.iter().map(|(e, _)| *e).collect::<Vec<_>>())),
    };
    Some((threat, reason, weight))
}

/// Most of the enemies' damage is `kind`.
fn heavy(threat: Threat, enemies: &[Enemy], kind: Damage, what: &str) -> Option<(Threat, String, f64)> {
    let known: Vec<&Enemy> = enemies.iter().filter(|e| e.damage != Damage::None).collect();
    let total: f64 = known.iter().map(|e| e.weight).sum();
    let side: Vec<&&Enemy> = known.iter().filter(|e| e.damage == kind).collect();
    let weight: f64 = side.iter().map(|e| e.weight).sum();
    (side.len() >= HEAVY_MIN && weight >= HEAVY_SHARE * total)
        .then(|| (threat, format!("{} of {} enemies deal {what} damage", side.len(), enemies.len()), weight))
}

/// Counter items for what the enemies hold now, most important threat first (summed
/// weight of the enemies behind it), at most `MAX_THREATS`; threats nothing in your
/// build can answer are left out. Empty in Arena, without Data Dragon item data, or
/// without the local player.
pub fn suggest(game: &LiveGame, data: &StaticData) -> Vec<CounterPick> {
    if game.map == ARENA || data.item_stats.is_empty() || data.shop.is_empty() {
        return Vec::new();
    }
    let Some(me) = game.me() else { return Vec::new() };
    let enemies: Vec<Enemy> = game
        .players
        .iter()
        .filter(|p| p.team != me.team)
        .map(|p| {
            let build = classify(p, data);
            let damage = match build.damage {
                Damage::None => champion(p, data).map_or(Damage::None, rated_damage),
                d => d,
            };
            Enemy {
                items: held(p).filter_map(|i| Some((i.price, i.count.max(1), *data.item_stats.get(&i.id)?))).collect(),
                weight: 1.0 + p.item_gold() as f64 / 3000.0 + p.kills as f64 / 5.0,
                build,
                damage,
                p,
            }
        })
        .collect();
    let mut found: Vec<(Threat, String, f64)> = [
        group(Threat::Healing, who(&enemies, |e| e.holds(|s| s.heals)), "healing"),
        group(Threat::Shields, who(&enemies, |e| e.holds(|s| s.shields)), "shields"),
        heavy(Threat::Physical, &enemies, Damage::Ad, "physical"),
        heavy(Threat::Magic, &enemies, Damage::Ap, "magic"),
        stacking(Threat::Armor, &enemies, |s| s.armor, "armor"),
        stacking(Threat::MagicResist, &enemies, |s| s.mr, "magic resist"),
        group(Threat::Crit, who(&enemies, |e| e.build.crit), "crit"),
        group(
            Threat::AttackSpeed,
            who(&enemies, |e| e.build.on_hit || e.sum(|s| s.attack_speed) >= ATTACK_SPEED_BUILD),
            "attack speed or on-hit",
        ),
    ]
    .into_iter()
    .flatten()
    .collect();
    found.sort_by(|a, b| b.2.total_cmp(&a.2));
    let p = profile(me, data);
    found
        .into_iter()
        .filter_map(|(threat, reason, _)| {
            let items = items_for(threat, &p, data, game.map);
            (!items.is_empty()).then_some(CounterPick { threat, reason, items })
        })
        .take(MAX_THREATS)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ddragon::ShopItem;
    use crate::live::LiveItem;

    const SR: i64 = crate::live::SUMMONERS_RIFT;

    fn champ(d: &mut StaticData, id: i64, key: &str, tags: &[&str], attack: i64, magic: i64) {
        let tags = tags.iter().map(|t| t.to_string()).collect();
        d.champions.insert(id, Champion { id: key.into(), name: key.into(), tags, attack, magic });
    }

    fn item(d: &mut StaticData, id: i64, name: &str, gold: i64, maps: &[i64], completed: bool, s: ItemStats) {
        d.items.insert(id, name.into());
        d.item_stats.insert(id, s);
        d.shop.insert(id, ShopItem { gold, maps: maps.to_vec(), purchasable: true, boots_tier: if s.boots { 2 } else { 0 }, from: Vec::new() });
        if completed {
            d.completed_items.insert(id);
        }
    }

    /// Hand-built from Data Dragon 16.20.1 stats and signals (variants trimmed to what the
    /// map rules need).
    fn data() -> StaticData {
        let mut d = StaticData::default();
        let base = ItemStats::default();
        let all = &[11, 12, 453];
        item(&mut d, 3072, "Bloodthirster", 3400, all, true, ItemStats { ad: 80.0, heals: true, shields: true, ..base });
        item(&mut d, 6333, "Death's Dance", 3300, all, true, ItemStats { ad: 60.0, armor: 50.0, heals: true, defensive: true, ..base });
        item(&mut d, 4633, "Riftmaker", 3100, all, true, ItemStats { ap: 70.0, health: 350.0, heals: true, defensive: true, ..base });
        item(&mut d, 3033, "Mortal Reminder", 3000, all, true, ItemStats { ad: 35.0, crit: true, anti_heal: true, percent_armor_pen: true, ..base });
        item(&mut d, 6609, "Chempunk Chainsword", 3000, all, true, ItemStats { ad: 45.0, health: 450.0, anti_heal: true, defensive: true, ..base });
        item(&mut d, 3123, "Executioner's Calling", 800, all, false, ItemStats { ad: 15.0, anti_heal: true, ..base });
        item(&mut d, 3165, "Morellonomicon", 2850, all, true, ItemStats { ap: 75.0, health: 350.0, anti_heal: true, defensive: true, ..base });
        item(&mut d, 3916, "Oblivion Orb", 800, all, false, ItemStats { ap: 25.0, anti_heal: true, ..base });
        let thornmail = ItemStats { armor: 75.0, health: 150.0, anti_heal: true, defensive: true, ..base };
        item(&mut d, 3075, "Thornmail", 2450, all, true, thornmail);
        item(&mut d, 323075, "Thornmail", 2650, &[11], true, thornmail);
        item(&mut d, 223075, "Thornmail", 2500, &[30], true, thornmail);
        item(&mut d, 3076, "Bramble Vest", 800, all, false, ItemStats { armor: 30.0, anti_heal: true, defensive: true, ..base });
        item(&mut d, 3157, "Zhonya's Hourglass", 3250, all, true, ItemStats { ap: 105.0, armor: 50.0, defensive: true, ..base });
        // Randuin's base id only on Summoner's Rift here, so ARAM has to use its 77xxxx id.
        let randuin = ItemStats { armor: 75.0, health: 350.0, anti_crit: true, defensive: true, ..base };
        item(&mut d, 3143, "Randuin's Omen", 2700, &[11], true, randuin);
        item(&mut d, 773143, "Randuin's Omen", 3125, &[12, 453], true, randuin);
        item(&mut d, 223143, "Randuin's Omen", 2500, &[30], true, randuin);
        item(&mut d, 3047, "Plated Steelcaps", 1200, all, false, ItemStats { armor: 25.0, boots: true, defensive: true, ..base });
        item(&mut d, 6673, "Immortal Shieldbow", 3000, all, true, ItemStats { ad: 55.0, crit: true, shields: true, ..base });
        champ(&mut d, 1, "Darius", &["Fighter", "Tank"], 9, 0);
        champ(&mut d, 2, "Jinx", &["Marksman"], 9, 2);
        champ(&mut d, 3, "Syndra", &["Mage"], 2, 9);
        champ(&mut d, 4, "Ornn", &["Tank", "Fighter"], 5, 3);
        champ(&mut d, 5, "Aatrox", &["Fighter"], 8, 3);
        champ(&mut d, 6, "Vladimir", &["Mage"], 2, 8);
        champ(&mut d, 7, "Zed", &["Assassin"], 9, 1);
        champ(&mut d, 8, "Caitlyn", &["Marksman"], 8, 2);
        champ(&mut d, 9, "Nautilus", &["Tank", "Support"], 4, 6);
        d
    }

    fn player(key: &str, team: &str, items: &[(i64, i64)]) -> LivePlayer {
        LivePlayer {
            name: key.into(),
            champion: key.into(),
            champion_key: key.into(),
            team: team.into(),
            position: String::new(),
            level: 11,
            kills: 0,
            deaths: 0,
            assists: 0,
            cs: 0,
            ward_score: 0.0,
            items: items.iter().map(|&(id, price)| LiveItem { id, name: String::new(), price, count: 1, consumable: false }).collect(),
            dead: false,
            respawn: 0.0,
            spells: Default::default(),
            keystone: 0,
            is_me: false,
        }
    }

    fn game(map: i64, me: LivePlayer, enemies: Vec<LivePlayer>) -> LiveGame {
        let mut me = me;
        me.is_me = true;
        LiveGame {
            game_time: 900.0,
            mode: "CLASSIC".into(),
            map,
            my_gold: 0.0,
            my_level: 11,
            abilities: [0; 4],
            players: std::iter::once(me).chain(enemies).collect(),
            objectives: vec![],
            dragons: (0, 0),
            feed: vec![],
            item_callouts: vec![],
            counters: vec![],
        }
    }

    fn pick(picks: &[CounterPick], t: Threat) -> Option<&CounterPick> {
        picks.iter().find(|c| c.threat == t)
    }

    fn ids(c: &CounterPick) -> Vec<i64> {
        c.items.iter().map(|i| i.id).collect()
    }

    #[test]
    fn healing_needs_two_healers_or_a_fed_one_and_answers_fit_the_build() {
        let d = data();
        let aatrox = || player("Aatrox", "CHAOS", &[(6333, 3300)]);
        let vlad = player("Vladimir", "CHAOS", &[(4633, 3100)]);
        let healing = |me: LivePlayer, foes: Vec<LivePlayer>| pick(&suggest(&game(SR, me, foes), &d), Threat::Healing).cloned();
        // One healer who is not ahead is no threat.
        assert_eq!(healing(player("Darius", "ORDER", &[]), vec![aatrox()]), None);
        // Fed, alone: it is.
        let mut fed = aatrox();
        (fed.kills, fed.deaths) = (6, 1);
        assert_eq!(healing(player("Darius", "ORDER", &[]), vec![fed]).unwrap().reason, "Aatrox is fed (6/1) and builds healing");
        let foes = || vec![aatrox(), vlad.clone()];
        let bruiser = healing(player("Darius", "ORDER", &[]), foes()).unwrap();
        assert_eq!(bruiser.reason, "2 enemies building healing (Aatrox and Vladimir)");
        // Fighter: health + damage first, Thornmail allowed, crit Mortal Reminder last.
        assert_eq!(ids(&bruiser), [6609, 3075, 3033]);
        // Marksman: the crit anti-heal first, no Thornmail.
        assert_eq!(ids(&healing(player("Jinx", "ORDER", &[]), foes()).unwrap()), [3033, 6609, 3123]);
        // Mage: only AP options, the cheap Oblivion Orb as the second one.
        assert_eq!(ids(&healing(player("Syndra", "ORDER", &[]), foes()).unwrap()), [3165, 3916]);
        // Tank build (from its items): no damage items at all; the held Thornmail is marked.
        let tank = healing(player("Ornn", "ORDER", &[(3075, 2450), (3143, 2700)]), foes()).unwrap();
        assert_eq!(ids(&tank), [3075, 3076]);
        assert_eq!(tank.items.iter().map(|i| i.owned).collect::<Vec<_>>(), [true, false]);
    }

    #[test]
    fn physical_heavy_team_gets_armor_that_fits() {
        let d = data();
        // Four of five deal physical damage (Darius and Zed by champion ratings, no items yet).
        let foes = || {
            vec![
                player("Aatrox", "CHAOS", &[(6333, 3300)]),
                player("Zed", "CHAOS", &[]),
                player("Caitlyn", "CHAOS", &[]),
                player("Darius", "CHAOS", &[]),
                player("Vladimir", "CHAOS", &[]),
            ]
        };
        let mage = suggest(&game(SR, player("Syndra", "ORDER", &[]), foes()), &d);
        let armor = pick(&mage, Threat::Physical).unwrap();
        assert_eq!(armor.reason, "4 of 5 enemies deal physical damage");
        assert_eq!(armor, &mage[0], "the heaviest threat comes first");
        // Zhonya's and boots; never Randuin's or Thornmail for a mage.
        assert_eq!(ids(armor), [3157, 3047]);
        // With tier-2 boots held, no boots are suggested.
        let shod = suggest(&game(SR, player("Syndra", "ORDER", &[(3047, 1200)]), foes()), &d);
        assert_eq!(ids(pick(&shod, Threat::Physical).unwrap()), [3157]);
        // A fighter may take pure armor.
        let fighter = suggest(&game(SR, player("Darius", "ORDER", &[]), foes()), &d);
        assert!(ids(pick(&fighter, Threat::Physical).unwrap()).contains(&3143));
        // Three of five is not heavy.
        let mut mixed = foes();
        mixed[1].champion_key = "Syndra".into();
        assert!(pick(&suggest(&game(SR, player("Syndra", "ORDER", &[]), mixed), &d), Threat::Physical).is_none());
    }

    #[test]
    fn items_valid_on_the_map_with_the_smallest_same_name_id() {
        let d = data();
        // Two healers, a fed crit carry: healing and crit threats for a tank player.
        let mut cait = player("Caitlyn", "CHAOS", &[(6673, 3000), (3033, 3000)]);
        (cait.kills, cait.deaths) = (8, 2);
        let foes = || vec![player("Aatrox", "CHAOS", &[(6333, 3300)]), player("Vladimir", "CHAOS", &[(4633, 3100)]), cait.clone()];
        let tank = || player("Nautilus", "ORDER", &[]);
        let all_ids = |map: i64| suggest(&game(map, tank(), foes()), &d).iter().flat_map(ids).collect::<Vec<_>>();
        let sr = all_ids(SR);
        // Base ids, not the support (32xxxx) or Arena (22xxxx) variants.
        assert!(sr.contains(&3075) && sr.contains(&3143));
        assert!(!sr.iter().any(|&id| id >= 100_000), "{sr:?}");
        // ARAM: Randuin's base id is not sold there, its ARAM id is.
        let aram = all_ids(12);
        assert!(aram.contains(&773143) && aram.contains(&3075) && !aram.contains(&3143));
        // Arena: teams do not identify duos, so nothing.
        assert!(all_ids(ARENA).is_empty());
        // The crit threat names the fed carry.
        let crit = suggest(&game(SR, tank(), foes()), &d);
        assert_eq!(pick(&crit, Threat::Crit).unwrap().reason, "Caitlyn is fed (8/2) and builds crit");
    }

    #[test]
    fn owned_counts_any_variant_of_the_item() {
        let d = data();
        let foes = vec![player("Aatrox", "CHAOS", &[(6333, 3300)]), player("Vladimir", "CHAOS", &[(4633, 3100)])];
        // The support Thornmail (323075) is held; the suggestion is the base id, marked owned.
        let picks = suggest(&game(SR, player("Nautilus", "ORDER", &[(323075, 2650)]), foes), &d);
        let healing = pick(&picks, Threat::Healing).unwrap();
        assert_eq!(healing.items[0], CounterItem { id: 3075, owned: true });
        assert!(!healing.items[1].owned);
        // No item data: nothing.
        assert!(suggest(&game(SR, player("Nautilus", "ORDER", &[]), vec![]), &StaticData::default()).is_empty());
    }
}
