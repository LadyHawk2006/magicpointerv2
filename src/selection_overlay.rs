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

const SNAP_DISTANCE: f64 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionMode {
    WindowSnap,
    RegionSelection,
    FullMonitor,
}

impl SelectionMode {
    pub fn next(self) -> Self {
        match self {
            Self::WindowSnap => Self::RegionSelection,
            Self::RegionSelection => Self::FullMonitor,
            Self::FullMonitor => Self::WindowSnap,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::WindowSnap => "Window Snap",
            Self::RegionSelection => "Region Selection",
            Self::FullMonitor => "Full Monitor",
        }
    }
}

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

            if local_x + local_w <= 0
                || local_y + local_h <= 0
                || local_x >= monitor_size.0
                || local_y >= monitor_size.1
            {
                return None;
            }

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

pub fn find_all_snapped_windows(
    windows: &[SnappedWindow],
    cursor_x: f64,
    cursor_y: f64,
) -> Vec<SnappedWindow> {
    windows
        .iter()
        .filter(|w| w.local_rect.contains(cursor_x, cursor_y))
        .cloned()
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

pub fn cycle_window_index(current: usize, total: usize, forward: bool) -> usize {
    if total == 0 {
        return 0;
    }
    if forward {
        (current + 1) % total
    } else {
        (current + total - 1) % total
    }
}

pub fn collect_guides(
    windows: &[SnappedWindow],
    monitor_width: i32,
    monitor_height: i32,
) -> (Vec<f64>, Vec<f64>) {
    let mut x_guides = vec![0.0, monitor_width as f64];
    let mut y_guides = vec![0.0, monitor_height as f64];

    for w in windows {
        let rx = w.local_rect.x as f64;
        let ry = w.local_rect.y as f64;
        let rw = w.local_rect.width as f64;
        let rh = w.local_rect.height as f64;
        x_guides.push(rx);
        x_guides.push(rx + rw);
        y_guides.push(ry);
        y_guides.push(ry + rh);
    }

    x_guides.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    x_guides.dedup();
    y_guides.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    y_guides.dedup();

    (x_guides, y_guides)
}

pub fn find_nearest_guide(guides: &[f64], val: f64, threshold: f64) -> Option<f64> {
    guides
        .iter()
        .copied()
        .filter(|&g| (g - val).abs() <= threshold)
        .min_by(|a, b| {
            let diff_a = (a - val).abs();
            let diff_b = (b - val).abs();
            diff_a
                .partial_cmp(&diff_b)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
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
    mode: SelectionMode,
    drag: DragStatus,
    hovered_window: Option<SnappedWindow>,
    overlapping_windows: Vec<SnappedWindow>,
    cycle_index: usize,
    magnetic_snap_x: Option<f64>,
    magnetic_snap_y: Option<f64>,
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

    let (x_guides, y_guides) = collect_guides(&snapped_windows, monitor_size.0, monitor_size.1);
    let x_guides = Rc::new(x_guides);
    let y_guides = Rc::new(y_guides);

    let local_cursor = (
        (cursor_pos.0 - monitor_origin.0) as f64,
        (cursor_pos.1 - monitor_origin.1) as f64,
    );
    let initial_overlapping =
        find_all_snapped_windows(&snapped_windows, local_cursor.0, local_cursor.1);
    let initial_hover = initial_overlapping.first().cloned();

    let state = Rc::new(RefCell::new(OverlayState {
        mode: SelectionMode::WindowSnap,
        drag: DragStatus::Idle,
        hovered_window: initial_hover.clone(),
        overlapping_windows: initial_overlapping,
        cycle_index: 0,
        magnetic_snap_x: None,
        magnetic_snap_y: None,
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
        let sw = width as f64;
        let sh = height as f64;

        // Base dark translucent backdrop
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.32);
        let _ = cr.paint();

        let current_state = state_draw.borrow();

        // 1. Draw top floating Mode Switcher Pill
        draw_mode_pill(cr, current_state.mode, sw);

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

                // Draw magnetic snap guideline alignment rays
                if let Some(sx) = current_state.magnetic_snap_x {
                    cr.set_source_rgba(0.20, 0.85, 0.95, 0.65);
                    cr.set_line_width(1.0);
                    cr.move_to(sx, 0.0);
                    cr.line_to(sx, sh);
                    let _ = cr.stroke();
                }
                if let Some(sy) = current_state.magnetic_snap_y {
                    cr.set_source_rgba(0.20, 0.85, 0.95, 0.65);
                    cr.set_line_width(1.0);
                    cr.move_to(0.0, sy);
                    cr.line_to(sw, sy);
                    let _ = cr.stroke();
                }

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
                    draw_dimension_badge(cr, rx, ry, rw, rh, sw, sh);
                }
            }
            _ => {
                match current_state.mode {
                    SelectionMode::FullMonitor => {
                        // Spotlight entire monitor
                        cr.save().expect("cairo save");
                        cr.set_operator(cairo::Operator::Clear);
                        cr.rectangle(0.0, 0.0, sw, sh);
                        let _ = cr.fill();
                        cr.restore().expect("cairo restore");

                        // Subtle inner tint & border
                        cr.set_source_rgba(0.23, 0.51, 0.96, 0.04);
                        cr.rectangle(0.0, 0.0, sw, sh);
                        let _ = cr.fill();

                        cr.set_source_rgba(0.23, 0.51, 0.96, 0.90);
                        cr.set_line_width(3.0);
                        cr.rectangle(0.0, 0.0, sw, sh);
                        let _ = cr.stroke();

                        draw_corner_brackets(cr, 4.0, 4.0, sw - 8.0, sh - 8.0, 28.0, 4.0);

                        // Center banner
                        draw_fullscreen_hint(cr, sw, sh);
                    }
                    SelectionMode::WindowSnap => {
                        if let Some(ref snapped) = current_state.hovered_window {
                            let rx = snapped.local_rect.x as f64;
                            let ry = snapped.local_rect.y as f64;
                            let rw = snapped.local_rect.width as f64;
                            let rh = snapped.local_rect.height as f64;

                            // Spotlight snapped window
                            cr.save().expect("cairo save");
                            cr.set_operator(cairo::Operator::Clear);
                            cr.rectangle(rx, ry, rw, rh);
                            let _ = cr.fill();
                            cr.restore().expect("cairo restore");

                            // Translucent highlight fill
                            cr.set_source_rgba(0.23, 0.51, 0.96, 0.06);
                            cr.rectangle(rx, ry, rw, rh);
                            let _ = cr.fill();

                            // Snap border
                            cr.set_source_rgba(0.23, 0.51, 0.96, 0.92);
                            cr.set_line_width(2.5);
                            cr.rectangle(rx, ry, rw, rh);
                            let _ = cr.stroke();

                            draw_corner_brackets(cr, rx, ry, rw, rh, 18.0, 3.5);

                            // Smart snap pill badge showing title & overlap cycle indicators
                            draw_snap_badge(
                                cr,
                                snapped,
                                current_state.cycle_index,
                                current_state.overlapping_windows.len(),
                                sw,
                            );
                        }
                    }
                    SelectionMode::RegionSelection => {
                        // Region selection only: clean dark overlay awaiting drag
                    }
                }
            }
        }
    });

    // Motion tracking
    let motion_controller = gtk4::EventControllerMotion::new();
    let state_motion = state.clone();
    let windows_motion = snapped_windows.clone();
    let da_motion = drawing_area.clone();

    motion_controller.connect_motion(move |_, x, y| {
        let mut st = state_motion.borrow_mut();
        if let DragStatus::Dragging { .. } = st.drag {
            return;
        }

        match st.mode {
            SelectionMode::WindowSnap => {
                let matching = find_all_snapped_windows(&windows_motion, x, y);
                if matching != st.overlapping_windows {
                    st.overlapping_windows = matching;
                    st.cycle_index = 0;
                    st.hovered_window = st.overlapping_windows.first().cloned();
                    if st.hovered_window.is_some() {
                        da_motion.set_cursor_from_name(Some("pointer"));
                    } else {
                        da_motion.set_cursor_from_name(Some("crosshair"));
                    }
                    da_motion.queue_draw();
                }
            }
            SelectionMode::FullMonitor => {
                da_motion.set_cursor_from_name(Some("pointer"));
            }
            SelectionMode::RegionSelection => {
                da_motion.set_cursor_from_name(Some("crosshair"));
            }
        }
    });

    let state_leave = state.clone();
    let da_leave = drawing_area.clone();
    motion_controller.connect_leave(move |_| {
        let mut st = state_leave.borrow_mut();
        if st.hovered_window.is_some() {
            st.hovered_window = None;
            st.overlapping_windows.clear();
            st.cycle_index = 0;
            da_leave.set_cursor_from_name(Some("crosshair"));
            da_leave.queue_draw();
        }
    });
    drawing_area.add_controller(motion_controller);

    // Mouse scroll controller for cycling overlapping windows
    let scroll_controller =
        gtk4::EventControllerScroll::new(gtk4::EventControllerScrollFlags::VERTICAL);
    let state_scroll = state.clone();
    let da_scroll = drawing_area.clone();
    scroll_controller.connect_scroll(move |_, _dx, dy| {
        let mut st = state_scroll.borrow_mut();
        if st.mode == SelectionMode::WindowSnap && st.overlapping_windows.len() > 1 {
            let forward = dy > 0.0;
            st.cycle_index =
                cycle_window_index(st.cycle_index, st.overlapping_windows.len(), forward);
            st.hovered_window = Some(st.overlapping_windows[st.cycle_index].clone());
            da_scroll.queue_draw();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    drawing_area.add_controller(scroll_controller);

    // Drag gesture with Edge Snapping Magnetism
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
    let x_guides_update = x_guides.clone();
    let y_guides_update = y_guides.clone();
    gesture_drag.connect_drag_update(move |_, offset_x, offset_y| {
        let mut st = state_update.borrow_mut();
        if st.mode == SelectionMode::FullMonitor {
            return;
        }

        match st.drag {
            DragStatus::Pending { start_x, start_y }
            | DragStatus::Dragging {
                start_x, start_y, ..
            } => {
                let dist_sq = offset_x * offset_x + offset_y * offset_y;
                if dist_sq >= 25.0 {
                    let mut cur_x = start_x + offset_x;
                    let mut cur_y = start_y + offset_y;

                    // Edge Snapping Magnetism: snap moving edge to nearest guide
                    if let Some(snap_x) =
                        find_nearest_guide(&x_guides_update, cur_x, SNAP_DISTANCE)
                    {
                        cur_x = snap_x;
                        st.magnetic_snap_x = Some(snap_x);
                    } else {
                        st.magnetic_snap_x = None;
                    }

                    if let Some(snap_y) =
                        find_nearest_guide(&y_guides_update, cur_y, SNAP_DISTANCE)
                    {
                        cur_y = snap_y;
                        st.magnetic_snap_y = Some(snap_y);
                    } else {
                        st.magnetic_snap_y = None;
                    }

                    st.drag = DragStatus::Dragging {
                        start_x,
                        start_y,
                        current_x: cur_x,
                        current_y: cur_y,
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

    gesture_drag.connect_drag_end(move |_, _offset_x, _offset_y| {
        let current_state = {
            let mut st = state_end.borrow_mut();
            let prev = (st.mode, st.drag);
            st.drag = DragStatus::Idle;
            st.magnetic_snap_x = None;
            st.magnetic_snap_y = None;
            prev
        };

        let (mode, drag_status) = current_state;

        // In FullMonitor mode: single click captures entire monitor
        if mode == SelectionMode::FullMonitor {
            if let Some(win) = win_weak.upgrade() {
                win.close();
            }
            let full_rect = Rect {
                x: monitor_origin.0,
                y: monitor_origin.1,
                width: monitor_size.0.max(1) as u32,
                height: monitor_size.1.max(1) as u32,
            };
            info!("Full monitor captured: {:?}", full_rect);
            on_selected(full_rect);
            return;
        }

        if let Some(win) = win_weak.upgrade() {
            win.close();
        }

        match drag_status {
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
                if mode == SelectionMode::WindowSnap {
                    let hovered = {
                        let st = state_end.borrow();
                        st.hovered_window.clone()
                    };

                    let target = hovered.or_else(|| {
                        find_snapped_window(&windows_end, start_x, start_y).cloned()
                    });

                    if let Some(snapped) = target {
                        info!(
                            "Window smart-snapped and captured: {:?} (class: {}, title: {})",
                            snapped.compositor_rect, snapped.class, snapped.title
                        );
                        on_selected(snapped.compositor_rect);
                    } else {
                        info!("Clicked empty screen area; dismissing selection");
                        on_dismiss_drag();
                    }
                } else {
                    info!("Clicked without drag in region mode; dismissing");
                    on_dismiss_drag();
                }
            }
            DragStatus::Idle => {
                on_dismiss_drag();
            }
        }
    });
    drawing_area.add_controller(gesture_drag);

    // Keyboard controller: Escape (dismiss), Space (cycle modes), Tab (cycle overlapping windows)
    let key_controller = gtk4::EventControllerKey::new();
    let win_weak_key = window.downgrade();
    let on_dismiss_key = on_dismiss_rc.clone();
    let state_key = state.clone();
    let da_key = drawing_area.clone();

    key_controller.connect_key_pressed(move |_, key, _, modifier| {
        if key == gdk::Key::Escape {
            if let Some(win) = win_weak_key.upgrade() {
                win.close();
            }
            on_dismiss_key();
            glib::Propagation::Stop
        } else if key == gdk::Key::space {
            // Spacebar Mode Switcher
            let mut st = state_key.borrow_mut();
            st.mode = st.mode.next();
            match st.mode {
                SelectionMode::WindowSnap => {
                    st.hovered_window = st.overlapping_windows.first().cloned();
                    if st.hovered_window.is_some() {
                        da_key.set_cursor_from_name(Some("pointer"));
                    } else {
                        da_key.set_cursor_from_name(Some("crosshair"));
                    }
                }
                SelectionMode::FullMonitor => {
                    st.hovered_window = None;
                    da_key.set_cursor_from_name(Some("pointer"));
                }
                SelectionMode::RegionSelection => {
                    st.hovered_window = None;
                    da_key.set_cursor_from_name(Some("crosshair"));
                }
            }
            da_key.queue_draw();
            glib::Propagation::Stop
        } else if key == gdk::Key::Tab || key == gdk::Key::ISO_Left_Tab {
            // Tab / Shift+Tab to Cycle Overlapping Windows
            let mut st = state_key.borrow_mut();
            if st.mode == SelectionMode::WindowSnap && st.overlapping_windows.len() > 1 {
                let forward = !modifier.contains(gdk::ModifierType::SHIFT_MASK);
                st.cycle_index =
                    cycle_window_index(st.cycle_index, st.overlapping_windows.len(), forward);
                st.hovered_window = Some(st.overlapping_windows[st.cycle_index].clone());
                da_key.queue_draw();
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(key_controller);

    window.set_child(Some(&drawing_area));
    window.present();
}

fn draw_mode_pill(cr: &cairo::Context, mode: SelectionMode, screen_w: f64) {
    let mode_str = mode.title();
    let text = match mode {
        SelectionMode::WindowSnap => {
            format!("[Space] Mode: {}  •  Click to capture, Drag for box", mode_str)
        }
        SelectionMode::RegionSelection => {
            format!("[Space] Mode: {}  •  Drag custom crop box", mode_str)
        }
        SelectionMode::FullMonitor => {
            format!("[Space] Mode: {}  •  Click anywhere to capture full screen", mode_str)
        }
    };

    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    cr.set_font_size(12.0);

    let extents = match cr.text_extents(&text) {
        Ok(ext) => ext,
        Err(_) => return,
    };

    let pill_w = extents.width() + 28.0;
    let pill_h = 28.0;
    let pill_x = (screen_w - pill_w) / 2.0;
    let pill_y = 14.0;

    // Pill background
    cr.set_source_rgba(0.08, 0.10, 0.14, 0.92);
    draw_rounded_rect(cr, pill_x, pill_y, pill_w, pill_h, 7.0);
    let _ = cr.fill();

    // Pill border
    cr.set_source_rgba(0.23, 0.51, 0.96, 0.80);
    cr.set_line_width(1.5);
    draw_rounded_rect(cr, pill_x, pill_y, pill_w, pill_h, 7.0);
    let _ = cr.stroke();

    // Text
    cr.set_source_rgba(0.95, 0.96, 0.99, 1.0);
    let text_x = pill_x + 14.0;
    let text_y = pill_y + (pill_h + extents.height()) / 2.0;
    cr.move_to(text_x, text_y);
    let _ = cr.show_text(&text);
}

fn draw_fullscreen_hint(cr: &cairo::Context, screen_w: f64, screen_h: f64) {
    let text = format!(
        "Full Screen ({} × {})  •  Click to capture",
        screen_w.round() as i32,
        screen_h.round() as i32
    );
    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    cr.set_font_size(14.0);

    let extents = match cr.text_extents(&text) {
        Ok(ext) => ext,
        Err(_) => return,
    };

    let pill_w = extents.width() + 32.0;
    let pill_h = 36.0;
    let pill_x = (screen_w - pill_w) / 2.0;
    let pill_y = (screen_h - pill_h) / 2.0;

    cr.set_source_rgba(0.08, 0.10, 0.14, 0.94);
    draw_rounded_rect(cr, pill_x, pill_y, pill_w, pill_h, 9.0);
    let _ = cr.fill();

    cr.set_source_rgba(0.23, 0.51, 0.96, 0.90);
    cr.set_line_width(2.0);
    draw_rounded_rect(cr, pill_x, pill_y, pill_w, pill_h, 9.0);
    let _ = cr.stroke();

    cr.set_source_rgba(0.95, 0.96, 0.99, 1.0);
    let text_x = pill_x + 16.0;
    let text_y = pill_y + (pill_h + extents.height()) / 2.0;
    cr.move_to(text_x, text_y);
    let _ = cr.show_text(&text);
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

fn draw_snap_badge(
    cr: &cairo::Context,
    snapped: &SnappedWindow,
    cycle_index: usize,
    total_overlapping: usize,
    screen_w: f64,
) {
    let rx = snapped.local_rect.x as f64;
    let ry = snapped.local_rect.y as f64;
    let rw = snapped.local_rect.width as f64;

    let display_title = if !snapped.title.trim().is_empty() {
        let trimmed = snapped.title.trim();
        if trimmed.chars().count() > 30 {
            let truncated: String = trimmed.chars().take(28).collect();
            format!("{}…", truncated)
        } else {
            trimmed.to_string()
        }
    } else if !snapped.class.trim().is_empty() {
        snapped.class.trim().to_string()
    } else {
        "Window".to_string()
    };

    let badge_text = if total_overlapping > 1 {
        format!(
            "[{}/{}] {} • {}×{}  (Tab/Scroll to cycle • Click to capture)",
            cycle_index + 1,
            total_overlapping,
            display_title,
            snapped.local_rect.width,
            snapped.local_rect.height
        )
    } else {
        format!(
            "{} • {}×{}  (Click to capture • Drag for custom)",
            display_title, snapped.local_rect.width, snapped.local_rect.height
        )
    };

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
            WindowGeometry {
                x: -50,
                y: 100,
                width: 400,
                height: 300,
                title: "Terminal".to_string(),
                class: "alacritty".to_string(),
            },
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
    fn mode_switcher_cycles_correctly() {
        let mode = SelectionMode::WindowSnap;
        assert_eq!(mode.next(), SelectionMode::RegionSelection);
        assert_eq!(mode.next().next(), SelectionMode::FullMonitor);
        assert_eq!(mode.next().next().next(), SelectionMode::WindowSnap);
    }

    #[test]
    fn overlapping_windows_cycle_index_forward_and_backward() {
        assert_eq!(cycle_window_index(0, 3, true), 1);
        assert_eq!(cycle_window_index(1, 3, true), 2);
        assert_eq!(cycle_window_index(2, 3, true), 0);
        assert_eq!(cycle_window_index(0, 3, false), 2);
        assert_eq!(cycle_window_index(2, 3, false), 1);
        assert_eq!(cycle_window_index(0, 0, true), 0);
    }

    #[test]
    fn edge_snapping_finds_nearest_guide() {
        let guides = vec![0.0, 100.0, 500.0, 1920.0];

        // 98.0 is within 10.0 of 100.0
        assert_eq!(find_nearest_guide(&guides, 98.0, 10.0), Some(100.0));
        assert_eq!(find_nearest_guide(&guides, 105.0, 10.0), Some(100.0));

        // 120.0 is too far from any guide
        assert_eq!(find_nearest_guide(&guides, 120.0, 10.0), None);

        // Exact match
        assert_eq!(find_nearest_guide(&guides, 500.0, 10.0), Some(500.0));
    }

    #[test]
    fn find_all_overlapping_windows() {
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
                title: "Top Window".to_string(),
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
                title: "Bottom Window".to_string(),
                class: "tiled".to_string(),
            },
        ];

        let matched = find_all_snapped_windows(&windows, 150.0, 150.0);
        assert_eq!(matched.len(), 2);
        assert_eq!(matched[0].class, "floating");
        assert_eq!(matched[1].class, "tiled");

        let single = find_all_snapped_windows(&windows, 60.0, 60.0);
        assert_eq!(single.len(), 1);
        assert_eq!(single[0].class, "tiled");
    }
}
