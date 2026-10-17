//! The single background thread. It polls local state (processes, LCU, Live
//! Client), runs champ-select automation and UI-requested actions, and
//! publishes a `Snapshot`; the UI only reads it and queues `Action`s.
//! Polling interval adapts: 1 s during champ select or a game, 2 s while the
//! client is open, 5 s when nothing Riot is running.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::sync::mpsc;

use parking_lot::Mutex;
use serde_json::Value;
use std::time::{Duration, Instant};

use crate::ddragon::{self, StaticData};
use crate::lcu::{
    self, Challenges, ChampSelect, Creds, GameSession, Lcu, LcuError, LolGame, LpChange, MatchDetail, Mastery, Mission,
    RankedQueue, RecPage, Summoner, TftGame,
};
use crate::live::{self, LiveGame};
use crate::riot::{self, Installs, Running};
use crate::settings::{PageSource, Settings};
use crate::skills::{Recorder, SkillHint};
use crate::meta::{self, MetaBuild, Pick, Picks, Query};
use crate::{itemset, net, win};

/// How long lock-in work that uses OP.GG (the item set, OP.GG runes and spells) waits for
/// its answer before falling back to your own build and Riot's page.
const META_WAIT: Duration = Duration::from_secs(20);
/// Label prefix of the OP.GG rune page among `rec_pages`.
const OPGG_LABEL: &str = "OP.GG";

const PAGE: usize = 20;

#[derive(Clone, PartialEq, Debug, Default)]
pub enum LcuStatus {
    #[default]
    ClientNotRunning,
    Connected {
        port: u16,
        source: &'static str,
    },
    Error(String),
}

/// Work the UI asks for; executed on the worker thread.
#[derive(Clone, Debug)]
pub enum Action {
    /// Apply `rec_pages[i]`'s runes.
    ApplyRunes(usize),
    /// Apply `rec_pages[i]`'s summoner spells.
    ApplySpells(usize),
    BenchSwap(i64),
    MatchDetail(u64),
    MoreHistory,
    /// `Name#TAG`
    Lookup(String),
    LookupMore,
    Spectate(String),
    LoadMissions,
    /// Riot-recommended runes + spells for every Swiftplay pre-pick slot.
    SwiftplayRunes,
    /// Riot's recommended pages for a champion in each of its positions and in ARAM.
    BrowseChampion(i64),
    /// Select an option in the OP.GG panel (build, rune page, spells, skill order).
    SelectMeta(Pick),
    /// Ask OP.GG again after a failed answer.
    RetryMeta,
}

#[derive(Clone, PartialEq, Debug)]
pub struct PostGame {
    pub detail: MatchDetail,
    pub lp: Option<LpChange>,
}

#[derive(Clone, PartialEq, Debug)]
pub struct Lookup {
    pub summoner: Summoner,
    pub ranked: Vec<RankedQueue>,
    pub history: Result<Vec<LolGame>, String>,
    pub mastery: Vec<Mastery>,
}

/// OP.GG's builds for the current pick.
#[derive(Clone, PartialEq, Debug)]
pub struct MetaState {
    pub champion_id: i64,
    /// `Err` when OP.GG has no table for this queue or no lane is known.
    pub query: Result<Query, String>,
    /// `None` while loading (or waiting for Data Dragon's items).
    pub build: Option<Result<MetaBuild, String>>,
    /// What is shown, offered as the OP.GG rune page and written on lock-in.
    pub picks: Picks,
}

pub struct Snapshot {
    pub installs: Installs,
    pub running: Running,
    pub lcu: LcuStatus,
    pub creds: Option<Creds>,
    pub phase: String,
    pub session: Option<GameSession>,
    pub summoner: Option<Summoner>,
    pub profile_error: Option<String>,
    pub ranked: Vec<RankedQueue>,
    pub mastery: Vec<Mastery>,
    pub challenges: Option<Challenges>,
    pub missions: Option<Result<Vec<Mission>, String>>,
    pub lol_history: Result<Vec<LolGame>, String>,
    /// The client returned fewer games than asked; no more pages.
    pub history_complete: bool,
    pub tft_history: Result<Vec<TftGame>, String>,
    pub match_details: HashMap<u64, Result<MatchDetail, String>>,
    pub champ_select: Option<ChampSelect>,
    /// Riot's recommended pages for the champion currently picked/hovered.
    pub rec_pages: Vec<RecPage>,
    /// Champions the player may pick in this champ select (owned, not banned).
    pub pickable: Vec<i64>,
    /// Champion browser: recommended pages per position for one champion.
    pub browse: Option<(i64, Result<Vec<RecPage>, String>)>,
    /// OP.GG build for the picked/hovered champion (when enabled in settings).
    pub meta: Option<MetaState>,
    /// What champ-select automation did this session (message, ok).
    pub champ_log: Vec<(String, bool)>,
    /// Ranked queues per puuid (champ select and game scouting, cached).
    pub ranks: HashMap<String, Result<Vec<RankedQueue>, String>>,
    pub post_game: Option<PostGame>,
    pub live: Option<LiveGame>,
    pub skill_hint: Option<SkillHint>,
    pub lookup: Option<Result<Lookup, String>>,
    pub busy: Option<&'static str>,
    /// Result of the last UI action: (sequence, message, ok).
    pub action_result: Option<(u64, String, bool)>,
    pub data: Arc<StaticData>,
    pub data_error: Option<String>,
}

impl Default for Snapshot {
    fn default() -> Self {
        Snapshot {
            installs: Installs::default(),
            running: Running::default(),
            lcu: LcuStatus::default(),
            creds: None,
            phase: String::new(),
            session: None,
            summoner: None,
            profile_error: None,
            ranked: Vec::new(),
            mastery: Vec::new(),
            challenges: None,
            missions: None,
            lol_history: Ok(Vec::new()),
            history_complete: false,
            tft_history: Ok(Vec::new()),
            match_details: HashMap::new(),
            champ_select: None,
            rec_pages: Vec::new(),
            pickable: Vec::new(),
            browse: None,
            meta: None,
            champ_log: Vec::new(),
            ranks: HashMap::new(),
            post_game: None,
            live: None,
            skill_hint: None,
            lookup: None,
            busy: None,
            action_result: None,
            data: Arc::default(),
            data_error: None,
        }
    }
}

