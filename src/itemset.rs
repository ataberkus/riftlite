//! Writes League item sets as JSON files in
//! `<League>/Config/Champions/<ChampionId>/Recommended/`, the same location and
//! schema the client reads for recommended sets. Contents come only from the
//! player's own match history (local LCU data).

use std::path::{Path, PathBuf};

use serde_json::json;

use crate::ddragon::StaticData;
use crate::lcu::LolGame;

const PREFIX: &str = "riftlite_";

pub fn write_from_game(league_root: &Path, data: &StaticData, game: &LolGame) -> Result<PathBuf, String> {
    let champ = data.champions.get(&game.champion_id).ok_or("unknown champion (Data Dragon not loaded?)")?;
    let build: Vec<_> = game.items[..6].iter().filter(|&&i| i > 0).map(|i| json!({"id": i.to_string(), "count": 1})).collect();
    if build.is_empty() {
        return Err("that game has no completed items".into());
    }
    let mut blocks = vec![json!({
        "type": format!(
            "Your build · {} {}/{}/{} · {}",
            if game.win { "Win" } else { "Loss" }, game.kills, game.deaths, game.assists, crate::ui::date(game.created_ms)
        ),
        "items": build,
        "showIfSummonerSpell": "",
        "hideIfSummonerSpell": ""
    })];
    if game.items[6] > 0 {
        blocks.push(json!({
            "type": "Trinket",
            "items": [{"id": game.items[6].to_string(), "count": 1}],
            "showIfSummonerSpell": "",
            "hideIfSummonerSpell": ""
        }));
    }
    let set = json!({
        "title": format!("riftlite · {} ({})", champ.name, crate::ui::date(game.created_ms)),
        "type": "custom",
        "map": "any",
        "mode": "any",
        "priority": false,
        "sortrank": 0,
        "startedFrom": "blank",
        "associatedChampions": [game.champion_id],
        "associatedMaps": [],
        "preferredItemSlots": [],
        "blocks": blocks
    });
    let dir = league_root.join("Config").join("Champions").join(&champ.id).join("Recommended");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(format!("{PREFIX}{}.json", game.game_id));
    std::fs::write(&path, serde_json::to_vec_pretty(&set).expect("item set serializes"))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(path)
}

/// Deletes only files riftlite created; other tools' sets are left alone.
pub fn clear_all(league_root: &Path) -> Result<usize, String> {
    let champions = league_root.join("Config").join("Champions");
    let Ok(rd) = std::fs::read_dir(&champions) else { return Ok(0) };
    let mut removed = 0;
    for champ in rd.flatten() {
        let Ok(files) = std::fs::read_dir(champ.path().join("Recommended")) else { continue };
        for f in files.flatten() {
            if f.file_name().to_string_lossy().starts_with(PREFIX) {
                std::fs::remove_file(f.path()).map_err(|e| format!("{}: {e}", f.path().display()))?;
                removed += 1;
            }
        }
    }
    Ok(removed)
}

/// The player's best recent game on `champion_id`: wins first, then KDA.
pub fn best_game(history: &[LolGame], champion_id: i64) -> Option<&LolGame> {
    history
        .iter()
        .filter(|g| g.champion_id == champion_id && g.items[..6].iter().any(|&i| i > 0))
        .max_by(|a, b| a.win.cmp(&b.win).then(a.kda().total_cmp(&b.kda())))
}
