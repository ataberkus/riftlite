//! The game's Replay API (https://127.0.0.1:2999/replay/*), available while a replay is
//! open and `EnableReplayApi=1` is set in `game.cfg`.
//!
//! UNVERIFIED: Riot documents this API (developer.riotgames.com, "Replay API") and League
//! Director drives `/replay/recording` with the same fields, but riftlite has not been run
//! against it; answers are read defensively and missing fields fall back to defaults.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

const BASE: &str = "https://127.0.0.1:2999/replay/";
/// The `game.cfg` key that turns the API on.
const KEY: &str = "EnableReplayApi";

fn cfg_path(league_root: &Path) -> PathBuf {
    league_root.join("Config").join("game.cfg")
}

/// `[General] EnableReplayApi` of `<League>/Config/game.cfg`; `None` when the file is unreadable.
/// A missing key reads as off (the game's default).
pub fn api_enabled(league_root: &Path) -> Option<bool> {
    std::fs::read_to_string(cfg_path(league_root)).ok().map(|cfg| enabled_in(&cfg))
}

/// Sets `EnableReplayApi=1` under `[General]` keeping every other line and the file's line endings;
/// adds the key (and section) when missing. Takes effect the next time the game starts. Only
/// called from a user button.
pub fn enable_api(league_root: &Path) -> Result<bool /* changed */, String> {
    let path = cfg_path(league_root);
    let old = std::fs::read_to_string(&path).map_err(|e| format!("reading {}: {e}", path.display()))?;
    if enabled_in(&old) {
        return Ok(false);
    }
    let tmp = path.with_extension("cfg.tmp");
    std::fs::write(&tmp, with_replay_api(&old)).and_then(|_| std::fs::rename(&tmp, &path)).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("writing {}: {e}", path.display())
    })?;
    Ok(true)
}

/// A `game.cfg` line without its line ending, surrounding blanks or a leading byte-order mark.
fn clean(line: &str) -> &str {
    line.trim_start_matches('\u{feff}').trim()
}

/// The value when a cleaned line sets `EnableReplayApi` (key case-insensitive, blanks around `=` allowed).
fn key_value(line: &str) -> Option<&str> {
    let (k, v) = line.split_once('=')?;
    k.trim().eq_ignore_ascii_case(KEY).then(|| v.trim())
}

/// Whether `[General]` sets `EnableReplayApi` to a non-zero number; the last setting wins.
fn enabled_in(cfg: &str) -> bool {
    let mut general = false;
    let mut on = false;
    for line in cfg.lines().map(clean) {
        if line.starts_with('[') {
            general = line.eq_ignore_ascii_case("[General]");
        } else if let Some(v) = key_value(line).filter(|_| general) {
            on = v.parse::<i64>().is_ok_and(|n| n != 0);
        }
    }
    on
}

/// `cfg` with every `[General] EnableReplayApi` line set to 1, or the key added right after the
/// first `[General]` header, or a `[General]` section appended. Other lines stay byte for byte;
/// added lines use the file's line ending (CRLF when it has any).
fn with_replay_api(cfg: &str) -> String {
    let eol = if cfg.contains("\r\n") { "\r\n" } else { "\n" };
    let set = format!("{KEY}=1");
    let mut out = String::with_capacity(cfg.len() + set.len() + 16);
    let mut general = false;
    let mut found = false;
    // Offset in `out` just past the first `[General]` header line.
    let mut after_header = None;
    for line in cfg.split_inclusive('\n') {
        let body = line.trim_end_matches(['\r', '\n']);
        let ending = &line[body.len()..];
        let c = clean(body);
        if c.starts_with('[') {
            general = c.eq_ignore_ascii_case("[General]");
            out.push_str(line);
            if general && after_header.is_none() {
                if ending.is_empty() {
                    out.push_str(eol);
                }
                after_header = Some(out.len());
            }
        } else if general && key_value(c).is_some() {
            found = true;
            out.push_str(&set);
            out.push_str(ending);
        } else {
            out.push_str(line);
        }
    }
    if found {
        return out;
    }
    match after_header {
        Some(at) => out.insert_str(at, &format!("{set}{eol}")),
        None => {
            if !out.is_empty() && !out.ends_with('\n') {
                out.push_str(eol);
            }
            out.push_str(&format!("[General]{eol}{set}{eol}"));
        }
    }
    out
}