impl Snapshot {
    pub fn league_root(&self) -> Option<std::path::PathBuf> {
        riot::league_root(&self.installs, self.creds.as_ref().and_then(|c| c.client_exe.as_deref()))
    }
}

pub struct Shared {
    pub snap: Mutex<Snapshot>,
    pub refresh: AtomicBool,
    /// Toggled by the Ctrl+Shift+O hotkey / tray menu; not persisted.
    pub overlay_hidden: AtomicBool,
    pub settings: Mutex<Settings>,
    actions: Mutex<Vec<Action>>,
}

impl Shared {
    pub fn request(&self, a: Action) {
        self.actions.lock().push(a);
    }
}

/// Assigns only when the value differs; returns whether it changed.
fn set<T: PartialEq>(slot: &mut T, v: T) -> bool {
    if *slot != v {
        *slot = v;
        true
    } else {
        false
    }
}

pub fn spawn(ctx: eframe::egui::Context, settings: Settings) -> Arc<Shared> {
    let shared = Arc::new(Shared {
        snap: Mutex::new(Snapshot::default()),
        refresh: AtomicBool::new(true),
        overlay_hidden: AtomicBool::new(false),
        settings: Mutex::new(settings),
        actions: Mutex::new(Vec::new()),
    });
    let s = shared.clone();
    let c = ctx.clone();
    std::thread::Builder::new()
        .name("riftlite-ddragon".into())
        .spawn(move || {
            let r = ddragon::load(&net::public_agent());
            let mut snap = s.snap.lock();
            match r {
                Ok(d) => snap.data = Arc::new(d),
                Err(e) => snap.data_error = Some(e),
            }
            drop(snap);
            c.request_repaint();
        })
        .expect("spawn ddragon thread");
    let s = shared.clone();
    std::thread::Builder::new().name("riftlite-poll".into()).spawn(move || poll_loop(&s, &ctx)).expect("spawn poll thread");
    shared
}

struct Conn {
    creds: Creds,
    lcu: Lcu,
    /// `/lol-game-data` names (items, perks, styles, spells), fetched once per connection.
    assets: Option<[Value; 4]>,
}

/// Champ-select automation state for one session.
#[derive(Default)]
struct Auto {
    session: String,
    map_id: i64,
    /// Champion the recommended pages were fetched for.
    rec_for: Option<i64>,
    /// Champion automation last ran for (re-runs after an ARAM swap/reroll).
    applied: Option<i64>,
    /// Lane used for the pick's recommendations (`lcu::perk_position` form).
    position: String,
    /// OP.GG builds for this session by champion and mode/lane. Only answers that worked are
    /// kept, so a failed request is asked again (next pick of the champion, or Retry).
    meta_cache: HashMap<(i64, Query), MetaBuild>,
    /// Requests running on their own threads.
    meta_inflight: HashSet<(i64, Query)>,
    meta_chan: Option<(mpsc::Sender<MetaAnswer>, mpsc::Receiver<MetaAnswer>)>,
    /// Lock-in work waiting for OP.GG.
    lockin: Option<LockIn>,
}

type MetaAnswer = ((i64, Query), Result<MetaBuild, String>);

/// What OP.GG gives lock-in work: the build and picks, why there is none, or `None` when
/// OP.GG builds are off for this pick.
type MetaUse = Option<Result<(MetaBuild, Picks), String>>;

/// Lock-in work that needs OP.GG's answer (or `META_WAIT` to pass) before it runs.
#[derive(Clone, Copy, Debug)]
struct LockIn {
    champion: i64,
    since: Instant,
    /// Apply the OP.GG rune page / spells (`PageSource::OpGg`).
    runes: bool,
    spells: bool,
    /// Write the item set.
    items: bool,
}

struct State {
    conn: Option<Conn>,
    last_phase: String,
    last_profile: Option<Instant>,
    history_len: usize,
    lookup_puuid: Option<String>,
    lookup_len: usize,
    auto: Auto,
    session_checked: Option<Instant>,
    eog_game: u64,
    recorder: Recorder,
    action_seq: u64,
    /// The current ready check was answered (by riftlite or the player); reset when the phase leaves ReadyCheck.
    ready_handled: bool,
}

