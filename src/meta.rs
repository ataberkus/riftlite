//! Aggregated champion builds from OP.GG's public MCP server
//! (`https://mcp-api.op.gg/mcp`, tool `lol_get_champion_analysis`; documented at
//! github.com/opgginc/opgg-mcp). OP.GG permits use of its data with the source
//! cited and without excessive requests: riftlite sends one query per picked
//! champion, containing only the champion id, mode and lane, and labels
//! everything it writes "OP.GG".
//!
//! The tool answers in a compact text notation rather than JSON:
//!
//! ```text
//! class Data: core_items,boots
//! class CoreItems: ids,play
//!
//! LolGetChampionAnalysis(Data(CoreItems([6655,4645],1518),CoreItems([3020],8855)))
//! ```
//!
//! Every constructor's arguments map to the field list of its `class` line, so
//! [`parse_notation`] turns it into ordinary JSON objects regardless of field order.

use serde::Deserialize;
use serde_json::{Map, Value, json};

const ENDPOINT: &str = "https://mcp-api.op.gg/mcp";
const PROTOCOL: &str = "2025-06-18";
/// Below this many games OP.GG still returns a "build", but it is noise (an off-role pick).
pub const MIN_GAMES: u64 = 100;

const FIELDS: &[&str] = &[
    "data.summary.id",
    "data.starter_items.{ids[],pick_rate,play,win}",
    "data.core_items.{ids[],pick_rate,play,win}",
    "data.boots.{ids[],pick_rate,play,win}",
    "data.fourth_items[].{ids[],pick_rate,play,win}",
    "data.fifth_items[].{ids[],pick_rate,play,win}",
    "data.sixth_items[].{ids[],pick_rate,play,win}",
    "data.skills.{order[],pick_rate,play,win}",
    "data.skill_masteries.{ids[],pick_rate,play,win}",
    "data.runes.{primary_page_id,primary_rune_ids[],secondary_page_id,secondary_rune_ids[],stat_mod_ids[],pick_rate,play,win}",
    "data.summoner_spells.{ids[],pick_rate,play,win}",
];

/// What to ask OP.GG for: its game mode and lane names.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Query {
    pub mode: &'static str,
    pub position: &'static str,
}

impl Query {
    pub fn label(&self) -> String {
        match self.position {
            "none" => self.mode.replace('_', " "),
            p => format!("{} {p}", self.mode),
        }
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
        // OP.GG ignores the lane for ARAM but still requires a valid one.
        "aram" => "mid",
        "urf" | "nexus_blitz" => "none",
        _ => match position {
            "TOP" => "top",
            "JUNGLE" => "jungle",
            "MIDDLE" => "mid",
            "BOTTOM" => "adc",
            "UTILITY" => "support",
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
    pub pick_rate: f64,
}

impl Choice {
    pub fn win_rate(&self) -> f64 {
        if self.games == 0 { 0.0 } else { self.wins as f64 / self.games as f64 }
    }

    /// "52% WR · 1,518 games"
    pub fn stats(&self) -> String {
        format!("{:.0}% WR · {} games", self.win_rate() * 100.0, thousands(self.games))
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
    starter_items: Option<Choice>,
    core_items: Option<Choice>,
    boots: Option<Choice>,
    fourth_items: Vec<Choice>,
    fifth_items: Vec<Choice>,
    sixth_items: Vec<Choice>,
    skills: Option<Skills>,
    skill_masteries: Option<Mastery>,
    runes: Option<Runes>,
    summoner_spells: Option<Choice>,
}

/// A champion's most played build on OP.GG for one mode and lane.
#[derive(Clone, PartialEq, Debug)]
pub struct MetaBuild {
    pub champion_id: i64,
    pub query: Query,
    pub starter: Option<Choice>,
    pub core: Choice,
    pub boots: Option<Choice>,
    /// Most played 4th, 5th and 6th item options.
    pub fourth: Vec<Choice>,
    pub fifth: Vec<Choice>,
    pub sixth: Vec<Choice>,
    pub skills: Option<Skills>,
    /// Max order, e.g. `["Q","E","W"]`.
    pub skill_priority: Vec<String>,
    pub runes: Option<Runes>,
    pub spells: Option<Choice>,
}

impl MetaBuild {
    /// Core items, boots, then the most played 4th/5th/6th items: one six-item build.
    pub fn full_build(&self) -> Vec<i64> {
        let mut out: Vec<i64> = Vec::new();
        let add = |id: i64, out: &mut Vec<i64>| {
            if out.len() < 6 && !out.contains(&id) {
                out.push(id);
            }
        };
        for &id in &self.core.ids {
            add(id, &mut out);
        }
        for id in self.boots.iter().flat_map(|b| b.ids.iter().copied()) {
            add(id, &mut out);
        }
        for slot in [&self.fourth, &self.fifth, &self.sixth] {
            // The most played option of each slot that is not already in the build.
            if let Some(id) = slot.iter().flat_map(|c| c.ids.iter().copied()).find(|id| !out.contains(id)) {
                add(id, &mut out);
            }
        }
        out
    }

    /// Every 4th–6th item option in order of appearance, minus what the core and boots already have.
    pub fn situational(&self) -> Vec<i64> {
        let taken: Vec<i64> = self.core.ids.iter().chain(self.boots.iter().flat_map(|b| b.ids.iter())).copied().collect();
        let mut out = Vec::new();
        for id in self.fourth.iter().chain(&self.fifth).chain(&self.sixth).flat_map(|c| c.ids.iter().copied()) {
            if !taken.contains(&id) && !out.contains(&id) {
                out.push(id);
            }
        }
        out
    }
}

/// Fetches and validates the build. Blocking (several seconds); call it off the poll thread.
pub fn fetch(agent: &ureq::Agent, champion_id: i64, query: Query) -> Result<MetaBuild, String> {
    let init = post(
        agent,
        None,
        &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {
            "protocolVersion": PROTOCOL,
            "capabilities": {},
            "clientInfo": {"name": "riftlite", "version": env!("CARGO_PKG_VERSION")}
        }}),
    )?;
    let session = init.session;
    post(agent, session.as_deref(), &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}))?;
    let call = post(
        agent,
        session.as_deref(),
        &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {
            "name": "lol_get_champion_analysis",
            "arguments": {
                "game_mode": query.mode,
                "champion": champion_id.to_string(),
                "position": query.position,
                "desired_output_fields": FIELDS
            }
        }}),
    )?;
    let reply = call.message.ok_or("OP.GG sent no reply")?;
    if let Some(e) = reply.get("error") {
        return Err(format!("OP.GG: {}", e["message"].as_str().unwrap_or("error")));
    }
    let result = &reply["result"];
    let text = result["content"][0]["text"].as_str().ok_or("OP.GG reply has no text content")?;
    if result["isError"].as_bool() == Some(true) {
        return Err(format!("OP.GG: {}", text.chars().take(200).collect::<String>()));
    }
    parse_build(champion_id, query, text)
}

