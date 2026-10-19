//! Match timeline analysis: deaths, gold/XP curves and skill orders from the client's
//! `game-timelines` endpoint (match-v4 shaped frames, one per minute; not verified against a
//! running client, so every field is read with a default).

use serde_json::Value;

/// Summoner's Rift playable bounds in game units (x west→east, y south→north).
const SR_MIN: (f32, f32) = (-120.0, -120.0);
const SR_MAX: (f32, f32) = (14870.0, 14980.0);
const SKILL_KEYS: [char; 4] = ['Q', 'W', 'E', 'R'];

#[derive(Clone, PartialEq, Debug, Default)]
pub struct Timeline {
    pub frames: Vec<Frame>,
    /// Every recognised event, time-ordered (the client's order within a timestamp is kept).
    pub events: Vec<Event>,
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct Frame {
    pub time_ms: i64,
    /// Participant id order.
    pub players: Vec<PlayerFrame>,
}

#[derive(Clone, Copy, PartialEq, Debug, Default)]
pub struct PlayerFrame {
    pub participant_id: i64,
    pub total_gold: i64,
    pub xp: i64,
    pub level: i64,
    /// Lane minions plus jungle monsters.
    pub cs: i64,
    /// Game coordinates; 0 when the frame has no position.
    pub x: i64,
    pub y: i64,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Event {
    pub time_ms: i64,
    pub kind: EventKind,
}

/// Participant ids are 1..=10; a killer of 0 is a turret, minion or monster.
#[derive(Clone, PartialEq, Debug)]
pub enum EventKind {
    Kill { killer: i64, victim: i64, assists: Vec<i64>, x: i64, y: i64 },
    /// `slot` 1..=4 = Q W E R.
    SkillUp { participant: i64, slot: i64 },
    ItemBought { participant: i64, item: i64 },
    /// `team` owned the destroyed building (100 blue, 200 red); `label` like `Red mid inhibitor`.
    Building { killer: i64, team: i64, label: String },
    /// `label` like `Dragon (Fire)`, `Baron`.
    Monster { killer: i64, label: String },
}

#[derive(Clone, PartialEq, Debug)]
pub struct Death {
    pub time_ms: i64,
    pub x: i64,
    pub y: i64,
    pub killer: i64,
    pub victim: i64,
    pub assists: Vec<i64>,
}

/// `me` minus the opponent at one frame.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct LaneDiff {
    pub minute: usize,
    pub gold: i64,
    pub cs: i64,
    pub xp: i64,
}

/// `None` when the timeline has no frames.
pub fn parse(v: &Value) -> Option<Timeline> {
    let raw = v["frames"].as_array().filter(|f| !f.is_empty())?;
    let n = |v: &Value, k: &str| v[k].as_i64().unwrap_or(0);
    let mut frames = Vec::with_capacity(raw.len());
    let mut events = Vec::new();
    for f in raw {
        let mut players: Vec<PlayerFrame> = f["participantFrames"]
            .as_object()
            .into_iter()
            .flatten()
            .map(|(key, p)| PlayerFrame {
                participant_id: p["participantId"].as_i64().or_else(|| key.parse().ok()).unwrap_or(0),
                total_gold: n(p, "totalGold"),
                xp: n(p, "xp"),
                level: n(p, "level"),
                cs: n(p, "minionsKilled") + n(p, "jungleMinionsKilled"),
                x: n(&p["position"], "x"),
                y: n(&p["position"], "y"),
            })
            .collect();
        players.sort_by_key(|p| p.participant_id);
        frames.push(Frame { time_ms: n(f, "timestamp"), players });
        events.extend(f["events"].as_array().into_iter().flatten().filter_map(parse_event));
    }
    events.sort_by_key(|e| e.time_ms);
    Some(Timeline { frames, events })
}

fn parse_event(e: &Value) -> Option<Event> {
    let n = |k: &str| e[k].as_i64().unwrap_or(0);
    let s = |k: &str| e[k].as_str().unwrap_or("");
    let kind = match s("type") {
        "CHAMPION_KILL" => EventKind::Kill {
            killer: n("killerId"),
            victim: n("victimId"),
            assists: e["assistingParticipantIds"].as_array().into_iter().flatten().filter_map(Value::as_i64).collect(),
            x: e["position"]["x"].as_i64().unwrap_or(0),
            y: e["position"]["y"].as_i64().unwrap_or(0),
        },
        // An evolution (Kha'Zix, Viktor…) spends no skill point.
        "SKILL_LEVEL_UP" if s("levelUpType") == "EVOLVE" => return None,
        "SKILL_LEVEL_UP" => EventKind::SkillUp { participant: n("participantId"), slot: n("skillSlot") },
        "ITEM_PURCHASED" => EventKind::ItemBought { participant: n("participantId"), item: n("itemId") },
        "BUILDING_KILL" => EventKind::Building {
            killer: n("killerId"),
            team: n("teamId"),
            label: building_label(n("teamId"), s("buildingType"), s("laneType"), s("towerType")),
        },
        "ELITE_MONSTER_KILL" => EventKind::Monster { killer: n("killerId"), label: monster_label(s("monsterType"), s("monsterSubType")) },
        _ => return None,
    };
    Some(Event { time_ms: n("timestamp"), kind })
}

/// `Blue top outer turret`, in the live feed's words; parts the event does not state are left out.
fn building_label(team: i64, building: &str, lane: &str, tower: &str) -> String {
    let owner = match team {
        100 => "Blue",
        200 => "Red",
        _ => "",
    };
    let kind = match building {
        "TOWER_BUILDING" => "turret",
        "INHIBITOR_BUILDING" => "inhibitor",
        _ => "structure",
    };
    let lane = match lane {
        "TOP_LANE" => "top",
        "MID_LANE" => "mid",
        "BOT_LANE" => "bot",
        _ => "",
    };
    let (lane, tier) = match tower {
        "OUTER_TURRET" => (lane, "outer"),
        "INNER_TURRET" => (lane, "inner"),
        "BASE_TURRET" => (lane, "inhibitor"),
        "NEXUS_TURRET" => ("", "nexus"),
        _ => (lane, ""),
    };
    let label = [owner, lane, tier, kind].into_iter().filter(|s| !s.is_empty()).collect::<Vec<_>>().join(" ");
    let mut c = label.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

/// `Dragon (Fire)`, `Elder Dragon`, `Baron`, `Rift Herald`, `Voidgrubs`; other monsters by their
/// raw type in words.
fn monster_label(monster: &str, sub: &str) -> String {
    match (monster, sub) {
        ("DRAGON", "ELDER_DRAGON") => "Elder Dragon".into(),
        ("DRAGON", sub) => {
            let element = match sub {
                "FIRE_DRAGON" => "Fire",
                "WATER_DRAGON" => "Ocean",
                "AIR_DRAGON" => "Cloud",
                "EARTH_DRAGON" => "Mountain",
                "HEXTECH_DRAGON" => "Hextech",
                "CHEMTECH_DRAGON" => "Chemtech",
                _ => return "Dragon".into(),
            };
            format!("Dragon ({element})")
        }
        ("BARON_NASHOR", _) => "Baron".into(),
        ("RIFTHERALD", _) => "Rift Herald".into(),
        ("HORDE", _) => "Voidgrubs".into(),
        ("", _) => "Monster".into(),
        (other, _) => {
            let words = other.to_lowercase().replace('_', " ");
            let mut c = words.chars();
            c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
        }
    }
}

impl Timeline {
    /// Skill points in order, e.g. `QEWQQRQ`.
    pub fn skill_order(&self, participant_id: i64) -> String {
        self.events
            .iter()
            .filter_map(|e| match e.kind {
                EventKind::SkillUp { participant, slot } if participant == participant_id => {
                    usize::try_from(slot - 1).ok().and_then(|i| SKILL_KEYS.get(i).copied())
                }
                _ => None,
            })
            .collect()
    }

    /// Kills with `participant_id` as the victim.
    pub fn deaths(&self, participant_id: i64) -> Vec<Death> {
        self.kills(|_, victim, _| victim == participant_id)
    }

    /// Kills `participant_id` made or assisted.
    pub fn takedowns(&self, participant_id: i64) -> Vec<Death> {
        self.kills(|killer, victim, assists| victim != participant_id && (killer == participant_id || assists.contains(&participant_id)))
    }

    fn kills(&self, keep: impl Fn(i64, i64, &[i64]) -> bool) -> Vec<Death> {
        self.events
            .iter()
            .filter_map(|e| match &e.kind {
                EventKind::Kill { killer, victim, assists, x, y } if keep(*killer, *victim, assists) => {
                    Some(Death { time_ms: e.time_ms, x: *x, y: *y, killer: *killer, victim: *victim, assists: assists.clone() })
                }
                _ => None,
            })
            .collect()
    }

    /// Per frame: (minutes, total gold of `team` minus everyone else's).
    pub fn gold_lead(&self, team: &[i64]) -> Vec<(f32, i64)> {
        self.frames
            .iter()
            .map(|f| {
                let lead: i64 = f.players.iter().map(|p| if team.contains(&p.participant_id) { p.total_gold } else { -p.total_gold }).sum();
                (f.time_ms as f32 / 60_000.0, lead)
            })
            .collect()
    }

    /// `me` minus `opponent` at frame `minute`; `None` when the frame or either player is missing.
    pub fn lane_diff(&self, me: i64, opponent: i64, minute: usize) -> Option<LaneDiff> {
        let f = self.frames.get(minute)?;
        let find = |id: i64| f.players.iter().find(|p| p.participant_id == id);
        let (a, b) = (find(me)?, find(opponent)?);
        Some(LaneDiff { minute, gold: a.total_gold - b.total_gold, cs: a.cs - b.cs, xp: a.xp - b.xp })
    }
}

/// Summoner's Rift game coordinates → 0..1 image coordinates, origin top-left (game y grows
/// north, image y grows down); clamped to the map.
pub fn sr_minimap(x: i64, y: i64) -> (f32, f32) {
    let u = (x as f32 - SR_MIN.0) / (SR_MAX.0 - SR_MIN.0);
    let v = 1.0 - (y as f32 - SR_MIN.1) / (SR_MAX.1 - SR_MIN.1);
    (u.clamp(0.0, 1.0), v.clamp(0.0, 1.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn frame(t: i64, gold: [i64; 4], events: Value) -> Value {
        let players: serde_json::Map<String, Value> = (1..=4)
            .map(|id| {
                let g = gold[id as usize - 1];
                (id.to_string(), json!({"participantId": id, "totalGold": g, "xp": g / 2, "minionsKilled": g / 100, "jungleMinionsKilled": 1, "level": 1, "position": {"x": 100, "y": 200}}))
            })
            .collect();
        json!({"timestamp": t, "participantFrames": players, "events": events})
    }

    fn skill(t: i64, who: i64, slot: i64) -> Value {
        json!({"type": "SKILL_LEVEL_UP", "timestamp": t, "participantId": who, "skillSlot": slot, "levelUpType": "NORMAL"})
    }

    fn kill(t: i64, killer: i64, victim: i64, assists: &[i64]) -> Value {
        json!({"type": "CHAMPION_KILL", "timestamp": t, "killerId": killer, "victimId": victim, "assistingParticipantIds": assists, "position": {"x": 7000, "y": 7100}})
    }

    #[test]
    fn skill_order_keeps_same_frame_order_and_skips_evolutions() {
        let tl = parse(&json!({"frames": [
            frame(0, [500; 4], json!([skill(1000, 1, 1), skill(1500, 2, 2)])),
            // Level 2 and 3 inside one frame, same timestamp: client order (E then W) must stay.
            frame(60_000, [500; 4], json!([
                skill(70_000, 1, 3), skill(70_000, 1, 2),
                {"type": "SKILL_LEVEL_UP", "timestamp": 75_000, "participantId": 1, "skillSlot": 4, "levelUpType": "EVOLVE"},
            ])),
            // Events of a later frame stamped earlier still sort by time.
            frame(120_000, [500; 4], json!([skill(119_000, 1, 1), skill(130_000, 1, 4)])),
        ]}))
        .expect("has frames");
        assert_eq!(tl.skill_order(1), "QEWQR");
        assert_eq!(tl.skill_order(2), "W");
        assert_eq!(tl.skill_order(9), "");
        assert!(parse(&json!({"frames": []})).is_none());
        assert!(parse(&json!({})).is_none());
    }

    #[test]
    fn deaths_and_takedowns_from_kill_events() {
        let tl = parse(&json!({"frames": [
            frame(0, [500; 4], json!([])),
            frame(60_000, [500; 4], json!([
                kill(65_000, 3, 1, &[4]),
                kill(90_000, 1, 3, &[]),
                kill(95_000, 2, 4, &[1]),
                // Executed by a turret: still a death, nobody's takedown.
                kill(100_000, 0, 1, &[]),
                {"type": "WARD_PLACED", "timestamp": 101_000, "creatorId": 1},
            ])),
        ]}))
        .unwrap();
        let deaths = tl.deaths(1);
        assert_eq!(deaths.iter().map(|d| (d.time_ms, d.killer)).collect::<Vec<_>>(), [(65_000, 3), (100_000, 0)]);
        assert_eq!(deaths[0].assists, [4]);
        assert_eq!((deaths[0].x, deaths[0].y), (7000, 7100));
        // A kill and an assist both count; the victim is kept.
        assert_eq!(tl.takedowns(1).iter().map(|d| d.victim).collect::<Vec<_>>(), [3, 4]);
        assert_eq!(tl.takedowns(4).iter().map(|d| d.victim).collect::<Vec<_>>(), [1]);
        assert_eq!(tl.events.len(), 4, "unknown event types are dropped");
    }

    #[test]
    fn gold_lead_sign_follows_the_team_and_lane_diff_reads_one_frame() {
        let tl = parse(&json!({"frames": [
            frame(0, [500, 500, 500, 500], json!([])),
            frame(60_000, [1200, 900, 700, 800], json!([])),
            frame(120_000, [1500, 1000, 2000, 1900], json!([])),
        ]}))
        .unwrap();
        let lead = tl.gold_lead(&[1, 2]);
        assert_eq!(lead, [(0.0, 0), (1.0, 600), (2.0, -1400)]);
        // The other side sees the mirror image.
        assert_eq!(tl.gold_lead(&[3, 4])[2].1, 1400);
        let d = tl.lane_diff(1, 3, 1).expect("frame 1 exists");
        assert_eq!((d.gold, d.xp, d.cs), (500, 250, 5));
        assert_eq!(tl.lane_diff(1, 3, 3), None);
        assert_eq!(tl.lane_diff(1, 7, 1), None);
    }

    #[test]
    fn building_and_monster_labels() {
        let ev = |v: Value| parse_event(&v).map(|e| e.kind);
        assert_eq!(
            ev(json!({"type": "BUILDING_KILL", "timestamp": 1, "killerId": 2, "teamId": 200, "buildingType": "INHIBITOR_BUILDING", "laneType": "MID_LANE"})),
            Some(EventKind::Building { killer: 2, team: 200, label: "Red mid inhibitor".into() })
        );
        assert_eq!(
            ev(json!({"type": "BUILDING_KILL", "teamId": 100, "buildingType": "TOWER_BUILDING", "laneType": "BOT_LANE", "towerType": "NEXUS_TURRET"})),
            Some(EventKind::Building { killer: 0, team: 100, label: "Blue nexus turret".into() })
        );
        assert_eq!(
            ev(json!({"type": "ELITE_MONSTER_KILL", "killerId": 6, "monsterType": "DRAGON", "monsterSubType": "FIRE_DRAGON"})),
            Some(EventKind::Monster { killer: 6, label: "Dragon (Fire)".into() })
        );
        // A sub-type riftlite does not know stays a plain dragon rather than a guess.
        assert_eq!(ev(json!({"type": "ELITE_MONSTER_KILL", "monsterType": "DRAGON"})), Some(EventKind::Monster { killer: 0, label: "Dragon".into() }));
        assert_eq!(ev(json!({"type": "ELITE_MONSTER_KILL", "monsterType": "ATAKHAN"})), Some(EventKind::Monster { killer: 0, label: "Atakhan".into() }));
    }

    #[test]
    fn minimap_corners_flip_y_and_clamp() {
        assert_eq!(sr_minimap(-120, -120), (0.0, 1.0), "blue fountain corner is bottom-left");
        assert_eq!(sr_minimap(14870, 14980), (1.0, 0.0), "red fountain corner is top-right");
        assert_eq!(sr_minimap(-5000, 20000), (0.0, 0.0));
        let (u, v) = sr_minimap(7375, 7430);
        assert!((u - 0.5).abs() < 0.001 && (v - 0.5).abs() < 0.001);
    }
}
