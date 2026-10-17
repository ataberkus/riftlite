//! Painted building blocks: cards, buttons, chips, stat tiles, toggles, bars,
//! sparklines, rank emblems, vector icons and Data Dragon image tiles.

use std::f32::consts::{PI, TAU};

use eframe::egui::{
    self, Align, Align2, Color32, CornerRadius, CursorIcon, Frame, Label, Layout, Margin, Mesh, Painter, Pos2, Rect, Response,
    RichText, Sense, Shadow, Shape, Stroke, StrokeKind, TextFormat, Ui, UiBuilder, Vec2, Widget, WidgetInfo,
    epaint::RectShape, pos2, text::LayoutJob, vec2,
};

use super::icons::{Icons, Tex};
use super::theme::*;
use crate::ddragon::StaticData;

// ---------------------------------------------------------------- colors & shapes

/// Linear blend of two colors, `t` = 0 gives `a`.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let l = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(l(a.r(), b.r()), l(a.g(), b.g()), l(a.b(), b.b()), l(a.a(), b.a()))
}

/// A rectangle with one color per corner: top-left, top-right, bottom-right, bottom-left.
pub fn gradient_rect(p: &Painter, rect: Rect, [tl, tr, br, bl]: [Color32; 4]) {
    let mut m = Mesh::default();
    m.colored_vertex(rect.left_top(), tl);
    m.colored_vertex(rect.right_top(), tr);
    m.colored_vertex(rect.right_bottom(), br);
    m.colored_vertex(rect.left_bottom(), bl);
    m.add_triangle(0, 1, 2);
    m.add_triangle(0, 2, 3);
    p.add(Shape::mesh(m));
}

fn paint_tex(p: &Painter, rect: Rect, tex: Tex, radius: impl Into<CornerRadius>, uv: Rect, tint: Color32) {
    p.add(RectShape::filled(rect, radius, tint).with_texture(tex.id, uv));
}

/// UV rectangle that covers `rect` with an image of `size` (like CSS `object-fit: cover`), with the
/// visible window placed at `focus` (0..1 on each axis).
fn cover_uv(size: Vec2, rect: Rect, focus: Vec2) -> Rect {
    let (ia, ra) = (size.x / size.y.max(1.0), rect.width() / rect.height().max(1.0));
    if ia > ra {
        let w = ra / ia;
        let x = (1.0 - w) * focus.x;
        Rect::from_min_max(pos2(x, 0.0), pos2(x + w, 1.0))
    } else {
        let h = ia / ra;
        let y = (1.0 - h) * focus.y;
        Rect::from_min_max(pos2(0.0, y), pos2(1.0, y + h))
    }
}

// ---------------------------------------------------------------- containers

pub fn card_frame() -> Frame {
    Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(RADIUS)
        .inner_margin(Margin::same(16))
        .outer_margin(Margin { bottom: 6, ..Margin::ZERO })
        .shadow(Shadow { offset: [0, 3], blur: 14, spread: 0, color: Color32::from_black_alpha(70) })
}

/// A full-width card.
pub fn card<R>(ui: &mut Ui, f: impl FnOnce(&mut Ui) -> R) -> R {
    framed(ui, card_frame(), f)
}

/// A full-width frame.
pub fn framed<R>(ui: &mut Ui, frame: Frame, f: impl FnOnce(&mut Ui) -> R) -> R {
    frame
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            f(ui)
        })
        .inner
}

/// A smaller inset panel inside a card.
pub fn inset_frame() -> Frame {
    Frame::new().fill(RAISED).corner_radius(10).inner_margin(Margin::symmetric(12, 10))
}

/// Page section heading with an accent bar; `right` fills the right side of the row.
pub fn section_with(ui: &mut Ui, title: &str, right: impl FnOnce(&mut Ui)) {
    ui.add_space(14.0);
    ui.horizontal(|ui| {
        let (r, _) = ui.allocate_exact_size(vec2(3.0, 16.0), Sense::hover());
        gradient_rect(ui.painter(), r, [CYAN, CYAN, ACCENT, ACCENT]);
        ui.add_space(2.0);
        ui.label(RichText::new(title).font(semibold(15.5)).color(TEXT));
        ui.with_layout(Layout::right_to_left(Align::Center), right);
    });
    ui.add_space(6.0);
}

pub fn section(ui: &mut Ui, title: &str) {
    section_with(ui, title, |_| {});
}

/// Small uppercase label used as a card title or a stat caption.
pub fn overline(ui: &mut Ui, text: &str) -> Response {
    ui.label(RichText::new(text.to_uppercase()).font(semibold(10.5)).color(MUTED))
}

/// A card title row: overline on the left, `right` on the right.
pub fn card_head(ui: &mut Ui, title: &str, right: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        overline(ui, title);
        ui.with_layout(Layout::right_to_left(Align::Center), right);
    });
    ui.add_space(2.0);
}

pub fn note(ui: &mut Ui, text: impl Into<String>) -> Response {
    ui.add(Label::new(RichText::new(text.into()).color(MUTED)).wrap())
}

pub fn small_note(ui: &mut Ui, text: impl Into<String>) -> Response {
    ui.add(Label::new(RichText::new(text.into()).size(11.5).color(MUTED)).wrap())
}

/// An error line with a cross icon.
pub fn error(ui: &mut Ui, text: impl Into<String>) -> Response {
    status_msg(ui, false, text)
}

