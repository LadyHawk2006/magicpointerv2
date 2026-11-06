use gtk4::cairo;
use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::RefCell;
use std::rc::Rc;
use tracing::info;

#[derive(Clone, Copy, Debug)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
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

    let drawing_area = gtk4::DrawingArea::new();
    drawing_area.add_css_class("selection-canvas");
    drawing_area.set_hexpand(true);
    drawing_area.set_vexpand(true);

    // Coordinates state: (start_x, start_y, current_x, current_y, is_dragging)
    let drag_state = Rc::new(RefCell::new(None::<(f64, f64, f64, f64)>));

    let state_draw = drag_state.clone();
    drawing_area.set_draw_func(move |_, cr, _width, _height| {
        cr.set_source_rgba(0.0, 0.0, 0.0, 0.28);
        let _ = cr.paint();

        if let Some((sx, sy, cx, cy)) = *state_draw.borrow() {
            let rx = sx.min(cx);
            let ry = sy.min(cy);
            let rw = (cx - sx).abs();
            let rh = (cy - sy).abs();

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

                // Draw accent corner highlights
                cr.set_source_rgba(0.55, 0.75, 1.0, 1.0);
                cr.set_line_width(3.0);
                let corner_len = 12.0_f64.min(rw / 3.0).min(rh / 3.0);

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
        }
    });

    let gesture_drag = gtk4::GestureDrag::new();
    let state_start = drag_state.clone();
    let da_start = drawing_area.clone();
    gesture_drag.connect_drag_begin(move |_, x, y| {
        *state_start.borrow_mut() = Some((x, y, x, y));
        da_start.queue_draw();
    });

    let state_update = drag_state.clone();
    let da_update = drawing_area.clone();
    gesture_drag.connect_drag_update(move |_gesture, offset_x, offset_y| {
        let update_opt = {
            let state = state_update.borrow();
            state.map(|(start_x, start_y, _, _)| {
                (start_x, start_y, start_x + offset_x, start_y + offset_y)
            })
        };
        if let Some(new_state) = update_opt {
            *state_update.borrow_mut() = Some(new_state);
            da_update.queue_draw();
        }
    });

    let state_end = drag_state.clone();
    let win_weak = window.downgrade();
    let on_selected = Rc::new(on_selected);
    let on_dismiss_rc = Rc::new(on_dismiss);
    let on_dismiss_drag = on_dismiss_rc.clone();

    gesture_drag.connect_drag_end(move |_, offset_x, offset_y| {
        let finish_opt = {
            let state = state_end.borrow();
            state.map(|(start_x, start_y, _, _)| {
                (start_x, start_y, start_x + offset_x, start_y + offset_y)
            })
        };

        if let Some((start_x, start_y, end_x, end_y)) = finish_opt {
            let rx = start_x.min(end_x);
            let ry = start_y.min(end_y);
            let rw = (end_x - start_x).abs();
            let rh = (end_y - start_y).abs();

            if let Some(win) = win_weak.upgrade() {
                win.close();
            }

            if rw >= 8.0 && rh >= 8.0 {
                let rect = Rect {
                    x: rx.round() as i32,
                    y: ry.round() as i32,
                    width: rw.round() as u32,
                    height: rh.round() as u32,
                };
                info!("Selection completed: {:?}", rect);
                on_selected(rect);
            } else {
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
