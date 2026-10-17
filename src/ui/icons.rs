//! Textures: the bundled logo and banner, and Data Dragon images (champions,
//! items, spells, runes, profile icons, splash art) fetched on first use by a few
//! background threads, cached on disk per patch and kept as downscaled textures.

use std::collections::HashMap;
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant};

use eframe::egui::{self, ColorImage, TextureFilter, TextureHandle, TextureId, TextureOptions, Vec2};
use parking_lot::Mutex;

const FETCH_THREADS: usize = 4;
/// A failed image (offline, 404) is asked for again after this long.
const RETRY: Duration = Duration::from_secs(60);

const OPTIONS: TextureOptions = TextureOptions::LINEAR.with_mipmap_mode(Some(TextureFilter::Linear));

enum Slot {
    Pending,
    Ready(TextureHandle),
    Failed(Instant),
}

struct Job {
    version: String,
    path: String,
    max_px: u32,
    /// Desaturate the image (locked champions).
    grey: bool,
}

/// A texture and its size in pixels.
#[derive(Clone, Copy)]
pub struct Tex {
    pub id: TextureId,
    pub size: Vec2,
}

impl From<&TextureHandle> for Tex {
    fn from(t: &TextureHandle) -> Self {
        Tex { id: t.id(), size: t.size_vec2() }
    }
}

pub struct Icons {
    pub logo: Tex,
    pub banner: Tex,
    _owned: [TextureHandle; 2],
    slots: Mutex<HashMap<String, Slot>>,
    jobs: Sender<Job>,
}

impl Icons {
    pub fn new(ctx: &egui::Context) -> Arc<Icons> {
        let logo = ctx.load_texture("riftlite-logo", decode(include_bytes!("../../assets/logo.png"), 256), OPTIONS);
        let banner = ctx.load_texture("riftlite-banner", decode(include_bytes!("../../assets/banner.jpg"), 1120), OPTIONS);
        let (tx, rx) = channel::<Job>();
        let icons = Arc::new(Icons {
            logo: (&logo).into(),
            banner: (&banner).into(),
            _owned: [logo, banner],
            slots: Mutex::new(HashMap::new()),
            jobs: tx,
        });
        let rx = Arc::new(Mutex::new(rx));
        for i in 0..FETCH_THREADS {
            let (rx, icons, ctx) = (rx.clone(), Arc::downgrade(&icons), ctx.clone());
            let _ = std::thread::Builder::new().name(format!("riftlite-img{i}")).spawn(move || fetch_loop(&rx, &icons, &ctx));
        }
        icons
    }

    /// The texture for a Data Dragon image path, downscaled to at most `max_px` on
    /// its longer side. `None` while it loads (a repaint follows) or when it failed.
    pub fn get(&self, version: &str, path: &str, max_px: u32) -> Option<Tex> {
        self.lookup(version, path, max_px, false)
    }

    /// Like [`Icons::get`], but desaturated to greyscale.
    pub fn get_grey(&self, version: &str, path: &str, max_px: u32) -> Option<Tex> {
        self.lookup(version, path, max_px, true)
    }

    fn lookup(&self, version: &str, path: &str, max_px: u32, grey: bool) -> Option<Tex> {
        let key = slot_key(path, grey);
        let mut slots = self.slots.lock();
        match slots.get(key.as_ref()) {
            Some(Slot::Ready(t)) => return Some(t.into()),
            Some(Slot::Pending) => return None,
            Some(Slot::Failed(at)) if at.elapsed() < RETRY => return None,
            _ => {}
        }
        slots.insert(key.into_owned(), Slot::Pending);
        let _ = self.jobs.send(Job { version: version.to_string(), path: path.to_string(), max_px, grey });
        None
    }
}

/// Cache key of an image; greyscale variants live beside the colour one.
fn slot_key(path: &str, grey: bool) -> std::borrow::Cow<'_, str> {
    if grey { format!("{path}#grey").into() } else { path.into() }
}

/// One fetch thread; ends when the `Icons` (and so the sender) is dropped.
fn fetch_loop(rx: &Mutex<Receiver<Job>>, icons: &Weak<Icons>, ctx: &egui::Context) {
    let agent = crate::net::public_agent();
    loop {
        let Ok(job) = rx.lock().recv() else { return };
        let image = crate::ddragon::fetch_image(&agent, &job.version, &job.path)
            .and_then(|b| image::load_from_memory(&b).map_err(|e| e.to_string()));
        let slot = match image {
            Ok(img) => {
                let mut color = to_color_image(img, job.max_px);
                if job.grey {
                    desaturate(&mut color);
                }
                Slot::Ready(ctx.load_texture(slot_key(&job.path, job.grey), color, OPTIONS))
            }
            Err(_) => Slot::Failed(Instant::now()),
        };
        let Some(icons) = icons.upgrade() else { return };
        icons.slots.lock().insert(slot_key(&job.path, job.grey).into_owned(), slot);
        ctx.request_repaint();
        ctx.request_repaint_of(super::overlay_id());
    }
}

fn decode(bytes: &[u8], max_px: u32) -> ColorImage {
    image::load_from_memory(bytes)
        .map(|i| to_color_image(i, max_px))
        .unwrap_or_else(|_| ColorImage::filled([1, 1], egui::Color32::TRANSPARENT))
}

/// Rec. 601 luma in place. Pixels are premultiplied, so luma never exceeds alpha.
fn desaturate(img: &mut ColorImage) {
    for px in &mut img.pixels {
        let [r, g, b, a] = px.to_array();
        let l = ((299 * r as u32 + 587 * g as u32 + 114 * b as u32) / 1000) as u8;
        *px = egui::Color32::from_rgba_premultiplied(l, l, l, a);
    }
}

fn to_color_image(img: image::DynamicImage, max_px: u32) -> ColorImage {
    let img = if img.width().max(img.height()) > max_px {
        img.resize(max_px, max_px, image::imageops::FilterType::CatmullRom)
    } else {
        img
    };
    let rgba = img.to_rgba8();
    ColorImage::from_rgba_unmultiplied([rgba.width() as usize, rgba.height() as usize], rgba.as_raw())
}