/// Turns the tool's text into a build, rejecting other champions and tiny samples.
pub fn parse_build(champion_id: i64, query: Query, text: &str) -> Result<MetaBuild, String> {
    let v = parse_notation(text)?;
    let raw: RawData = serde_json::from_value(v["data"].clone()).map_err(|e| format!("OP.GG build: {e}"))?;
    if raw.summary.id != champion_id {
        return Err(format!("OP.GG answered for champion {} instead of {champion_id}", raw.summary.id));
    }
    let core = raw.core_items.filter(|c| !c.ids.is_empty()).ok_or("OP.GG has no build for this champion and mode")?;
    if core.games < MIN_GAMES {
        let n = core.games;
        return Err(format!("OP.GG has only {n} game{} of this build ({}); too few to trust", if n == 1 { "" } else { "s" }, query.label()));
    }
    Ok(MetaBuild {
        champion_id,
        query,
        starter: raw.starter_items.filter(|c| !c.ids.is_empty()),
        core,
        boots: raw.boots.filter(|c| !c.ids.is_empty()),
        fourth: raw.fourth_items,
        fifth: raw.fifth_items,
        sixth: raw.sixth_items,
        skills: raw.skills.filter(|s| !s.order.is_empty()),
        skill_priority: raw.skill_masteries.map(|m| m.ids).unwrap_or_default(),
        runes: raw.runes,
        spells: raw.summoner_spells.filter(|c| c.ids.len() == 2),
    })
}

struct Reply {
    session: Option<String>,
    /// The JSON-RPC response; `None` for an accepted notification.
    message: Option<Value>,
}

fn post(agent: &ureq::Agent, session: Option<&str>, body: &Value) -> Result<Reply, String> {
    let mut req = agent
        .post(ENDPOINT)
        .set("Content-Type", "application/json")
        .set("Accept", "application/json, text/event-stream");
    if let Some(s) = session {
        req = req.set("Mcp-Session-Id", s).set("MCP-Protocol-Version", PROTOCOL);
    }
    let resp = match req.send_json(body.clone()) {
        Ok(r) => r,
        Err(ureq::Error::Status(code, r)) => {
            let detail = r.into_string().unwrap_or_default();
            return Err(format!("OP.GG HTTP {code} {}", detail.chars().take(160).collect::<String>()));
        }
        Err(e) => return Err(format!("OP.GG unreachable: {e}")),
    };
    let session = resp.header("Mcp-Session-Id").map(str::to_string);
    let streamed = resp.content_type() == "text/event-stream";
    let text = resp.into_string().map_err(|e| format!("OP.GG read: {e}"))?;
    let message = if text.trim().is_empty() {
        None
    } else if streamed {
        // Streamable HTTP may answer as server-sent events; the response is the last `data:` message.
        let data = text.lines().filter_map(|l| l.strip_prefix("data:")).last().ok_or("OP.GG stream had no data")?;
        Some(serde_json::from_str(data.trim()).map_err(|e| format!("OP.GG reply: {e}"))?)
    } else {
        Some(serde_json::from_str(&text).map_err(|e| format!("OP.GG reply: {e}"))?)
    };
    Ok(Reply { session, message })
}

