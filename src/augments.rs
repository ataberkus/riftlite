//! ARAM Mayhem augment tiers on the in-game overlay.
//!
//! The game's Live Client Data API and the League client's LCU do not say which
//! augments are offered (neither lists any augment endpoint). While a Mayhem game
//! window has the foreground, this thread reads the card titles off the screen
//! instead: the title band of the game's client area is copied from the screen
//! device context (like a screenshot; the game process is never opened) and read
//! with Windows' built-in OCR in the client's language. Titles are matched to ids
//! through the client's own `/lol-game-data/assets/v1/cherry-augments.json` names,
//! and ids to Blitz's tier (S–D) from `data.v2.iesdev.com`, the JSON behind
//! blitz.gg's ARAM Mayhem augment tier list (undocumented, may change).

use std::collections::HashMap;
use std::sync::{Arc, mpsc};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use eframe::egui;
use image::{GrayImage, imageops};
use serde_json::Value;
use windows::Globalization::Language;
use windows::Graphics::Imaging::{BitmapPixelFormat, SoftwareBitmap};
use windows::Media::Ocr::OcrEngine;
use windows::Storage::Streams::DataWriter;
use windows::core::HSTRING;

use crate::lcu::{Creds, Lcu};
use crate::worker::{Shared, set};
use crate::{net, win};

const TIERS_URL: &str = "https://data.v2.iesdev.com/api/v1/query_objects/prod/lol/aram_mayhem_augments";
/// Blitz updates its tiers about daily.
const TIERS_TTL: Duration = Duration::from_secs(6 * 3600);
const RETRY: Duration = Duration::from_secs(300);
const CLIENT_RETRY: Duration = Duration::from_secs(30);
/// Pause between screen reads while a Mayhem game has the foreground. A read (capture,
/// scaling, OCR, matching) takes about 20 ms, so badges follow the cards within ~0.2 s.
const SCAN_INTERVAL: Duration = Duration::from_millis(200);
/// How soon an idle thread notices it should read again (the game regained the foreground,
/// the overlay was shown): flag checks only, no screen reads.
const IDLE_CHECK: Duration = Duration::from_millis(100);

// Card layout as fractions of the game's client-area height; the cards scale with it
// and sit centered horizontally (measured at 1280×720 and 1920×1080).
/// Title band: card titles are centered at 0.41; the rarity tag below them starts at 0.435.
const TITLE_TOP: f32 = 0.365;
const TITLE_BOTTOM: f32 = 0.44;
/// Half the width of the searched band around the horizontal center (three cards span ±0.49).
const BAND_HALF_WIDTH: f32 = 0.55;
/// The reroll button under each card is centered 0.295 below the title and 0.030 wide on
/// either side of the card's center; the badge sits just left of it.
const BADGE_BELOW_TITLE: f32 = 0.295;
const BADGE_LEFT_OF_CENTER: f32 = 0.064;
/// The band is scaled so the client height maps to this many pixels: titles become ~40 px tall,
/// a size Windows OCR reads reliably.
const OCR_HEIGHT: f32 = 2105.0;
/// Minimum similarity (1 − edit distance / length) between an OCR'd title and an augment name.
const MIN_SIMILARITY: f32 = 0.75;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tier {
    S,
    A,
    B,
    C,
    D,
}

impl Tier {
    /// Blitz ranks 1 (best) to 5; checked against the letters on blitz.gg's tier list page.
    fn from_blitz(rank: i64) -> Option<Tier> {
        Some(match rank {
            1 => Tier::S,
            2 => Tier::A,
            3 => Tier::B,
            4 => Tier::C,
            5 => Tier::D,
            _ => return None,
        })
    }

    pub fn letter(self) -> &'static str {
        match self {
            Tier::S => "S",
            Tier::A => "A",
            Tier::B => "B",
            Tier::C => "C",
            Tier::D => "D",
        }
    }
}

/// One offered augment card.
#[derive(Clone, PartialEq, Debug)]
pub struct Card {
    pub id: i64,
    /// `None` when Blitz has no tier for it or its tiers could not be loaded.
    pub tier: Option<Tier>,
    /// Badge center on screen, physical pixels.
    pub at: [f32; 2],
}