/// `/replay/playback` state. Times in seconds of game time; `speed` 1.0 = real time.
#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Playback {
    pub time: f64,
    pub length: f64,
    pub paused: bool,
    pub speed: f64,
}

/// `/replay/recording` state. Times in seconds of game time; `path` is the output folder the game writes to.
#[derive(Clone, PartialEq, Debug)]
pub struct Recording {
    pub recording: bool,
    pub start: f64,
    pub end: f64,
    pub current: f64,
    pub path: String,
}

/// Turns a request failure into an explanation of its usual cause.
fn failed(what: &str, e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, resp) => {
            let body = resp.into_string().unwrap_or_default();
            let body: String = body.trim().chars().take(200).collect();
            if body.is_empty() { format!("Replay API /replay/{what}: HTTP {code}") } else { format!("Replay API /replay/{what}: HTTP {code} ({body})") }
        }
        e if e.kind() == ureq::ErrorKind::ConnectionFailed => {
            "Replay API unreachable: no replay open, or EnableReplayApi is off (Settings)".into()
        }
        e => format!("Replay API /replay/{what}: {e}"),
    }
}

fn get(agent: &ureq::Agent, what: &str) -> Result<Value, String> {
    agent
        .get(&format!("{BASE}{what}"))
        .call()
        .map_err(|e| failed(what, e))?
        .into_json()
        .map_err(|e| format!("Replay API /replay/{what}: unreadable answer ({e})"))
}

fn post(agent: &ureq::Agent, what: &str, body: Value) -> Result<(), String> {
    agent.post(&format!("{BASE}{what}")).send_json(body).map_err(|e| failed(what, e))?;
    Ok(())
}

/// Current replay position and length (UNVERIFIED, see the module doc). `agent` is `net::riot_local_agent()`.
pub fn playback(agent: &ureq::Agent) -> Result<Playback, String> {
    let v = get(agent, "playback")?;
    let length = v["length"].as_f64().ok_or("Replay API /replay/playback: answer without a replay length")?;
    Ok(Playback {
        time: v["time"].as_f64().unwrap_or(0.0),
        length,
        paused: v["paused"].as_bool().unwrap_or(false),
        speed: v["speed"].as_f64().unwrap_or(1.0),
    })
}

/// Jumps to `time` (seconds of game time) and resumes playback (UNVERIFIED, see the module doc).
pub fn seek(agent: &ureq::Agent, time: f64) -> Result<(), String> {
    post(agent, "playback", json!({ "time": time, "paused": false }))
}