/// Parses the `class X: a,b` + `X(…)` notation into JSON. The outermost constructor
/// becomes the returned object.
pub fn parse_notation(text: &str) -> Result<Value, String> {
    let mut classes: std::collections::HashMap<&str, Vec<&str>> = std::collections::HashMap::new();
    let mut body = String::new();
    for line in text.lines() {
        if let Some(decl) = line.strip_prefix("class ") {
            let (name, fields) = decl.split_once(':').ok_or_else(|| format!("bad class line: {line}"))?;
            classes.insert(name.trim(), fields.split(',').map(str::trim).filter(|f| !f.is_empty()).collect());
        } else {
            body.push_str(line);
            body.push('\n');
        }
    }
    let mut p = Parser { s: body.as_bytes(), i: 0, classes: &classes };
    let v = p.value()?;
    p.ws();
    if p.i != p.s.len() {
        return Err(format!("unexpected text at {}", p.i));
    }
    Ok(v)
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
    classes: &'a std::collections::HashMap<&'a str, Vec<&'a str>>,
}

impl Parser<'_> {
    fn ws(&mut self) {
        while self.s.get(self.i).is_some_and(u8::is_ascii_whitespace) {
            self.i += 1;
        }
    }

    fn value(&mut self) -> Result<Value, String> {
        self.ws();
        match self.s.get(self.i).copied() {
            None => Err("unexpected end".into()),
            Some(b'[') => {
                self.i += 1;
                Ok(Value::Array(self.list(b']')?))
            }
            Some(b'"') => {
                let start = self.i;
                self.i += 1;
                while let Some(&b) = self.s.get(self.i) {
                    self.i += 1;
                    match b {
                        b'\\' => self.i += 1,
                        b'"' => {
                            let lit = std::str::from_utf8(&self.s[start..self.i]).map_err(|e| e.to_string())?;
                            return serde_json::from_str(lit).map_err(|e| format!("bad string {lit}: {e}"));
                        }
                        _ => {}
                    }
                }
                Err("unterminated string".into())
            }
            Some(b) if b == b'-' || b.is_ascii_digit() => {
                let start = self.i;
                while self.s.get(self.i).is_some_and(|b| b.is_ascii_digit() || b"+-.eE".contains(b)) {
                    self.i += 1;
                }
                let lit = std::str::from_utf8(&self.s[start..self.i]).expect("ascii");
                serde_json::from_str(lit).map_err(|e| format!("bad number {lit}: {e}"))
            }
            Some(b) if b.is_ascii_alphabetic() || b == b'_' => {
                let start = self.i;
                while self.s.get(self.i).is_some_and(|b| b.is_ascii_alphanumeric() || *b == b'_') {
                    self.i += 1;
                }
                let name = std::str::from_utf8(&self.s[start..self.i]).expect("ascii");
                self.ws();
                if self.s.get(self.i) != Some(&b'(') {
                    return match name {
                        "true" => Ok(Value::Bool(true)),
                        "false" => Ok(Value::Bool(false)),
                        "null" | "None" => Ok(Value::Null),
                        _ => Err(format!("unknown word {name}")),
                    };
                }
                self.i += 1;
                let args = self.list(b')')?;
                let fields = self.classes.get(name).ok_or_else(|| format!("undeclared class {name}"))?;
                if fields.len() != args.len() {
                    return Err(format!("{name} has {} fields but {} values", fields.len(), args.len()));
                }
                Ok(Value::Object(fields.iter().map(|f| f.to_string()).zip(args).collect::<Map<_, _>>()))
            }
            Some(b) => Err(format!("unexpected '{}' at {}", b as char, self.i)),
        }
    }

    fn list(&mut self, close: u8) -> Result<Vec<Value>, String> {
        let mut out = Vec::new();
        self.ws();
        if self.s.get(self.i) == Some(&close) {
            self.i += 1;
            return Ok(out);
        }
        loop {
            out.push(self.value()?);
            self.ws();
            match self.s.get(self.i).copied() {
                Some(b',') => self.i += 1,
                Some(b) if b == close => {
                    self.i += 1;
                    return Ok(out);
                }
                _ => return Err(format!("expected ',' or '{}' at {}", close as char, self.i)),
            }
        }
    }
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

    /// Trimmed from a real `lol_get_champion_analysis` answer (Ziggs, ranked mid).
    const ZIGGS: &str = "class LolGetChampionAnalysis: data\n\
class Data: summary,core_items,boots,starter_items,fourth_items,summoner_spells,runes,skills,skill_masteries,counters_meta\n\
class Summary: id\n\
class CoreItems: ids,ids_names,play,win,pick_rate\n\
class SummonerSpells: ids,win,play,pick_rate\n\
class Runes: primary_page_id,primary_rune_ids,secondary_page_id,secondary_rune_ids,stat_mod_ids,play,win,pick_rate\n\
class Skills: order,play,win,pick_rate\n\
class CountersMeta: message\n\
\n\
LolGetChampionAnalysis(Data(Summary(115),CoreItems([6655,4645,3089],[\"Luden's Echo\",\"Shadowflame\",\"Rabadon's Deathcap\"],1518,844,0.22),\
CoreItems([3020],[\"Sorcerer's Shoes\"],8855,4481,0.88),CoreItems([1056,2003,2003],[\"Doran's Ring\",\"Health Potion\",\"Health Potion\"],9486,4730,0.92),\
[CoreItems([3089],[\"Rabadon's Deathcap\"],903,470,0.27),CoreItems([3157],[\"Zhonya's Hourglass\"],675,383,0.2)],SummonerSpells([4,12],4018,8121,0.78),\
Runes(8200,[8229,8226,8210,8237],8000,[8009,8014],[5008,5008,5001],3440,1687,0.32),Skills([\"Q\",\"E\",\"W\",\"Q\"],3086,1682,0.42),\
SummonerSpells([\"Q\",\"E\",\"W\"],3814,7009,0.96),CountersMeta(\"Insufficient matchup sample.\")))";

    const MID: Query = Query { mode: "ranked", position: "mid" };

    #[test]
    fn notation_maps_fields_by_declared_class_not_position() {
        let b = parse_build(115, MID, ZIGGS).expect("parses");
        assert_eq!(b.core.ids, [6655, 4645, 3089]);
        assert_eq!((b.core.games, b.core.wins), (1518, 844));
        // `SummonerSpells` declares win before play, unlike `CoreItems`.
        assert_eq!(b.spells.as_ref().map(|s| (s.ids.clone(), s.games, s.wins)), Some((vec![4, 12], 8121, 4018)));
        assert_eq!(b.skill_priority, ["Q", "E", "W"]);
        assert_eq!(b.runes.as_ref().and_then(Runes::perks), Some(vec![8229, 8226, 8210, 8237, 8009, 8014, 5008, 5008, 5001]));
        assert_eq!(b.starter.as_ref().map(|s| s.ids.clone()), Some(vec![1056, 2003, 2003]));
        // Rabadon's is already core, so the full build takes the next 4th-item option.
        assert_eq!(b.full_build(), [6655, 4645, 3089, 3020, 3157]);
        assert_eq!(b.situational(), [3157]);
    }

    #[test]
    fn builds_for_another_champion_or_tiny_samples_are_rejected() {
        assert!(parse_build(62, MID, ZIGGS).unwrap_err().contains("instead of 62"));
        let sona = "class LolGetChampionAnalysis: data\nclass Data: summary,core_items\nclass Summary: id\nclass CoreItems: ids,play\n\n\
LolGetChampionAnalysis(Data(Summary(37),CoreItems([3070,3100,3040],1)))";
        assert!(parse_build(37, Query { mode: "ranked", position: "jungle" }, sona).unwrap_err().contains("only 1 game "));
        let empty = "class LolGetChampionAnalysis: data\nclass Data: summary\nclass Summary: id\n\nLolGetChampionAnalysis(Data(Summary(37)))";
        assert!(parse_build(37, MID, empty).is_err());
        assert!(parse_notation("class A: x,y\n\nA(1)").is_err(), "arity mismatch is an error, not a shifted mapping");
    }

    #[test]
    fn queues_map_to_opgg_modes_and_lanes() {
        assert_eq!(query_for(420, 11, "MIDDLE"), Ok(Query { mode: "ranked", position: "mid" }));
        assert_eq!(query_for(440, 11, "UTILITY"), Ok(Query { mode: "flex", position: "support" }));
        assert_eq!(query_for(2400, 12, "NONE"), Ok(Query { mode: "aram", position: "mid" }));
        assert_eq!(query_for(1900, 11, "TOP"), Ok(Query { mode: "urf", position: "none" }));
        assert!(query_for(430, 11, "NONE").is_err(), "blind pick without a lane");
        assert!(query_for(1700, 30, "NONE").is_err(), "Arena");
    }
}
