//! Window chrome for the undecorated main window: the title bar (move, double-click
//! maximize, window buttons) and the edge/corner resize zones. Moving and resizing are
//! native (`StartDrag` / `BeginResize`), like the overlay's edit mode.

use eframe::egui::{
    self, Align2, CursorIcon, Frame, Id, Panel, Pos2, Rect, ResizeDirection, Response, Sense, Stroke, StrokeKind, Ui,
    ViewportCommand, pos2, vec2,
};

use super::icons::Icons;
use super::theme::*;
use super::widgets::{Icon, paint_icon};

const BAR_HEIGHT: f32 = 34.0;
const BUTTON_WIDTH: f32 = 46.0;
/// Thickness of the resize zones along the window edges.
const EDGE: f32 = 5.0;
/// Side of the resize zones in the window corners.
const CORNER: f32 = 12.0;

fn maximized(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false))
}

/// Logo and name on the left, minimize / maximize-restore / close on the right; the rest
/// of the bar moves the window and double-clicking it toggles maximize.
pub(super) fn title_bar(ui: &mut Ui, icons: &Icons) {
    Panel::top("titlebar").exact_size(BAR_HEIGHT).resizable(false).frame(Frame::new().fill(SIDEBAR)).show(ui, |ui| {
        let ctx = ui.ctx().clone();
        let bar = ui.max_rect();
        // Registered first so the window buttons, added later, sit on top of it.
        let drag = ui.interact(bar, Id::unique("riftlite-titlebar"), Sense::click_and_drag());
        let max = maximized(&ctx);
        if drag.double_clicked() {
            ctx.send_viewport_cmd(ViewportCommand::Maximized(!max));
        } else if drag.drag_started() {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }

        let p = ui.painter();
        p.hline(bar.x_range(), bar.bottom() - 0.5, Stroke::new(1.0, BORDER));
        let logo = Rect::from_center_size(pos2(bar.left() + 12.0 + 9.0, bar.center().y), vec2(18.0, 18.0));
        p.image(icons.logo.id, logo, Rect::from_min_max(Pos2::ZERO, pos2(1.0, 1.0)), egui::Color32::WHITE);
        p.text(pos2(logo.right() + 8.0, bar.center().y), Align2::LEFT_CENTER, "riftlite", semibold(12.5), BODY);

        let mut right = bar.right();
        let mut button = |icon: Icon, close: bool, tip: &str| {
            let rect = Rect::from_min_max(pos2(right - BUTTON_WIDTH, bar.top()), pos2(right, bar.bottom() - 1.0));
            right -= BUTTON_WIDTH;
            window_button(ui, rect, icon, close).on_hover_text(tip).clicked()
        };
        if button(Icon::Cross, true, "Close") {
            ctx.send_viewport_cmd(ViewportCommand::Close);
        }
        if button(if max { Icon::Restore } else { Icon::Maximize }, false, if max { "Restore" } else { "Maximize" }) {
            ctx.send_viewport_cmd(ViewportCommand::Maximized(!max));
        }
        if button(Icon::Minimize, false, "Minimize") {
            ctx.send_viewport_cmd(ViewportCommand::Minimized(true));
        }
    });
}

fn window_button(ui: &mut Ui, rect: Rect, icon: Icon, close: bool) -> Response {
    let resp = ui.interact(rect, Id::unique(("riftlite-window-button", icon as u8)), Sense::click());
    let active = resp.hovered() || resp.is_pointer_button_down_on();
    let (fill, color) = match (active, close) {
        (true, true) => (LOSS, TEXT),
        (true, false) => (HOVER, TEXT),
        (false, _) => (SIDEBAR, MUTED),
    };
    let p = ui.painter();
    p.rect_filled(rect, 0, fill);
    paint_icon(p, Rect::from_center_size(rect.center(), vec2(15.0, 15.0)), icon, color);
    resp
}

/// Outlines the window and restores edge/corner resizing lost with the native frame.
/// Call last, so the zones sit above every panel's widgets.
pub(super) fn resize_edges(ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    if maximized(&ctx) || ctx.input(|i| i.viewport().fullscreen.unwrap_or(false)) {
        return;
    }
    let r = ctx.content_rect();
    ui.painter().rect_stroke(r, 0, Stroke::new(1.0, BORDER_STRONG), StrokeKind::Inside);

    let (l, t, rt, b) = (r.left(), r.top(), r.right(), r.bottom());
    let zone = |x0: f32, y0: f32, x1: f32, y1: f32| Rect::from_min_max(pos2(x0, y0), pos2(x1, y1));
    // Corners last: they overlap the edges and must win.
    let zones = [
        (zone(l, t, rt, t + EDGE), ResizeDirection::North, CursorIcon::ResizeNorth),
        (zone(l, b - EDGE, rt, b), ResizeDirection::South, CursorIcon::ResizeSouth),
        (zone(l, t, l + EDGE, b), ResizeDirection::West, CursorIcon::ResizeWest),
        (zone(rt - EDGE, t, rt, b), ResizeDirection::East, CursorIcon::ResizeEast),
        (zone(l, t, l + CORNER, t + CORNER), ResizeDirection::NorthWest, CursorIcon::ResizeNorthWest),
        (zone(rt - CORNER, t, rt, t + CORNER), ResizeDirection::NorthEast, CursorIcon::ResizeNorthEast),
        (zone(l, b - CORNER, l + CORNER, b), ResizeDirection::SouthWest, CursorIcon::ResizeSouthWest),
        (zone(rt - CORNER, b - CORNER, rt, b), ResizeDirection::SouthEast, CursorIcon::ResizeSouthEast),
    ];
    for (i, (rect, dir, cursor)) in zones.into_iter().enumerate() {
        let resp = ui.interact(rect, Id::unique(("riftlite-resize", i)), Sense::drag());
        if resp.hovered() || resp.dragged() {
            ctx.set_cursor_icon(cursor);
        }
        if resp.drag_started() {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
        }
    }
}
