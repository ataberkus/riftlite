//! Riot's official Live Client Data API (https://127.0.0.1:2999), available
//! while a League game is running. Read-only; no game memory access.

use serde_json::Value;

/// Summoner's Rift; dragons, Baron and timed inhibitor respawns only exist there.
pub const SUMMONERS_RIFT: i64 = 11;
const DRAGON_RESPAWN: f64 = 300.0;
const ELDER_RESPAWN: f64 = 360.0;
const BARON_RESPAWN: f64 = 360.0;
const SR_INHIB_RESPAWN: f64 = 300.0;
/// First spawns on Summoner's Rift (League wiki): Dragon 5:00, Voidgrubs 8:00 until 14:45,
/// Rift Herald 15:00 until 19:45, Baron 20:00.
const DRAGON_FIRST: f64 = 300.0;
const GRUBS_FIRST: f64 = 480.0;
const GRUBS_GONE: f64 = 885.0;
const HERALD_FIRST: f64 = 900.0;
const HERALD_GONE: f64 = 1185.0;
const BARON_FIRST: f64 = 1200.0;
/// Feed entries kept, newest first.
const FEED_LEN: usize = 20;

#[derive(Clone, PartialEq, Debug)]
pub struct LiveItem {
    pub id: i64,
    pub name: String,
    /// Shop price the game reports for one unit.
    pub price: i64,
    pub count: i64,
    pub consumable: bool,
}

#[derive(Clone, PartialEq, Debug)]
pub struct LivePlayer {
    pub name: String,
    pub champion: String,
    /// Data Dragon id (`MissFortune`), from `rawChampionName`.
    pub champion_key: String,
    pub team: String,
    /// `TOP`/`JUNGLE`/`MIDDLE`/`BOTTOM`/`UTILITY`; `NONE`/empty outside Summoner's Rift.
    pub position: String,
    pub level: i64,
    pub kills: i64,
    pub deaths: i64,
    pub assists: i64,
    pub cs: i64,
    pub ward_score: f64,
    pub items: Vec<LiveItem>,
    pub dead: bool,
    pub respawn: f64,
    /// Summoner spells as Data Dragon ids (`SummonerFlash`), from each spell's `rawDisplayName`;
    /// empty when the game names a spell differently.
    pub spells: [String; 2],
    /// Keystone rune id (`runes.keystone.id`); 0 when the game reports none.
    pub keystone: i64,
    pub is_me: bool,
}