/// Records `start..end` (seconds of game time) as a WebM video into `dir` (created when missing):
/// seeks to `start`, then asks the game to record at 30 fps until `end` (UNVERIFIED, see the
/// module doc). Refuses while a recording is already running.
pub fn record(agent: &ureq::Agent, start: f64, end: f64, dir: &Path) -> Result<(), String> {
    if !(start >= 0.0 && end > start) {
        return Err(format!("invalid clip {start:.0}–{end:.0} s: needs 0 ≤ start < end"));
    }
    if recording(agent)?.recording {
        return Err("a recording is already running; wait until it finishes".into());
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    seek(agent, start)?;
    post(
        agent,
        "recording",
        json!({
            "recording": true,
            "codec": "webm",
            "startTime": start,
            "endTime": end,
            "framesPerSecond": 30,
            "enforceFrameRate": true,
            "lossless": false,
            "path": dir.to_string_lossy(),
        }),
    )
}

/// The game's recording state (UNVERIFIED, see the module doc).
pub fn recording(agent: &ureq::Agent) -> Result<Recording, String> {
    let v = get(agent, "recording")?;
    Ok(Recording {
        recording: v["recording"].as_bool().unwrap_or(false),
        start: v["startTime"].as_f64().unwrap_or(0.0),
        end: v["endTime"].as_f64().unwrap_or(0.0),
        current: v["currentTime"].as_f64().unwrap_or(0.0),
        path: v["path"].as_str().unwrap_or_default().to_string(),
    })
}

/// Where clips go: `%USERPROFILE%\Videos\riftlite` when `USERPROFILE` is set, else
/// `<local_dir>\clips`. Not created here.
pub fn clips_dir() -> PathBuf {
    match std::env::var_os("USERPROFILE").filter(|h| !h.is_empty()) {
        Some(home) => PathBuf::from(home).join("Videos").join("riftlite"),
        None => crate::settings::local_dir().join("clips"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn existing_zero_is_flipped_in_place_and_everything_else_kept() {
        let cfg = "[General]\nWindowMode=2\nEnableReplayApi=0\nCfgVersion=1\n[HUD]\nEnableReplayApi=0\n";
        let out = with_replay_api(cfg);
        // Only the [General] key changes; the same key in another section is not the game's switch.
        assert_eq!(out, "[General]\nWindowMode=2\nEnableReplayApi=1\nCfgVersion=1\n[HUD]\nEnableReplayApi=0\n");
        assert!(!enabled_in(cfg));
        assert!(enabled_in(&out));
    }

    #[test]
    fn missing_key_is_added_under_general_without_touching_other_sections() {
        let cfg = "[HUD]\nShowTimestamps=1\n[General]\nWindowMode=2\n\n[Performance]\nFrameCapType=4\n";
        let out = with_replay_api(cfg);
        assert_eq!(out, "[HUD]\nShowTimestamps=1\n[General]\nEnableReplayApi=1\nWindowMode=2\n\n[Performance]\nFrameCapType=4\n");
        assert!(enabled_in(&out));
    }

    #[test]
    fn missing_general_section_is_appended() {
        assert_eq!(with_replay_api("[HUD]\nShowTimestamps=1"), "[HUD]\nShowTimestamps=1\n[General]\nEnableReplayApi=1\n");
        assert_eq!(with_replay_api(""), "[General]\nEnableReplayApi=1\n");
    }

    #[test]
    fn crlf_files_stay_crlf() {
        let cfg = "[General]\r\nWindowMode=2\r\n[HUD]\r\nShowTimestamps=1\r\n";
        assert_eq!(with_replay_api(cfg), "[General]\r\nEnableReplayApi=1\r\nWindowMode=2\r\n[HUD]\r\nShowTimestamps=1\r\n");
        let zero = "[General]\r\nEnableReplayApi=0\r\nWindowMode=2";
        assert_eq!(with_replay_api(zero), "[General]\r\nEnableReplayApi=1\r\nWindowMode=2");
        // A header that ends the file still gets its own line ending before the added key.
        assert_eq!(with_replay_api("[HUD]\r\nX=1\r\n[General]"), "[HUD]\r\nX=1\r\n[General]\r\nEnableReplayApi=1\r\n");
    }

    #[test]
    fn enabled_flag_reads_only_general_and_tolerates_spacing() {
        assert!(enabled_in("\u{feff}[General]\nenablereplayapi = 1\n"));
        assert!(!enabled_in("[General]\nWindowMode=2\n"), "missing key is the game's default: off");
        assert!(!enabled_in("[HUD]\nEnableReplayApi=1\n"), "other sections do not count");
        assert!(!enabled_in("[General]\nEnableReplayApi=yes\n"), "not a number");
        // The last setting wins, as when the game reads the file.
        assert!(!enabled_in("[General]\nEnableReplayApi=1\nEnableReplayApi=0\n"));
        assert!(enabled_in("[general]\r\nEnableReplayApi=1\r\n[HUD]\r\nEnableReplayApi=0\r\n"));
    }
}
