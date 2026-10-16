//! Riot install discovery, running-process classification and game launch.
//!
//! Sources (all local): `%ProgramData%\Riot Games\RiotClientInstalls.json`,
//! the Riot Client product metadata YAML, and the per-user uninstall registry
//! keys the Riot Client writes.

use std::path::{Path, PathBuf};
use std::process::Command;

use winreg::RegKey;
use winreg::enums::HKEY_CURRENT_USER;

use crate::win::Process;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Product {
    League,
    Tft,
}

impl Product {
    pub const ALL: [Product; 2] = [Product::League, Product::Tft];

    pub fn label(self) -> &'static str {
        match self {
            Product::League => "League of Legends",
            Product::Tft => "Teamfight Tactics",
        }
    }

    fn riot_id(self) -> &'static str {
        match self {
            Product::League => "league_of_legends",
            Product::Tft => "teamfighttactics",
        }
    }

    pub fn client_exe(self) -> &'static str {
        match self {
            Product::League => "LeagueClient.exe",
            Product::Tft => "TFTClient.exe",
        }
    }
}

#[derive(Clone, PartialEq, Debug)]
pub struct Install {
    pub product: Product,
    pub path: PathBuf,
    pub source: &'static str,
    /// The product's client executable exists under `path`.
    pub verified: bool,
}

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Installs {
    pub riot_client: Option<PathBuf>,
    pub products: Vec<Install>,
}

impl Installs {
    pub fn get(&self, p: Product) -> Option<&Install> {
        self.products.iter().find(|i| i.product == p)
    }
}

fn program_data() -> PathBuf {
    std::env::var_os("ProgramData").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"))
}

fn riot_client_from_installs_json() -> Option<PathBuf> {
    let text = std::fs::read_to_string(program_data().join(r"Riot Games\RiotClientInstalls.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    ["rc_default", "rc_live"]
        .iter()
        .filter_map(|k| v.get(*k)?.as_str())
        .map(PathBuf::from)
        .find(|p| p.is_file())
}

fn install_from_metadata(p: Product) -> Option<PathBuf> {
    let dir = program_data().join(format!(r"Riot Games\Metadata\{0}.live", p.riot_id()));
    let text = std::fs::read_to_string(dir.join(format!("{0}.live.product_settings.yaml", p.riot_id()))).ok()?;
    text.lines().find_map(|l| {
        let v = l.trim().strip_prefix("product_install_full_path:")?;
        Some(PathBuf::from(v.trim().trim_matches('"')))
    })
}

fn uninstall_key(p: Product) -> Option<RegKey> {
    RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(format!(r"Software\Microsoft\Windows\CurrentVersion\Uninstall\Riot Game {0}.live", p.riot_id()))
        .ok()
}

fn install_from_registry(p: Product) -> Option<PathBuf> {
    let loc: String = uninstall_key(p)?.get_value("InstallLocation").ok()?;
    Some(PathBuf::from(loc))
}

/// `UninstallString` is `"...\RiotClientServices.exe" --uninstall-product=...`.
fn riot_client_from_registry() -> Option<PathBuf> {
    Product::ALL.iter().find_map(|&p| {
        let s: String = uninstall_key(p)?.get_value("UninstallString").ok()?;
        let exe = s.trim_start().strip_prefix('"')?.split('"').next()?;
        Some(PathBuf::from(exe)).filter(|p| p.is_file())
    })
}

/// `league_override`: a folder the user picked in Settings; used when it holds the League client.
pub fn scan_installs(league_override: Option<&Path>) -> Installs {
    let riot_client = riot_client_from_installs_json().or_else(riot_client_from_registry);
    let products = Product::ALL
        .iter()
        .filter_map(|&p| {
            let manual = league_override
                .filter(|d| p == Product::League && d.join(p.client_exe()).is_file())
                .map(|d| (d.to_path_buf(), "manual setting"));
            let (path, source) = manual
                .or_else(|| install_from_metadata(p).map(|x| (x, "Riot Client metadata")))
                .or_else(|| install_from_registry(p).map(|x| (x, "uninstall registry")))?;
            let verified = path.join(p.client_exe()).is_file();
            Some(Install { product: p, path, source, verified })
        })
        .collect();
    Installs { riot_client, products }
}

#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub struct Running {
    pub riot_client: Option<u32>,
    /// `LeagueClientUx.exe`, the process that hosts the LCU API (also used for TFT).
    pub league_client: Option<u32>,
    pub league_game: Option<u32>,
    pub tft_client: Option<u32>,
    pub tft_game: Option<u32>,
}

impl Running {
    pub fn from_processes(procs: &[Process]) -> Self {
        let find = |names: &[&str]| procs.iter().find(|p| names.contains(&p.name.as_str())).map(|p| p.pid);
        Running {
            riot_client: find(&["riotclientservices.exe", "riot client.exe"]),
            league_client: find(&["leagueclientux.exe"]),
            league_game: find(&["league of legends.exe"]),
            tft_client: find(&["tftclient.exe"]),
            tft_game: find(&["tftclient-win64-shipping.exe"]),
        }
    }

    pub fn any_game(&self) -> bool {
        self.league_game.is_some() || self.tft_game.is_some()
    }
}

/// Launches through the Riot Client, exactly like the desktop shortcuts it creates.
pub fn launch(installs: &Installs, p: Product) -> Result<(), String> {
    let rc = installs.riot_client.as_deref().ok_or("Riot Client not found")?;
    Command::new(rc)
        .arg(format!("--launch-product={0}", p.riot_id()))
        .arg("--launch-patchline=live")
        .spawn()
        .map(drop)
        .map_err(|e| format!("failed to start {}: {e}", rc.display()))
}

pub fn league_root(installs: &Installs, client_exe: Option<&Path>) -> Option<PathBuf> {
    client_exe
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .or_else(|| installs.get(Product::League).map(|i| i.path.clone()))
}