impl LivePlayer {
    /// Gold value of the items held (consumables excluded).
    pub fn item_gold(&self) -> i64 {
        self.items.iter().filter(|i| !i.consumable).map(|i| i.price * i.count.max(1)).sum()
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Objective {
    pub label: String,
    /// Game time at which it is back up, when the respawn time is known.
    pub ready_at: Option<f64>,
    /// Game time of the kill that started the timer.
    pub since: f64,
    /// Team that took it (`ORDER`/`CHAOS`) when the killer could be matched.
    pub taken_by: Option<String>,
    /// Game time at which it leaves the map untaken (Voidgrubs, Rift Herald).
    pub until: Option<f64>,
    /// The event feed reports its kill. Voidgrub kills are not reported, so "up" may be stale.
    pub kill_tracked: bool,
}

/// One line of the recent-events feed.
#[derive(Clone, PartialEq, Debug)]
pub struct FeedEvent {
    pub time: f64,
    pub text: String,
    /// Team the event favors (`ORDER`/`CHAOS`), when known.
    pub team: Option<String>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct LiveGame {
    pub game_time: f64,
    pub mode: String,
    pub map: i64,
    pub my_gold: f64,
    pub my_level: i64,
    /// Q, W, E, R levels of the local player.
    pub abilities: [i64; 4],
    pub players: Vec<LivePlayer>,
    pub objectives: Vec<Objective>,
    /// Dragons per team (ORDER, CHAOS), elder excluded.
    pub dragons: (usize, usize),
    /// Kills, objectives and structures from the event feed, newest first.
    pub feed: Vec<FeedEvent>,
    /// Finished items enemies bought while riftlite watched, newest first ([`ItemWatch`]).
    pub item_callouts: Vec<ItemCallout>,
    /// Items that answer what the enemies hold now, most important threat first
    /// ([`crate::counters::suggest`]; filled by the worker, empty from the API parse).
    pub counters: Vec<crate::counters::CounterPick>,
}

/// An enemy finished an item.
#[derive(Clone, PartialEq, Debug)]
pub struct ItemCallout {
    /// Game time of the poll that first showed it.
    pub time: f64,
    pub champion: String,
    pub champion_key: String,
    pub item: i64,
}

/// Remembers each enemy's finished items between polls; the Live Client API has no purchase
/// events. Items already held when riftlite first sees a player are not called out.
#[derive(Default)]
pub struct ItemWatch {
    last_time: f64,
    held: std::collections::HashMap<String, Vec<i64>>,
    callouts: Vec<ItemCallout>,
}

impl ItemWatch {
    /// Updates from this poll and fills `g.item_callouts`.
    pub fn observe(&mut self, g: &mut LiveGame, completed: &std::collections::HashSet<i64>) {
        // The clock went back: another game.
        if g.game_time + 1.0 < self.last_time {
            *self = ItemWatch::default();
        }
        self.last_time = g.game_time;
        let Some(my_team) = g.me().map(|m| m.team.clone()) else { return };
        for p in g.players.iter().filter(|p| p.team != my_team) {
            let now: Vec<i64> = p.items.iter().map(|i| i.id).filter(|id| completed.contains(id)).collect();
            let key = format!("{}|{}|{}", p.team, p.champion_key, p.name);
            if let Some(before) = self.held.insert(key, now.clone()) {
                for &item in now.iter().filter(|id| !before.contains(id)) {
                    self.callouts.insert(
                        0,
                        ItemCallout { time: g.game_time, champion: p.champion.clone(), champion_key: p.champion_key.clone(), item },
                    );
                }
            }
        }
        self.callouts.truncate(FEED_LEN);
        g.item_callouts = self.callouts.clone();
    }
}

impl LiveGame {
    /// The mode as League names it in the client ("ARAM Mayhem" for `KIWI`, …).
    pub fn mode_name(&self) -> String {
        match self.mode.as_str() {
            "CLASSIC" if self.map == 11 => "Summoner's Rift".into(),
            "CLASSIC" => "Classic".into(),
            "KIWI" => "ARAM Mayhem".into(),
            "ARAM" => "ARAM".into(),
            "URF" | "ARURF" => "URF".into(),
            "CHERRY" => "Arena".into(),
            "SWIFTPLAY" => "Swiftplay".into(),
            "ONEFORALL" => "One for All".into(),
            "PRACTICETOOL" => "Practice Tool".into(),
            "STRAWBERRY" => "Swarm".into(),
            "NEXUSBLITZ" => "Nexus Blitz".into(),
            "TUTORIAL" | "TUTORIAL_MODULE_1" | "TUTORIAL_MODULE_2" | "TUTORIAL_MODULE_3" => "Tutorial".into(),
            m => {
                let mut c = m.chars();
                c.next().map_or_else(String::new, |f| f.to_uppercase().chain(c.flat_map(char::to_lowercase)).collect())
            }
        }
    }

    pub fn me(&self) -> Option<&LivePlayer> {
        self.players.iter().find(|p| p.is_me)
    }

    pub fn team_kills(&self, team: &str) -> i64 {
        self.players.iter().filter(|p| p.team == team).map(|p| p.kills).sum()
    }

    pub fn team_gold(&self, team: &str) -> i64 {
        self.players.iter().filter(|p| p.team == team).map(LivePlayer::item_gold).sum()
    }

    /// The enemy in the same position (Summoner's Rift with assigned roles only).
    pub fn lane_opponent(&self) -> Option<&LivePlayer> {
        let me = self.me()?;
        if matches!(me.position.as_str(), "" | "NONE") {
            return None;
        }
        self.players.iter().find(|p| p.team != me.team && p.position == me.position)
    }

    /// Skill points available to spend right now.
    pub fn unspent_skill_points(&self) -> i64 {
        (self.my_level - self.abilities.iter().sum::<i64>()).max(0)
    }
}

pub fn fetch(agent: &ureq::Agent) -> Option<LiveGame> {
    let v: Value = agent.get("https://127.0.0.1:2999/liveclientdata/allgamedata").call().ok()?.into_json().ok()?;
    parse(&v)
}

fn name_of(p: &Value) -> String {
    // Hidden/streamer-mode players arrive as riotId "#"; trim so they render as empty, not "#".
    p["riotId"].as_str().or(p["summonerName"].as_str()).unwrap_or("").trim_matches('#').to_string()
}

fn parse(v: &Value) -> Option<LiveGame> {
    let gd = v.get("gameData")?;
    let ap = &v["activePlayer"];
    let me = name_of(ap);
    let players: Vec<LivePlayer> = v["allPlayers"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|p| {
            let name = name_of(p);
            let sc = &p["scores"];
            LivePlayer {
                is_me: !me.is_empty() && name == me,
                champion: p["championName"].as_str().unwrap_or("").to_string(),
                champion_key: p["rawChampionName"]
                    .as_str()
                    .unwrap_or("")
                    .trim_start_matches("game_character_displayname_")
                    .to_string(),
                team: p["team"].as_str().unwrap_or("").to_string(),
                position: p["position"].as_str().unwrap_or("").to_string(),
                level: p["level"].as_i64().unwrap_or(0),
                kills: sc["kills"].as_i64().unwrap_or(0),
                deaths: sc["deaths"].as_i64().unwrap_or(0),
                assists: sc["assists"].as_i64().unwrap_or(0),
                cs: sc["creepScore"].as_i64().unwrap_or(0),
                ward_score: sc["wardScore"].as_f64().unwrap_or(0.0),
                items: p["items"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|i| LiveItem {
                        id: i["itemID"].as_i64().unwrap_or(0),
                        name: i["displayName"].as_str().unwrap_or("").to_string(),
                        price: i["price"].as_i64().unwrap_or(0),
                        count: i["count"].as_i64().unwrap_or(1),
                        consumable: i["consumable"].as_bool().unwrap_or(false),
                    })
                    .collect(),
                dead: p["isDead"].as_bool().unwrap_or(false),
                respawn: p["respawnTimer"].as_f64().unwrap_or(0.0),
                spells: [spell_key(&p["summonerSpells"]["summonerSpellOne"]), spell_key(&p["summonerSpells"]["summonerSpellTwo"])],
                keystone: p["runes"]["keystone"]["id"].as_i64().unwrap_or(0),
                name,
            }
        })
        .collect();
    let map = gd["mapNumber"].as_i64().unwrap_or(0);
    let ab = &ap["abilities"];
    let lvl = |k: &str| ab[k]["abilityLevel"].as_i64().unwrap_or(0);
    let game_time = gd["gameTime"].as_f64().unwrap_or(0.0);
    let classic = gd["gameMode"] == "CLASSIC";
    let events = &v["events"]["Events"];
    let (objectives, dragons) = objectives(events, map, &players, game_time, classic);
    let feed = feed(events, &players);
    Some(LiveGame {
        game_time,
        mode: gd["gameMode"].as_str().unwrap_or("").to_string(),
        map,
        my_gold: ap["currentGold"].as_f64().unwrap_or(0.0),
        my_level: ap["level"].as_i64().unwrap_or(0),
        abilities: [lvl("Q"), lvl("W"), lvl("E"), lvl("R")],
        players,
        objectives,
        dragons,
        feed,
        item_callouts: Vec::new(),
        counters: Vec::new(),
    })
}

/// `GeneratedTip_SummonerSpell_SummonerFlash_DisplayName` → `SummonerFlash`, the Data Dragon id.
fn spell_key(spell: &Value) -> String {
    spell["rawDisplayName"]
        .as_str()
        .and_then(|r| r.strip_prefix("GeneratedTip_SummonerSpell_")?.strip_suffix("_DisplayName"))
        .unwrap_or("")
        .to_string()
}

/// Team of an event's `KillerName` (matched against riot id or its game-name part).
fn killer_team(e: &Value, players: &[LivePlayer]) -> Option<String> {
    let k = e["KillerName"].as_str()?;
    players
        .iter()
        .find(|p| p.name == k || p.name.split('#').next() == Some(k))
        .map(|p| p.team.clone())
}

/// Respawn timers derived from the game's own event feed; on classic Summoner's Rift also the
/// first spawns.
fn objectives(events: &Value, map: i64, players: &[LivePlayer], now: f64, classic: bool) -> (Vec<Objective>, (usize, usize)) {
    let mut dragons = (0usize, 0usize);
    let mut dragon: Option<Objective> = None;
    let mut baron: Option<Objective> = None;
    let mut herald_taken = false;
    // Inhibitor name → (kill time, team that killed it); removed on respawn.
    let mut inhibs: Vec<(String, f64, Option<String>)> = Vec::new();
    let timer = |label: &str, ready_at: f64, since: f64, taken_by: Option<String>| Objective {
        label: label.into(),
        ready_at: Some(ready_at),
        since,
        taken_by,
        until: None,
        kill_tracked: true,
    };
    for e in events.as_array().into_iter().flatten() {
        let t = e["EventTime"].as_f64().unwrap_or(0.0);
        let team = killer_team(e, players);
        match e["EventName"].as_str().unwrap_or("") {
            "DragonKill" => {
                let kind = e["DragonType"].as_str().unwrap_or("");
                let elder = kind.eq_ignore_ascii_case("Elder");
                if !elder {
                    match team.as_deref() {
                        Some("ORDER") => dragons.0 += 1,
                        Some("CHAOS") => dragons.1 += 1,
                        _ => {}
                    }
                }
                // After a soul (4 drakes), only the Elder spawns.
                let soul = dragons.0 >= 4 || dragons.1 >= 4;
                let (label, wait) = if elder || soul { ("Elder", ELDER_RESPAWN) } else { ("Dragon", DRAGON_RESPAWN) };
                dragon = Some(timer(label, t + wait, t, team));
            }
            "BaronKill" => baron = Some(timer("Baron", t + BARON_RESPAWN, t, team)),
            "HeraldKill" => herald_taken = true,
            "InhibKilled" => {
                if let Some(name) = e["InhibKilled"].as_str() {
                    inhibs.retain(|(n, ..)| n != name);
                    inhibs.push((name.to_string(), t, team));
                }
            }
            "InhibRespawned" => {
                if let Some(name) = e["InhibRespawned"].as_str() {
                    inhibs.retain(|(n, ..)| n != name);
                }
            }
            _ => {}
        }
    }
    let first = map == SUMMONERS_RIFT && classic;
    if first {
        dragon = dragon.or_else(|| Some(timer("Dragon", DRAGON_FIRST, 0.0, None)));
        baron = baron.or_else(|| Some(timer("Baron", BARON_FIRST, 0.0, None)));
    }
    let mut out: Vec<Objective> = dragon.into_iter().collect();
    if first && now < GRUBS_GONE {
        out.push(Objective { until: Some(GRUBS_GONE), kill_tracked: false, ..timer("Voidgrubs", GRUBS_FIRST, 0.0, None) });
    }
    if first && now < HERALD_GONE && !herald_taken {
        out.push(Objective { until: Some(HERALD_GONE), ..timer("Rift Herald", HERALD_FIRST, 0.0, None) });
    }
    out.extend(baron);
    for (name, t, team) in inhibs {
        out.push(Objective {
            label: structure_label(&name),
            ready_at: (map == SUMMONERS_RIFT).then_some(t + SR_INHIB_RESPAWN),
            since: t,
            taken_by: team,
            until: None,
            kill_tracked: true,
        });
    }
    (out, dragons)
}

/// `Red top outer turret` from a structure name, as far as it can be read. Two formats exist:
/// Riot's documented `Turret_T2_L_03_A` / `Barracks_T1_L1`, and `Turret_TChaos_L2_P3_<id>` /
/// `Inhib_TOrder_L0_P1_<id>` (lanes L0 bot, L1 mid, L2 top; P3 outer, P2 inner, P1 inhibitor,
/// P4/P5 nexus). Parts it cannot place are left out.
fn structure_label(name: &str) -> String {
    let parts: Vec<&str> = name.split('_').collect();
    let kind = match parts.first() {
        Some(&"Turret") => "turret",
        Some(&("Barracks" | "Inhib")) => "inhibitor",
        _ => "structure",
    };
    let owner = match parts.get(1) {
        Some(&("T1" | "TOrder")) => "Blue",
        Some(&("T2" | "TChaos")) => "Red",
        _ => "",
    };
    let riot_tier = |n: &str| match n {
        "03" => "outer",
        "02" => "inner",
        _ => "",
    };
    let (lane, tier) = match parts.get(2..).unwrap_or(&[]) {
        [l, p, ..] if l.len() == 2 && l.starts_with('L') && p.starts_with('P') => {
            let lane = match *l {
                "L0" => "bot",
                "L1" => "mid",
                "L2" => "top",
                _ => "",
            };
            match (*p, kind) {
                ("P3", _) => (lane, "outer"),
                ("P2", _) => (lane, "inner"),
                ("P1", "turret") => (lane, "inhibitor"),
                ("P4" | "P5", _) => ("", "nexus"),
                _ => (lane, ""),
            }
        }
        ["L", n, ..] => ("top", riot_tier(n)),
        ["R", n, ..] => ("bot", riot_tier(n)),
        ["L1", ..] => ("top", ""),
        ["C1", ..] => ("mid", ""),
        ["R1", ..] => ("bot", ""),
        _ => ("", ""),
    };
    let label = [owner, lane, tier, kind].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
    let mut c = label.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// A killer or victim name as a champion when it is a player, else what it is.
fn who(name: &str, players: &[LivePlayer]) -> String {
    if let Some(p) = players.iter().find(|p| p.name == name || p.name.split('#').next() == Some(name)) {
        return if p.champion.is_empty() { p.name.clone() } else { p.champion.clone() };
    }
    match name {
        n if n.starts_with("Turret") => "A turret".into(),
        n if n.starts_with("Minion") => "Minions".into(),
        n if n.starts_with("SRU") => "A monster".into(),
        "" => "Someone".into(),
        n => n.into(),
    }
}

/// Kills, multikills, aces, objectives and structures, newest first.
fn feed(events: &Value, players: &[LivePlayer]) -> Vec<FeedEvent> {
    let mut out = Vec::new();
    for e in events.as_array().into_iter().flatten() {
        let s = |k: &str| e[k].as_str().unwrap_or("");
        let killer = who(s("KillerName"), players);
        let stolen = if e["Stolen"] == "True" || e["Stolen"] == true { " (stolen)" } else { "" };
        let text = match s("EventName") {
            "ChampionKill" => format!("{killer} killed {}", who(s("VictimName"), players)),
            "Multikill" => {
                let kind = match e["KillStreak"].as_i64().unwrap_or(0) {
                    2 => "double kill",
                    3 => "triple kill",
                    4 => "quadra kill",
                    5 => "penta kill",
                    _ => continue,
                };
                format!("{killer}: {kind}")
            }
            "Ace" => format!("Ace by {}", if s("AcingTeam") == "CHAOS" { "red" } else { "blue" }),
            "TurretKilled" => format!("{killer} destroyed the {}", structure_label(s("TurretKilled")).to_lowercase()),
            "InhibKilled" => format!("{killer} destroyed the {}", structure_label(s("InhibKilled")).to_lowercase()),
            "InhibRespawned" => format!("{} respawned", structure_label(s("InhibRespawned"))),
            "DragonKill" if s("DragonType").eq_ignore_ascii_case("Elder") => format!("{killer} took the Elder Dragon{stolen}"),
            "DragonKill" => format!("{killer} took the {} dragon{stolen}", s("DragonType").to_lowercase()),
            "HeraldKill" => format!("{killer} took the Rift Herald{stolen}"),
            "BaronKill" => format!("{killer} took Baron{stolen}"),
            _ => continue,
        };
        let team = match s("EventName") {
            "Ace" => Some(s("AcingTeam").to_string()).filter(|t| !t.is_empty()),
            "InhibRespawned" => None,
            _ => killer_team(e, players),
        };
        out.push(FeedEvent { time: e["EventTime"].as_f64().unwrap_or(0.0), text, team });
    }
    out.reverse();
    out.truncate(FEED_LEN);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn player(name: &str, team: &str) -> LivePlayer {
        LivePlayer {
            name: name.into(),
            champion: String::new(),
            champion_key: String::new(),
            team: team.into(),
            position: String::new(),
            level: 1,
            kills: 0,
            deaths: 0,
            assists: 0,
            cs: 0,
            ward_score: 0.0,
            items: vec![],
            dead: false,
            respawn: 0.0,
            spells: Default::default(),
            keystone: 0,
            is_me: false,
        }
    }

    #[test]
    fn dragon_soul_switches_to_elder_and_inhibs_clear_on_respawn() {
        let players = [player("Blue#TR1", "ORDER"), player("Red#TR1", "CHAOS")];
        let mut ev: Vec<Value> = (0..4)
            .map(|i| serde_json::json!({"EventName": "DragonKill", "EventTime": 400.0 + 400.0 * i as f64, "DragonType": "Fire", "KillerName": "Blue"}))
            .collect();
        ev.push(serde_json::json!({"EventName": "InhibKilled", "EventTime": 1500.0, "InhibKilled": "Barracks_T2_C1", "KillerName": "Blue#TR1"}));
        ev.push(serde_json::json!({"EventName": "InhibKilled", "EventTime": 1510.0, "InhibKilled": "Barracks_T2_L1", "KillerName": "Blue"}));
        ev.push(serde_json::json!({"EventName": "InhibRespawned", "EventTime": 1810.0, "InhibRespawned": "Barracks_T2_C1"}));
        let (obj, drakes) = objectives(&Value::from(ev), SUMMONERS_RIFT, &players, 1900.0, false);
        assert_eq!(drakes, (4, 0));
        assert_eq!(obj[0].label, "Elder");
        assert_eq!(obj[0].ready_at, Some(1600.0 + ELDER_RESPAWN));
        assert_eq!(obj.len(), 2, "respawned mid inhibitor must be gone");
        assert_eq!(obj[1].label, "Red top inhibitor");
        assert_eq!(obj[1].ready_at, Some(1510.0 + SR_INHIB_RESPAWN));
        assert_eq!(obj[1].taken_by.as_deref(), Some("ORDER"));
    }

    #[test]
    fn inhibitor_respawn_unknown_off_summoners_rift() {
        let ev = serde_json::json!([{"EventName": "InhibKilled", "EventTime": 600.0, "InhibKilled": "Barracks_T1_C1", "KillerName": "x"}]);
        let (obj, _) = objectives(&ev, 12, &[], 700.0, true);
        assert_eq!(obj[0].ready_at, None);
        assert_eq!(obj[0].taken_by, None);
    }

    #[test]
    fn first_spawns_until_taken_or_gone() {
        let ev = serde_json::json!([{"EventName": "HeraldKill", "EventTime": 950.0, "KillerName": "x", "Stolen": "False"}]);
        let labels = |now: f64, ev: &Value| objectives(ev, SUMMONERS_RIFT, &[], now, true).0.into_iter().map(|o| o.label).collect::<Vec<_>>();
        assert_eq!(labels(100.0, &Value::Null), ["Dragon", "Voidgrubs", "Rift Herald", "Baron"]);
        // Grubs leave at 14:45; the Herald is gone once taken.
        assert_eq!(labels(960.0, &ev), ["Dragon", "Baron"]);
        assert!(objectives(&Value::Null, SUMMONERS_RIFT, &[], 100.0, false).0.is_empty(), "no first spawns outside classic");
    }

    #[test]
    fn structure_names_in_both_formats() {
        assert_eq!(structure_label("Turret_T2_L_03_A"), "Red top outer turret");
        assert_eq!(structure_label("Turret_T1_C_05_A"), "Blue turret");
        assert_eq!(structure_label("Turret_TChaos_L0_P2_528623213"), "Red bot inner turret");
        assert_eq!(structure_label("Turret_TOrder_L1_P5_3625540808"), "Blue nexus turret");
        assert_eq!(structure_label("Inhib_TOrder_L2_P1_2669080337"), "Blue top inhibitor");
        assert_eq!(structure_label("Turret_TOrder_L2_P1_1543541934"), "Blue top inhibitor turret");
    }

    #[test]
    fn item_watch_calls_out_only_new_enemy_items() {
        let item = |id: i64| LiveItem { id, name: String::new(), price: 0, count: 1, consumable: false };
        let game = |t: f64, mine: &[i64], theirs: &[i64]| {
            let mut me = player("Me", "ORDER");
            me.is_me = true;
            me.items = mine.iter().copied().map(item).collect();
            let mut foe = player("Foe", "CHAOS");
            foe.champion = "Zed".into();
            foe.items = theirs.iter().copied().map(item).collect();
            LiveGame {
                game_time: t,
                mode: "CLASSIC".into(),
                map: SUMMONERS_RIFT,
                my_gold: 0.0,
                my_level: 1,
                abilities: [0; 4],
                players: vec![me, foe],
                objectives: vec![],
                dragons: (0, 0),
                feed: vec![],
                item_callouts: vec![],
                counters: vec![],
            }
        };
        let completed = std::collections::HashSet::from([3071, 6692]);
        let mut w = ItemWatch::default();
        // Held at first sight: not called out.
        let mut g = game(600.0, &[], &[3071]);
        w.observe(&mut g, &completed);
        assert!(g.item_callouts.is_empty());
        // Own items, components and repeats are ignored; the new finished item is not.
        let mut g = game(700.0, &[6692], &[3071, 6692, 1036]);
        w.observe(&mut g, &completed);
        assert_eq!(g.item_callouts.iter().map(|c| (c.item, c.time, c.champion.as_str())).collect::<Vec<_>>(), [(6692, 700.0, "Zed")]);
        // A new game (clock went back) forgets the old one.
        let mut g = game(30.0, &[], &[6692]);
        w.observe(&mut g, &completed);
        assert!(g.item_callouts.is_empty());
    }
}