fn poll_loop(shared: &Shared, ctx: &eframe::egui::Context) {
    let local = net::riot_local_agent();
    let mut st = State {
        conn: None,
        last_phase: String::new(),
        last_profile: None,
        history_len: PAGE,
        lookup_puuid: None,
        lookup_len: 0,
        auto: Auto::default(),
        session_checked: None,
        eog_game: 0,
        recorder: Recorder::default(),
        action_seq: 0,
        ready_handled: false,
    };
    let mut last_install_scan: Option<Instant> = None;
    loop {
        let mut changed = false;
        let forced = shared.refresh.swap(false, Ordering::Relaxed);

        if forced || last_install_scan.is_none_or(|t| t.elapsed() > Duration::from_secs(60)) {
            let installs = riot::scan_installs(shared.settings.lock().league_override().as_deref());
            changed |= set(&mut shared.snap.lock().installs, installs);
            last_install_scan = Some(Instant::now());
        }

        let running = Running::from_processes(&win::processes());
        changed |= set(&mut shared.snap.lock().running, running);

        // --- LCU connection ---
        match running.league_client {
            None => {
                if st.conn.take().is_some() || st.last_phase.is_empty() {
                    let mut s = shared.snap.lock();
                    changed |= set(&mut s.lcu, LcuStatus::ClientNotRunning);
                    s.creds = None;
                    changed |= set(&mut s.phase, String::new());
                    s.champ_select = None;
                    s.session = None;
                }
                st.last_phase = "-".into();
            }
            Some(pid) if st.conn.as_ref().is_none_or(|c| c.creds.pid != pid) => {
                let league = shared.snap.lock().installs.get(riot::Product::League).map(|i| i.path.clone());
                match lcu::discover(pid, league.as_deref()) {
                    Ok(creds) => {
                        st.conn = Some(Conn { lcu: Lcu::new(local.clone(), &creds), creds, assets: None });
                        st.last_profile = None;
                    }
                    Err(e) => changed |= set(&mut shared.snap.lock().lcu, LcuStatus::Error(e)),
                }
            }
            Some(_) => {}
        }

        if st.conn.is_some() {
            changed |= poll_lcu(shared, &mut st, forced);
        }

        // --- Live game ---
        let live_game = if running.any_game() { live::fetch(&local) } else { None };
        let hint = match &live_game {
            Some(g) => g.me().and_then(|me| st.recorder.observe(&me.champion_key, g.abilities, g.unspent_skill_points())),
            None => {
                st.recorder.finish();
                None
            }
        };
        let mut s = shared.snap.lock();
        let live_changed = set(&mut s.live, live_game) | set(&mut s.skill_hint, hint);
        drop(s);
        if live_changed {
            changed = true;
            ctx.request_repaint_of(crate::ui::overlay_id());
        }

        // --- UI actions ---
        let actions = std::mem::take(&mut *shared.actions.lock());
        for a in actions {
            run_action(shared, &mut st, a);
            changed = true;
        }

        if changed {
            ctx.request_repaint();
        }
        let s = shared.snap.lock();
        // A ready check lasts ~10 s: poll it at 1 s like champ select; other phases are unaffected.
        let busy = s.live.is_some() || s.champ_select.is_some() || running.any_game() || s.phase == "ReadyCheck";
        let idle = running.league_client.is_none() && running.riot_client.is_none();
        drop(s);
        let wait = if busy { 1000 } else if idle { 5000 } else { 2000 };
        // Sleep in slices so a UI refresh or action is picked up quickly.
        for _ in 0..wait / 100 {
            if shared.refresh.load(Ordering::Relaxed) || !shared.actions.lock().is_empty() {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}

fn poll_lcu(shared: &Shared, st: &mut State, forced: bool) -> bool {
    let c = st.conn.as_mut().expect("connected");
    let phase = match c.lcu.phase() {
        Ok(p) => p,
        Err(e) => {
            // Client restarting or shutting down: rediscover on the next tick.
            st.conn = None;
            return set(&mut shared.snap.lock().lcu, LcuStatus::Error(format!("LCU: {e}")));
        }
    };
    let mut changed = false;
    {
        let mut s = shared.snap.lock();
        changed |= set(&mut s.lcu, LcuStatus::Connected { port: c.creds.port, source: c.creds.source });
        s.creds = Some(c.creds.clone());
        changed |= set(&mut s.phase, phase.clone());
    }

    // Client asset names for ids Data Dragon lacks; merged once both are loaded.
    if c.assets.is_none() {
        let get = |p: &str| c.lcu.get(&format!("/lol-game-data/assets/v1/{p}")).unwrap_or(Value::Null);
        let a = [get("items.json"), get("perks.json"), get("perkstyles.json"), get("summoner-spells.json")];
        if a.iter().all(|v| !v.is_null()) {
            c.assets = Some(a);
        }
    }
    if let Some([items, perks, styles, spells]) = &c.assets {
        let mut s = shared.snap.lock();
        if !s.data.version.is_empty() && !s.data.has_client_assets {
            s.data = Arc::new(s.data.with_client_assets(items, perks, styles, spells));
            changed = true;
        }
    }

    let phase_changed = phase != st.last_phase;
    let in_game_phase = matches!(phase.as_str(), "GameStart" | "InProgress" | "Reconnect");
    let session_stale = in_game_phase
        && shared.snap.lock().session.as_ref().is_none_or(|s| s.players.is_empty())
        && st.session_checked.is_none_or(|t| t.elapsed() > Duration::from_secs(10));
    if phase_changed || session_stale {
        let session = if phase == "None" { None } else { c.lcu.session() };
        st.session_checked = Some(Instant::now());
        changed |= set(&mut shared.snap.lock().session, session);
        if phase_changed {
            // Match history and LP change after a game; refresh then.
            if matches!(phase.as_str(), "EndOfGame" | "PreEndOfGame" | "None" | "Lobby") {
                st.last_profile = None;
            }
            st.last_phase = phase.clone();
        }
    }
    if forced || st.last_profile.is_none_or(|t| t.elapsed() > Duration::from_secs(300)) {
        changed |= refresh_profile(shared, &c.lcu, st.history_len);
        st.last_profile = Some(Instant::now());
    }

    // --- Champ select ---
    let cs = if phase == "ChampSelect" { c.lcu.champ_select().ok().flatten() } else { None };
    if let Some(cs) = &cs {
        changed |= champ_select_automation(shared, &c.lcu, &mut st.auto, cs);
        let puuids: Vec<String> = cs.my_team.iter().chain(&cs.their_team).map(|p| p.puuid.clone()).collect();
        changed |= fetch_ranks(shared, &c.lcu, puuids);
    } else if !st.auto.session.is_empty() {
        st.auto = Auto::default();
    }
    changed |= set(&mut shared.snap.lock().champ_select, cs);

    // --- Loading screen / in game scouting ---
    if in_game_phase {
        let puuids: Vec<String> =
            shared.snap.lock().session.iter().flat_map(|s| s.players.iter().map(|p| p.puuid.clone())).collect();
        changed |= fetch_ranks(shared, &c.lcu, puuids);
    }

    // --- Post game ---
    if phase == "EndOfGame" {
        if let Ok(Some(detail)) = c.lcu.end_of_game() {
            if detail.game_id != st.eog_game {
                st.eog_game = detail.game_id;
                let lp = c.lcu.lp_change().filter(|l| l.game_id == 0 || l.game_id == detail.game_id);
                let mut detail = detail;
                // The end-of-game block names the mode only; the gameflow session still knows the queue.
                detail.queue_id = shared.snap.lock().session.as_ref().map_or(0, |s| s.queue_id);
                shared.snap.lock().post_game = Some(PostGame { detail, lp });
                changed = true;
            }
        }
    }

    // --- Ready check (only queried while the gameflow phase says one is open) ---
    if phase != "ReadyCheck" {
        st.ready_handled = false;
    } else if !st.ready_handled && shared.settings.lock().auto_accept {
        if let Ok(Some(rc)) = c.lcu.ready_check() {
            match lcu::ready_check_action(&rc) {
                lcu::ReadyAction::Accept => {
                    st.ready_handled = true;
                    let r = c.lcu.accept_ready_check().map(|_| "Match found: ready check accepted".to_string());
                    finish_action(shared, st, r.map_err(|e| format!("Auto-accept failed: {e}")));
                    changed = true;
                }
                // The player already answered (accepted or declined): leave this check alone.
                lcu::ReadyAction::Answered => st.ready_handled = true,
                lcu::ReadyAction::Wait => {}
            }
        }
    }
    changed
}

/// Ranks for players not seen yet; failures are cached too so they are not retried every second.
fn fetch_ranks(shared: &Shared, lcu: &Lcu, puuids: Vec<String>) -> bool {
    let missing: Vec<String> = {
        let s = shared.snap.lock();
        puuids.into_iter().filter(|p| !p.is_empty() && !s.ranks.contains_key(p)).collect()
    };
    for p in &missing {
        let r = lcu.ranked_of(p).map_err(|e| e.to_string());
        shared.snap.lock().ranks.insert(p.clone(), r);
    }
    !missing.is_empty()
}

fn log(shared: &Shared, msg: String, ok: bool) {
    shared.snap.lock().champ_log.push((msg, ok));
}

fn champ_select_automation(shared: &Shared, lcu: &Lcu, auto: &mut Auto, cs: &ChampSelect) -> bool {
    let mut changed = false;
    if cs.session_id != auto.session {
        *auto = Auto { session: cs.session_id.clone(), ..Auto::default() };
        let mut s = shared.snap.lock();
        s.champ_log.clear();
        s.rec_pages.clear();
        s.meta = None;
        auto.map_id = s.session.as_ref().map(|g| g.map_id).filter(|&m| m > 0).unwrap_or(0);
        drop(s);
        let pickable = lcu.pickable_champions();
        shared.snap.lock().pickable = pickable;
        if auto.map_id == 0 {
            auto.map_id = lcu.session().map(|g| g.map_id).filter(|&m| m > 0).unwrap_or(live::SUMMONERS_RIFT);
        }
        changed = true;
    }
    let champ = cs.my_champion();
    if champ != auto.rec_for {
        auto.rec_for = champ;
        let pages = match champ {
            Some(c) => {
                let mut pos = lcu::perk_position(cs.me().map_or("", |m| m.position.as_str())).to_string();
                if pos == "NONE" && auto.map_id == live::SUMMONERS_RIFT {
                    pos = lcu.recommended_positions(c).into_iter().next().unwrap_or(pos);
                }
                auto.position = pos.clone();
                match lcu.recommended_pages(c, &pos, auto.map_id) {
                    Ok(p) => p,
                    Err(e) => {
                        log(shared, format!("Recommended runes unavailable: {e}"), false);
                        Vec::new()
                    }
                }
            }
            None => Vec::new(),
        };
        let meta = match champ.filter(|_| shared.settings.lock().meta_builds) {
            None => None,
            Some(c) => {
                let query = meta::query_for(cs.queue_id, auto.map_id, &auto.position);
                let build = match &query {
                    Err(e) => Some(Err(e.clone())),
                    // Not cached yet: `sync_meta` requests it.
                    Ok(q) => auto.meta_cache.get(&(c, *q)).cloned().map(Ok),
                };
                Some(MetaState { champion_id: c, query, build, picks: Picks::default() })
            }
        };
        let confident = shared.settings.lock().meta_sort_confident;
        let mut s = shared.snap.lock();
        s.rec_pages = pages;
        s.meta = meta;
        if let Some(m) = s.meta.as_mut() {
            adopt_build(m, confident);
        }
        sync_meta_page(&mut s);
        changed = true;
    }
    changed |= sync_meta(shared, auto);
    if cs.locked && champ.is_some() && auto.applied != champ {
        auto.applied = champ;
        let champion = champ.expect("checked");
        let prefs = shared.settings.lock().clone();
        let opgg_pages = prefs.page_source == PageSource::OpGg && prefs.meta_builds;
        if !opgg_pages {
            // Riot's page is already here. Outcomes are recorded in `champ_log` for the Champ select tab.
            if prefs.auto_runes {
                let _ = apply_runes(shared, lcu, 0, cs);
            }
            if prefs.auto_spells {
                let _ = apply_spells(shared, lcu, 0, cs);
            }
        }
        let job = LockIn {
            champion,
            since: Instant::now(),
            runes: opgg_pages && prefs.auto_runes,
            spells: opgg_pages && prefs.auto_spells,
            items: prefs.auto_item_set,
        };
        auto.lockin = (job.runes || job.spells || job.items).then_some(job);
        changed = true;
    }
    if let Some(job) = auto.lockin {
        // A swap after lock-in re-arms the automation for the new champion.
        if champ != Some(job.champion) {
            auto.lockin = None;
            changed = true;
        } else {
            let meta_on = shared.settings.lock().meta_builds;
            let ready = lockin_meta(shared.snap.lock().meta.as_ref().filter(|_| meta_on), job.champion, job.since.elapsed());
            if let Some(meta) = ready {
                if job.runes || job.spells {
                    apply_opgg_pages(shared, lcu, cs, job, &meta);
                }
                if job.items {
                    lock_in_item_set(shared, job.champion, meta, meta::mode_for(cs.queue_id, auto.map_id).ok());
                }
                auto.lockin = None;
                changed = true;
            }
        }
    }
    changed
}

/// Requests the pick's OP.GG builds when they are missing, takes in answers, and keeps the
/// shown builds ranked as Settings ask.
fn sync_meta(shared: &Shared, auto: &mut Auto) -> bool {
    let mut changed = false;
    let confident = shared.settings.lock().meta_sort_confident;
    let want = {
        let s = shared.snap.lock();
        // Classifying needs Data Dragon's items; until they load the panel shows "loading".
        s.meta
            .as_ref()
            .filter(|m| m.build.is_none() && !s.data.item_stats.is_empty())
            .and_then(|m| Some(((m.champion_id, *m.query.as_ref().ok()?), s.data.clone())))
    };
    if let Some((key, data)) = want.filter(|(k, _)| !auto.meta_inflight.contains(k)) {
        auto.meta_inflight.insert(key);
        if let Err(e) = request_meta(auto, data, key, confident) {
            auto.meta_inflight.remove(&key);
            if let Some(m) = shared.snap.lock().meta.as_mut() {
                m.build = Some(Err(e));
            }
            changed = true;
        }
    }
    if let Some((_, rx)) = &auto.meta_chan {
        for (key, r) in rx.try_iter() {
            auto.meta_inflight.remove(&key);
            if let Ok(b) = &r {
                auto.meta_cache.insert(key, b.clone());
            }
            let mut s = shared.snap.lock();
            if let Some(m) = s.meta.as_mut().filter(|m| m.champion_id == key.0 && m.query.as_ref().ok() == Some(&key.1)) {
                m.build = Some(r);
                adopt_build(m, confident);
                sync_meta_page(&mut s);
                changed = true;
            }
        }
    }
    // The ranking setting changed while builds are shown: re-rank, keeping the picked build.
    let mut s = shared.snap.lock();
    if let Some(m) = s.meta.as_mut()
        && let Some(Ok(b)) = &mut m.build
        && b.confident != confident
    {
        let picked = b.cores.get(m.picks.build).map(|c| c.items.ids.clone());
        b.sort(confident);
        m.picks.build = picked.and_then(|ids| b.index_of(&ids)).unwrap_or(0);
        changed = true;
    }
    changed
}

/// A build just arrived for `m`: rank it as Settings ask and start on the build last picked
/// for this champion and mode (else the best one) and OP.GG's most played options.
fn adopt_build(m: &mut MetaState, confident: bool) {
    let Some(Ok(b)) = &mut m.build else { return };
    if b.confident != confident {
        b.sort(confident);
    }
    let build = meta::saved_build(m.champion_id, b.query.mode).and_then(|ids| b.index_of(&ids)).unwrap_or(0);
    m.picks = Picks { build, ..Picks::default() };
}

fn request_meta(auto: &mut Auto, data: Arc<StaticData>, key: (i64, Query), confident: bool) -> Result<(), String> {
    let (tx, _) = auto.meta_chan.get_or_insert_with(mpsc::channel);
    let tx = tx.clone();
    std::thread::Builder::new()
        .name("riftlite-opgg".into())
        .spawn(move || {
            let r = meta::fetch(&net::public_agent(), &data, key.0, key.1, confident);
            let _ = tx.send((key, r));
        })
        .map(drop)
        .map_err(|e| format!("could not start the OP.GG request: {e}"))
}

/// Keeps exactly one OP.GG page (the picked runes and spells, when OP.GG has both) after Riot's pages.
fn sync_meta_page(s: &mut Snapshot) {
    s.rec_pages.retain(|p| !p.label.starts_with(OPGG_LABEL));
    let page = s.meta.as_ref().and_then(|m| match &m.build {
        Some(Ok(b)) => meta_page(b, m.picks),
        _ => None,
    });
    s.rec_pages.extend(page);
}

fn meta_page(b: &MetaBuild, picks: Picks) -> Option<RecPage> {
    let r = b.runes.get(picks.runes)?;
    let spells = b.spells.get(picks.spells)?;
    let which = if picks.runes == 0 { "most played runes".to_string() } else { format!("runes option {}", picks.runes + 1) };
    Some(RecPage {
        position: b.query.position.to_uppercase(),
        primary_style: r.primary_page_id,
        sub_style: r.secondary_page_id,
        perks: r.perks()?,
        spells: [spells.ids[0], spells.ids[1]],
        label: format!("{OPGG_LABEL} {} · {which} · {}", b.query.label(), r.stats()),
    })
}

/// OP.GG's part of lock-in work: `None` while it should keep waiting (up to `META_WAIT`),
/// else what it gets (see `MetaUse`).
fn lockin_meta(meta: Option<&MetaState>, champion: i64, waited: Duration) -> Option<MetaUse> {
    let Some(m) = meta.filter(|m| m.champion_id == champion) else { return Some(None) };
    match &m.build {
        None if waited < META_WAIT => None,
        None => Some(Some(Err("OP.GG did not answer in time".into()))),
        Some(r) => Some(Some(r.clone().map(|b| (b, m.picks)))),
    }
}

/// Lock-in runes and spells from the picked OP.GG page, or from Riot's page (`rec_pages[0]`)
/// when OP.GG has none for this pick; the reason is logged.
fn apply_opgg_pages(shared: &Shared, lcu: &Lcu, cs: &ChampSelect, job: LockIn, meta: &MetaUse) {
    let opgg = shared.snap.lock().rec_pages.iter().position(|p| p.label.starts_with(OPGG_LABEL));
    let i = opgg.unwrap_or_else(|| {
        let why = match meta {
            Some(Err(e)) => e.clone(),
            Some(Ok(_)) => "OP.GG has no rune page for this mode".into(),
            None => "OP.GG builds are off".into(),
        };
        log(shared, format!("OP.GG runes and spells unavailable ({why}); using Riot's recommended page"), false);
        0
    });
    if job.runes {
        let _ = apply_runes(shared, lcu, i, cs);
    }
    if job.spells {
        let _ = apply_spells(shared, lcu, i, cs);
    }
}

/// Writes the lock-in item set from OP.GG's build, else from your best game on the champion
/// in `mode` (the OP.GG mode of this queue).
fn lock_in_item_set(shared: &Shared, champ: i64, meta: MetaUse, mode: Option<&str>) {
    let s = shared.snap.lock();
    let name = s.data.champion_name(champ);
    let history = s.lol_history.as_deref().unwrap_or(&[]);
    let best = itemset::best_game(history, champ, mode);
    let r = match (s.league_root(), meta) {
        (None, _) => Err("League install not found; set it in Settings".to_string()),
        (Some(root), Some(Ok((b, picks)))) => itemset::write_meta(&root, &s.data, &b, picks, best).map(|_| {
            let own = if best.is_some() { " + your best game" } else { "" };
            let c = &b.cores[picks.build];
            format!("Item set for {name} written: OP.GG {} {} ({}){own}", b.query.label(), c.label(), c.items.stats())
        }),
        (Some(root), meta) => {
            let why = match meta {
                Some(Err(e)) => format!(" ({e})"),
                _ => String::new(),
            };
            match best {
                None => Err(format!("No {name} game with items in this mode in your last {} matches{why}", history.len())),
                Some(g) => itemset::write_from_game(&root, &s.data, g).map(|_| format!("Item set for {name} written from your best game{why}")),
            }
        }
    };
    drop(s);
    match r {
        Ok(m) => log(shared, m, true),
        Err(m) => log(shared, format!("Item set: {m}"), false),
    }
}

/// Applies a selection in the OP.GG panel. A picked build is remembered per champion and
/// mode. A new build or skill order rewrites the lock-in item set if it was already written;
/// rune pages and spells update the OP.GG page among the recommended pages.
fn select_meta(shared: &Shared, auto: &Auto, pick: Pick) -> Result<String, String> {
    let (champ, msg, save, rewrite) = {
        let mut s = shared.snap.lock();
        let m = s.meta.as_mut().ok_or("no OP.GG build loaded")?;
        let Some(Ok(b)) = &m.build else { return Err("no OP.GG build loaded".into()) };
        let missing = || "no such OP.GG option".to_string();
        let (msg, save, rewrite) = match pick {
            Pick::Build(i) => {
                let c = b.cores.get(i).ok_or_else(missing)?;
                m.picks.build = i;
                (format!("OP.GG {} build selected", c.label()), Some((b.query.mode, c.items.ids.clone())), true)
            }
            Pick::Runes(i) => {
                b.runes.get(i).ok_or_else(missing)?;
                m.picks.runes = i;
                ("OP.GG rune page selected".to_string(), None, false)
            }
            Pick::Spells(i) => {
                b.spells.get(i).ok_or_else(missing)?;
                m.picks.spells = i;
                ("OP.GG summoner spells selected".to_string(), None, false)
            }
            Pick::Skills(i) => {
                b.skills.get(i).ok_or_else(missing)?;
                m.picks.skills = i;
                ("OP.GG skill order selected".to_string(), None, true)
            }
        };
        let champ = m.champion_id;
        sync_meta_page(&mut s);
        (champ, msg, save, rewrite)
    };
    if let Some((mode, ids)) = &save {
        // Losing a remembered choice is harmless: the best build is the default.
        let _ = meta::save_build(champ, mode, ids);
    }
    let written = rewrite && auto.applied == Some(champ) && auto.lockin.is_none() && shared.settings.lock().auto_item_set;
    if !written {
        return Ok(msg);
    }
    let meta = lockin_meta(shared.snap.lock().meta.as_ref(), champ, META_WAIT).flatten();
    let mode = meta.as_ref().and_then(|r| r.as_ref().ok()).map(|(b, _)| b.query.mode);
    lock_in_item_set(shared, champ, meta, mode);
    Ok(format!("{msg}; item set rewritten"))
}

fn apply_runes(shared: &Shared, lcu: &Lcu, i: usize, cs: &ChampSelect) -> Result<String, String> {
    let (rec, champ) = {
        let s = shared.snap.lock();
        (s.rec_pages.get(i).cloned(), cs.my_champion().map(|c| s.data.champion_name(c)))
    };
    let r = match rec {
        None => Err("no recommended page for this pick".to_string()),
        Some(rec) => lcu
            .apply_runes(&rec, champ.as_deref().unwrap_or("pick"))
            .map(|name| format!("Runes applied to page \"{name}\" ({})", rec.label))
            .map_err(|e| format!("Runes: {e}")),
    };
    match &r {
        Ok(m) => log(shared, m.clone(), true),
        Err(m) => log(shared, m.clone(), false),
    }
    r
}

fn apply_spells(shared: &Shared, lcu: &Lcu, i: usize, cs: &ChampSelect) -> Result<String, String> {
    let (rec, data) = {
        let s = shared.snap.lock();
        (s.rec_pages.get(i).cloned(), s.data.clone())
    };
    let r = match rec {
        None => Err("no recommended spells for this pick".to_string()),
        Some(rec) if rec.spells.contains(&0) => Err("Riot recommends no spells here".to_string()),
        Some(rec) => lcu
            .apply_spells(rec.spells, cs.my_spells)
            .map(|s| format!("Spells set: {} + {}", data.spell_name(s[0]), data.spell_name(s[1])))
            .map_err(|e| format!("Spells: {e}")),
    };
    match &r {
        Ok(m) => log(shared, m.clone(), true),
        Err(m) => log(shared, m.clone(), false),
    }
    r
}

/// Manual buttons act on the champ select the UI is showing.
fn with_champ_select(shared: &Shared, f: impl FnOnce(&ChampSelect) -> Result<String, String>) -> Result<String, String> {
    let cs = shared.snap.lock().champ_select.clone();
    cs.as_ref().map_or(Err("not in champ select".into()), f)
}

fn run_action(shared: &Shared, st: &mut State, a: Action) {
    let Some(c) = &st.conn else {
        return finish_action(shared, st, Err("League client not connected".into()));
    };
    let lcu = &c.lcu;
    let r: Result<String, String> = match a {
        Action::ApplyRunes(i) => with_champ_select(shared, |cs| apply_runes(shared, lcu, i, cs)),
        Action::ApplySpells(i) => with_champ_select(shared, |cs| apply_spells(shared, lcu, i, cs)),
        Action::BenchSwap(champ) => {
            let name = shared.snap.lock().data.champion_name(champ);
            lcu.bench_swap(champ).map(|_| format!("Swapped to {name}")).map_err(|e| format!("Bench swap: {e}"))
        }
        Action::MatchDetail(id) => {
            let me = shared.snap.lock().summoner.as_ref().map(|s| s.puuid.clone()).unwrap_or_default();
            let r = lcu.match_detail(id).map(|d| d.with_me(&me)).map_err(|e| e.to_string());
            let msg = r.as_ref().err().map(|e| format!("Match {id}: {e}"));
            shared.snap.lock().match_details.insert(id, r);
            msg.map_or(Ok(String::new()), Err)
        }
        Action::MoreHistory => {
            let puuid = shared.snap.lock().summoner.as_ref().map(|s| s.puuid.clone()).unwrap_or_default();
            match lcu.lol_history(&puuid, st.history_len, st.history_len + PAGE - 1) {
                Ok(more) => {
                    st.history_len += PAGE;
                    let mut s = shared.snap.lock();
                    s.history_complete = more.len() < PAGE;
                    let n = more.len();
                    if let Ok(h) = &mut s.lol_history {
                        let fresh: Vec<LolGame> = more.into_iter().filter(|g| !h.iter().any(|x| x.game_id == g.game_id)).collect();
                        h.extend(fresh);
                    }
                    Ok(format!("Loaded {n} more games"))
                }
                Err(e) => Err(format!("Match history: {e}")),
            }
        }
        Action::Lookup(riot_id) => {
            shared.snap.lock().busy = Some("Looking up player…");
            let r = lookup(lcu, &riot_id);
            st.lookup_puuid = r.as_ref().ok().map(|l| l.summoner.puuid.clone());
            st.lookup_len = PAGE;
            let msg = r.as_ref().err().cloned();
            let mut s = shared.snap.lock();
            s.lookup = Some(r);
            s.busy = None;
            msg.map_or(Ok(String::new()), Err)
        }
        Action::LookupMore => match st.lookup_puuid.clone() {
            None => Err("no player looked up".into()),
            Some(p) => match lcu.lol_history_of(&p, st.lookup_len, st.lookup_len + PAGE - 1) {
                Ok(more) => {
                    st.lookup_len += PAGE;
                    let n = more.len();
                    if let Some(Ok(l)) = &mut shared.snap.lock().lookup {
                        if let Ok(h) = &mut l.history {
                            h.extend(more);
                        }
                    }
                    Ok(format!("Loaded {n} more games"))
                }
                Err(e) => Err(format!("Match history: {e}")),
            },
        },
        Action::Spectate(puuid) => lcu
            .spectate(&puuid)
            .map(|_| "Spectator requested; the League client starts it".to_string())
            .map_err(|e| format!("Spectate: {e} (the player must be in a game that can be spectated)")),
        Action::SelectMeta(pick) => select_meta(shared, &st.auto, pick),
        Action::RetryMeta => {
            let mut s = shared.snap.lock();
            match s.meta.as_mut() {
                // `sync_meta` asks again on the next poll.
                Some(m) if m.query.is_ok() && matches!(m.build, Some(Err(_))) => {
                    m.build = None;
                    Ok("Asking OP.GG again".to_string())
                }
                _ => Err("No failed OP.GG request to retry".to_string()),
            }
        }
        Action::BrowseChampion(champ) => {
            let mut positions = lcu.recommended_positions(champ);
            positions.truncate(2);
            let mut pages = Vec::new();
            let mut err = None;
            let maps = positions.iter().map(|p| (p.as_str(), live::SUMMONERS_RIFT)).chain([("NONE", 12)]);
            for (pos, map) in maps {
                match lcu.recommended_pages(champ, pos, map) {
                    Ok(p) => pages.extend(p.into_iter().take(1).map(|mut p| {
                        p.position = if map == 12 { "ARAM".into() } else { pos.to_string() };
                        p
                    })),
                    Err(e) => err = Some(e.to_string()),
                }
            }
            let r = match (pages.is_empty(), err) {
                (true, Some(e)) => Err(e),
                _ => Ok(pages),
            };
            shared.snap.lock().browse = Some((champ, r));
            Ok(String::new())
        }
        Action::LoadMissions => {
            let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as i64);
            let r = lcu.missions(now).map_err(|e| e.to_string());
            let msg = r.as_ref().err().map(|e| format!("Missions: {e}"));
            shared.snap.lock().missions = Some(r);
            msg.map_or(Ok(String::new()), Err)
        }
        Action::SwiftplayRunes => {
            let data = shared.snap.lock().data.clone();
            swiftplay_runes(lcu, &data)
        }
    };
    finish_action(shared, st, r);
}

fn finish_action(shared: &Shared, st: &mut State, r: Result<String, String>) {
    let (msg, ok) = match r {
        Ok(m) if m.is_empty() => return,
        Ok(m) => (m, true),
        Err(m) => (m, false),
    };
    st.action_seq += 1;
    shared.snap.lock().action_result = Some((st.action_seq, msg, ok));
}

fn lookup(lcu: &Lcu, riot_id: &str) -> Result<Lookup, String> {
    let (name, tag) = riot_id.trim().rsplit_once('#').ok_or("enter a Riot ID as Name#TAG")?;
    let puuid = lcu.lookup(name.trim(), tag.trim()).map_err(|e| match e {
        LcuError::Status(404) => format!("{riot_id}: no such player on this region"),
        e => format!("lookup: {e}"),
    })?;
    let summoner = lcu.summoner_by_puuid(&puuid).unwrap_or(Summoner {
        game_name: name.trim().to_string(),
        tag_line: tag.trim().to_string(),
        level: 0,
        puuid: puuid.clone(),
    });
    Ok(Lookup {
        ranked: lcu.ranked_of(&puuid).unwrap_or_default(),
        history: lcu.lol_history_of(&puuid, 0, PAGE - 1).map_err(|e| format!("match history: {e}")),
        mastery: lcu.mastery(&puuid).map(|mut m| {
            m.truncate(10);
            m
        })
        .unwrap_or_default(),
        summoner,
    })
}

/// Writes Riot's recommended page + spells into each Swiftplay slot that has a champion.
fn swiftplay_runes(lcu: &Lcu, data: &StaticData) -> Result<String, String> {
    let mut slots = lcu.player_slots().map_err(|e| format!("Swiftplay slots: {e}"))?;
    let mut done = Vec::new();
    for slot in &mut slots {
        let champ = slot["championId"].as_i64().unwrap_or(0);
        if champ <= 0 {
            continue;
        }
        let pos = lcu::perk_position(slot["positionPreference"].as_str().unwrap_or(""));
        let pages = lcu.recommended_pages(champ, pos, live::SUMMONERS_RIFT).map_err(|e| format!("Recommended runes: {e}"))?;
        let Some(rec) = pages.first() else { continue };
        slot["perks"] = Value::from(rec.perks_string());
        if !rec.spells.contains(&0) {
            let cur = [slot["spell1"].as_i64().unwrap_or(0), slot["spell2"].as_i64().unwrap_or(0)];
            let s = lcu::order_spells(rec.spells, cur);
            slot["spell1"] = Value::from(s[0]);
            slot["spell2"] = Value::from(s[1]);
        }
        done.push(data.champion_name(champ));
    }
    if done.is_empty() {
        return Err("No Swiftplay champion picked yet".into());
    }
    lcu.set_player_slots(&slots).map_err(|e| format!("Swiftplay slots: {e}"))?;
    Ok(format!("Recommended runes set for {}", done.join(", ")))
}

fn refresh_profile(shared: &Shared, lcu: &Lcu, history_len: usize) -> bool {
    let summoner = match lcu.summoner() {
        Ok(s) if !s.puuid.is_empty() => s,
        Ok(_) | Err(LcuError::Status(404)) => {
            let mut s = shared.snap.lock();
            return set(&mut s.profile_error, Some("Waiting for login to finish in the League client".into()));
        }
        Err(e) => return set(&mut shared.snap.lock().profile_error, Some(format!("summoner: {e}"))),
    };
    let ranked = lcu.ranked().unwrap_or_default();
    let lol = lcu.lol_history(&summoner.puuid, 0, history_len - 1).map_err(|e| format!("LoL match history: {e}"));
    let tft = lcu.tft_history(&summoner.puuid).map_err(|e| format!("TFT match history: {e}"));
    let mastery = lcu.mastery(&summoner.puuid).unwrap_or_default();
    let challenges = lcu.challenges().ok();
    let mut s = shared.snap.lock();
    let mut changed = set(&mut s.profile_error, None);
    if let Ok(g) = &lol {
        changed |= set(&mut s.history_complete, g.len() < history_len);
    }
    changed |= set(&mut s.summoner, Some(summoner));
    changed |= set(&mut s.ranked, ranked);
    changed |= set(&mut s.lol_history, lol);
    changed |= set(&mut s.tft_history, tft);
    changed |= set(&mut s.mastery, mastery);
    changed |= set(&mut s.challenges, challenges);
    changed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state(build: Option<Result<MetaBuild, String>>) -> MetaState {
        let q = Query { mode: "ranked", position: "ADC" };
        MetaState { champion_id: 145, query: Ok(q), build, picks: Picks { build: 0, runes: 0, spells: 0, skills: 1 } }
    }

    #[test]
    fn lockin_waits_for_opgg_then_falls_back_after_meta_wait() {
        let q = Query { mode: "ranked", position: "ADC" };
        let json = r#"{"data":{"summary":{"id":145},"core_items":[{"ids":[6672,3124,3302],"play":500,"win":260}]}}"#;
        let b = meta::parse_build(145, q, json, &StaticData::default(), false).expect("parses");
        let early = Duration::from_secs(5);
        // Still loading before META_WAIT: keep waiting.
        assert!(lockin_meta(Some(&state(None)), 145, early).is_none());
        // Arriving late but within META_WAIT: used, with the player's picks.
        match lockin_meta(Some(&state(Some(Ok(b.clone())))), 145, early) {
            Some(Some(Ok((got, picks)))) => assert_eq!((got, picks.skills), (b, 1)),
            other => panic!("expected the build, got {other:?}"),
        }
        // Never arriving: an explained fallback at META_WAIT, not a wait forever.
        match lockin_meta(Some(&state(None)), 145, META_WAIT) {
            Some(Some(Err(e))) => assert!(e.contains("did not answer in time")),
            other => panic!("expected a timeout, got {other:?}"),
        }
        // A failed answer falls back at once; OP.GG off or another champion's state: no OP.GG part.
        assert!(matches!(lockin_meta(Some(&state(Some(Err("OP.GG HTTP 500".into())))), 145, early), Some(Some(Err(_)))));
        assert!(matches!(lockin_meta(None, 145, early), Some(None)));
        assert!(matches!(lockin_meta(Some(&state(None)), 62, early), Some(None)));
    }
}