/// A success/failure line with a check or cross icon.
pub fn status_msg(ui: &mut Ui, ok: bool, text: impl Into<String>) -> Response {
    ui.horizontal(|ui| {
        icon(ui, if ok { Icon::Check } else { Icon::Cross }, 14.0, result_color(ok));
        ui.add(Label::new(RichText::new(text.into()).color(if ok { BODY } else { mix(LOSS, TEXT, 0.25) })).wrap());
    })
    .response
}

/// A centered empty-state card.
pub fn empty_state(ui: &mut Ui, ic: Icon, title: &str, msg: &str, extra: impl FnOnce(&mut Ui)) {
    card(ui, |ui| {
        ui.vertical_centered(|ui| {
            ui.add_space(14.0);
            let (r, _) = ui.allocate_exact_size(Vec2::splat(52.0), Sense::hover());
            ui.painter().circle_filled(r.center(), 26.0, RAISED);
            ui.painter().circle_stroke(r.center(), 26.0, Stroke::new(1.0, BORDER_STRONG));
            paint_icon(ui.painter(), Rect::from_center_size(r.center(), Vec2::splat(24.0)), ic, ACCENT_TEXT);
            ui.add_space(6.0);
            ui.label(RichText::new(title).font(semibold(16.0)).color(TEXT));
            ui.add(Label::new(RichText::new(msg).color(MUTED)).wrap());
            extra(ui);
            ui.add_space(12.0);
        });
    });
}

// ---------------------------------------------------------------- buttons & chips

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

/// A painted button. Disable it with `ui.add_enabled`.
pub struct Btn<'a> {
    text: &'a str,
    kind: Kind,
    icon: Option<Icon>,
    small: bool,
    selected: bool,
}

impl<'a> Btn<'a> {
    pub fn new(kind: Kind, text: &'a str) -> Self {
        Btn { text, kind, icon: None, small: false, selected: false }
    }
    pub fn primary(text: &'a str) -> Self {
        Self::new(Kind::Primary, text)
    }
    pub fn secondary(text: &'a str) -> Self {
        Self::new(Kind::Secondary, text)
    }
    pub fn ghost(text: &'a str) -> Self {
        Self::new(Kind::Ghost, text)
    }
    pub fn danger(text: &'a str) -> Self {
        Self::new(Kind::Danger, text)
    }
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }
    pub fn small(mut self) -> Self {
        self.small = true;
        self
    }
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }
}

impl Widget for Btn<'_> {
    fn ui(self, ui: &mut Ui) -> Response {
        let font = semibold(if self.small { 12.0 } else { 13.0 });
        let galley = ui.painter().layout_no_wrap(self.text.to_string(), font, Color32::PLACEHOLDER);
        let pad = if self.small { vec2(10.0, 3.0) } else { vec2(14.0, 6.0) };
        let icon_size = if self.small { 13.0 } else { 15.0 };
        let icon_w = match (self.icon, self.text.is_empty()) {
            (None, _) => 0.0,
            (Some(_), true) => icon_size,
            (Some(_), false) => icon_size + 6.0,
        };
        let min_h = if self.small { 24.0 } else { 32.0 };
        let size = vec2(galley.size().x + icon_w + 2.0 * pad.x, (galley.size().y + 2.0 * pad.y).max(min_h));
        let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
        let enabled = ui.is_enabled();
        resp.widget_info(|| WidgetInfo::labeled(egui::Role::Button, enabled, self.text));
        if ui.is_rect_visible(rect) {
            let hovered = enabled && resp.hovered();
            let pressed = enabled && resp.is_pointer_button_down_on();
            let (mut fill, mut stroke, mut text) = match (self.kind, self.selected) {
                (Kind::Primary, _) => {
                    let f = if pressed {
                        mix(ACCENT, BG, 0.2)
                    } else if hovered {
                        mix(ACCENT, Color32::WHITE, 0.12)
                    } else {
                        ACCENT
                    };
                    (f, Stroke::NONE, Color32::WHITE)
                }
                (_, true) => (alpha(ACCENT, 0.22), Stroke::new(1.0, alpha(ACCENT, 0.8)), ACCENT_TEXT),
                (Kind::Secondary, false) => {
                    let f = if pressed {
                        mix(HOVER, BG, 0.3)
                    } else if hovered {
                        HOVER
                    } else {
                        RAISED
                    };
                    (f, Stroke::new(1.0, if hovered { mix(BORDER_STRONG, ACCENT, 0.35) } else { BORDER_STRONG }), TEXT)
                }
                (Kind::Ghost, false) => (if hovered { RAISED } else { Color32::TRANSPARENT }, Stroke::NONE, if hovered { TEXT } else { BODY }),
                (Kind::Danger, false) => {
                    (alpha(LOSS, if hovered { 0.24 } else { 0.13 }), Stroke::new(1.0, alpha(LOSS, 0.5)), mix(LOSS, TEXT, 0.35))
                }
            };
            if !enabled {
                fill = fill.gamma_multiply(0.45);
                stroke.color = stroke.color.gamma_multiply(0.45);
                text = text.gamma_multiply(0.4);
            }
            let radius = if self.small { 6 } else { 8 };
            ui.painter().rect(rect, radius, fill, stroke, StrokeKind::Inside);
            if self.kind == Kind::Primary && enabled {
                // Soft top highlight.
                let top = Rect::from_min_size(rect.min, vec2(rect.width(), rect.height() * 0.5));
                let hl = Color32::from_white_alpha(18);
                ui.painter().add(RectShape::filled(top, CornerRadius { nw: radius, ne: radius, sw: 0, se: 0 }, hl));
            }
            let mut x = rect.left() + pad.x;
            if let Some(ic) = self.icon {
                let r = Rect::from_min_size(pos2(x, rect.center().y - icon_size / 2.0), Vec2::splat(icon_size));
                paint_icon(ui.painter(), r, ic, text);
                x += icon_w;
            }
            ui.painter().galley(pos2(x, rect.center().y - galley.size().y / 2.0), galley, text);
        }
        if enabled { resp.on_hover_cursor(CursorIcon::PointingHand) } else { resp }
    }
}

