//! Palette, fonts and the egui style shared by the main window and the overlay.

use std::sync::{Arc, LazyLock};

use eframe::egui::{
    self, Color32, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Margin, Shadow, Stroke, TextStyle, Visuals,
    style::WidgetVisuals, vec2,
};

pub const BG: Color32 = Color32::from_rgb(0x0A, 0x0D, 0x13);
pub const SIDEBAR: Color32 = Color32::from_rgb(0x0D, 0x11, 0x19);
pub const SURFACE: Color32 = Color32::from_rgb(0x12, 0x17, 0x21);
pub const RAISED: Color32 = Color32::from_rgb(0x19, 0x1F, 0x2B);
pub const HOVER: Color32 = Color32::from_rgb(0x21, 0x28, 0x37);
pub const BORDER: Color32 = Color32::from_rgb(0x1F, 0x26, 0x33);
pub const BORDER_STRONG: Color32 = Color32::from_rgb(0x2D, 0x36, 0x48);
pub const TEXT: Color32 = Color32::from_rgb(0xEE, 0xF1, 0xF6);
pub const BODY: Color32 = Color32::from_rgb(0xC1, 0xC8, 0xD5);
pub const MUTED: Color32 = Color32::from_rgb(0x7A, 0x84, 0x97);
pub const FAINT: Color32 = Color32::from_rgb(0x4B, 0x54, 0x66);
pub const ACCENT: Color32 = Color32::from_rgb(0x7B, 0x6C, 0xFF);
pub const ACCENT_TEXT: Color32 = Color32::from_rgb(0xC9, 0xC2, 0xFF);
pub const CYAN: Color32 = Color32::from_rgb(0x3E, 0xC8, 0xFF);
pub const WIN: Color32 = Color32::from_rgb(0x2F, 0xD3, 0x9A);
pub const LOSS: Color32 = Color32::from_rgb(0xFF, 0x55, 0x6B);
/// Timers about to expire, first places, skill hints.
pub const SOON: Color32 = Color32::from_rgb(0xFF, 0xC1, 0x4D);
pub const BLUE_TEAM: Color32 = Color32::from_rgb(0x4C, 0x9D, 0xFF);
pub const RED_TEAM: Color32 = Color32::from_rgb(0xFF, 0x60, 0x6E);

pub const RADIUS: u8 = 12;

static SEMIBOLD: LazyLock<FontFamily> = LazyLock::new(|| FontFamily::Name("semibold".into()));
static DISPLAY: LazyLock<FontFamily> = LazyLock::new(|| FontFamily::Name("display".into()));

/// Inter, the body face.
pub fn body(size: f32) -> FontId {
    FontId::proportional(size)
}

/// Inter SemiBold: labels, names, buttons.
pub fn semibold(size: f32) -> FontId {
    FontId::new(size, SEMIBOLD.clone())
}

/// Barlow Semi Condensed Bold: titles and big numbers.
pub fn display(size: f32) -> FontId {
    FontId::new(size, DISPLAY.clone())
}

pub fn install(ctx: &egui::Context) {
    let mut fonts = FontDefinitions::default();
    for (name, bytes) in [
        ("inter", &include_bytes!("../../assets/fonts/Inter-Regular.ttf")[..]),
        ("inter-semibold", include_bytes!("../../assets/fonts/Inter-SemiBold.ttf")),
        ("barlow-bold", include_bytes!("../../assets/fonts/BarlowSemiCondensed-Bold.ttf")),
    ] {
        fonts.font_data.insert(name.into(), Arc::new(FontData::from_static(bytes)));
    }
    // egui's defaults stay behind ours for glyphs the Latin subsets lack (→, ★, ±).
    let fallbacks = fonts.families.get(&FontFamily::Proportional).cloned().unwrap_or_default();
    let chain = |first: &[&str]| first.iter().map(|s| s.to_string()).chain(fallbacks.iter().cloned()).collect::<Vec<_>>();
    fonts.families.insert(FontFamily::Proportional, chain(&["inter"]));
    fonts.families.insert(SEMIBOLD.clone(), chain(&["inter-semibold", "inter"]));
    fonts.families.insert(DISPLAY.clone(), chain(&["barlow-bold", "inter-semibold"]));
    ctx.set_fonts(fonts);
    ctx.set_theme(egui::Theme::Dark);
    ctx.style_mut_of(egui::Theme::Dark, style);
}

fn style(s: &mut egui::Style) {
    s.text_styles = [
        (TextStyle::Small, body(11.0)),
        (TextStyle::Body, body(13.5)),
        (TextStyle::Button, semibold(13.0)),
        (TextStyle::Heading, display(26.0)),
        (TextStyle::Monospace, FontId::monospace(12.5)),
    ]
    .into();
    let sp = &mut s.spacing;
    sp.item_spacing = vec2(8.0, 6.0);
    sp.button_padding = vec2(12.0, 5.0);
    sp.interact_size = vec2(36.0, 26.0);
    sp.icon_width = 16.0;
    sp.icon_width_inner = 9.0;
    sp.icon_spacing = 8.0;
    sp.window_margin = Margin::same(12);
    sp.menu_margin = Margin::same(8);
    sp.tooltip_width = 340.0;
    sp.scroll.bar_width = 6.0;
    sp.scroll.floating_width = 3.0;
    s.interaction.tooltip_delay = 0.15;
    s.visuals = visuals();
}

