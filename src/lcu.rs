//! League Client (LCU) API: credential discovery and the endpoints riftlite uses.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

use crate::net;
use crate::win;

#[derive(Clone, PartialEq, Debug)]
pub struct Creds {
    pub pid: u32,
    pub port: u16,
    pub password: String,
    pub client_exe: Option<PathBuf>,
    pub source: &'static str,
}

/// Value of `--name=value` in a Windows command line; tolerates quoting of the
/// whole argument (`"--app-port=123"`) or of the value (`--name="x y"`).
pub fn cmdline_arg<'a>(cmd: &'a str, name: &str) -> Option<&'a str> {
    let key = format!("--{name}=");
    let start = cmd.find(&key)? + key.len();
    let rest = &cmd[start..];
    let value = if let Some(q) = rest.strip_prefix('"') {
        &q[..q.find('"').unwrap_or(q.len())]
    } else {
        &rest[..rest.find(|c: char| c == '"' || c.is_whitespace()).unwrap_or(rest.len())]
    };
    (!value.is_empty()).then_some(value)
}

/// `lockfile` format: `LeagueClient:<pid>:<port>:<password>:<protocol>`.
pub fn parse_lockfile(text: &str) -> Option<(u16, String)> {
    let mut parts = text.trim().split(':');
    let _name = parts.next()?;
    let _pid = parts.next()?;
    let port = parts.next()?.parse().ok()?;
    let password = parts.next()?.to_string();
    Some((port, password))
}

fn lockfile_near(start: &Path, fallback_root: Option<&Path>) -> Option<(u16, String)> {
    start
        .ancestors()
        .take(4)
        .chain(fallback_root)
        .find_map(|dir| parse_lockfile(&std::fs::read_to_string(dir.join("lockfile")).ok()?))
}

/// Same discovery order as the original client integration: the
/// `LeagueClientUx.exe` command line first, then the `lockfile` next to the
/// client (walking up from its directory), then the install directory.
pub fn discover(pid: u32, league_install: Option<&Path>) -> Result<Creds, String> {
    let client_exe = win::image_path(pid);
    if let Some(cmd) = win::command_line(pid) {
        let port = cmdline_arg(&cmd, "app-port").and_then(|p| p.parse().ok());
        let token = cmdline_arg(&cmd, "remoting-auth-token");
        if let (Some(port), Some(token)) = (port, token) {
            return Ok(Creds { pid, port, password: token.to_string(), client_exe, source: "command line" });
        }
    }
    let dir = client_exe.as_deref().and_then(Path::parent).or(league_install);
    let (port, password) = dir
        .and_then(|d| lockfile_near(d, league_install))
        .ok_or("LeagueClientUx is running but neither its command line nor a lockfile exposed credentials")?;
    Ok(Creds { pid, port, password, client_exe, source: "lockfile" })
}

pub struct Lcu {
    agent: ureq::Agent,
    base: String,
    auth: String,
}

#[derive(Debug)]
pub enum LcuError {
    /// HTTP status from the client (e.g. 404 when there is no champ select).
    Status(u16),
    /// A write the client rejected, with its `message` (e.g. invalid rune page).
    Refused(u16, String),
    Transport(String),
}

impl std::fmt::Display for LcuError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LcuError::Status(s) => write!(f, "HTTP {s}"),
            LcuError::Refused(s, m) if m.is_empty() => write!(f, "HTTP {s}"),
            LcuError::Refused(s, m) => write!(f, "HTTP {s}: {m}"),
            LcuError::Transport(e) => f.write_str(e),
        }
    }
}

/// Riot's position names as used by the perks recommender.
pub fn perk_position(assigned: &str) -> &'static str {
    match assigned.to_ascii_lowercase().as_str() {
        "top" => "TOP",
        "jungle" => "JUNGLE",
        "middle" | "mid" => "MIDDLE",
        "bottom" | "bot" => "BOTTOM",
        "utility" | "support" => "UTILITY",
        _ => "NONE",
    }
}

const FLASH: i64 = 4;
const PAGE_PREFIX: &str = "riftlite";
/// Match-history endpoints make the client query Riot's servers (always for
/// other players, and for uncached pages of our own), so they outlast the
/// agent's loopback timeout. Connect timeout still comes from the agent.
const MATCH_HISTORY_TIMEOUT: Duration = Duration::from_secs(20);

impl Lcu {
    pub fn new(agent: ureq::Agent, creds: &Creds) -> Self {
        Lcu { agent, base: format!("https://127.0.0.1:{}", creds.port), auth: net::basic_auth("riot", &creds.password) }
    }

    /// One request. Empty bodies (204, or the client's empty 200s) become `Null`.
    fn send(&self, method: &str, path: &str, body: Option<&Value>) -> Result<Value, LcuError> {
        self.send_timed(method, path, body, None)
    }

    /// `send`, with `timeout` overriding the agent's overall timeout for this request only.
    fn send_timed(&self, method: &str, path: &str, body: Option<&Value>, timeout: Option<Duration>) -> Result<Value, LcuError> {
        let mut req = self.agent.request(method, &format!("{}{path}", self.base)).set("Authorization", &self.auth);
        if let Some(t) = timeout {
            req = req.timeout(t);
        }
        let resp = match body {
            Some(b) => req.send_json(b),
            None => req.call(),
        };
        match resp {
            Ok(r) => {
                let text = r.into_string().map_err(|e| LcuError::Transport(e.to_string()))?;
                if text.trim().is_empty() {
                    return Ok(Value::Null);
                }
                serde_json::from_str(&text).map_err(|e| LcuError::Transport(e.to_string()))
            }
            Err(ureq::Error::Status(code, _)) if method == "GET" => Err(LcuError::Status(code)),
            Err(ureq::Error::Status(code, r)) => {
                let msg = r.into_json::<Value>().ok().and_then(|v| v["message"].as_str().map(str::to_string));
                Err(LcuError::Refused(code, msg.unwrap_or_default()))
            }
            Err(e) => Err(LcuError::Transport(e.to_string())),
        }
    }

    pub fn get(&self, path: &str) -> Result<Value, LcuError> {
        self.send("GET", path, None)
    }

    fn get_match_history(&self, path: &str) -> Result<Value, LcuError> {
        self.send_timed("GET", path, None, Some(MATCH_HISTORY_TIMEOUT))
    }

    pub fn phase(&self) -> Result<String, LcuError> {
        Ok(self.get("/lol-gameflow/v1/gameflow-phase")?.as_str().unwrap_or("Unknown").to_string())
    }