/// A rounded tag: tinted background, colored text.
pub fn pill(ui: &mut Ui, text: impl Into<String>, color: Color32) -> Response {
    let galley = ui.painter().layout_no_wrap(text.into(), semibold(11.0), color);
    let size = galley.size() + vec2(16.0, 6.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    if ui.is_rect_visible(rect) {
        let r = (size.y / 2.0) as u8;
        ui.painter().rect(rect, r, alpha(color, 0.13), Stroke::new(1.0, alpha(color, 0.35)), StrokeKind::Inside);
        ui.painter().galley(rect.center() - galley.size() / 2.0, galley, color);
    }
    resp
}

/// A small filled status dot.
pub fn dot(ui: &mut Ui, color: Color32) {
    let (r, _) = ui.allocate_exact_size(vec2(10.0, 10.0), Sense::hover());
    ui.painter().circle_filled(r.center(), 6.0, alpha(color, 0.18));
    ui.painter().circle_filled(r.center(), 3.5, color);
}

/// A selectable row (OP.GG option, list entry): accent tint and bar when picked. `true` when clicked.
pub fn option_row(ui: &mut Ui, picked: bool, add: impl FnOnce(&mut Ui)) -> Response {
    let bg = ui.painter().add(Shape::Noop);
    let inner = Frame::new().inner_margin(Margin::symmetric(10, 6)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(add);
    });
    let rect = inner.response.rect;
    let resp = ui.interact(rect, inner.response.id.with("row"), Sense::click());
    let hovered = resp.hovered();
    let fill = if picked {
        alpha(ACCENT, 0.16)
    } else if hovered {
        RAISED
    } else {
        Color32::TRANSPARENT
    };
    let stroke = if picked { Stroke::new(1.0, alpha(ACCENT, 0.55)) } else { Stroke::NONE };
    ui.painter().set(bg, RectShape::new(rect, 8, fill, stroke, StrokeKind::Inside));
    if picked {
        let bar = Rect::from_min_size(rect.left_top() + vec2(0.0, 6.0), vec2(3.0, rect.height() - 12.0));
        ui.painter().rect_filled(bar, 2, ACCENT);
    }
    resp.on_hover_cursor(CursorIcon::PointingHand)
}

// ---------------------------------------------------------------- inputs

/// An on/off switch with a label and optional help text; the whole row toggles. Returns whether it changed.
pub fn toggle(ui: &mut Ui, on: &mut bool, label: &str, help: Option<&str>) -> bool {
    let bg = ui.painter().add(Shape::Noop);
    let id = ui.next_auto_id();
    let t = ui.ctx().animate_bool_with_time(id, *on, 0.12);
    let inner = Frame::new().inner_margin(Margin::symmetric(10, 8)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal_top(|ui| {
            let (r, _) = ui.allocate_exact_size(vec2(36.0, 20.0), Sense::hover());
            let track = mix(HOVER, ACCENT, t);
            ui.painter().rect(r, 10, track, Stroke::new(1.0, mix(BORDER_STRONG, ACCENT, t)), StrokeKind::Inside);
            let x = egui::lerp(r.left() + 10.0..=r.right() - 10.0, t);
            ui.painter().circle_filled(pos2(x, r.center().y), 7.0, mix(BODY, Color32::WHITE, t));
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 2.0;
                ui.add(Label::new(RichText::new(label).font(semibold(13.0)).color(TEXT)).wrap());
                if let Some(h) = help {
                    ui.add(Label::new(RichText::new(h).size(12.0).color(MUTED)).wrap());
                }
            });
        });
    });
    let resp = ui.interact(inner.response.rect, id, Sense::click());
    let fill = if resp.hovered() { alpha(RAISED, 0.8) } else { Color32::TRANSPARENT };
    ui.painter().set(bg, RectShape::filled(inner.response.rect, 8, fill));
    resp.widget_info(|| WidgetInfo::selected(egui::Role::CheckBox, true, *on, label));
    if resp.clicked() {
        *on = !*on;
        return true;
    }
    false
}

/// A segmented control; returns whether the value changed.
pub fn segmented<T: PartialEq + Copy>(ui: &mut Ui, value: &mut T, options: &[(T, &str)]) -> bool {
    let mut changed = false;
    Frame::new().fill(Color32::from_rgb(0x0D, 0x11, 0x19)).stroke(Stroke::new(1.0, BORDER)).corner_radius(9).inner_margin(Margin::same(3)).show(
        ui,
        |ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            ui.horizontal_wrapped(|ui| {
                for (v, label) in options {
                    let picked = *value == *v;
                    let kind = if picked { Kind::Primary } else { Kind::Ghost };
                    if ui.add(Btn::new(kind, label).small()).clicked() && !picked {
                        *value = *v;
                        changed = true;
                    }
                }
            });
        },
    );
    changed
}