/// The augment choice currently on screen.
#[derive(Clone, PartialEq, Debug)]
pub struct Offer {
    /// The game's client area on screen: left, top, width, height in physical pixels.
    pub frame: [i32; 4],
    /// Left to right.
    pub cards: Vec<Card>,
}

impl Offer {
    /// Badge edge length, physical pixels.
    pub fn badge_size(&self) -> f32 {
        self.frame[3] as f32 * 0.044
    }
}

pub fn spawn(shared: Arc<Shared>, ctx: egui::Context) {
    std::thread::Builder::new()
        .name("riftlite-augments".into())
        .spawn(move || run(&shared, &ctx))
        .expect("spawn augments thread");
}

/// Names and OCR for one League client connection.
struct Client {
    port: u16,
    names: Vec<Name>,
    engine: OcrEngine,
}

struct Name {
    id: i64,
    folded: Vec<char>,
}

type Tiers = Result<HashMap<i64, Tier>, String>;

fn run(shared: &Shared, ctx: &egui::Context) {
    let public = net::public_agent();
    let local = net::riot_local_agent();
    // Blitz is asked on its own thread: a slow or unreachable server (up to the agent's
    // timeouts, again every RETRY) must not hold up screen reads.
    let (tiers_tx, tiers_rx) = mpsc::channel::<Tiers>();
    let mut tiers: Option<(Instant, Tiers)> = None;
    let mut fetching = false;
    let mut client: Option<Client> = None;
    let mut client_error: Option<(Instant, u16, String)> = None;
    loop {
        if let Ok(r) = tiers_rx.try_recv() {
            tiers = Some((Instant::now(), r));
            fetching = false;
        }
        let (wanted, active, creds) = {
            let s = shared.snap.lock();
            let settings = shared.settings.lock();
            let mayhem = s.live.as_ref().is_some_and(|g| g.mode == "KIWI");
            let wanted = mayhem && settings.overlay_enabled && settings.augment_tiers;
            let active =
                wanted && !shared.overlay_hidden.load(Ordering::Relaxed) && shared.game_focused.load(Ordering::Relaxed);
            (wanted, active, s.creds.clone())
        };
        // Asked as soon as a Mayhem game is up, focused or not, so the tiers are in before
        // the first augment choice.
        let stale = tiers.as_ref().is_none_or(|(at, r)| at.elapsed() > if r.is_ok() { TIERS_TTL } else { RETRY });
        if wanted && stale && !fetching {
            fetching = true;
            let (agent, tx) = (public.clone(), tiers_tx.clone());
            std::thread::Builder::new()
                .name("riftlite-augment-tiers".into())
                .spawn(move || {
                    let _ = tx.send(fetch_tiers(&agent));
                })
                .expect("spawn augment tiers thread");
        }
        if !active {
            publish(shared, ctx, None, None);
            std::thread::sleep(IDLE_CHECK);
            continue;
        }

        let Some(creds) = creds else {
            publish(shared, ctx, None, Some("League client not connected: augment names unavailable".into()));
            std::thread::sleep(Duration::from_secs(1));
            continue;
        };
        if client.as_ref().is_none_or(|c| c.port != creds.port) {
            client = None;
            let retry = client_error.as_ref().is_none_or(|(at, port, _)| *port != creds.port || at.elapsed() > CLIENT_RETRY);
            if retry {
                match load_client(&local, &creds) {
                    Ok(c) => {
                        client = Some(c);
                        client_error = None;
                    }
                    Err(e) => client_error = Some((Instant::now(), creds.port, e)),
                }
            }
        }
        let Some(c) = &client else {
            let e = client_error.as_ref().map(|(_, _, e)| e.clone());
            publish(shared, ctx, None, e);
            std::thread::sleep(Duration::from_secs(1));
            continue;
        };

        // Until Blitz first answers (a fraction of a second), no badges rather than "?" ones.
        let Some((_, tier_result)) = &tiers else {
            std::thread::sleep(IDLE_CHECK);
            continue;
        };
        let (tier_map, tier_error) = match tier_result {
            Ok(m) => (Some(m), None),
            Err(e) => (None, Some(e.clone())),
        };
        match detect(c) {
            Ok(offer) => {
                let offer = offer.map(|mut o| {
                    for card in &mut o.cards {
                        card.tier = tier_map.and_then(|m| m.get(&card.id).copied());
                    }
                    o
                });
                publish(shared, ctx, offer, tier_error);
            }
            Err(e) => publish(shared, ctx, None, Some(e)),
        }
        std::thread::sleep(SCAN_INTERVAL);
    }
}