    /// Gameflow session: queue, map and, once a game is starting, every player's puuid.
    pub fn session(&self) -> Option<GameSession> {
        let s = self.get("/lol-gameflow/v1/session").ok()?;
        let gd = &s["gameData"];
        let q = &gd["queue"];
        let mut players = Vec::new();
        for (key, team) in [("teamOne", 100), ("teamTwo", 200)] {
            for p in gd[key].as_array().into_iter().flatten() {
                let puuid = p["puuid"].as_str().unwrap_or("");
                if !puuid.is_empty() {
                    players.push(SessionPlayer { puuid: puuid.to_string(), champion_id: p["championId"].as_i64().unwrap_or(0), team });
                }
            }
        }
        Some(GameSession {
            description: q["description"].as_str().filter(|d| !d.is_empty()).or(q["gameMode"].as_str()).map(str::to_string),
            queue_id: q["id"].as_i64().unwrap_or(0),
            map_id: s["map"]["id"].as_i64().or(q["mapId"].as_i64()).unwrap_or(0),
            game_id: gd["gameId"].as_u64().unwrap_or(0),
            players,
        })
    }

    pub fn summoner(&self) -> Result<Summoner, LcuError> {
        let v = self.get("/lol-summoner/v1/current-summoner")?;
        Ok(Summoner {
            game_name: v["gameName"].as_str().or(v["displayName"].as_str()).unwrap_or("").to_string(),
            tag_line: v["tagLine"].as_str().unwrap_or("").to_string(),
            level: v["summonerLevel"].as_u64().unwrap_or(0),
            puuid: v["puuid"].as_str().unwrap_or("").to_string(),
            profile_icon: v["profileIconId"].as_i64().unwrap_or(0),
        })
    }

    pub fn ranked(&self) -> Result<Vec<RankedQueue>, LcuError> {
        Ok(parse_ranked(&self.get("/lol-ranked/v1/current-ranked-stats")?))
    }

    /// Any player's ranked queues (champ select / loading-screen scouting, lookup).
    pub fn ranked_of(&self, puuid: &str) -> Result<Vec<RankedQueue>, LcuError> {
        Ok(parse_ranked(&self.get(&format!("/lol-ranked/v1/ranked-stats/{puuid}"))?))
    }

    /// Games `beg..=end` (newest first) of the local player.
    pub fn lol_history(&self, puuid: &str, beg: usize, end: usize) -> Result<Vec<LolGame>, LcuError> {
        let v = self.get_match_history(&format!("/lol-match-history/v1/products/lol/current-summoner/matches?begIndex={beg}&endIndex={end}"))?;
        Ok(parse_lol_history(&v, puuid))
    }

    /// Games `beg..=end` of any player.
    pub fn lol_history_of(&self, puuid: &str, beg: usize, end: usize) -> Result<Vec<LolGame>, LcuError> {
        let v = self.get_match_history(&format!("/lol-match-history/v1/products/lol/{puuid}/matches?begIndex={beg}&endIndex={end}"))?;
        Ok(parse_lol_history(&v, puuid))
    }

    pub fn tft_history(&self, puuid: &str) -> Result<Vec<TftGame>, LcuError> {
        let v = self.get_match_history(&format!("/lol-match-history/v1/products/tft/{puuid}/matches?begin=0&count=20"))?;
        Ok(parse_tft_history(&v, puuid))
    }

    /// All ten players of one finished game.
    pub fn match_detail(&self, game_id: u64) -> Result<MatchDetail, LcuError> {
        parse_match_detail(&self.get_match_history(&format!("/lol-match-history/v1/games/{game_id}"))?)
            .ok_or_else(|| LcuError::Transport("match has no participants".into()))
    }

    /// `Ok(None)` when not in champ select (the client answers 404).
    pub fn champ_select(&self) -> Result<Option<ChampSelect>, LcuError> {
        let mut cs = match self.get("/lol-champ-select/v1/session") {
            Ok(v) => parse_champ_select(&v),
            Err(LcuError::Status(404)) => return Ok(None),
            Err(e) => return Err(e),
        };
        if cs.picking_card() {
            // Card-select ARAM (Mayhem…): the offered cards are not in the session. The
            // client answers 404 once the cards are gone.
            cs.card_choices =
                self.get("/lol-lobby-team-builder/champ-select/v1/subset-champion-list").map(|v| parse_champion_ids(&v)).unwrap_or_default();
        }
        Ok(Some(cs))
    }

    /// Locks `champion_id` into the local player's pick action, as clicking a champion card does.
    pub fn pick_champion(&self, action_id: i64, champion_id: i64) -> Result<(), LcuError> {
        let body = serde_json::json!({"championId": champion_id, "completed": true});
        self.send("PATCH", &format!("/lol-champ-select/v1/session/actions/{action_id}"), Some(&body)).map(drop)
    }

    /// Champions the local player may pick right now (owned, not banned).
    pub fn pickable_champions(&self) -> Vec<i64> {
        self.get("/lol-champ-select/v1/pickable-champion-ids").map(|v| parse_champion_ids(&v)).unwrap_or_default()
    }

    /// Riot's own rune + summoner spell recommendations (the client's "recommended" pages).
    pub fn recommended_pages(&self, champion_id: i64, position: &str, map_id: i64) -> Result<Vec<RecPage>, LcuError> {
        let v = self.get(&format!("/lol-perks/v1/recommended-pages/champion/{champion_id}/position/{position}/map/{map_id}"))?;
        Ok(v.as_array().into_iter().flatten().filter_map(parse_rec_page).collect())
    }

    /// Positions Riot recommends for a champion, best first.
    pub fn recommended_positions(&self, champion_id: i64) -> Vec<String> {
        self.get("/lol-perks/v1/recommended-champion-positions")
            .ok()
            .and_then(|v| {
                v[champion_id.to_string()]["recommendedPositions"]
                    .as_array()
                    .map(|a| a.iter().filter_map(|p| p.as_str().map(str::to_string)).collect())
            })
            .unwrap_or_default()
    }