/// A wrapping row of tiles, top-aligned (`horizontal_wrapped` centers each item on the first one's row).
pub fn tiles<R>(ui: &mut Ui, f: impl FnOnce(&mut Ui) -> R) -> R {
    ui.with_layout(Layout::left_to_right(Align::Min).with_main_wrap(true), f).inner
}

// ---------------------------------------------------------------- data display

/// A stat tile: caption, big value, optional sub line.
pub fn stat_tile(ui: &mut Ui, label: &str, value: impl Into<String>, color: Color32, sub: Option<&str>) -> Response {
    inset_frame()
        .show(ui, |ui| {
            ui.set_min_width(92.0);
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 1.0;
                overline(ui, label);
                ui.label(RichText::new(value.into()).font(display(24.0)).color(color));
                if let Some(s) = sub {
                    ui.label(RichText::new(s).size(11.5).color(MUTED));
                }
            });
        })
        .response
}

/// A thin progress bar.
pub fn progress(ui: &mut Ui, frac: f32, width: f32, color: Color32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 6.0), Sense::hover());
    let frac = frac.clamp(0.0, 1.0);
    ui.painter().rect_filled(rect, 3, Color32::from_rgb(0x0C, 0x10, 0x17));
    if frac > 0.0 {
        let fill = Rect::from_min_size(rect.min, vec2((rect.width() * frac).max(6.0), rect.height()));
        ui.painter().rect_filled(fill, 3, color);
        let hl = Rect::from_min_size(fill.min, vec2(fill.width(), 2.0));
        ui.painter().rect_filled(hl, 1, Color32::from_white_alpha(28));
    }
    resp
}

/// Win/loss split bar.
pub fn wr_bar(ui: &mut Ui, wins: i64, losses: i64, width: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(width, 6.0), Sense::hover());
    let total = (wins + losses).max(1) as f32;
    let split = rect.left() + rect.width() * wins as f32 / total;
    ui.painter().rect_filled(rect, 3, if losses > 0 { LOSS } else { RAISED });
    if wins > 0 {
        let r = Rect::from_min_max(rect.min, pos2(split.max(rect.left() + 3.0), rect.bottom()));
        ui.painter().rect_filled(r, 3, WIN);
    }
    resp
}

/// One small block per game, oldest first: green wins, red losses.
pub fn result_strip(ui: &mut Ui, wins: impl Iterator<Item = bool>) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        for w in wins {
            let (r, _) = ui.allocate_exact_size(vec2(7.0, 16.0), Sense::hover());
            ui.painter().rect_filled(r, 2, alpha(result_color(w), 0.85));
        }
    });
}

/// A filled-area line chart; hovering shows each point's value. `reference` draws a dashed line.
pub fn sparkline(ui: &mut Ui, values: &[f64], size: Vec2, color: Color32, reference: Option<f64>, fmt: fn(f64) -> String) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    if values.len() < 2 || !ui.is_rect_visible(rect) {
        return resp;
    }
    let p = ui.painter_at(rect.expand(4.0));
    let mut lo = values.iter().copied().fold(f64::INFINITY, f64::min);
    let mut hi = values.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    if let Some(r) = reference {
        lo = lo.min(r);
        hi = hi.max(r);
    }
    let span = (hi - lo).max(1e-6);
    let (lo, hi) = (lo - span * 0.12, hi + span * 0.12);
    let y = |v: f64| rect.bottom() - ((v - lo) / (hi - lo)) as f32 * rect.height();
    let step = rect.width() / (values.len() - 1) as f32;
    let pts: Vec<Pos2> = values.iter().enumerate().map(|(i, &v)| pos2(rect.left() + i as f32 * step, y(v))).collect();
    let mut m = Mesh::default();
    let (top, bottom) = (alpha(color, 0.28), alpha(color, 0.0));
    for w in pts.windows(2) {
        let base = m.vertices.len() as u32;
        m.colored_vertex(w[0], top);
        m.colored_vertex(w[1], top);
        m.colored_vertex(pos2(w[1].x, rect.bottom()), bottom);
        m.colored_vertex(pos2(w[0].x, rect.bottom()), bottom);
        m.add_triangle(base, base + 1, base + 2);
        m.add_triangle(base, base + 2, base + 3);
    }
    p.add(Shape::mesh(m));
    if let Some(r) = reference {
        let ry = y(r);
        p.add(Shape::dashed_line(&[pos2(rect.left(), ry), pos2(rect.right(), ry)], Stroke::new(1.0, FAINT), 3.0, 3.0));
    }
    p.add(Shape::line(pts.clone(), Stroke::new(1.8, color)));
    let last = *pts.last().expect("two or more points");
    p.circle(last, 3.0, color, Stroke::new(1.5, SURFACE));
    if let Some(hover) = resp.hover_pos() {
        let i = (((hover.x - rect.left()) / step).round() as usize).min(values.len() - 1);
        p.line_segment([pos2(pts[i].x, rect.top()), pos2(pts[i].x, rect.bottom())], Stroke::new(1.0, alpha(TEXT, 0.25)));
        p.circle_filled(pts[i], 3.5, TEXT);
        let text = format!("Game {} of {}: {}", i + 1, values.len(), fmt(values[i]));
        return resp.on_hover_text_at_pointer(text);
    }
    resp
}

