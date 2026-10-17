//! Formatting and per-player statistics shared by the tabs and the overlay.

use crate::lcu::{LolGame, MatchDetail, MatchPlayer};
use crate::live::LivePlayer;
use crate::win;
use crate::worker::{LcuStatus, Snapshot};

/// Per-game averages of the player's own games.
#[derive(Default, Clone, Copy)]
pub struct Avg {
    pub n: usize,
    pub wins: usize,
    pub kills: f64,
    pub deaths: f64,
    pub assists: f64,
    pub cs_min: f64,
    pub gold_min: f64,
    pub dmg_min: f64,
    pub vision_min: f64,
}

impl Avg {
    pub fn kda(&self) -> f64 {
        (self.kills + self.assists) / self.deaths.max(1.0)
    }

    pub fn win_rate(&self) -> f64 {
        self.wins as f64 / self.n.max(1) as f64
    }
}

pub fn averages<'a>(games: impl IntoIterator<Item = &'a LolGame>) -> Option<Avg> {
    let mut a = Avg::default();
    for g in games {
        let m = (g.duration_s as f64 / 60.0).max(1.0);
        a.n += 1;
        a.wins += g.win as usize;
        a.kills += g.kills as f64;
        a.deaths += g.deaths as f64;
        a.assists += g.assists as f64;
        a.cs_min += g.cs as f64 / m;
        a.gold_min += g.gold as f64 / m;
        a.dmg_min += g.damage as f64 / m;
        a.vision_min += g.vision as f64 / m;
    }
    if a.n == 0 {
        return None;
    }
    let n = a.n as f64;
    for v in [&mut a.kills, &mut a.deaths, &mut a.assists, &mut a.cs_min, &mut a.gold_min, &mut a.dmg_min, &mut a.vision_min] {
        *v /= n;
    }
    Some(a)
}

pub fn cs_per_min(g: &LolGame) -> f64 {
    g.cs as f64 / (g.duration_s as f64 / 60.0).max(1.0)
}

/// A scouted player's Solo/Duo rank (Flex if unranked in Solo).
pub struct Scout {
    /// Empty when unranked.
    pub tier: String,
    /// `Gold II · 45 LP · 53% of 120`, `Unranked`.
    pub text: String,
}

pub fn scout(s: &Snapshot, puuid: &str) -> Option<Scout> {
    let q = s.ranks.get(puuid)?.as_ref().ok()?;
    let pick = |name: &str| q.iter().find(|r| r.queue == name && r.is_ranked());
    Some(match pick("RANKED_SOLO_5x5").map(|r| (r, "")).or_else(|| pick("RANKED_FLEX_SR").map(|r| (r, " (Flex)"))) {
        None => Scout { tier: String::new(), text: "Unranked".into() },
        Some((r, flex)) => {
            let total = r.wins + r.losses;
            let wr = if total > 0 { format!(" · {:.0}% of {total}", r.wins as f64 * 100.0 / total as f64) } else { String::new() };
            Scout { tier: r.tier.clone(), text: format!("{}{flex}{wr}", r.rank()) }
        }
    })
}

/// Puuid of a live player, matched through the gameflow session by champion and team.
pub fn live_puuid<'a>(s: &'a Snapshot, p: &LivePlayer) -> Option<&'a str> {
    let champ = s.data.champion_by_key(&p.champion_key)?;
    let team = if p.team == "ORDER" { 100 } else { 200 };
    s.session.as_ref()?.players.iter().find(|x| x.champion_id == champ && x.team == team).map(|x| x.puuid.as_str())
}

/// Your CS/min baseline for this pick: same champion (2+ games), else the queue, else everything.
pub fn cs_baseline(s: &Snapshot, champion_key: &str) -> Option<(f64, String)> {
    let history = s.lol_history.as_deref().ok()?;
    let champ = s.data.champion_by_key(champion_key);
    if let Some(a) = averages(history.iter().filter(|g| Some(g.champion_id) == champ)).filter(|a| a.n >= 2) {
        return Some((a.cs_min, format!("your {} avg", s.data.champion_name(champ?))));
    }
    let queue = s.session.as_ref().map_or(0, |g| g.queue_id);
    if let Some(a) = averages(history.iter().filter(|g| queue > 0 && g.queue_id == queue)).filter(|a| a.n >= 3) {
        return Some((a.cs_min, format!("your {} avg", queue_name(queue))));
    }
    averages(history).map(|a| (a.cs_min, "your avg".to_string()))
}