    /// Writes `rec` into riftlite's own rune page (created once, then reused)
    /// and makes it current. Never overwrites the player's other pages.
    pub fn apply_runes(&self, rec: &RecPage, champion: &str) -> Result<String, LcuError> {
        let mut name = format!("{PAGE_PREFIX}: {champion}");
        name.truncate(name.char_indices().nth(25).map_or(name.len(), |(i, _)| i));
        let body = serde_json::json!({
            "name": name,
            "primaryStyleId": rec.primary_style,
            "subStyleId": rec.sub_style,
            "selectedPerkIds": rec.perks,
            "current": true,
        });
        let pages = self.get("/lol-perks/v1/pages")?;
        let ours = pages.as_array().into_iter().flatten().find(|p| {
            p["isEditable"] == true && p["name"].as_str().is_some_and(|n| n.starts_with(PAGE_PREFIX))
        });
        let id = match ours.and_then(|p| p["id"].as_i64()) {
            Some(id) => {
                // Replacing in place keeps the slot; if the client rejects PUT, re-create the same page.
                match self.send("PUT", &format!("/lol-perks/v1/pages/{id}"), Some(&body)) {
                    Ok(_) => id,
                    Err(_) => {
                        self.send("DELETE", &format!("/lol-perks/v1/pages/{id}"), None)?;
                        self.send("POST", "/lol-perks/v1/pages", Some(&body))?["id"].as_i64().unwrap_or(0)
                    }
                }
            }
            None => {
                let inv = self.get("/lol-perks/v1/inventory")?;
                if inv["canAddCustomPage"] != true {
                    return Err(LcuError::Refused(
                        409,
                        "no free rune page; delete one in the client (riftlite reuses one page named \"riftlite: …\" and never overwrites yours)".into(),
                    ));
                }
                self.send("POST", "/lol-perks/v1/pages", Some(&body))?["id"].as_i64().unwrap_or(0)
            }
        };
        if id > 0 {
            self.send("PUT", "/lol-perks/v1/currentpage", Some(&Value::from(id)))?;
        }
        Ok(name)
    }

    /// Sets summoner spells, keeping Flash on whichever key the player has it on now.
    pub fn apply_spells(&self, spells: [i64; 2], current: [i64; 2]) -> Result<[i64; 2], LcuError> {
        let chosen = order_spells(spells, current);
        self.send(
            "PATCH",
            "/lol-champ-select/v1/session/my-selection",
            Some(&serde_json::json!({"spell1Id": chosen[0], "spell2Id": chosen[1]})),
        )?;
        Ok(chosen)
    }

    /// ARAM: take a champion from the shared bench.
    pub fn bench_swap(&self, champion_id: i64) -> Result<(), LcuError> {
        self.send("POST", &format!("/lol-champ-select/v1/session/bench/swap/{champion_id}"), None).map(drop)
    }

    /// Swiftplay pre-lobby slots (champion + position per slot).
    pub fn player_slots(&self) -> Result<Vec<Value>, LcuError> {
        Ok(self.get("/lol-lobby/v1/lobby/members/localMember/player-slots")?.as_array().cloned().unwrap_or_default())
    }

    pub fn set_player_slots(&self, slots: &[Value]) -> Result<(), LcuError> {
        self.send("PUT", "/lol-lobby/v1/lobby/members/localMember/player-slots", Some(&Value::from(slots.to_vec()))).map(drop)
    }

    pub fn lookup(&self, game_name: &str, tag_line: &str) -> Result<String, LcuError> {
        let v = self.get(&format!("/lol-summoner/v1/alias/lookup?gameName={}&tagLine={}", url_encode(game_name), url_encode(tag_line)))?;
        v["puuid"].as_str().filter(|p| !p.is_empty()).map(str::to_string).ok_or(LcuError::Status(404))
    }

    pub fn summoner_by_puuid(&self, puuid: &str) -> Result<Summoner, LcuError> {
        let v = self.get(&format!("/lol-summoner/v2/summoners/puuid/{puuid}"))?;
        Ok(Summoner {
            game_name: v["gameName"].as_str().unwrap_or("").to_string(),
            tag_line: v["tagLine"].as_str().unwrap_or("").to_string(),
            level: v["summonerLevel"].as_u64().unwrap_or(0),
            puuid: puuid.to_string(),
            profile_icon: v["profileIconId"].as_i64().unwrap_or(0),
        })
    }

    /// Mastery per champion, highest points first.
    pub fn mastery(&self, puuid: &str) -> Result<Vec<Mastery>, LcuError> {
        let v = self.get(&format!("/lol-champion-mastery/v1/{puuid}/champion-mastery"))?;
        let mut m: Vec<Mastery> = v
            .as_array()
            .into_iter()
            .flatten()
            .map(|c| Mastery {
                champion_id: c["championId"].as_i64().unwrap_or(0),
                level: c["championLevel"].as_i64().unwrap_or(0),
                points: c["championPoints"].as_i64().unwrap_or(0),
                last_play_ms: c["lastPlayTime"].as_i64().unwrap_or(0),
            })
            .collect();
        m.sort_by(|a, b| b.points.cmp(&a.points));
        Ok(m)
    }

    pub fn challenges(&self) -> Result<Challenges, LcuError> {
        let v = self.get("/lol-challenges/v1/summary-player-data/local-player")?;
        Ok(Challenges {
            level: v["overallChallengeLevel"].as_str().unwrap_or("").to_string(),
            score: v["totalChallengeScore"].as_i64().unwrap_or(0),
            to_next: v["pointsUntilNextRank"].as_i64().unwrap_or(0),
            categories: v["categoryProgress"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|c| ChallengeCategory {
                    name: c["category"].as_str().unwrap_or("").to_string(),
                    level: c["level"].as_str().unwrap_or("").to_string(),
                    current: c["current"].as_i64().unwrap_or(0),
                    max: c["max"].as_i64().unwrap_or(0),
                })
                .collect(),
        })
    }

    /// Open League missions with objective progress.
    pub fn missions(&self, now_ms: i64) -> Result<Vec<Mission>, LcuError> {
        Ok(parse_missions(&self.get("/lol-missions/v1/missions")?, now_ms))
    }