/// `k / d / a` with the deaths in red, on one line.
pub fn kda(ui: &mut Ui, k: i64, d: i64, a: i64, size: f32) -> Response {
    let mut job = LayoutJob::default();
    let f = |c: Color32| TextFormat { font_id: display(size), color: c, ..Default::default() };
    job.append(&k.to_string(), 0.0, f(TEXT));
    job.append(" / ", 0.0, f(FAINT));
    job.append(&d.to_string(), 0.0, f(LOSS));
    job.append(" / ", 0.0, f(FAINT));
    job.append(&a.to_string(), 0.0, f(TEXT));
    ui.add(Label::new(job).extend())
}

/// A ▲/▼ trend arrow and percent against an average; muted within ±5%.
pub fn trend(ui: &mut Ui, now: f64, avg: f64, higher_better: bool) {
    if avg <= 0.0 {
        return;
    }
    let pct = (now - avg) * 100.0 / avg;
    let good = (pct >= 0.0) == higher_better;
    let color = if pct.abs() < 5.0 { MUTED } else { result_color(good) };
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 3.0;
        icon(ui, if pct >= 0.0 { Icon::ArrowUp } else { Icon::ArrowDown }, 10.0, color);
        ui.label(RichText::new(format!("{}{pct:.0}%", if pct >= 0.0 { "+" } else { "" })).font(semibold(12.5)).color(color));
    });
}

/// A filled five-pointed star centered at `c`.
pub fn paint_star(p: &Painter, c: Pos2, r: f32, color: Color32) {
    let pts: Vec<Pos2> = (0..10)
        .map(|k| {
            let a = -PI / 2.0 + k as f32 * TAU / 10.0;
            c + if k % 2 == 0 { r } else { r * 0.45 } * vec2(a.cos(), a.sin())
        })
        .collect();
    let mut m = Mesh::default();
    m.colored_vertex(c, color);
    for q in &pts {
        m.colored_vertex(*q, color);
    }
    for k in 0..10u32 {
        m.add_triangle(0, 1 + k, 1 + (k + 1) % 10);
    }
    p.add(Shape::mesh(m));
    // A thin outline anti-aliases the mesh edges.
    p.add(Shape::closed_line(pts, Stroke::new(0.6, color)));
}

// ---------------------------------------------------------------- rank emblem

fn tier_letters(tier: &str) -> &'static str {
    match tier.to_ascii_uppercase().as_str() {
        "IRON" => "I",
        "BRONZE" => "B",
        "SILVER" => "S",
        "GOLD" => "G",
        "PLATINUM" => "P",
        "EMERALD" => "E",
        "DIAMOND" => "D",
        "MASTER" => "M",
        "GRANDMASTER" => "GM",
        "CHALLENGER" => "C",
        "GRAY" | "GREEN" | "BLUE" | "PURPLE" | "ORANGE" | "HYPER" => "★",
        _ => "–",
    }
}

fn is_ranked_tier(tier: &str) -> bool {
    !matches!(tier.to_ascii_uppercase().as_str(), "" | "NONE" | "UNRANKED")
}

/// A hexagonal tier emblem.
pub fn paint_emblem(p: &Painter, rect: Rect, tier: &str) {
    let c = rect.center();
    let r = rect.width().min(rect.height()) / 2.0;
    let hex = |rad: f32| -> Vec<Pos2> {
        (0..6).map(|k| {
            let a = -PI / 2.0 + k as f32 * TAU / 6.0;
            c + rad * vec2(a.cos(), a.sin())
        })
        .collect()
    };
    let color = tier_color(tier);
    if !is_ranked_tier(tier) {
        p.add(Shape::convex_polygon(hex(r * 0.92), RAISED, Stroke::new(1.5, BORDER_STRONG)));
        p.text(c, Align2::CENTER_CENTER, "–", display(r * 0.8), FAINT);
        return;
    }
    let apex = matches!(tier.to_ascii_uppercase().as_str(), "MASTER" | "GRANDMASTER" | "CHALLENGER");
    if apex {
        p.add(Shape::closed_line(hex(r), Stroke::new(1.5, alpha(color, 0.55))));
    }
    // Gradient fill: light at the top, dark at the bottom.
    let outer = hex(r * 0.86);
    let mut m = Mesh::default();
    m.colored_vertex(c, mix(color, BG, 0.25));
    for q in &outer {
        let t = ((q.y - (c.y - r)) / (2.0 * r)).clamp(0.0, 1.0);
        m.colored_vertex(*q, mix(mix(color, Color32::WHITE, 0.25), mix(color, BG, 0.55), t));
    }
    for k in 0..6u32 {
        m.add_triangle(0, 1 + k, 1 + (k + 1) % 6);
    }
    p.add(Shape::mesh(m));
    p.add(Shape::convex_polygon(hex(r * 0.62), alpha(BG, 0.82), Stroke::new(1.2, alpha(color, 0.9))));
    p.text(c + vec2(0.0, r * 0.03), Align2::CENTER_CENTER, tier_letters(tier), display(r * 0.62), mix(color, Color32::WHITE, 0.2));
}

pub fn rank_emblem(ui: &mut Ui, tier: &str, size: f32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    if ui.is_rect_visible(rect) {
        paint_emblem(ui.painter(), rect, tier);
    }
    resp
}