fn publish(shared: &Shared, ctx: &egui::Context, offer: Option<Offer>, error: Option<String>) {
    let mut s = shared.snap.lock();
    let changed = set(&mut s.augments, offer) | set(&mut s.augment_error, error);
    drop(s);
    if changed {
        // The main pass sizes the overlay viewport to the game while cards are shown.
        ctx.request_repaint();
        ctx.request_repaint_of(crate::ui::overlay_id());
    }
}

fn fetch_tiers(agent: &ureq::Agent) -> Result<HashMap<i64, Tier>, String> {
    let v: Value = agent
        .get(TIERS_URL)
        .call()
        .map_err(|e| format!("Blitz tiers: {e}"))?
        .into_json()
        .map_err(|e| format!("Blitz tiers: {e}"))?;
    parse_tiers(&v)
}

fn parse_tiers(v: &Value) -> Result<HashMap<i64, Tier>, String> {
    let m: HashMap<i64, Tier> = v["data"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|x| Some((x["augment_id"].as_str()?.parse().ok()?, Tier::from_blitz(x["stats"]["tier"].as_i64()?)?)))
        .collect();
    if m.is_empty() { Err("Blitz tiers: the data format changed".into()) } else { Ok(m) }
}

fn load_client(agent: &ureq::Agent, creds: &Creds) -> Result<Client, String> {
    let lcu = Lcu::new(agent.clone(), creds);
    let assets = lcu.get("/lol-game-data/assets/v1/cherry-augments.json").map_err(|e| format!("Augment names: {e}"))?;
    let locale = lcu.get("/riotclient/region-locale").map_err(|e| format!("Client language: {e}"))?;
    let locale = locale["locale"].as_str().unwrap_or("en_US");
    let names = names(&assets);
    if names.is_empty() {
        return Err("Augment names: the client listed none".into());
    }
    Ok(Client { port: creds.port, names, engine: ocr_engine(locale)? })
}

/// Augment names in the client's language. Arena and Mayhem versions of an augment share a
/// name; the Mayhem one (`ARAM_` name id) is kept.
fn names(assets: &Value) -> Vec<Name> {
    let all: Vec<(i64, String, bool)> = assets
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| {
            let name = fold(a["nameTRA"].as_str()?);
            let aram = a["augmentNameId"].as_str().is_some_and(|n| n.starts_with("ARAM_"));
            Some((a["id"].as_i64()?, name, aram)).filter(|(_, n, _)| !n.is_empty())
        })
        .collect();
    let aram: std::collections::HashSet<&str> = all.iter().filter(|(_, _, a)| *a).map(|(_, n, _)| n.as_str()).collect();
    all.iter()
        .filter(|(_, n, a)| *a || !aram.contains(n.as_str()))
        .map(|(id, n, _)| Name { id: *id, folded: n.chars().collect() })
        .collect()
}

fn ocr_engine(locale: &str) -> Result<OcrEngine, String> {
    let tag = locale.replace('_', "-");
    let primary = tag.split('-').next().unwrap_or_default();
    for t in [tag.as_str(), primary] {
        let Ok(lang) = Language::CreateLanguage(&HSTRING::from(t)) else { continue };
        if OcrEngine::IsLanguageSupported(&lang).unwrap_or(false) {
            return OcrEngine::TryCreateFromLanguage(&lang).map_err(|e| format!("Windows OCR: {e}"));
        }
    }
    Err(format!(
        "Windows has no OCR for the game's language ({tag}); add it in Settings > Time & language > Language"
    ))
}

/// The cards on screen now, `None` when no augment choice is shown.
fn detect(c: &Client) -> Result<Option<Offer>, String> {
    let Some(frame @ [left, top, width, height]) = win::league_game_frame() else { return Ok(None) };
    let h = height as f32;
    let center = width as f32 / 2.0;
    let x0 = (center - BAND_HALF_WIDTH * h).max(0.0) as i32;
    let x1 = ((center + BAND_HALF_WIDTH * h) as i32).min(width);
    let y0 = (TITLE_TOP * h) as i32;
    let y1 = (TITLE_BOTTOM * h) as i32;
    let (bw, bh) = (x1 - x0, y1 - y0);
    let Some(bgra) = win::capture_screen(left + x0, top + y0, bw, bh) else { return Ok(None) };
    read_band(c, frame, [x0, y0, bw, bh], &bgra)
}

