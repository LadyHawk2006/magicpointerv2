use crate::hypr_ipc::{self, WindowGeometry};
use crate::prompt_overlay;
use gtk4::cairo;
use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::RefCell;
use std::rc::Rc;
use tracing::info;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub fn contains(&self, px: f64, py: f64) -> bool {
        let x = self.x as f64;
        let y = self.y as f64;
        let w = self.width as f64;
        let h = self.height as f64;
        px >= x && px <= x + w && py >= y && py <= y + h
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SnappedWindow {
    pub compositor_rect: Rect,
    pub local_rect: Rect,
    pub title: String,
    pub class: String,
}

pub fn convert_windows_to_local(
    windows: &[WindowGeometry],
    monitor_origin: (i32, i32),
    monitor_size: (i32, i32),
) -> Vec<SnappedWindow> {
    windows
        .iter()
        .filter_map(|w| {
            let local_x = w.x - monitor_origin.0;
            let local_y = w.y - monitor_origin.1;
            let local_w = w.width;
            let local_h = w.height;

            if local_w <= 0 || local_h <= 0 {
                return None;
            }

            // Window must intersect this monitor
            if local_x + local_w <= 0
                || local_y + local_h <= 0
                || local_x >= monitor_size.0
                || local_y >= monitor_size.1
            {
                return None;
            }

            // Clamp visible region to the monitor bounds for clean highlight & capture
            let clamped_x = local_x.max(0);
            let clamped_y = local_y.max(0);
            let clamped_right = (local_x + local_w).min(monitor_size.0);
            let clamped_bottom = (local_y + local_h).min(monitor_size.1);
            let clamped_w = clamped_right - clamped_x;
            let clamped_h = clamped_bottom - clamped_y;

            if clamped_w < 8 || clamped_h < 8 {
                return None;
            }

            Some(SnappedWindow {
                compositor_rect: Rect {
                    x: clamped_x + monitor_origin.0,
                    y: clamped_y + monitor_origin.1,
                    width: clamped_w as u32,
                    height: clamped_h as u32,
                },
                local_rect: Rect {
                    x: clamped_x,
                    y: clamped_y,
                    width: clamped_w as u32,
                    height: clamped_h as u32,
                },
                title: w.title.clone(),
                class: w.class.clone(),
            })
        })
        .collect()
}

pub fn find_snapped_window(
    windows: &[SnappedWindow],
    cursor_x: f64,
    cursor_y: f64,
) -> Option<&SnappedWindow> {
    windows
        .iter()
        .find(|w| w.local_rect.contains(cursor_x, cursor_y))
}

#[derive(Clone, Copy, Debug)]
enum DragStatus {
    Idle,
    Pending {
        start_x: f64,
        start_y: f64,
    },
    Dragging {
        start_x: f64,
        start_y: f64,
        current_x: f64,
        current_y: f64,
    },
}

struct OverlayState {
    drag: DragStatus,
    hovered_window: Option<SnappedWindow>,
}

pub fn show_selection_overlay<F, D>(app: &gtk4::Application, on_selected: F, on_dismiss: D)
where
    F: Fn(Rect) + 'static,
    D: Fn() + 'static,
{
    let window = gtk4::ApplicationWindow::new(app);

    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_keyboard_mode(KeyboardMode::Exclusive);
    window.set_exclusive_zone(-1);

    // Full screen anchors
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Bottom, true);
    window.set_anchor(Edge::Left, true);
    window.set_anchor(Edge::Right, true);

    window.add_css_class("selection-window");

    let (cursor_pos, visible_windows) = match hypr_ipc::get_hyprland_state() {
        Ok(state) => {
            let windows = hypr_ipc::get_visible_windows().unwrap_or_default();
            (state.cursor, windows)
        }
        Err(_) => ((0, 0), Vec::new()),
    };

    let display = gdk::Display::default();
    let (monitor_origin, monitor_size) = if let Some(ref disp) = display {
        if let Some((monitor, geom)) = prompt_overlay::find_monitor(disp, cursor_pos) {
            window.set_monitor(Some(&monitor));
            ((geom.x(), geom.y()), (geom.width(), geom.height()))
        } else {
            ((0, 0), (i32::MAX, i32::MAX))
        }
    } else {
        ((0, 0), (i32::MAX, i32::MAX))
    };

    let snapped_windows = Rc::new(convert_windows_to_local(
        &visible_windows,
        monitor_origin,
        monitor_size,
    ));

    // Initial hover detection based on current cursor position
    let initial_hover = find_snapped_window(
        &snapped_windows,
        (cursor_pos.0 - monitor_origin.0) as f64,
        (cursor_pos.1 - monitor_origin.1) as f64,
    )
    .cloned();

    let state = Rc::new(RefCell::new(OverlayState {
        drag: DragStatus::Idle,
        hovered_window: initial_hover.clone(),
    }));

    let drawing_area = gtk4::DrawingArea::new();
    drawing_area.add_css_class("selection-canvas");
    drawing_area.set_hexpand(true);
    drawing_area.set_vexpand(true);
    if initial_hover.is_some() {
        drawing_area.set_cursor_from_name(Some("pointer"));
    } else {
        drawing_area.set_cursor_from_name(Some("crosshair"));
    }

    let state_draw = state.clone();
    drawing_area.set_draw_func(move |_, cr, width, height| {
        // Subtle dark backdrop overlay
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.32);
        let _ = cr.paint();

        let current_state = state_draw.borrow();
        match current_state.drag {
            DragStatus::Dragging {
                start_x,
                start_y,
                current_x,
                current_y,
            } => {
                let rx = start_x.min(current_x);
                let ry = start_y.min(current_y);
                let rw = (current_x - start_x).abs();
                let rh = (current_y - start_y).abs();

                if rw > 1.0 && rh > 1.0 {
                    // Clear custom selection cutout
                    cr.save().expect("cairo save");
                    cr.set_operator(cairo::Operator::Clear);
                    cr.rectangle(rx, ry, rw, rh);
                    let _ = cr.fill();
                    cr.restore().expect("cairo restore");

                    // Selection box outline
                    cr.set_source_rgba(0.23, 0.51, 0.96, 0.95);
                    cr.set_line_width(2.0);
                    cr.rectangle(rx, ry, rw, rh);
                    let _ = cr.stroke();

                    // Corner accent highlights
                    draw_corner_brackets(cr, rx, ry, rw, rh, 14.0, 3.0);

                    // Dimension badge
                    draw_dimension_badge(cr, rx, ry, rw, rh, width as f64, height as f64);
                }
            }
            _ => {
                if let Some(ref snapped) = current_state.hovered_window {
                    let rx = snapped.local_rect.x as f64;
                    let ry = snapped.local_rect.y as f64;
                    let rw = snapped.local_rect.width as f64;
                    let rh = snapped.local_rect.height as f64;

                    // Spotlight the snapped window by clearing the dark tint
                    cr.save().expect("cairo save");
                    cr.set_operator(cairo::Operator::Clear);
                    cr.rectangle(rx, ry, rw, rh);
                    let _ = cr.fill();
                    cr.restore().expect("cairo restore");

                    // Translucent highlight fill
                    cr.set_source_rgba(0.23, 0.51, 0.96, 0.06);
                    cr.rectangle(rx, ry, rw, rh);
                    let _ = cr.fill();

                    // Crisp snap border
                    cr.set_source_rgba(0.23, 0.51, 0.96, 0.92);
                    cr.set_line_width(2.5);
                    cr.rectangle(rx, ry, rw, rh);
                    let _ = cr.stroke();

                    // Corner brackets
                    draw_corner_brackets(cr, rx, ry, rw, rh, 18.0, 3.5);

                    // Smart snap info pill badge
                    draw_snap_badge(cr, snapped, width as f64);
                }
            }
        }
    });

    // Motion tracking for smart window snap hover
    let motion_controller = gtk4::EventControllerMotion::new();
    let state_motion = state.clone();
    let windows_motion = snapped_windows.clone();
    let da_motion = drawing_area.clone();

    motion_controller.connect_motion(move |_, x, y| {
        let mut st = state_motion.borrow_mut();
        if let DragStatus::Dragging { .. } = st.drag {
            return;
        }

        let found = find_snapped_window(&windows_motion, x, y).cloned();
        if st.hovered_window != found {
            if found.is_some() {
                da_motion.set_cursor_from_name(Some("pointer"));
            } else {
                da_motion.set_cursor_from_name(Some("crosshair"));
            }
            st.hovered_window = found;
            da_motion.queue_draw();
        }
    });

    let state_leave = state.clone();
    let da_leave = drawing_area.clone();
    motion_controller.connect_leave(move |_| {
        let mut st = state_leave.borrow_mut();
        if st.hovered_window.is_some() {
            st.hovered_window = None;
            da_leave.set_cursor_from_name(Some("crosshair"));
            da_leave.queue_draw();
        }
    });
    drawing_area.add_controller(motion_controller);

    // Drag gesture for custom region selection & click-to-capture detection
    let gesture_drag = gtk4::GestureDrag::new();

    let state_start = state.clone();
    gesture_drag.connect_drag_begin(move |_, x, y| {
        let mut st = state_start.borrow_mut();
        st.drag = DragStatus::Pending {
            start_x: x,
            start_y: y,
        };
    });

    let state_update = state.clone();
    let da_update = drawing_area.clone();
    gesture_drag.connect_drag_update(move |_, offset_x, offset_y| {
        let mut st = state_update.borrow_mut();
        match st.drag {
            DragStatus::Pending { start_x, start_y }
            | DragStatus::Dragging {
                start_x, start_y, ..
            } => {
                let dist_sq = offset_x * offset_x + offset_y * offset_y;
                // 5px threshold to separate intentional drag from a stationary click
                if dist_sq >= 25.0 {
                    st.drag = DragStatus::Dragging {
                        start_x,
                        start_y,
                        current_x: start_x + offset_x,
                        current_y: start_y + offset_y,
                    };
                    da_update.set_cursor_from_name(Some("crosshair"));
                    da_update.queue_draw();
                }
            }
            DragStatus::Idle => {}
        }
    });

    let state_end = state.clone();
    let win_weak = window.downgrade();
    let on_selected = Rc::new(on_selected);
    let on_dismiss_rc = Rc::new(on_dismiss);
    let on_dismiss_drag = on_dismiss_rc.clone();
    let windows_end = snapped_windows.clone();

    gesture_drag.connect_drag_end(move |_, offset_x, offset_y| {
        let current_status = {
            let mut st = state_end.borrow_mut();
            let prev = st.drag;
            st.drag = DragStatus::Idle;
            prev
        };

        if let Some(win) = win_weak.upgrade() {
            win.close();
        }

        match current_status {
            DragStatus::Dragging {
                start_x, start_y, ..
            } => {
                let end_x = start_x + offset_x;
                let end_y = start_y + offset_y;
                let rx = start_x.min(end_x);
                let ry = start_y.min(end_y);
                let rw = (end_x - start_x).abs();
                let rh = (end_y - start_y).abs();

                if rw >= 8.0 && rh >= 8.0 {
                    let rect = Rect {
                        x: rx.round() as i32 + monitor_origin.0,
                        y: ry.round() as i32 + monitor_origin.1,
                        width: rw.round() as u32,
                        height: rh.round() as u32,
                    };
                    info!("Custom selection completed: {:?}", rect);
                    on_selected(rect);
                } else {
                    on_dismiss_drag();
                }
            }
            DragStatus::Pending { start_x, start_y } => {
                // Clicked without dragging: capture snapped window if present
                let hovered = {
                    let st = state_end.borrow();
                    st.hovered_window.clone()
                };

                let target_window = hovered.or_else(|| {
                    find_snapped_window(&windows_end, start_x, start_y).cloned()
                });

                if let Some(snapped) = target_window {
                    info!(
                        "Window smart-snapped and captured: {:?} (class: {}, title: {})",
                        snapped.compositor_rect, snapped.class, snapped.title
                    );
                    on_selected(snapped.compositor_rect);
                } else {
                    info!("Clicked empty screen area; dismissing selection");
                    on_dismiss_drag();
                }
            }
            DragStatus::Idle => {
                on_dismiss_drag();
            }
        }
    });
    drawing_area.add_controller(gesture_drag);

    // Escape key to dismiss selection
    let key_controller = gtk4::EventControllerKey::new();
    let win_weak_key = window.downgrade();
    let on_dismiss_key = on_dismiss_rc.clone();
    key_controller.connect_key_pressed(move |_, key, _, _| {
        if key == gdk::Key::Escape {
            if let Some(win) = win_weak_key.upgrade() {
                win.close();
            }
            on_dismiss_key();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(key_controller);

    window.set_child(Some(&drawing_area));
    window.present();
}

fn draw_corner_brackets(
    cr: &cairo::Context,
    rx: f64,
    ry: f64,
    rw: f64,
    rh: f64,
    max_len: f64,
    line_width: f64,
) {
    cr.set_source_rgba(0.55, 0.75, 1.0, 1.0);
    cr.set_line_width(line_width);
    let corner_len = max_len.min(rw / 3.0).min(rh / 3.0);

    // Top-left
    cr.move_to(rx, ry + corner_len);
    cr.line_to(rx, ry);
    cr.line_to(rx + corner_len, ry);

    // Top-right
    cr.move_to(rx + rw - corner_len, ry);
    cr.line_to(rx + rw, ry);
    cr.line_to(rx + rw, ry + corner_len);

    // Bottom-left
    cr.move_to(rx, ry + rh - corner_len);
    cr.line_to(rx, ry + rh);
    cr.line_to(rx + corner_len, ry + rh);

    // Bottom-right
    cr.move_to(rx + rw - corner_len, ry + rh);
    cr.line_to(rx + rw, ry + rh);
    cr.line_to(rx + rw, ry + rh - corner_len);
    let _ = cr.stroke();
}

fn draw_rounded_rect(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(x + r, y + h - r, r, std::f64::consts::FRAC_PI_2, std::f64::consts::PI);
    cr.arc(
        x + r,
        y + r,
        r,
        std::f64::consts::PI,
        3.0 * std::f64::consts::FRAC_PI_2,
    );
    cr.close_path();
}

fn draw_snap_badge(cr: &cairo::Context, snapped: &SnappedWindow, screen_w: f64) {
    let rx = snapped.local_rect.x as f64;
    let ry = snapped.local_rect.y as f64;
    let rw = snapped.local_rect.width as f64;
    let display_title = if !snapped.title.trim().is_empty() {
        let trimmed = snapped.title.trim();
        if trimmed.chars().count() > 34 {
            let truncated: String = trimmed.chars().take(32).collect();
            format!("{}…", truncated)
        } else {
            trimmed.to_string()
        }
    } else if !snapped.class.trim().is_empty() {
        snapped.class.trim().to_string()
    } else {
        "Window".to_string()
    };

    let badge_text = format!(
        "{} • {}×{}  (Click to capture • Drag for custom)",
        display_title, snapped.local_rect.width, snapped.local_rect.height
    );

    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    cr.set_font_size(12.0);

    let extents = match cr.text_extents(&badge_text) {
        Ok(ext) => ext,
        Err(_) => return,
    };

    let pill_w = extents.width() + 28.0;
    let pill_h = 28.0;

    let pill_y = if ry >= pill_h + 8.0 {
        ry - pill_h - 6.0
    } else {
        (ry + 10.0).min(screen_w)
    };
    let pill_x = (rx + (rw - pill_w) / 2.0).clamp(12.0, (screen_w - pill_w - 12.0).max(12.0));

    // Pill background
    cr.set_source_rgba(0.08, 0.10, 0.14, 0.92);
    draw_rounded_rect(cr, pill_x, pill_y, pill_w, pill_h, 6.0);
    let _ = cr.fill();

    // Pill border
    cr.set_source_rgba(0.35, 0.60, 1.0, 0.75);
    cr.set_line_width(1.5);
    draw_rounded_rect(cr, pill_x, pill_y, pill_w, pill_h, 6.0);
    let _ = cr.stroke();

    // Pill text
    cr.set_source_rgba(0.95, 0.96, 0.99, 1.0);
    let text_x = pill_x + 14.0;
    let text_y = pill_y + (pill_h + extents.height()) / 2.0;
    cr.move_to(text_x, text_y);
    let _ = cr.show_text(&badge_text);
}

fn draw_dimension_badge(
    cr: &cairo::Context,
    rx: f64,
    ry: f64,
    rw: f64,
    rh: f64,
    screen_w: f64,
    screen_h: f64,
) {
    if rw < 40.0 || rh < 24.0 {
        return;
    }

    let dim_text = format!("{} × {}", rw.round() as i32, rh.round() as i32);
    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    cr.set_font_size(11.0);

    let extents = match cr.text_extents(&dim_text) {
        Ok(ext) => ext,
        Err(_) => return,
    };

    let pill_w = extents.width() + 16.0;
    let pill_h = 22.0;

    let pill_y = if ry + rh + pill_h + 8.0 <= screen_h {
        ry + rh + 6.0
    } else if ry >= pill_h + 8.0 {
        ry - pill_h - 6.0
    } else {
        ry + rh - pill_h - 6.0
    };
    let pill_x = (rx + rw - pill_w).clamp(6.0, screen_w - pill_w - 6.0);

    cr.set_source_rgba(0.08, 0.10, 0.14, 0.90);
    draw_rounded_rect(cr, pill_x, pill_y, pill_w, pill_h, 5.0);
    let _ = cr.fill();

    cr.set_source_rgba(0.35, 0.60, 1.0, 0.60);
    cr.set_line_width(1.0);
    draw_rounded_rect(cr, pill_x, pill_y, pill_w, pill_h, 5.0);
    let _ = cr.stroke();

    cr.set_source_rgba(0.92, 0.94, 0.98, 1.0);
    let text_x = pill_x + 8.0;
    let text_y = pill_y + (pill_h + extents.height()) / 2.0;
    cr.move_to(text_x, text_y);
    let _ = cr.show_text(&dim_text);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_contains_point() {
        let rect = Rect {
            x: 100,
            y: 100,
            width: 200,
            height: 150,
        };
        assert!(rect.contains(100.0, 100.0));
        assert!(rect.contains(150.0, 150.0));
        assert!(rect.contains(300.0, 250.0));
        assert!(!rect.contains(99.0, 100.0));
        assert!(!rect.contains(301.0, 150.0));
        assert!(!rect.contains(150.0, 251.0));
    }

    #[test]
    fn convert_windows_to_local_clamps_and_translates() {
        let windows = vec![
            WindowGeometry {
                x: 100,
                y: 200,
                width: 800,
                height: 600,
                title: "Editor".to_string(),
                class: "code".to_string(),
            },
            // Partially offscreen to the left
            WindowGeometry {
                x: -50,
                y: 100,
                width: 400,
                height: 300,
                title: "Terminal".to_string(),
                class: "alacritty".to_string(),
            },
            // Completely outside monitor
            WindowGeometry {
                x: 2500,
                y: 100,
                width: 500,
                height: 400,
                title: "Offscreen".to_string(),
                class: "other".to_string(),
            },
        ];

        let monitor_origin = (0, 0);
        let monitor_size = (1920, 1080);
        let snapped = convert_windows_to_local(&windows, monitor_origin, monitor_size);

        assert_eq!(snapped.len(), 2);
        assert_eq!(snapped[0].local_rect.x, 100);
        assert_eq!(snapped[0].local_rect.y, 200);
        assert_eq!(snapped[0].local_rect.width, 800);
        assert_eq!(snapped[0].local_rect.height, 600);

        // Clamped terminal
        assert_eq!(snapped[1].local_rect.x, 0);
        assert_eq!(snapped[1].local_rect.y, 100);
        assert_eq!(snapped[1].local_rect.width, 350);
        assert_eq!(snapped[1].local_rect.height, 300);
    }

    #[test]
    fn convert_windows_supports_multi_monitor_offset() {
        let windows = vec![WindowGeometry {
            x: 2020,
            y: 150,
            width: 1000,
            height: 700,
            title: "Browser".to_string(),
            class: "firefox".to_string(),
        }];

        let monitor_origin = (1920, 0);
        let monitor_size = (1920, 1080);
        let snapped = convert_windows_to_local(&windows, monitor_origin, monitor_size);

        assert_eq!(snapped.len(), 1);
        assert_eq!(snapped[0].local_rect.x, 100);
        assert_eq!(snapped[0].local_rect.y, 150);
        assert_eq!(snapped[0].compositor_rect.x, 2020);
        assert_eq!(snapped[0].compositor_rect.y, 150);
    }

    #[test]
    fn find_snapped_window_picks_first_matching_in_order() {
        let windows = vec![
            SnappedWindow {
                compositor_rect: Rect {
                    x: 100,
                    y: 100,
                    width: 200,
                    height: 200,
                },
                local_rect: Rect {
                    x: 100,
                    y: 100,
                    width: 200,
                    height: 200,
                },
                title: "Floating Window (Topmost)".to_string(),
                class: "floating".to_string(),
            },
            SnappedWindow {
                compositor_rect: Rect {
                    x: 50,
                    y: 50,
                    width: 500,
                    height: 500,
                },
                local_rect: Rect {
                    x: 50,
                    y: 50,
                    width: 500,
                    height: 500,
                },
                title: "Tiled Window (Underneath)".to_string(),
                class: "tiled".to_string(),
            },
        ];

        // Cursor is inside both windows, should pick the topmost one (first in vector)
        let found = find_snapped_window(&windows, 150.0, 150.0).unwrap();
        assert_eq!(found.class, "floating");

        // Cursor is only inside tiled window
        let found2 = find_snapped_window(&windows, 60.0, 60.0).unwrap();
        assert_eq!(found2.class, "tiled");

        // Cursor is outside both
        assert!(find_snapped_window(&windows, 10.0, 10.0).is_none());
    }
}