// ---------------------------------------------------------------- vector icons

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Icon {
    Home,
    Profile,
    Swords,
    Hex,
    Target,
    Live,
    Chart,
    Search,
    Grid,
    Gear,
    Refresh,
    Play,
    Check,
    Cross,
    Clock,
    Save,
    Eye,
    Trash,
    Folder,
    Bolt,
    ChevronDown,
    ChevronUp,
    ArrowUp,
    ArrowDown,
    Skull,
}

/// Paints `icon` into `rect` (drawn on a 24-unit grid).
pub fn paint_icon(p: &Painter, rect: Rect, icon: Icon, color: Color32) {
    let s = rect.width().min(rect.height()) / 24.0;
    let o = rect.center() - Vec2::splat(12.0 * s);
    let at = |x: f32, y: f32| o + vec2(x, y) * s;
    let st = Stroke::new((1.8 * s).max(1.1), color);
    let line = |pts: &[(f32, f32)]| {
        p.add(Shape::line(pts.iter().map(|&(x, y)| at(x, y)).collect(), st));
    };
    let closed = |pts: &[(f32, f32)]| {
        p.add(Shape::closed_line(pts.iter().map(|&(x, y)| at(x, y)).collect(), st));
    };
    let fill = |pts: &[(f32, f32)]| {
        p.add(Shape::convex_polygon(pts.iter().map(|&(x, y)| at(x, y)).collect(), color, Stroke::NONE));
    };
    let arc = |cx: f32, cy: f32, rx: f32, ry: f32, a0: f32, a1: f32| -> Vec<Pos2> {
        (0..=20).map(|i| {
            let a = (a0 + (a1 - a0) * i as f32 / 20.0).to_radians();
            at(cx + rx * a.cos(), cy + ry * a.sin())
        })
        .collect()
    };
    match icon {
        Icon::Home => {
            line(&[(3.0, 11.0), (12.0, 3.5), (21.0, 11.0)]);
            line(&[(5.5, 9.5), (5.5, 20.0), (18.5, 20.0), (18.5, 9.5)]);
            line(&[(10.0, 20.0), (10.0, 14.0), (14.0, 14.0), (14.0, 20.0)]);
        }
        Icon::Profile => {
            p.circle_stroke(at(12.0, 8.0), 4.0 * s, st);
            p.add(Shape::line(arc(12.0, 21.0, 7.5, 6.5, 180.0, 360.0), st));
        }
        Icon::Swords => {
            line(&[(5.0, 19.0), (19.0, 5.0)]);
            line(&[(19.0, 19.0), (5.0, 5.0)]);
            line(&[(3.5, 15.0), (9.0, 20.5)]);
            line(&[(15.0, 20.5), (20.5, 15.0)]);
        }
        Icon::Hex => {
            let hex = |r: f32| (0..6).map(move |k| {
                let a = -PI / 2.0 + k as f32 * TAU / 6.0;
                (12.0 + r * a.cos(), 12.0 + r * a.sin())
            });
            closed(&hex(9.5).collect::<Vec<_>>());
            fill(&hex(3.5).collect::<Vec<_>>());
        }
        Icon::Target => {
            p.circle_stroke(at(12.0, 12.0), 8.0 * s, st);
            p.circle_filled(at(12.0, 12.0), 2.6 * s, color);
            for (a, b) in [((12.0, 1.5), (12.0, 6.0)), ((12.0, 18.0), (12.0, 22.5)), ((1.5, 12.0), (6.0, 12.0)), ((18.0, 12.0), (22.5, 12.0))] {
                line(&[a, b]);
            }
        }
        Icon::Live => {
            p.circle_filled(at(12.0, 12.0), 3.0 * s, color);
            for r in [6.5, 10.0] {
                p.add(Shape::line(arc(12.0, 12.0, r, r, 140.0, 220.0), st));
                p.add(Shape::line(arc(12.0, 12.0, r, r, -40.0, 40.0), st));
            }
        }
        Icon::Chart => {
            let thick = Stroke::new(3.0 * s, color);
            for (x, top) in [(6.0, 13.0), (12.0, 6.0), (18.0, 10.0)] {
                p.line_segment([at(x, 20.0), at(x, top)], thick);
            }
            line(&[(3.0, 21.5), (21.0, 21.5)]);
        }
        Icon::Search => {
            p.circle_stroke(at(10.5, 10.5), 6.5 * s, st);
            line(&[(15.5, 15.5), (20.5, 20.5)]);
        }
        Icon::Grid => {
            for (x, y) in [(4.0, 4.0), (14.0, 4.0), (4.0, 14.0), (14.0, 14.0)] {
                p.rect_stroke(Rect::from_min_size(at(x, y), Vec2::splat(6.0 * s)), (1.5 * s) as u8, st, StrokeKind::Middle);
            }
        }
        Icon::Gear => {
            let pts: Vec<Pos2> = (0..32)
                .map(|k| {
                    let a = k as f32 * TAU / 32.0 + TAU / 64.0;
                    let r = if (k / 2) % 2 == 0 { 9.5 } else { 7.2 };
                    at(12.0 + r * a.cos(), 12.0 + r * a.sin())
                })
                .collect();
            p.add(Shape::closed_line(pts, st));
            p.circle_stroke(at(12.0, 12.0), 3.0 * s, st);
        }
        Icon::Refresh => {
            p.add(Shape::line(arc(12.0, 12.0, 7.5, 7.5, -60.0, 230.0), st));
            let tip = at(12.0 + 7.5 * (-60f32).to_radians().cos(), 12.0 + 7.5 * (-60f32).to_radians().sin());
            p.add(Shape::convex_polygon(vec![tip + vec2(-4.0, -2.5) * s, tip + vec2(2.5, -3.5) * s, tip + vec2(1.0, 3.0) * s], color, Stroke::NONE));
        }
        Icon::Play => fill(&[(8.0, 5.0), (19.0, 12.0), (8.0, 19.0)]),
        Icon::Check => line(&[(5.0, 12.5), (10.0, 17.5), (19.0, 7.0)]),
        Icon::Cross => {
            line(&[(6.5, 6.5), (17.5, 17.5)]);
            line(&[(17.5, 6.5), (6.5, 17.5)]);
        }
        Icon::Clock => {
            p.circle_stroke(at(12.0, 12.0), 8.5 * s, st);
            line(&[(12.0, 7.0), (12.0, 12.0), (15.5, 14.0)]);
        }
        Icon::Save => {
            line(&[(12.0, 4.0), (12.0, 15.0)]);
            line(&[(7.5, 10.5), (12.0, 15.0), (16.5, 10.5)]);
            line(&[(4.0, 16.0), (4.0, 20.0), (20.0, 20.0), (20.0, 16.0)]);
        }
        Icon::Eye => {
            let mut pts = arc(12.0, 12.0, 10.0, 6.5, 180.0, 360.0);
            pts.extend(arc(12.0, 12.0, 10.0, 6.5, 0.0, 180.0));
            p.add(Shape::closed_line(pts, st));
            p.circle_filled(at(12.0, 12.0), 2.8 * s, color);
        }
        Icon::Trash => {
            line(&[(4.0, 6.5), (20.0, 6.5)]);
            line(&[(9.0, 6.5), (9.0, 3.5), (15.0, 3.5), (15.0, 6.5)]);
            line(&[(6.0, 6.5), (7.0, 20.5), (17.0, 20.5), (18.0, 6.5)]);
        }
        Icon::Folder => closed(&[(3.0, 6.0), (9.0, 6.0), (11.0, 8.5), (21.0, 8.5), (21.0, 19.0), (3.0, 19.0)]),
        Icon::Bolt => closed(&[(13.5, 2.5), (5.0, 13.5), (11.0, 13.5), (10.0, 21.5), (19.0, 10.0), (13.0, 10.0)]),
        Icon::ChevronDown => line(&[(6.0, 9.0), (12.0, 15.0), (18.0, 9.0)]),
        Icon::ChevronUp => line(&[(6.0, 15.0), (12.0, 9.0), (18.0, 15.0)]),
        Icon::ArrowUp => fill(&[(12.0, 5.0), (20.0, 18.0), (4.0, 18.0)]),
        Icon::ArrowDown => fill(&[(4.0, 6.0), (20.0, 6.0), (12.0, 19.0)]),
        Icon::Skull => {
            let mut pts = arc(12.0, 10.5, 8.0, 7.5, 150.0, 390.0);
            pts.extend([at(16.0, 19.5), at(8.0, 19.5)]);
            p.add(Shape::closed_line(pts, st));
            p.circle_filled(at(9.0, 11.0), 1.9 * s, color);
            p.circle_filled(at(15.0, 11.0), 1.9 * s, color);
        }
    }
}