/// The cards whose titles are in `bgra`, the band `[x, y, width, height]` of the client area.
fn read_band(c: &Client, frame: [i32; 4], [x0, y0, bw, bh]: [i32; 4], bgra: &[u8]) -> Result<Option<Offer>, String> {
    let [left, top, _, height] = frame;
    let h = height as f32;
    // Light titles on dark cards → dark text on a light background, scaled for OCR.
    let gray = GrayImage::from_fn(bw as u32, bh as u32, |x, y| {
        let i = (y as usize * bw as usize + x as usize) * 4;
        image::Luma([255 - bgra[i].max(bgra[i + 1]).max(bgra[i + 2])])
    });
    let max_dim = OcrEngine::MaxImageDimension().unwrap_or(2600) as f32;
    let scale = (OCR_HEIGHT / h).min(max_dim / bw as f32);
    let gray = imageops::resize(
        &gray,
        (bw as f32 * scale) as u32,
        (bh as f32 * scale).max(1.0) as u32,
        imageops::FilterType::Triangle,
    );
    let words = recognize(&c.engine, &gray).map_err(|e| format!("Windows OCR: {e}"))?;

    let mut cards: Vec<Card> = Vec::new();
    for seg in segments(&words) {
        let Some(id) = best_match(&seg.text, &c.names) else { continue };
        if cards.iter().any(|k| k.id == id) {
            continue;
        }
        let cx = left as f32 + x0 as f32 + seg.cx / scale;
        let cy = top as f32 + y0 as f32 + seg.cy / scale;
        cards.push(Card { id, tier: None, at: [cx - BADGE_LEFT_OF_CENTER * h, cy + BADGE_BELOW_TITLE * h] });
    }
    // One stray match is not a card choice (an item or chat line can read like a name).
    if cards.len() < 2 {
        return Ok(None);
    }
    cards.sort_by(|a, b| a.at[0].total_cmp(&b.at[0]));
    Ok(Some(Offer { frame, cards }))
}

struct Word {
    line: usize,
    text: String,
    x: f32,
    y: f32,
    w: f32,
    h: f32,
}

fn recognize(engine: &OcrEngine, img: &GrayImage) -> windows::core::Result<Vec<Word>> {
    let writer = DataWriter::new()?;
    writer.WriteBytes(img.as_raw())?;
    let buffer = writer.DetachBuffer()?;
    let bitmap =
        SoftwareBitmap::CreateCopyFromBuffer(&buffer, BitmapPixelFormat::Gray8, img.width() as i32, img.height() as i32)?;
    let result = engine.RecognizeAsync(&bitmap)?.join()?;
    let mut out = Vec::new();
    for (line, l) in result.Lines()?.into_iter().enumerate() {
        for w in l.Words()? {
            let r = w.BoundingRect()?;
            out.push(Word { line, text: w.Text()?.to_string(), x: r.X, y: r.Y, w: r.Width, h: r.Height });
        }
    }
    Ok(out)
}

/// One title candidate: adjacent words of one OCR line, center in band pixels.
struct Segment {
    text: String,
    cx: f32,
    cy: f32,
}

/// Splits OCR lines at wide gaps, so titles of neighboring cards read as one line stay apart.
fn segments(words: &[Word]) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut cur: Vec<&Word> = Vec::new();
    let flush = |cur: &mut Vec<&Word>, out: &mut Vec<Segment>| {
        let (Some(first), Some(last)) = (cur.first(), cur.last()) else { return };
        let text = cur.iter().map(|w| w.text.as_str()).collect::<Vec<_>>().join(" ");
        let cy = cur.iter().map(|w| w.y + w.h / 2.0).sum::<f32>() / cur.len() as f32;
        out.push(Segment { text, cx: (first.x + last.x + last.w) / 2.0, cy });
        cur.clear();
    };
    for w in words {
        if let Some(prev) = cur.last() {
            if prev.line != w.line || w.x - (prev.x + prev.w) > 1.5 * prev.h.max(w.h) {
                flush(&mut cur, &mut out);
            }
        }
        cur.push(w);
    }
    flush(&mut cur, &mut out);
    out
}