pub fn as_lol_game(d: &MatchDetail, p: &MatchPlayer) -> LolGame {
    LolGame {
        game_id: d.game_id,
        created_ms: d.created_ms,
        duration_s: d.duration_s,
        queue_id: d.queue_id,
        champion_id: p.champion_id,
        win: p.win,
        kills: p.kills,
        deaths: p.deaths,
        assists: p.assists,
        cs: p.cs,
        gold: p.gold,
        damage: p.damage,
        damage_taken: p.damage_taken,
        vision: p.vision,
        items: p.items,
    }
}

pub fn no_data_hint(s: &Snapshot) -> &'static str {
    match s.lcu {
        LcuStatus::Connected { .. } if s.summoner.is_none() => "Waiting for the League client to finish logging in…",
        LcuStatus::Connected { .. } => "No games found.",
        _ => "Start the League client to load match history.",
    }
}

pub fn title(s: &str) -> String {
    let lower = s.to_lowercase().replace('_', " ");
    let mut c = lower.chars();
    c.next().map(|f| f.to_uppercase().chain(c).collect()).unwrap_or_default()
}

pub fn kilo(g: i64) -> String {
    format!("{:.1}k", g as f64 / 1000.0)
}

pub fn signed(v: i64) -> String {
    if v >= 0 { format!("+{v}") } else { v.to_string() }
}

pub fn position(p: &str) -> &str {
    match p {
        "top" => "Top",
        "jungle" => "Jungle",
        "middle" => "Mid",
        "bottom" => "Bot",
        "utility" => "Support",
        "" => "—",
        other => other,
    }
}

pub fn position_label(p: &str) -> &str {
    match p {
        "TOP" => "Top",
        "JUNGLE" => "Jungle",
        "MIDDLE" => "Mid",
        "BOTTOM" => "Bot",
        "UTILITY" => "Support",
        "NONE" => "Any role",
        other => other,
    }
}

pub fn queue_name(id: i64) -> String {
    match id {
        400 => "Normal Draft",
        420 => "Ranked Solo/Duo",
        430 => "Normal Blind",
        440 => "Ranked Flex",
        450 => "ARAM",
        2400 => "ARAM Mayhem",
        480 | 490 => "Swiftplay",
        700 => "Clash",
        900 | 1010 => "ARURF",
        1700 | 1710 => "Arena",
        1090 => "TFT Normal",
        1100 => "TFT Ranked",
        1130 => "TFT Hyper Roll",
        1160 => "TFT Double Up",
        1210 => "TFT Choncc's Treasure",
        0 => "Custom",
        _ => return format!("Queue {id}"),
    }
    .into()
}

pub fn mmss(secs: f64) -> String {
    let s = secs.max(0.0) as u64;
    format!("{}:{:02}", s / 60, s % 60)
}

pub fn date(unix_ms: i64) -> String {
    let t = win::local_time(unix_ms);
    format!("{:04}-{:02}-{:02}", t.0, t.1, t.2)
}

pub fn datetime(unix_ms: i64) -> String {
    let t = win::local_time(unix_ms);
    format!("{:04}-{:02}-{:02} {:02}:{:02}", t.0, t.1, t.2, t.3, t.4)
}

/// `5m ago`, `3h ago`, `2d ago`; the date after a week.
pub fn ago(unix_ms: i64) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
    let mins = (now - unix_ms).max(0) / 60_000;
    match mins {
        0 => "just now".into(),
        1..=59 => format!("{mins}m ago"),
        60..=1439 => format!("{}h ago", mins / 60),
        1440..=10079 => format!("{}d ago", mins / 1440),
        _ => date(unix_ms),
    }
}