pub fn icon(ui: &mut Ui, ic: Icon, size: f32, color: Color32) -> Response {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
    if ui.is_rect_visible(rect) {
        paint_icon(ui.painter(), rect, ic, color);
    }
    resp
}

// ---------------------------------------------------------------- Data Dragon images

/// Image tiles backed by the icon cache; requested only when on screen.
#[derive(Clone, Copy)]
pub struct Gfx<'a> {
    pub icons: &'a Icons,
    pub data: &'a StaticData,
}

/// How a missing image is drawn.
enum Fallback<'s> {
    Text(&'s str),
    Empty,
}

impl<'a> Gfx<'a> {
    pub fn new(icons: &'a Icons, data: &'a StaticData) -> Self {
        Gfx { icons, data }
    }

    pub fn tex(&self, path: Option<String>, max_px: u32) -> Option<Tex> {
        self.icons.get(&self.data.version, &path?, max_px)
    }

    fn tile(&self, ui: &mut Ui, path: impl FnOnce() -> Option<String>, size: f32, radius: u8, crop: f32, fallback: Fallback) -> Response {
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
        if !ui.is_rect_visible(rect) {
            return resp;
        }
        let p = ui.painter();
        match self.tex(path(), 96) {
            Some(t) => {
                let uv = Rect::from_min_max(pos2(crop, crop), pos2(1.0 - crop, 1.0 - crop));
                paint_tex(p, rect, t, radius, uv, Color32::WHITE);
                p.rect_stroke(rect, radius, Stroke::new(1.0, Color32::from_white_alpha(14)), StrokeKind::Inside);
            }
            None => {
                p.rect(rect, radius, RAISED, Stroke::new(1.0, BORDER), StrokeKind::Inside);
                if let Fallback::Text(t) = fallback {
                    let initials: String = t.chars().filter(|c| c.is_alphanumeric()).take(2).collect();
                    p.text(rect.center(), Align2::CENTER_CENTER, initials, semibold(size * 0.34), MUTED);
                }
            }
        }
        resp
    }

    /// Square champion portrait (Data Dragon's built-in frame cropped away); hover shows the name.
    pub fn champ(&self, ui: &mut Ui, id: i64, size: f32) -> Response {
        let name = self.data.champion_name(id);
        let radius = (size * 0.22) as u8;
        self.tile(ui, || self.data.champion_icon(id), size, radius, 0.07, Fallback::Text(&name)).on_hover_text(name)
    }

