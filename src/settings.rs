//! Settings file and Windows autostart (per-user `Run` key, no elevation).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_VALUE: &str = "riftlite";

pub fn local_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from).unwrap_or_else(std::env::temp_dir).join("riftlite")
}

/// Where lock-in runes and summoner spells come from.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum PageSource {
    /// Riot's recommended page for the pick.
    #[default]
    Riot,
    /// The OP.GG rune page and spells selected in champ select; Riot's page when OP.GG
    /// has none or does not answer in time.
    OpGg,
}

/// Default overlay panel size, in points.
pub const OVERLAY_SIZE: [f32; 2] = [320.0, 320.0];
/// Smallest size the overlay can be resized to, in points.
pub const OVERLAY_MIN_SIZE: [f32; 2] = [200.0, 120.0];

/// Which overlay sections are drawn.
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(default)]
pub struct OverlaySections {
    /// Header: logo and game clock.
    pub clock: bool,
    /// Your KDA, CS and CS per minute against your average.
    pub kda: bool,
    /// Your goals for the role (CS/min, deaths, vision) and how this game is going.
    pub goals: bool,
    /// Team kills and your current gold.
    pub kills_gold: bool,
    /// Item value, team and lane.
    pub item_diff: bool,
    /// "Level up X" from your last games on the champion.
    pub skill_hint: bool,
    /// The OP.GG build picked in champ select, items you own outlined.
    pub build: bool,
    /// Items that answer what the enemies build (healing, shields, armor/MR stacking).
    pub counters: bool,
    pub objectives: bool,
    /// Dead enemies with respawn timers.
    pub dead_enemies: bool,
    /// Enemy summoner spells and keystones (which spells, not their cooldowns).
    pub enemy_spells: bool,
    /// The last few kills, objectives and structures.
    pub events: bool,
    /// Finished items enemies bought in the last minute.
    pub enemy_items: bool,
    /// Enemy ranks during loading and the first minute and a half.
    pub scouts: bool,
    /// Your notes on the champion and the lane matchup, during the first minute and a half.
    pub notes: bool,
    /// ARAM Mayhem: where the augment tiers come from (or why they are missing).
    pub augment_line: bool,
}

impl Default for OverlaySections {
    fn default() -> Self {
        OverlaySections {
            clock: true,
            kda: true,
            goals: true,
            kills_gold: true,
            item_diff: true,
            skill_hint: true,
            build: true,
            counters: true,
            objectives: true,
            dead_enemies: true,
            enemy_spells: false,
            events: false,
            enemy_items: false,
            scouts: true,
            notes: true,
            augment_line: true,
        }
    }
}

