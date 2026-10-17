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

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Settings {
    pub overlay_enabled: bool,
    /// Overlay offset from the top-right corner of the primary monitor, in points.
    pub overlay_margin_right: f32,
    pub overlay_margin_top: f32,
    pub switch_tabs_automatically: bool,
    /// Accept the matchmaking ready check automatically (unless you already answered it).
    pub auto_accept: bool,
    /// On lock-in, write the `page_source` rune page into riftlite's own page.
    pub auto_runes: bool,
    /// On lock-in, set the `page_source` summoner spells (Flash stays on its key).
    pub auto_spells: bool,
    /// Runes and spells for `auto_runes` / `auto_spells`.
    pub page_source: PageSource,
    /// On lock-in, write an item set: OP.GG's build when `meta_builds` is on and it has
    /// one, otherwise the build from your best recent game on the champion in this mode.
    pub auto_item_set: bool,
    /// Fetch OP.GG's aggregated builds for the pick: written as the lock-in item set
    /// (with your best game as an extra block) and offered as a rune page.
    pub meta_builds: bool,
    /// Rank OP.GG builds by the 95 % lower bound of their win rate instead of the win rate.
    pub meta_sort_confident: bool,
    /// League install folder chosen by hand; empty = detect automatically.
    pub league_folder: String,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            overlay_enabled: true,
            overlay_margin_right: 24.0,
            overlay_margin_top: 120.0,
            switch_tabs_automatically: true,
            auto_accept: false,
            auto_runes: true,
            auto_spells: false,
            page_source: PageSource::Riot,
            auto_item_set: true,
            meta_builds: true,
            meta_sort_confident: false,
            league_folder: String::new(),
        }
    }
}

impl Settings {
    pub fn league_override(&self) -> Option<PathBuf> {
        let f = self.league_folder.trim();
        (!f.is_empty()).then(|| PathBuf::from(f))
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