fn visuals() -> Visuals {
    let mut v = Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = RAISED;
    v.window_stroke = Stroke::new(1.0, BORDER_STRONG);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v.window_shadow = Shadow { offset: [0, 8], blur: 24, spread: 0, color: Color32::from_black_alpha(150) };
    v.popup_shadow = Shadow { offset: [0, 6], blur: 18, spread: 0, color: Color32::from_black_alpha(130) };
    v.extreme_bg_color = Color32::from_rgb(0x0B, 0x0F, 0x16);
    v.text_edit_bg_color = Some(Color32::from_rgb(0x0D, 0x11, 0x19));
    v.faint_bg_color = Color32::from_rgb(0x16, 0x1C, 0x27);
    v.code_bg_color = RAISED;
    v.hyperlink_color = CYAN;
    v.warn_fg_color = SOON;
    v.error_fg_color = LOSS;
    v.weak_text_color = Some(MUTED);
    v.selection.bg_fill = Color32::from_rgb(0x2E, 0x2A, 0x5C);
    v.selection.stroke = Stroke::new(1.0, ACCENT_TEXT);
    v.text_cursor.stroke = Stroke::new(2.0, ACCENT);
    let r = CornerRadius::same(8);
    let w = &mut v.widgets;
    w.noninteractive = WidgetVisuals {
        bg_fill: SURFACE,
        weak_bg_fill: SURFACE,
        bg_stroke: Stroke::new(1.0, BORDER),
        corner_radius: r,
        fg_stroke: Stroke::new(1.0, BODY),
        expansion: 0.0,
    };
    w.inactive = WidgetVisuals {
        bg_fill: HOVER,
        weak_bg_fill: RAISED,
        bg_stroke: Stroke::new(1.0, BORDER_STRONG),
        corner_radius: r,
        fg_stroke: Stroke::new(1.0, TEXT),
        expansion: 0.0,
    };
    w.hovered = WidgetVisuals {
        bg_fill: Color32::from_rgb(0x2A, 0x32, 0x45),
        weak_bg_fill: HOVER,
        bg_stroke: Stroke::new(1.0, Color32::from_rgb(0x55, 0x4E, 0xB0)),
        corner_radius: r,
        fg_stroke: Stroke::new(1.5, Color32::WHITE),
        expansion: 0.0,
    };
    w.active = WidgetVisuals {
        bg_fill: Color32::from_rgb(0x31, 0x3A, 0x50),
        weak_bg_fill: Color32::from_rgb(0x27, 0x2F, 0x41),
        bg_stroke: Stroke::new(1.0, ACCENT),
        corner_radius: r,
        fg_stroke: Stroke::new(2.0, Color32::WHITE),
        expansion: 0.0,
    };
    w.open = w.active;
    v
}

/// Ranked tier colors (also Hyper Roll's rated tiers).
pub fn tier_color(tier: &str) -> Color32 {
    match tier.to_ascii_uppercase().as_str() {
        "IRON" => Color32::from_rgb(0x8E, 0x82, 0x7C),
        "BRONZE" => Color32::from_rgb(0xC0, 0x7D, 0x55),
        "SILVER" | "GRAY" => Color32::from_rgb(0xA9, 0xB9, 0xC9),
        "GOLD" => Color32::from_rgb(0xEC, 0xBC, 0x5C),
        "PLATINUM" => Color32::from_rgb(0x4F, 0xD3, 0xC2),
        "EMERALD" | "GREEN" => Color32::from_rgb(0x2E, 0xCC, 0x7A),
        "DIAMOND" | "BLUE" => Color32::from_rgb(0x74, 0x9C, 0xFF),
        "MASTER" | "PURPLE" => Color32::from_rgb(0xC0, 0x6C, 0xFF),
        "GRANDMASTER" | "ORANGE" => Color32::from_rgb(0xFF, 0x63, 0x5A),
        "CHALLENGER" | "HYPER" => Color32::from_rgb(0xFF, 0xD8, 0x6B),
        _ => FAINT,
    }
}

/// KDA ratio color: gold for 5+, cyan 3.5+, green 2.5+, red under 1.5.
pub fn kda_color(kda: f64) -> Color32 {
    match kda {
        k if k >= 5.0 => SOON,
        k if k >= 3.5 => CYAN,
        k if k >= 2.5 => WIN,
        k if k < 1.5 => LOSS,
        _ => BODY,
    }
}

/// Win-rate color for a fraction in 0..=1.
pub fn wr_color(wr: f64) -> Color32 {
    match wr {
        w if w >= 0.6 => SOON,
        w if w >= 0.52 => WIN,
        w if w <= 0.45 => LOSS,
        _ => BODY,
    }
}

pub fn result_color(win: bool) -> Color32 {
    if win { WIN } else { LOSS }
}

/// TFT placement: gold for 1st, green top 4, red bottom 4.
pub fn placement_color(p: i64) -> Color32 {
    match p {
        1 => SOON,
        2..=4 => WIN,
        _ => LOSS,
    }
}

/// `c` at `a` (0..=1) opacity.
pub fn alpha(c: Color32, a: f32) -> Color32 {
    c.gamma_multiply(a)
}