    /// Champion portrait by Data Dragon key (`MissFortune`), for the Live Client API.
    pub fn champ_key(&self, ui: &mut Ui, key: &str, name: &str, size: f32) -> Response {
        let radius = (size * 0.22) as u8;
        self.tile(ui, || self.data.champion_icon_by_key(key), size, radius, 0.07, Fallback::Text(name)).on_hover_text(name)
    }

    /// Champion portrait painted at a given rect (no allocation), dimmed by `tint`.
    pub fn paint_champ(&self, p: &Painter, rect: Rect, id: i64, tint: Color32) {
        let radius = (rect.width() * 0.22) as u8;
        match self.tex(self.data.champion_icon(id), 96) {
            Some(t) => paint_tex(p, rect, t, radius, Rect::from_min_max(pos2(0.07, 0.07), pos2(0.93, 0.93)), tint),
            None => {
                p.rect(rect, radius, RAISED, Stroke::new(1.0, BORDER), StrokeKind::Inside);
            }
        }
    }

    pub fn item(&self, ui: &mut Ui, id: i64, size: f32) -> Response {
        if id <= 0 {
            return self.tile(ui, || None, size, 5, 0.0, Fallback::Empty);
        }
        self.tile(ui, || self.data.item_icon(id), size, 5, 0.0, Fallback::Text(&self.data.item_name(id))).on_hover_text(self.data.item_name(id))
    }

    /// A row of item slots (empty slots drawn as placeholders).
    pub fn items(&self, ui: &mut Ui, ids: &[i64], size: f32) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            for &i in ids {
                self.item(ui, i, size);
            }
        });
    }

    pub fn spell(&self, ui: &mut Ui, id: i64, size: f32) -> Response {
        let name = self.data.spell_name(id);
        self.tile(ui, || self.data.spell_icon(id), size, 5, 0.0, Fallback::Text(&name)).on_hover_text(name)
    }

    /// Rune or rune path icon on a dark disc.
    pub fn perk(&self, ui: &mut Ui, id: i64, size: f32) -> Response {
        let (rect, resp) = ui.allocate_exact_size(Vec2::splat(size), Sense::hover());
        let name = self.data.perk_name(id);
        if ui.is_rect_visible(rect) {
            let p = ui.painter();
            p.circle_filled(rect.center(), size / 2.0, Color32::from_rgb(0x0C, 0x10, 0x17));
            p.circle_stroke(rect.center(), size / 2.0, Stroke::new(1.0, BORDER_STRONG));
            if let Some(t) = self.tex(self.data.perk_icon(id), 96) {
                let inner = rect.shrink(size * 0.12);
                paint_tex(p, inner, t, 0, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), Color32::WHITE);
            } else {
                let initials: String = name.chars().filter(|c| c.is_alphanumeric()).take(2).collect();
                p.text(rect.center(), Align2::CENTER_CENTER, initials, semibold(size * 0.34), MUTED);
            }
        }
        resp.on_hover_text(name)
    }

    pub fn profile(&self, ui: &mut Ui, icon_id: i64, name: &str, size: f32) -> Response {
        self.tile(ui, || self.data.profile_icon(icon_id), size, (size * 0.3) as u8, 0.0, Fallback::Text(name))
    }

    /// Splash art of a champion (wide, for headers).
    pub fn splash(&self, id: i64) -> Option<Tex> {
        self.tex(self.data.champion_splash(id), 1024)
    }
}

/// A rounded header banner: the image cover-cropped behind dark gradients, contents laid out
/// left to right, vertically centered, inside a margin.
pub fn hero<R>(ui: &mut Ui, tex: Option<Tex>, height: f32, focus: Vec2, contents: impl FnOnce(&mut Ui) -> R) -> R {
    let width = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(vec2(width, height), Sense::hover());
    let p = ui.painter_at(rect.expand(1.0));
    p.rect_filled(rect, RADIUS, SURFACE);
    if let Some(t) = tex {
        paint_tex(&p, rect, t, RADIUS, cover_uv(t.size, rect, focus), Color32::WHITE);
    }
    let mid = Rect::from_min_max(rect.min, pos2(rect.left() + rect.width() * 0.75, rect.bottom()));
    gradient_rect(&p, mid, [alpha(BG, 0.94), alpha(BG, 0.25), alpha(BG, 0.25), alpha(BG, 0.94)]);
    let rest = Rect::from_min_max(pos2(mid.right(), rect.top()), rect.max);
    gradient_rect(&p, rest, [alpha(BG, 0.25), alpha(BG, 0.1), alpha(BG, 0.1), alpha(BG, 0.25)]);
    let low = Rect::from_min_max(pos2(rect.left(), rect.top() + rect.height() * 0.45), rect.max);
    gradient_rect(&p, low, [Color32::TRANSPARENT, Color32::TRANSPARENT, alpha(BG, 0.7), alpha(BG, 0.7)]);
    p.rect_stroke(rect, RADIUS, Stroke::new(1.0, BORDER), StrokeKind::Inside);
    let inner = rect.shrink2(vec2(24.0, 18.0));
    let mut child = ui.new_child(UiBuilder::new().max_rect(inner).layout(Layout::left_to_right(Align::Center)));
    let r = contents(&mut child);
    ui.add_space(8.0);
    r
}
