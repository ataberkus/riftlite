#![windows_subsystem = "windows"]
//! riftlite: a lightweight League of Legends / Teamfight Tactics companion.
//! One native process with a background poller thread; no embedded browser,
//! no script runtime, no injection, no elevation.

mod augments;
mod ddragon;
mod itemset;
mod lcu;
mod live;
mod lp;
mod meta;
mod net;
mod riot;
mod settings;
mod skills;
mod tray;
mod ui;
mod win;
mod worker;

use eframe::egui;

const TITLE: &str = "riftlite";

fn main() -> eframe::Result {
    if !win::acquire_single_instance(TITLE) {
        return Ok(());
    }
    let start_minimized = std::env::args().any(|a| a == "--minimized");
    let mut viewport = egui::ViewportBuilder::default()
        .with_title(TITLE)
        .with_app_id("riftlite")
        .with_inner_size([1180.0, 780.0])
        .with_min_inner_size([760.0, 480.0])
        .with_transparent(true)
        // The title bar, window buttons and resize edges are drawn by `ui::chrome`.
        .with_decorations(false)
        // Autostart / background start must not steal focus (e.g. from a running game).
        .with_active(!start_minimized);
    if let Ok(icon) = eframe::icon_data::from_png_bytes(include_bytes!("../assets/logo.png")) {
        viewport = viewport.with_icon(icon);
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native(TITLE, options, Box::new(move |cc| Ok(Box::new(ui::App::new(cc, start_minimized, TITLE)))))
}