fn best_match(text: &str, names: &[Name]) -> Option<i64> {
    let t: Vec<char> = fold(text).chars().collect();
    if t.len() < 3 {
        return None;
    }
    let mut best: Option<(i64, f32)> = None;
    for n in names {
        let sim = 1.0 - levenshtein(&t, &n.folded) as f32 / t.len().max(n.folded.len()) as f32;
        if sim >= MIN_SIMILARITY && best.is_none_or(|(_, s)| sim > s) {
            best = Some((n.id, sim));
        }
    }
    best.map(|(id, _)| id)
}

/// Lowercase letters and digits only, Latin diacritics dropped (OCR often misses them).
fn fold(s: &str) -> String {
    s.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'ı' | 'î' | 'í' | 'ì' | 'ï' => 'i',
            'ğ' => 'g',
            'ü' | 'û' | 'ú' | 'ù' => 'u',
            'ş' | 'ș' | 'ß' => 's',
            'ö' | 'ô' | 'ó' | 'ò' | 'õ' => 'o',
            'ç' => 'c',
            'â' | 'à' | 'á' | 'ä' | 'ã' | 'å' => 'a',
            'é' | 'è' | 'ê' | 'ë' => 'e',
            'ñ' => 'n',
            'ý' | 'ÿ' => 'y',
            c => c,
        })
        .filter(|c| c.is_alphanumeric())
        .collect()
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut diag = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let next = (diag + usize::from(ca != cb)).min(row[j] + 1).min(row[j + 1] + 1);
            diag = row[j + 1];
            row[j + 1] = next;
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assets() -> Value {
        serde_json::json!([
            {"id": 205, "augmentNameId": "ADAPt", "nameTRA": "Yetenek Gücüne Güç Kat"},
            {"id": 1205, "augmentNameId": "ARAM_ADAPt", "nameTRA": "Yetenek Gücüne Güç Kat"},
            {"id": 1056, "augmentNameId": "ARAM_MindtoMatter", "nameTRA": "Zihinden Bedene"},
            {"id": 2018, "augmentNameId": "PureSkill", "nameTRA": "Saf Yetenek"},
            {"id": 1141, "augmentNameId": "ARAM_AllForYou", "nameTRA": "Hepsi Senin İçin"}
        ])
    }

    #[test]
    fn titles_match_despite_lost_diacritics_and_prefer_the_mayhem_augment() {
        let n = names(&assets());
        assert_eq!(best_match("Yetenek Gcne G Kat", &n), Some(1205));
        assert_eq!(best_match("Zihinden Bedene", &n), Some(1056));
        assert_eq!(best_match("Saf Yetenek", &n), Some(2018));
        assert_eq!(best_match("HEPSİ SENİN İÇİN", &n), Some(1141));
        // The rarity tag under a title is not an augment.
        assert_eq!(best_match("Genel", &n), None);
        assert_eq!(best_match("Direnç", &n), None);
    }

    #[test]
    fn titles_on_one_ocr_line_split_at_wide_gaps() {
        let w = |x: f32, text: &str| Word { line: 0, text: text.into(), x, y: 10.0, w: 30.0 * text.len() as f32 / 4.0, h: 30.0 };
        let words = [w(0.0, "Saf"), w(40.0, "Yetenek"), w(700.0, "Zihinden"), w(770.0, "Bedene")];
        let s = segments(&words);
        assert_eq!(s.iter().map(|s| s.text.as_str()).collect::<Vec<_>>(), ["Saf Yetenek", "Zihinden Bedene"]);
    }

    #[test]
    fn blitz_ranks_map_to_letters() {
        let v = serde_json::json!({"data": [
            {"augment_id": "1205", "stats": {"tier": 1}},
            {"augment_id": "1056", "stats": {"tier": 5}},
            {"augment_id": "x", "stats": {"tier": 2}},
            {"augment_id": "2018", "stats": {"tier": 9}}
        ]});
        let m = parse_tiers(&v).unwrap();
        assert_eq!((m.get(&1205), m.get(&1056), m.len()), (Some(&Tier::S), Some(&Tier::D), 2));
        assert!(parse_tiers(&serde_json::json!({"data": []})).is_err());
    }
}