impl OverlaySections {
    /// Short label and flag of every section, in drawing order.
    pub fn toggles(&mut self) -> [(&'static str, &mut bool); 16] {
        [
            ("Clock", &mut self.clock),
            ("KDA / CS", &mut self.kda),
            ("Goals", &mut self.goals),
            ("Kills / gold", &mut self.kills_gold),
            ("Item value", &mut self.item_diff),
            ("Skill hint", &mut self.skill_hint),
            ("Build", &mut self.build),
            ("Counter items", &mut self.counters),
            ("Objectives", &mut self.objectives),
            ("Dead enemies", &mut self.dead_enemies),
            ("Enemy spells", &mut self.enemy_spells),
            ("Recent events", &mut self.events),
            ("Enemy items", &mut self.enemy_items),
            ("Enemy ranks", &mut self.scouts),
            ("Notes", &mut self.notes),
            ("Augment source", &mut self.augment_line),
        ]
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Settings {
    pub overlay_enabled: bool,
    /// Overlay panel's top-left corner in screen points (relative to the primary monitor's
    /// top-left); `None` until it is first shown, then placed at the monitor's top-right.
    pub overlay_pos: Option<[f32; 2]>,
    /// Overlay panel size in points.
    pub overlay_size: [f32; 2],
    pub overlay_sections: OverlaySections,
    /// Offsets from the top-right corner written by older versions; read once to place
    /// `overlay_pos`, never written back.
    #[serde(rename = "overlay_margin_right", skip_serializing)]
    legacy_margin_right: Option<f32>,
    #[serde(rename = "overlay_margin_top", skip_serializing)]
    legacy_margin_top: Option<f32>,
    /// Show the overlay only while the League game window is the foreground window.
    pub overlay_focus_only: bool,
    /// ARAM Mayhem: read the offered augment cards off the screen and show Blitz's tier
    /// under each on the overlay.
    pub augment_tiers: bool,
    pub switch_tabs_automatically: bool,
    /// Accept the matchmaking ready check automatically (unless you already answered it).
    pub auto_accept: bool,
    /// On lock-in, write the `page_source` rune page into riftlite's own page; an OP.GG rune
    /// page picked in champ select is written at once.
    pub auto_runes: bool,
    /// On lock-in, set the `page_source` summoner spells (Flash stays on its key). OP.GG
    /// spells picked in champ select are set at once whatever this says.
    pub auto_spells: bool,
    /// Runes and spells for `auto_runes` / `auto_spells`.
    pub page_source: PageSource,
    /// On lock-in, write an item set: OP.GG's build when `meta_builds` is on and it has
    /// one, otherwise the build from your best recent game on the champion in this mode.
    pub auto_item_set: bool,
    /// Fetch OP.GG's aggregated builds for the pick: written as the lock-in item set
    /// (with your best game as an extra block) and offered as a rune page.
    pub meta_builds: bool,
    /// League install folder chosen by hand; empty = detect automatically.
    pub league_folder: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            overlay_enabled: true,
            overlay_pos: None,
            overlay_size: OVERLAY_SIZE,
            overlay_sections: OverlaySections::default(),
            legacy_margin_right: None,
            legacy_margin_top: None,
            overlay_focus_only: true,
            augment_tiers: true,
            switch_tabs_automatically: true,
            auto_accept: false,
            auto_runes: true,
            auto_spells: false,
            page_source: PageSource::Riot,
            auto_item_set: true,
            meta_builds: true,
            league_folder: String::new(),
        }
    }
}

impl Settings {
    pub fn league_override(&self) -> Option<PathBuf> {
        let f = self.league_folder.trim();
        (!f.is_empty()).then(|| PathBuf::from(f))
    }

    /// Overlay panel position. The first call places it at the top-right corner of a
    /// monitor `monitor_width` points wide (keeping an older version's offsets) and
    /// returns `changed: true` so the caller saves it.
    pub fn resolve_overlay_pos(&mut self, monitor_width: f32) -> ([f32; 2], bool) {
        if let Some(p) = self.overlay_pos {
            return (p, false);
        }
        let right = self.legacy_margin_right.take().unwrap_or(24.0);
        let top = self.legacy_margin_top.take().unwrap_or(120.0);
        let p = [monitor_width - self.overlay_size[0] - right, top];
        self.overlay_pos = Some(p);
        (p, true)
    }
}

fn path() -> PathBuf {
    local_dir().join("settings.json")
}

impl Settings {
    pub fn load() -> Self {
        std::fs::read(path()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let p = path();
        std::fs::create_dir_all(p.parent().expect("settings path has a parent")).map_err(|e| e.to_string())?;
        let tmp = p.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).expect("settings serialize"))
            .and_then(|_| std::fs::rename(&tmp, &p))
            .map_err(|e| e.to_string())
    }
}

pub fn autostart_enabled() -> bool {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(RUN_KEY, KEY_READ)
        .and_then(|k| k.get_value::<String, _>(RUN_VALUE))
        .is_ok()
}

pub fn set_autostart(enabled: bool) -> Result<(), String> {
    let key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(RUN_KEY, KEY_SET_VALUE)
        .map_err(|e| e.to_string())?;
    if enabled {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        key.set_value(RUN_VALUE, &format!("\"{}\" --minimized", exe.display())).map_err(|e| e.to_string())
    } else {
        match key.delete_value(RUN_VALUE) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.to_string()),
            _ => Ok(()),
        }
    }
}
