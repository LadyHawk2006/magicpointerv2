pub mod geometry;
pub mod render;

pub use geometry::{
    Rect, SNAP_DISTANCE, SelectionMode, SnappedWindow, collect_guides, convert_windows_to_local,
    cycle_window_index, find_all_snapped_windows, find_nearest_guide, find_snapped_window,
};

use crate::hypr_ipc;
use crate::prompt_overlay;
use gtk4::cairo;
use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::RefCell;
use std::rc::Rc;
use tracing::info;

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

        cr.set_source_rgba(0.0, 0.0, 0.0, 0.32);
        let _ = cr.paint();

        let current_state = state_draw.borrow();
        render::draw_mode_pill(cr, current_state.mode, sw);

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
                    cr.save().expect("cairo save");
                    cr.set_operator(cairo::Operator::Clear);
                    cr.rectangle(rx, ry, rw, rh);
                    let _ = cr.fill();
                    cr.restore().expect("cairo restore");

                    cr.set_source_rgba(0.23, 0.51, 0.96, 0.95);
                    cr.set_line_width(2.0);
                    cr.rectangle(rx, ry, rw, rh);
                    let _ = cr.stroke();

                    render::draw_corner_brackets(cr, rx, ry, rw, rh, 14.0, 3.0);
                    render::draw_dimension_badge(cr, rx, ry, rw, rh, sw, sh);
                }
            }
            _ => match current_state.mode {
                SelectionMode::FullMonitor => {
                    cr.save().expect("cairo save");
                    cr.set_operator(cairo::Operator::Clear);
                    cr.rectangle(0.0, 0.0, sw, sh);
                    let _ = cr.fill();
                    cr.restore().expect("cairo restore");

                    cr.set_source_rgba(0.23, 0.51, 0.96, 0.04);
                    cr.rectangle(0.0, 0.0, sw, sh);
                    let _ = cr.fill();

                    cr.set_source_rgba(0.23, 0.51, 0.96, 0.90);
                    cr.set_line_width(3.0);
                    cr.rectangle(0.0, 0.0, sw, sh);
                    let _ = cr.stroke();

                    render::draw_corner_brackets(cr, 4.0, 4.0, sw - 8.0, sh - 8.0, 28.0, 4.0);
                    render::draw_fullscreen_hint(cr, sw, sh);
                }
                SelectionMode::WindowSnap => {
                    if let Some(ref snapped) = current_state.hovered_window {
                        let rx = snapped.local_rect.x as f64;
                        let ry = snapped.local_rect.y as f64;
                        let rw = snapped.local_rect.width as f64;
                        let rh = snapped.local_rect.height as f64;

                        cr.save().expect("cairo save");
                        cr.set_operator(cairo::Operator::Clear);
                        cr.rectangle(rx, ry, rw, rh);
                        let _ = cr.fill();
                        cr.restore().expect("cairo restore");

                        cr.set_source_rgba(0.23, 0.51, 0.96, 0.06);
                        cr.rectangle(rx, ry, rw, rh);
                        let _ = cr.fill();

                        cr.set_source_rgba(0.23, 0.51, 0.96, 0.92);
                        cr.set_line_width(2.5);
                        cr.rectangle(rx, ry, rw, rh);
                        let _ = cr.stroke();

                        render::draw_corner_brackets(cr, rx, ry, rw, rh, 18.0, 3.5);
                        render::draw_snap_badge(
                            cr,
                            snapped,
                            current_state.cycle_index,
                            current_state.overlapping_windows.len(),
                            sw,
                        );
                    }
                }
                SelectionMode::RegionSelection => {}
            },
        }
    });

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

                    if let Some(snap_x) = find_nearest_guide(&x_guides_update, cur_x, SNAP_DISTANCE)
                    {
                        cur_x = snap_x;
                        st.magnetic_snap_x = Some(snap_x);
                    } else {
                        st.magnetic_snap_x = None;
                    }

                    if let Some(snap_y) = find_nearest_guide(&y_guides_update, cur_y, SNAP_DISTANCE)
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
    let app_end = app.clone();
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

        if mode == SelectionMode::FullMonitor {
            let full_rect = Rect {
                x: monitor_origin.0,
                y: monitor_origin.1,
                width: monitor_size.0.max(1) as u32,
                height: monitor_size.1.max(1) as u32,
            };
            info!("Full monitor captured: {:?}", full_rect);
            dismiss_and_capture(&app_end, win_weak.upgrade(), full_rect, &on_selected);
            return;
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
                    dismiss_and_capture(&app_end, win_weak.upgrade(), rect, &on_selected);
                } else {
                    if let Some(win) = win_weak.upgrade() {
                        win.close();
                    }
                    on_dismiss_drag();
                }
            }
            DragStatus::Pending { start_x, start_y } => {
                if mode == SelectionMode::WindowSnap {
                    let hovered = {
                        let st = state_end.borrow();
                        st.hovered_window.clone()
                    };

                    let target = hovered
                        .or_else(|| find_snapped_window(&windows_end, start_x, start_y).cloned());

                    if let Some(snapped) = target {
                        info!(
                            "Window smart-snapped and captured: {:?} (class: {}, title: {})",
                            snapped.compositor_rect, snapped.class, snapped.title
                        );
                        dismiss_and_capture(
                            &app_end,
                            win_weak.upgrade(),
                            snapped.compositor_rect,
                            &on_selected,
                        );
                    } else {
                        info!("Clicked empty screen area; dismissing selection");
                        if let Some(win) = win_weak.upgrade() {
                            win.close();
                        }
                        on_dismiss_drag();
                    }
                } else {
                    info!("Clicked without drag in region mode; dismissing");
                    if let Some(win) = win_weak.upgrade() {
                        win.close();
                    }
                    on_dismiss_drag();
                }
            }
            DragStatus::Idle => {
                if let Some(win) = win_weak.upgrade() {
                    win.close();
                }
                on_dismiss_drag();
            }
        }
    });
    drawing_area.add_controller(gesture_drag);

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

fn dismiss_and_capture<F>(
    app: &gtk4::Application,
    window: Option<gtk4::ApplicationWindow>,
    rect: Rect,
    on_selected: &Rc<F>,
) where
    F: Fn(Rect) + 'static,
{
    // Prevent GTK application from exiting when the selection window closes
    // before the prompt overlay window is created.
    let hold_guard = app.hold();

    if let Some(win) = window {
        win.set_opacity(0.0);
        win.set_visible(false);
        win.close();
    }
    if let Some(display) = gdk::Display::default() {
        display.sync();
    }

    let on_selected = on_selected.clone();
    glib::timeout_add_local_once(std::time::Duration::from_millis(75), move || {
        let _guard = hold_guard;
        on_selected(rect);
    });
}