    /// End-of-game stats of the game that just ended (404 outside EndOfGame).
    pub fn end_of_game(&self) -> Result<Option<MatchDetail>, LcuError> {
        match self.get("/lol-end-of-game/v1/eog-stats-block") {
            Ok(v) => Ok(parse_eog(&v)),
            Err(LcuError::Status(404)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// LP change of the last ranked game; the client answers with an empty body when there is none.
    pub fn lp_change(&self) -> Option<LpChange> {
        let v = self.get("/lol-ranked/v1/current-lp-change-notification").ok()?;
        let queue = v["queueType"].as_str().filter(|q| !q.is_empty())?;
        Some(LpChange {
            queue: queue.to_string(),
            tier: v["tier"].as_str().unwrap_or("").to_string(),
            division: v["division"].as_str().unwrap_or("").to_string(),
            lp: v["leaguePoints"].as_i64().unwrap_or(0),
            delta: v["leaguePointsDelta"].as_i64().unwrap_or(0),
            game_id: v["gameId"].as_u64().unwrap_or(0),
        })
    }

    /// Spectate a player's live game through the client (it starts the spectator itself).
    pub fn spectate(&self, puuid: &str) -> Result<(), LcuError> {
        let body = serde_json::json!({
            "allowObserveMode": "ALL",
            "dropInSpectateGameId": "",
            "gameQueueType": "",
            "puuid": puuid,
            "spectatorKey": "",
        });
        self.send("POST", "/lol-spectator/v1/spectate/launch", Some(&body)).map(drop)
    }

    /// The open ready check; `Ok(None)` when there is none (the client answers 404).
    pub fn ready_check(&self) -> Result<Option<ReadyCheck>, LcuError> {
        match self.get("/lol-matchmaking/v1/ready-check") {
            Ok(v) => Ok(Some(ReadyCheck {
                state: v["state"].as_str().unwrap_or("").to_string(),
                player_response: v["playerResponse"].as_str().unwrap_or("").to_string(),
            })),
            Err(LcuError::Status(404)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    pub fn accept_ready_check(&self) -> Result<(), LcuError> {
        self.send("POST", "/lol-matchmaking/v1/ready-check/accept", None).map(drop)
    }
}

/// `/lol-matchmaking/v1/ready-check`: `state` is `InProgress`, `EveryoneReady`,
/// `StrangerNotReady`, `PartyNotReady`, `Invalid` or `Error`; `playerResponse` is
/// `None`, `Accepted` or `Declined`.
#[derive(Clone, PartialEq, Debug)]
pub struct ReadyCheck {
    pub state: String,
    pub player_response: String,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum ReadyAction {
    /// In progress and the player has not answered.
    Accept,
    /// The player accepted or declined this check already; never override that.
    Answered,
    /// Nothing to answer yet (or the check is already resolving).
    Wait,
}

pub fn ready_check_action(rc: &ReadyCheck) -> ReadyAction {
    match (rc.state.as_str(), rc.player_response.as_str()) {
        (_, "Accepted" | "Declined") => ReadyAction::Answered,
        ("InProgress", "None") => ReadyAction::Accept,
        _ => ReadyAction::Wait,
    }
}

fn url_encode(s: &str) -> String {
    s.bytes()
        .map(|b| if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) { (b as char).to_string() } else { format!("%{b:02X}") })
        .collect()
}

/// Puts Flash (if recommended) on the key the player already uses for it.
pub fn order_spells(rec: [i64; 2], current: [i64; 2]) -> [i64; 2] {
    let flash_slot = current.iter().position(|&s| s == FLASH);
    match (flash_slot, rec.iter().position(|&s| s == FLASH)) {
        (Some(want), Some(have)) if want != have => [rec[1], rec[0]],
        _ => rec,
    }
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Summoner {
    pub game_name: String,
    pub tag_line: String,
    pub level: u64,
    pub puuid: String,
    /// Profile icon id (Data Dragon `img/profileicon/<id>.png`); 0 when unknown.
    pub profile_icon: i64,
}

#[derive(Clone, PartialEq, Debug)]
pub struct RankedQueue {
    pub queue: String,
    pub tier: String,
    pub division: String,
    pub lp: i64,
    pub wins: i64,
    pub losses: i64,
}

impl RankedQueue {
    pub fn label(&self) -> &str {
        match self.queue.as_str() {
            "RANKED_SOLO_5x5" => "Ranked Solo/Duo",
            "RANKED_FLEX_SR" => "Ranked Flex",
            "RANKED_TFT" => "TFT Ranked",
            "RANKED_TFT_DOUBLE_UP" => "TFT Double Up",
            "RANKED_TFT_TURBO" => "TFT Hyper Roll",
            "CHERRY" => "Arena",
            q => q,
        }
    }

    pub fn is_tft(&self) -> bool {
        self.queue.contains("TFT")
    }

    pub fn is_ranked(&self) -> bool {
        !matches!(self.tier.as_str(), "" | "NONE" | "UNRANKED")
    }

    /// Hides internal/legacy queues (e.g. `JADE_RANKED_SOLO_5x5`) unless the player has a rank in them.
    pub fn worth_showing(&self) -> bool {
        self.label() != self.queue || self.is_ranked()
    }

    pub fn rank(&self) -> String {
        match self.tier.as_str() {
            _ if !self.is_ranked() => "Unranked".into(),
            t if self.division.is_empty() || self.division == "NA" => format!("{} · {} LP", title_case(t), self.lp),
            t => format!("{} {} · {} LP", title_case(t), self.division, self.lp),
        }
    }
}

fn title_case(s: &str) -> String {
    let lower = s.to_lowercase();
    let mut c = lower.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

fn parse_ranked(v: &Value) -> Vec<RankedQueue> {
    let str_of = |q: &Value, k: &str| q[k].as_str().unwrap_or("").to_string();
    v["queues"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|q| {
            // Hyper Roll reports a rated tier instead of a division ladder.
            let rated = q["ratedTier"].as_str().filter(|t| !t.is_empty() && *t != "NONE");
            RankedQueue {
                queue: str_of(q, "queueType"),
                tier: rated.map(str::to_string).unwrap_or_else(|| str_of(q, "tier")),
                division: if rated.is_some() { String::new() } else { str_of(q, "division") },
                lp: if rated.is_some() { q["ratedRating"].as_i64() } else { q["leaguePoints"].as_i64() }.unwrap_or(0),
                wins: q["wins"].as_i64().unwrap_or(0),
                losses: q["losses"].as_i64().unwrap_or(0),
            }
        })
        .filter(|q| !q.queue.is_empty())
        .collect()
}

#[derive(Clone, PartialEq, Debug)]
pub struct LolGame {
    pub game_id: u64,
    pub created_ms: i64,
    pub duration_s: i64,
    pub queue_id: i64,
    pub champion_id: i64,
    pub win: bool,
    pub kills: i64,
    pub deaths: i64,
    pub assists: i64,
    pub cs: i64,
    pub gold: i64,
    pub damage: i64,
    pub damage_taken: i64,
    pub vision: i64,
    /// item0..item5 then trinket (item6); 0 = empty slot.
    pub items: [i64; 7],
}

impl LolGame {
    pub fn kda(&self) -> f64 {
        (self.kills + self.assists) as f64 / self.deaths.max(1) as f64
    }
}

fn parse_lol_history(v: &Value, puuid: &str) -> Vec<LolGame> {
    v["games"]["games"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|g| {
            let participant_id = g["participantIdentities"]
                .as_array()
                .into_iter()
                .flatten()
                .find(|p| p["player"]["puuid"].as_str() == Some(puuid))
                .and_then(|p| p["participantId"].as_i64());
            let parts = g["participants"].as_array()?;
            let me = participant_id
                .and_then(|id| parts.iter().find(|p| p["participantId"].as_i64() == Some(id)))
                .or_else(|| parts.first())?;
            let s = &me["stats"];
            let n = |k: &str| s[k].as_i64().unwrap_or(0);
            Some(LolGame {
                game_id: g["gameId"].as_u64().unwrap_or(0),
                created_ms: g["gameCreation"].as_i64().unwrap_or(0),
                duration_s: g["gameDuration"].as_i64().unwrap_or(0),
                queue_id: g["queueId"].as_i64().unwrap_or(0),
                champion_id: me["championId"].as_i64().unwrap_or(0),
                win: s["win"].as_bool().unwrap_or(false),
                kills: n("kills"),
                deaths: n("deaths"),
                assists: n("assists"),
                cs: n("totalMinionsKilled") + n("neutralMinionsKilled"),
                gold: n("goldEarned"),
                damage: n("totalDamageDealtToChampions"),
                damage_taken: n("totalDamageTaken"),
                vision: n("visionScore"),
                items: std::array::from_fn(|i| n(&format!("item{i}"))),
            })
        })
        .collect()
}

#[derive(Clone, PartialEq, Debug)]
pub struct TftGame {
    pub datetime_ms: i64,
    pub length_s: i64,
    pub queue_id: i64,
    pub set: i64,
    pub placement: i64,
    pub level: i64,
    /// (character id, star level)
    pub units: Vec<(String, i64)>,
    /// (trait id, units, style); style > 0 means the trait is active.
    pub traits: Vec<(String, i64, i64)>,
}

fn parse_tft_history(v: &Value, puuid: &str) -> Vec<TftGame> {
    v["games"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|g| {
            let j = &g["json"];
            let me = j["participants"].as_array()?.iter().find(|p| p["puuid"].as_str() == Some(puuid))?;
            let mut traits: Vec<(String, i64, i64)> = me["traits"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|t| {
                    let style = t["style"].as_i64().unwrap_or(0);
                    (t["name"].as_str().unwrap_or("").to_string(), t["num_units"].as_i64().unwrap_or(0), style)
                })
                .filter(|t| t.2 > 0)
                .collect();
            traits.sort_by(|a, b| b.2.cmp(&a.2).then(b.1.cmp(&a.1)));
            Some(TftGame {
                datetime_ms: j["game_datetime"].as_i64().unwrap_or(0),
                length_s: j["game_length"].as_f64().unwrap_or(0.0) as i64,
                queue_id: j["queue_id"].as_i64().unwrap_or(0),
                set: j["tft_set_number"].as_i64().unwrap_or(0),
                placement: me["placement"].as_i64().unwrap_or(0),
                level: me["level"].as_i64().unwrap_or(0),
                units: me["units"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|u| (u["character_id"].as_str().unwrap_or("").to_string(), u["tier"].as_i64().unwrap_or(1)))
                    .collect(),
                traits,
            })
        })
        .collect()
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct CsPlayer {
    pub champion_id: i64,
    /// `championId` is set (picked or locked), not just a pick intent.
    pub picked: bool,
    pub position: String,
    pub name: String,
    pub puuid: String,
    pub is_me: bool,
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct ChampSelect {
    pub session_id: String,
    pub queue_id: i64,
    pub phase: String,
    pub my_team: Vec<CsPlayer>,
    pub their_team: Vec<CsPlayer>,
    pub bans: Vec<i64>,
    /// The local player's champion is final (pick locked, finalization, or a no-pick mode like ARAM).
    pub locked: bool,
    pub my_spells: [i64; 2],
    pub bench_enabled: bool,
    pub bench: Vec<i64>,
    /// `allowSubsetChampionPicks`: ARAM card select, where the player picks one of a few offered
    /// champions during `BAN_PICK` (ARAM Mayhem and other ARAM queues with champion cards).
    pub subset_picks: bool,
    /// The offered champion cards (`/lol-lobby-team-builder/champ-select/v1/subset-champion-list`),
    /// fetched only while `picking_card()`.
    pub card_choices: Vec<i64>,
    /// The local player's pick action that is not completed yet; picking a card completes it.
    pub my_pick_action: Option<i64>,
}

impl ChampSelect {
    pub fn me(&self) -> Option<&CsPlayer> {
        self.my_team.iter().find(|p| p.is_me)
    }

    pub fn my_champion(&self) -> Option<i64> {
        self.me().map(|p| p.champion_id).filter(|&c| c > 0)
    }

    /// The client shows its "Pick your champion" cards.
    pub fn picking_card(&self) -> bool {
        self.subset_picks && self.phase == "BAN_PICK" && !self.locked
    }
}

fn parse_champion_ids(v: &Value) -> Vec<i64> {
    v.as_array().into_iter().flatten().filter_map(Value::as_i64).filter(|&c| c > 0).collect()
}

fn parse_champ_select(v: &Value) -> ChampSelect {
    let me = v["localPlayerCellId"].as_i64();
    let team = |key: &str| -> Vec<CsPlayer> {
        v[key]
            .as_array()
            .into_iter()
            .flatten()
            .map(|p| {
                let pick = p["championId"].as_i64().filter(|&c| c > 0);
                CsPlayer {
                    champion_id: pick.or(p["championPickIntent"].as_i64()).unwrap_or(0),
                    picked: pick.is_some(),
                    position: p["assignedPosition"].as_str().unwrap_or("").to_string(),
                    name: p["gameName"].as_str().unwrap_or("").to_string(),
                    puuid: p["puuid"].as_str().unwrap_or("").to_string(),
                    is_me: p["cellId"].as_i64() == me,
                }
            })
            .collect()
    };
    let actions: Vec<&Value> =
        v["actions"].as_array().into_iter().flatten().flat_map(|turn| turn.as_array().into_iter().flatten()).collect();
    let bans = actions
        .iter()
        .filter(|a| a["type"] == "ban" && a["completed"] == true)
        .filter_map(|a| a["championId"].as_i64().filter(|&c| c > 0))
        .collect();
    let my_picks: Vec<&&Value> = actions.iter().filter(|a| a["type"] == "pick" && a["actorCellId"].as_i64() == me).collect();
    let my_team = team("myTeam");
    let phase = v["timer"]["phase"].as_str().unwrap_or("").to_string();
    let subset_picks = v["allowSubsetChampionPicks"].as_bool().unwrap_or(false);
    let mine = my_team.iter().find(|p| p.is_me);
    // Plain ARAM has no pick actions and assigns the champion; card-select ARAM assigns
    // nothing final during BAN_PICK until the player's pick action completes.
    let assigned = my_picks.is_empty() && !(subset_picks && phase == "BAN_PICK");
    let locked = mine.is_some_and(|p| p.picked)
        && (phase == "FINALIZATION" || assigned || my_picks.iter().any(|a| a["completed"] == true));
    let my_pick_action = my_picks.iter().find(|a| a["completed"] != true).and_then(|a| a["id"].as_i64());
    let mine_raw = v["myTeam"].as_array().into_iter().flatten().find(|p| p["cellId"].as_i64() == me);
    let spell = |k: &str| mine_raw.and_then(|p| p[k].as_i64()).unwrap_or(0);
    ChampSelect {
        session_id: v["id"].as_str().unwrap_or("").to_string(),
        queue_id: v["queueId"].as_i64().unwrap_or(0),
        phase,
        locked,
        my_spells: [spell("spell1Id"), spell("spell2Id")],
        bench_enabled: v["benchEnabled"].as_bool().unwrap_or(false),
        bench: v["benchChampions"].as_array().into_iter().flatten().filter_map(|b| b["championId"].as_i64()).collect(),
        subset_picks,
        card_choices: Vec::new(),
        my_pick_action,
        their_team: team("theirTeam"),
        my_team,
        bans,
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct SessionPlayer {
    pub puuid: String,
    pub champion_id: i64,
    /// 100 (blue) or 200 (red).
    pub team: i64,
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct GameSession {
    pub description: Option<String>,
    pub queue_id: i64,
    pub map_id: i64,
    pub game_id: u64,
    pub players: Vec<SessionPlayer>,
}

/// One of Riot's recommended rune pages, with its recommended summoner spells.
#[derive(Clone, PartialEq, Debug)]
pub struct RecPage {
    pub position: String,
    pub primary_style: i64,
    pub sub_style: i64,
    /// Keystone, 3 primary, 2 secondary, 3 stat shards.
    pub perks: Vec<i64>,
    pub spells: [i64; 2],
    /// Riot's localized style label for the page (e.g. "Ability damage"), falling
    /// back to its attribute (`kMana` → `Mana`).
    pub label: String,
}

impl RecPage {
    /// The Swiftplay slot / rune-page JSON string the client stores per slot.
    pub fn perks_string(&self) -> String {
        serde_json::json!({"perkIds": self.perks, "perkStyle": self.primary_style, "perkSubStyle": self.sub_style}).to_string()
    }
}

fn parse_rec_page(p: &Value) -> Option<RecPage> {
    let perks: Vec<i64> = p["perks"].as_array()?.iter().filter_map(|x| x["id"].as_i64()).collect();
    if perks.len() != 9 {
        return None;
    }
    let s: Vec<i64> = p["summonerSpellIds"].as_array().into_iter().flatten().filter_map(Value::as_i64).collect();
    Some(RecPage {
        position: p["position"].as_str().unwrap_or("").to_string(),
        primary_style: p["primaryPerkStyleId"].as_i64()?,
        sub_style: p["secondaryPerkStyleId"].as_i64()?,
        perks,
        spells: [s.first().copied().unwrap_or(0), s.get(1).copied().unwrap_or(0)],
        label: p["keystone"]["recommendationDescriptor"]
            .as_str()
            .filter(|d| !d.is_empty())
            .or_else(|| p["primaryRecommendationAttribute"].as_str().map(|a| a.strip_prefix('k').unwrap_or(a)))
            .unwrap_or("")
            .to_string(),
    })
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct MatchPlayer {
    pub name: String,
    pub puuid: String,
    pub champion_id: i64,
    pub team: i64,
    pub win: bool,
    pub kills: i64,
    pub deaths: i64,
    pub assists: i64,
    pub cs: i64,
    pub gold: i64,
    pub damage: i64,
    pub damage_taken: i64,
    pub vision: i64,
    pub level: i64,
    pub items: [i64; 7],
    pub spells: [i64; 2],
    pub is_me: bool,
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct MatchDetail {
    pub game_id: u64,
    pub created_ms: i64,
    pub duration_s: i64,
    pub queue_id: i64,
    pub mode: String,
    pub players: Vec<MatchPlayer>,
}

impl MatchDetail {
    pub fn me(&self) -> Option<&MatchPlayer> {
        self.players.iter().find(|p| p.is_me)
    }

    pub fn team_sum(&self, team: i64, f: impl Fn(&MatchPlayer) -> i64) -> i64 {
        self.players.iter().filter(|p| p.team == team).map(f).sum()
    }

    /// Marks the player with `puuid` as the viewer.
    pub fn with_me(mut self, puuid: &str) -> Self {
        for p in &mut self.players {
            p.is_me = !puuid.is_empty() && p.puuid == puuid;
        }
        self
    }
}

fn parse_match_detail(g: &Value) -> Option<MatchDetail> {
    let idents = g["participantIdentities"].as_array()?;
    let players: Vec<MatchPlayer> = g["participants"]
        .as_array()?
        .iter()
        .map(|p| {
            let pid = p["participantId"].as_i64();
            let who = idents.iter().find(|i| i["participantId"].as_i64() == pid).map(|i| &i["player"]);
            let s = &p["stats"];
            let n = |k: &str| s[k].as_i64().unwrap_or(0);
            let name = who.map(|w| riot_id(w["gameName"].as_str().unwrap_or(""), w["tagLine"].as_str().unwrap_or(""))).unwrap_or_default();
            MatchPlayer {
                name,
                puuid: who.and_then(|w| w["puuid"].as_str()).unwrap_or("").to_string(),
                champion_id: p["championId"].as_i64().unwrap_or(0),
                team: p["teamId"].as_i64().unwrap_or(0),
                win: s["win"].as_bool().unwrap_or(false),
                kills: n("kills"),
                deaths: n("deaths"),
                assists: n("assists"),
                cs: n("totalMinionsKilled") + n("neutralMinionsKilled"),
                gold: n("goldEarned"),
                damage: n("totalDamageDealtToChampions"),
                damage_taken: n("totalDamageTaken"),
                vision: n("visionScore"),
                level: n("champLevel"),
                items: std::array::from_fn(|i| n(&format!("item{i}"))),
                spells: [p["spell1Id"].as_i64().unwrap_or(0), p["spell2Id"].as_i64().unwrap_or(0)],
                is_me: false,
            }
        })
        .collect();
    if players.is_empty() {
        return None;
    }
    Some(MatchDetail {
        game_id: g["gameId"].as_u64().unwrap_or(0),
        created_ms: g["gameCreation"].as_i64().unwrap_or(0),
        duration_s: g["gameDuration"].as_i64().unwrap_or(0),
        queue_id: g["queueId"].as_i64().unwrap_or(0),
        mode: g["gameMode"].as_str().unwrap_or("").to_string(),
        players,
    })
}

fn riot_id(name: &str, tag: &str) -> String {
    if tag.is_empty() { name.to_string() } else { format!("{name}#{tag}") }
}

/// `/lol-end-of-game/v1/eog-stats-block`: players carry an upper-case stats map.
fn parse_eog(v: &Value) -> Option<MatchDetail> {
    let mut players = Vec::new();
    for t in v["teams"].as_array()? {
        let team = t["teamId"].as_i64().unwrap_or(0);
        let won = t["isWinningTeam"].as_bool().unwrap_or(false);
        for p in t["players"].as_array().into_iter().flatten() {
            let s = &p["stats"];
            let n = |k: &str| s[k].as_i64().unwrap_or(0);
            let items: Vec<i64> = p["items"].as_array().into_iter().flatten().filter_map(Value::as_i64).collect();
            players.push(MatchPlayer {
                name: riot_id(p["riotIdGameName"].as_str().unwrap_or(""), p["riotIdTagLine"].as_str().unwrap_or("")),
                puuid: p["puuid"].as_str().unwrap_or("").to_string(),
                champion_id: p["championId"].as_i64().unwrap_or(0),
                team,
                win: won,
                kills: n("CHAMPIONS_KILLED"),
                deaths: n("NUM_DEATHS"),
                assists: n("ASSISTS"),
                cs: n("MINIONS_KILLED") + n("NEUTRAL_MINIONS_KILLED"),
                gold: n("GOLD_EARNED"),
                damage: n("TOTAL_DAMAGE_DEALT_TO_CHAMPIONS"),
                damage_taken: n("TOTAL_DAMAGE_TAKEN"),
                vision: n("VISION_SCORE"),
                level: n("LEVEL"),
                items: std::array::from_fn(|i| items.get(i).copied().unwrap_or(0)),
                spells: [p["spell1Id"].as_i64().unwrap_or(0), p["spell2Id"].as_i64().unwrap_or(0)],
                is_me: p["isLocalPlayer"].as_bool().unwrap_or(false),
            });
        }
    }
    if players.is_empty() {
        return None;
    }
    Some(MatchDetail {
        game_id: v["gameId"].as_u64().unwrap_or(0),
        created_ms: v["endOfGameTimestamp"].as_i64().unwrap_or(0),
        duration_s: v["gameLength"].as_i64().unwrap_or(0),
        queue_id: 0,
        mode: v["gameMode"].as_str().unwrap_or("").to_string(),
        players,
    })
}

#[derive(Clone, PartialEq, Debug)]
pub struct LpChange {
    pub queue: String,
    pub tier: String,
    pub division: String,
    pub lp: i64,
    pub delta: i64,
    pub game_id: u64,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Mastery {
    pub champion_id: i64,
    pub level: i64,
    pub points: i64,
    pub last_play_ms: i64,
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct ChallengeCategory {
    pub name: String,
    pub level: String,
    pub current: i64,
    pub max: i64,
}

#[derive(Clone, PartialEq, Debug, Default)]
pub struct Challenges {
    pub level: String,
    pub score: i64,
    pub to_next: i64,
    pub categories: Vec<ChallengeCategory>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Mission {
    pub title: String,
    pub objective: String,
    pub current: i64,
    pub total: i64,
    pub end_ms: i64,
}

/// Pending League missions the client shows (not completed, not expired, not post-reward toasts).
fn parse_missions(v: &Value, now_ms: i64) -> Vec<Mission> {
    v.as_array()
        .into_iter()
        .flatten()
        .filter(|m| m["status"] == "PENDING" && m["displayType"] != "AFTER_COMPLETION")
        .filter(|m| {
            m["display"]["locations"].as_array().into_iter().flatten().any(|l| l.as_str().is_some_and(|l| l.starts_with("LOL")))
        })
        .filter(|m| m["endTime"].as_i64().is_none_or(|e| e <= 0 || e > now_ms))
        .flat_map(|m| {
            let title = m["title"].as_str().unwrap_or("").to_string();
            m["objectives"].as_array().into_iter().flatten().map(move |o| Mission {
                title: title.clone(),
                objective: strip_tags(o["description"].as_str().unwrap_or("")),
                current: o["progress"]["currentProgress"].as_i64().unwrap_or(0),
                total: o["progress"]["totalCount"].as_i64().unwrap_or(0),
                end_ms: m["endTime"].as_i64().unwrap_or(0),
            })
        })
        .filter(|m| m.total > 0)
        .collect()
}

/// Removes the client's inline HTML (`<br>`, `<lol-uikit-…>` tooltips).
pub fn strip_tags(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut in_tag = false;
    for c in s.chars() {
        match c {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cmdline_arg_handles_quoting_styles() {
        let cmd = r#""C:/Riot Games/League of Legends/LeagueClientUx.exe" "--remoting-auth-token=AbC_d-1" "--app-port=51234" --install-directory="C:/Riot Games/League of Legends" --x=1"#;
        assert_eq!(cmdline_arg(cmd, "remoting-auth-token"), Some("AbC_d-1"));
        assert_eq!(cmdline_arg(cmd, "app-port"), Some("51234"));
        assert_eq!(cmdline_arg(cmd, "install-directory"), Some("C:/Riot Games/League of Legends"));
        assert_eq!(cmdline_arg(cmd, "x"), Some("1"));
        assert_eq!(cmdline_arg(cmd, "missing"), None);
    }

    #[test]
    fn lockfile_parses_port_and_password() {
        assert_eq!(parse_lockfile("LeagueClient:1234:56789:s3cr3t:https\n"), Some((56789, "s3cr3t".into())));
        assert_eq!(parse_lockfile("garbage"), None);
    }

    #[test]
    fn hyper_roll_uses_rated_tier() {
        let v = serde_json::json!({"queues": [
            {"queueType": "RANKED_TFT_TURBO", "tier": "", "ratedTier": "BLUE", "ratedRating": 1450, "wins": 3, "losses": 0},
            {"queueType": "RANKED_SOLO_5x5", "tier": "GOLD", "division": "II", "leaguePoints": 40, "wins": 10, "losses": 9, "ratedTier": "NONE"}
        ]});
        let q = parse_ranked(&v);
        assert_eq!(q[0].rank(), "Blue · 1450 LP");
        assert_eq!(q[1].rank(), "Gold II · 40 LP");
    }

    #[test]
    fn lock_detection_covers_draft_and_no_pick_modes() {
        let session = |actions: Value, phase: &str, champ: i64| {
            serde_json::json!({
                "id": "s1", "localPlayerCellId": 2, "timer": {"phase": phase}, "actions": actions,
                "myTeam": [{"cellId": 2, "championId": champ, "championPickIntent": 0, "spell1Id": 4, "spell2Id": 14, "puuid": "me"}],
                "benchEnabled": true, "benchChampions": [{"championId": 99}]
            })
        };
        let hovering = serde_json::json!([[{"type": "pick", "actorCellId": 2, "completed": false, "championId": 103}]]);
        let locked = serde_json::json!([[{"type": "pick", "actorCellId": 2, "completed": true, "championId": 103}]]);
        assert!(!parse_champ_select(&session(hovering, "BAN_PICK", 103)).locked, "hover in draft is not a lock");
        assert!(parse_champ_select(&session(locked, "BAN_PICK", 103)).locked);
        // ARAM: no pick actions, the champion is assigned.
        let aram = parse_champ_select(&session(serde_json::json!([]), "BAN_PICK", 115));
        assert!(aram.locked);
        assert_eq!((aram.my_spells, aram.bench.as_slice()), ([4, 14], &[99][..]));
        assert!(!parse_champ_select(&session(serde_json::json!([]), "BAN_PICK", 0)).locked);
    }

    #[test]
    fn aram_card_select_waits_for_the_picked_card() {
        // Shape from the client's card-select UI: `allowSubsetChampionPicks` with an open pick
        // action in BAN_PICK; clicking a card PATCHes that action with `completed: true`.
        let session = |phase: &str, champ: i64, completed: Option<bool>| {
            let actions = completed.map_or(serde_json::json!([]), |c| {
                serde_json::json!([[{"id": 7, "type": "pick", "actorCellId": 2, "completed": c, "championId": champ}]])
            });
            parse_champ_select(&serde_json::json!({
                "id": "s2", "queueId": 2400, "localPlayerCellId": 2, "timer": {"phase": phase}, "actions": actions,
                "allowSubsetChampionPicks": true, "benchEnabled": true, "benchChampions": [],
                "myTeam": [{"cellId": 2, "championId": champ, "championPickIntent": 0, "puuid": "me"}]
            }))
        };
        let choosing = session("BAN_PICK", 0, Some(false));
        assert!(choosing.picking_card() && !choosing.locked);
        assert_eq!(choosing.my_pick_action, Some(7));
        let shown = session("BAN_PICK", 360, Some(false));
        assert!(!shown.locked && shown.picking_card(), "a champion shown before the pick completes is no lock");
        assert!(!session("BAN_PICK", 360, None).locked, "card select without pick actions does not assign yet");
        let picked = session("BAN_PICK", 360, Some(true));
        assert!(picked.locked && !picked.picking_card());
        assert_eq!(picked.my_pick_action, None);
        assert!(session("FINALIZATION", 360, None).locked);
        assert_eq!(parse_champion_ids(&serde_json::json!([360, 17, 0])), vec![360, 17]);
    }

    #[test]
    fn ready_check_accepts_only_unanswered_in_progress_checks() {
        let rc = |state: &str, resp: &str| ReadyCheck { state: state.into(), player_response: resp.into() };
        assert_eq!(ready_check_action(&rc("InProgress", "None")), ReadyAction::Accept);
        assert_eq!(ready_check_action(&rc("InProgress", "Declined")), ReadyAction::Answered, "a decline is final");
        assert_eq!(ready_check_action(&rc("InProgress", "Accepted")), ReadyAction::Answered, "never accept twice");
        assert_eq!(ready_check_action(&rc("StrangerNotReady", "Accepted")), ReadyAction::Answered);
        for state in ["EveryoneReady", "StrangerNotReady", "PartyNotReady", "Invalid", "Error", ""] {
            assert_eq!(ready_check_action(&rc(state, "None")), ReadyAction::Wait, "{state}");
        }
    }

    #[test]
    fn recommended_spells_keep_flash_on_its_key() {
        assert_eq!(order_spells([4, 12], [14, 4]), [12, 4]);
        assert_eq!(order_spells([4, 12], [4, 14]), [4, 12]);
        assert_eq!(order_spells([32, 4], [6, 7]), [32, 4], "no Flash today: keep Riot's order");
    }

    #[test]
    fn end_of_game_block_maps_upper_case_stats() {
        let v = serde_json::json!({"gameId": 7, "gameLength": 923, "gameMode": "KIWI", "teams": [
            {"teamId": 100, "isWinningTeam": true, "players": [{"isLocalPlayer": true, "championId": 115, "riotIdGameName": "A", "riotIdTagLine": "TR1",
                "items": [6655, 0, 0, 0, 0, 0, 2052], "spell1Id": 4, "spell2Id": 7,
                "stats": {"CHAMPIONS_KILLED": 5, "NUM_DEATHS": 6, "ASSISTS": 24, "MINIONS_KILLED": 11, "NEUTRAL_MINIONS_KILLED": 2, "GOLD_EARNED": 12620, "TOTAL_DAMAGE_DEALT_TO_CHAMPIONS": 24543}}]},
            {"teamId": 200, "isWinningTeam": false, "players": [{"isLocalPlayer": false, "championId": 1, "stats": {}}]}
        ]});
        let d = parse_eog(&v).unwrap();
        let me = d.me().unwrap();
        assert_eq!((me.name.as_str(), me.kills, me.deaths, me.assists, me.cs, me.damage), ("A#TR1", 5, 6, 24, 13, 24543));
        assert_eq!(me.items[6], 2052);
        assert!(me.win && !d.players[1].win);
        assert_eq!(d.duration_s, 923);
    }
}
