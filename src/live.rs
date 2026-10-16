//! Riot's official Live Client Data API (https://127.0.0.1:2999), available
//! while a League game is running. Read-only; no game memory access.

use serde_json::Value;

/// Summoner's Rift; dragons, Baron and timed inhibitor respawns only exist there.
pub const SUMMONERS_RIFT: i64 = 11;
const DRAGON_RESPAWN: f64 = 300.0;
const ELDER_RESPAWN: f64 = 360.0;
const BARON_RESPAWN: f64 = 360.0;
const SR_INHIB_RESPAWN: f64 = 300.0;

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
}

impl LiveGame {
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
                name,
            }
        })
        .collect();
    let map = gd["mapNumber"].as_i64().unwrap_or(0);
    let ab = &ap["abilities"];
    let lvl = |k: &str| ab[k]["abilityLevel"].as_i64().unwrap_or(0);
    let (objectives, dragons) = objectives(&v["events"]["Events"], map, &players);
    Some(LiveGame {
        game_time: gd["gameTime"].as_f64().unwrap_or(0.0),
        mode: gd["gameMode"].as_str().unwrap_or("").to_string(),
        map,
        my_gold: ap["currentGold"].as_f64().unwrap_or(0.0),
        my_level: ap["level"].as_i64().unwrap_or(0),
        abilities: [lvl("Q"), lvl("W"), lvl("E"), lvl("R")],
        players,
        objectives,
        dragons,
    })
}

/// Team of an event's `KillerName` (matched against riot id or its game-name part).
fn killer_team(e: &Value, players: &[LivePlayer]) -> Option<String> {
    let k = e["KillerName"].as_str()?;
    players
        .iter()
        .find(|p| p.name == k || p.name.split('#').next() == Some(k))
        .map(|p| p.team.clone())
}

/// Respawn timers derived from the game's own event feed.
fn objectives(events: &Value, map: i64, players: &[LivePlayer]) -> (Vec<Objective>, (usize, usize)) {
    let mut dragons = (0usize, 0usize);
    let mut dragon: Option<Objective> = None;
    let mut baron: Option<Objective> = None;
    // Inhibitor name → (kill time, team that killed it); removed on respawn.
    let mut inhibs: Vec<(String, f64, Option<String>)> = Vec::new();
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
                dragon = Some(Objective { label: label.into(), ready_at: Some(t + wait), since: t, taken_by: team });
            }
            "BaronKill" => {
                baron = Some(Objective { label: "Baron".into(), ready_at: Some(t + BARON_RESPAWN), since: t, taken_by: team })
            }
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
    let mut out: Vec<Objective> = dragon.into_iter().chain(baron).collect();
    for (name, t, team) in inhibs {
        // `Barracks_T1_L1`: T1 is ORDER's (blue) inhibitor, T2 CHAOS's.
        let owner = if name.contains("_T1") { "Blue" } else if name.contains("_T2") { "Red" } else { "" };
        let lane = match name.rsplit('_').next().unwrap_or("") {
            "L1" => " top",
            "C1" => " mid",
            "R1" => " bot",
            _ => "",
        };
        out.push(Objective {
            label: format!("{owner}{lane} inhibitor").trim_start().to_string(),
            ready_at: (map == SUMMONERS_RIFT).then_some(t + SR_INHIB_RESPAWN),
            since: t,
            taken_by: team,
        });
    }
    (out, dragons)
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
        let (obj, drakes) = objectives(&Value::from(ev), SUMMONERS_RIFT, &players);
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
        let (obj, _) = objectives(&ev, 12, &[]);
        assert_eq!(obj[0].ready_at, None);
        assert_eq!(obj[0].taken_by, None);
    }
}
