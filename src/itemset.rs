//! Writes League item sets as JSON files in
//! `<League>/Config/Champions/<ChampionId>/Recommended/`, the same location and
//! schema the client reads for recommended sets. Contents come from the player's
//! own match history (local LCU data) and, optionally, OP.GG's aggregated builds
//! (`meta.rs`). Every file riftlite writes starts with `riftlite_`.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::ddragon::StaticData;
use crate::lcu::LolGame;
use crate::meta::{self, MetaBuild, Picks};

const PREFIX: &str = "riftlite_";

pub fn write_from_game(league_root: &Path, data: &StaticData, game: &LolGame) -> Result<PathBuf, String> {
    let mut blocks = vec![personal_block(game).ok_or("that game has no completed items")?];
    if game.items[6] > 0 {
        blocks.push(block("Trinket".into(), &[game.items[6]]));
    }
    let title = format!("riftlite · {} ({})", data.champion_name(game.champion_id), crate::ui::date(game.created_ms));
    write_set(league_root, data, game.champion_id, &game.game_id.to_string(), &title, &[], blocks)
}

/// The picked OP.GG build (`picks.build`, an index into `build.cores`) as Starting / Core /
/// Boots / Full build / Situational blocks, OP.GG's other trusted starts and boots, the best
/// build of every other group (AP, AD, On-hit, Crit, Tank, Utility) as "Alternative" blocks,
/// then the player's own best game on the champion in this mode. One file per champion and
/// OP.GG mode, shown by the client only on that mode's map.
pub fn write_meta(
    league_root: &Path,
    data: &StaticData,
    build: &MetaBuild,
    picks: Picks,
    personal: Option<&LolGame>,
) -> Result<PathBuf, String> {
    let q = build.query;
    let core = picks.build;
    let chosen = build.cores.get(core).ok_or("no such OP.GG build")?;
    let mut blocks = Vec::new();
    if let Some((s, others)) = build.starters.split_first() {
        let skills = match build.skill_priority(picks.skills).as_slice() {
            [] => String::new(),
            p => format!("max {} · ", p.join(" > ")),
        };
        blocks.push(block(format!("Starting · {skills}OP.GG {}", s.stats()), &s.ids));
        for o in others.iter().take(2) {
            blocks.push(block(format!("Starting alternative · OP.GG {}", o.stats()), &o.ids));
        }
    }
    blocks.push(block(format!("Core {} · OP.GG {}", chosen.label(), chosen.items.stats()), &chosen.items.ids));
    if let Some((b, others)) = build.boots.split_first() {
        blocks.push(block(format!("Boots · OP.GG {}", b.stats()), &b.ids));
        let ids: Vec<i64> = others.iter().flat_map(|o| o.ids.iter().copied()).collect();
        if !ids.is_empty() {
            blocks.push(block("Boots · other OP.GG options, most played first".into(), &ids));
        }
    }
    blocks.push(block(format!("Full build {} · core, boots, most finished items", chosen.label()), &build.full_build(core, data)));
    let situational = build.situational(core, data);
    if !situational.is_empty() {
        blocks.push(block("Situational · OP.GG often finished items".into(), &situational));
    }
    let mut listed = vec![core];
    for g in build.groups() {
        if let Some(&best) = build.in_group(g).first().filter(|i| !listed.contains(i)) {
            listed.push(best);
            let c = &build.cores[best];
            blocks.push(block(format!("Alternative {} · {} · OP.GG {}", g.label(), c.label(), c.items.stats()), &c.items.ids));
        }
    }
    blocks.extend(personal.and_then(personal_block));
    let title = format!("riftlite · {} · OP.GG {} · {}", data.champion_name(build.champion_id), q.label(), chosen.label());
    let maps: &[i64] = match q.mode {
        "aram" => &[12],
        "nexus_blitz" => &[21],
        "arena" => &[30],
        _ => &[11],
    };
    write_set(league_root, data, build.champion_id, &format!("meta_{}_{}", build.champion_id, q.mode), &title, maps, blocks)
}

fn personal_block(game: &LolGame) -> Option<Value> {
    let items: Vec<i64> = game.items[..6].iter().copied().filter(|&i| i > 0).collect();
    (!items.is_empty()).then(|| {
        let title = format!(
            "Your build · {} {}/{}/{} · {}",
            if game.win { "Win" } else { "Loss" },
            game.kills,
            game.deaths,
            game.assists,
            crate::ui::date(game.created_ms)
        );
        block(title, &items)
    })
}

/// A block with repeated ids folded into a count (two potions → one entry, count 2), order kept.
fn block(title: String, ids: &[i64]) -> Value {
    let mut items: Vec<(i64, u32)> = Vec::new();
    for &id in ids {
        match items.iter_mut().find(|(i, _)| *i == id) {
            Some((_, n)) => *n += 1,
            None => items.push((id, 1)),
        }
    }
    json!({
        "type": title,
        "items": items.iter().map(|(id, n)| json!({"id": id.to_string(), "count": n})).collect::<Vec<_>>(),
        "showIfSummonerSpell": "",
        "hideIfSummonerSpell": ""
    })
}

/// The item-set document the client reads.
fn item_set_json(champion_id: i64, title: &str, maps: &[i64], blocks: Vec<Value>) -> Value {
    json!({
        "title": title,
        "type": "custom",
        "map": "any",
        "mode": "any",
        "priority": false,
        "sortrank": 0,
        "startedFrom": "blank",
        "associatedChampions": [champion_id],
        "associatedMaps": maps,
        "preferredItemSlots": [],
        "blocks": blocks
    })
}

fn write_set(
    league_root: &Path,
    data: &StaticData,
    champion_id: i64,
    stem: &str,
    title: &str,
    maps: &[i64],
    blocks: Vec<Value>,
) -> Result<PathBuf, String> {
    let champ = data.champions.get(&champion_id).ok_or("unknown champion (Data Dragon not loaded?)")?;
    let set = item_set_json(champion_id, title, maps, blocks);
    let dir = league_root.join("Config").join("Champions").join(&champ.id).join("Recommended");
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let path = dir.join(format!("{PREFIX}{stem}.json"));
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

/// The player's best recent game on `champion_id`: wins first, then KDA. With `mode`, only
/// games of that OP.GG mode (same item pool), so an ARAM game never stands in for ranked.
pub fn best_game<'a>(history: &'a [LolGame], champion_id: i64, mode: Option<&str>) -> Option<&'a LolGame> {
    history
        .iter()
        .filter(|g| g.champion_id == champion_id && g.items[..6].iter().any(|&i| i > 0))
        .filter(|g| mode.is_none_or(|m| meta::mode_for(g.queue_id, 0).is_ok_and(|gm| gm == m)))
        .max_by(|a, b| a.win.cmp(&b.win).then(a.kda().total_cmp(&b.kda())))
}
