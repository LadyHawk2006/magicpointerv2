use super::annotation::{
    Annotation, AnnotationTool, draw_annotations_on_context, large_image_texture,
};
use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use image::DynamicImage;
use std::cell::RefCell;
use std::rc::Rc;

pub fn show_annotation_modal<F>(
    app: &gtk4::Application,
    monitor: &gdk::Monitor,
    monitor_geometry: gdk::Rectangle,
    image: &DynamicImage,
    initial_annotations: Vec<Annotation>,
    on_done: F,
) where
    F: FnOnce(Vec<Annotation>) + 'static,
{
    let modal_window = gtk4::ApplicationWindow::new(app);
    modal_window.init_layer_shell();
    modal_window.set_layer(Layer::Overlay);
    modal_window.set_keyboard_mode(KeyboardMode::Exclusive);
    modal_window.set_exclusive_zone(-1);
    modal_window.set_monitor(Some(monitor));

    modal_window.set_anchor(Edge::Top, true);
    modal_window.set_anchor(Edge::Bottom, true);
    modal_window.set_anchor(Edge::Left, true);
    modal_window.set_anchor(Edge::Right, true);
    modal_window.add_css_class("annotation-modal-scrim");

    let modal_card = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    modal_card.add_css_class("annotation-modal-card");
    modal_card.set_halign(gtk4::Align::Center);
    modal_card.set_valign(gtk4::Align::Center);

    let max_canvas_w = (monitor_geometry.width() - 80).clamp(400, 1280);
    let max_canvas_h = (monitor_geometry.height() - 120).clamp(300, 720);

    let (img_w, img_h) = (image.width() as f64, image.height() as f64);
    let scale = (max_canvas_w as f64 / img_w)
        .min(max_canvas_h as f64 / img_h)
        .min(1.0);
    let canvas_w = ((img_w * scale).round() as i32).max(300);
    let canvas_h = ((img_h * scale).round() as i32).max(200);

    let large_texture = large_image_texture(image, canvas_w as u32, canvas_h as u32);

    let annotations = Rc::new(RefCell::new(initial_annotations));
    let current_tool = Rc::new(RefCell::new(AnnotationTool::Pen));
    let active_drawing = Rc::new(RefCell::new(None::<Annotation>));

    let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    header.add_css_class("annotation-modal-header");

    let title_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    let title = gtk4::Label::new(Some("Markup & Annotation"));
    title.add_css_class("annotation-modal-title");
    title.set_halign(gtk4::Align::Start);
    let subtitle = gtk4::Label::new(Some(&format!("{} × {} px", image.width(), image.height())));
    subtitle.add_css_class("annotation-modal-subtitle");
    subtitle.set_halign(gtk4::Align::Start);
    title_box.append(&title);
    title_box.append(&subtitle);
    header.append(&title_box);

    let spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    header.append(&spacer);

    let tools_box = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);

    let pen_btn = gtk4::Button::with_label("Pen");
    pen_btn.add_css_class("annotation-tool-pill");
    pen_btn.add_css_class("active");

    let arrow_btn = gtk4::Button::with_label("Arrow");
    arrow_btn.add_css_class("annotation-tool-pill");

    let rect_btn = gtk4::Button::with_label("Box");
    rect_btn.add_css_class("annotation-tool-pill");

    let clear_btn = gtk4::Button::with_label("Clear");
    clear_btn.add_css_class("annotation-tool-pill");

    let done_btn = gtk4::Button::with_label("Done");
    done_btn.add_css_class("annotation-done-btn");

    tools_box.append(&pen_btn);
    tools_box.append(&arrow_btn);
    tools_box.append(&rect_btn);
    tools_box.append(&clear_btn);
    tools_box.append(&done_btn);
    header.append(&tools_box);

    modal_card.append(&header);

    let canvas_container = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    canvas_container.add_css_class("annotation-canvas-container");
    canvas_container.set_size_request(canvas_w, canvas_h);

    let canvas_overlay = gtk4::Overlay::new();
    let picture = gtk4::Picture::for_paintable(&large_texture);
    picture.set_can_shrink(true);
    picture.set_keep_aspect_ratio(true);
    picture.set_size_request(canvas_w, canvas_h);
    canvas_overlay.set_child(Some(&picture));

    let da = gtk4::DrawingArea::new();
    da.set_cursor_from_name(Some("crosshair"));
    da.set_size_request(canvas_w, canvas_h);

    let state_annots = annotations.clone();
    let state_active = active_drawing.clone();
    da.set_draw_func(move |_, cr, width, height| {
        let annots = state_annots.borrow();
        let active = state_active.borrow();
        draw_annotations_on_context(cr, &annots, active.as_ref(), width as f64, height as f64);
    });

    let tool_pen = current_tool.clone();
    let b_pen = pen_btn.clone();
    let b_arrow = arrow_btn.clone();
    let b_rect = rect_btn.clone();
    pen_btn.connect_clicked(move |_| {
        *tool_pen.borrow_mut() = AnnotationTool::Pen;
        b_pen.add_css_class("active");
        b_arrow.remove_css_class("active");
        b_rect.remove_css_class("active");
    });

    let tool_arrow = current_tool.clone();
    let b_pen2 = pen_btn.clone();
    let b_arrow2 = arrow_btn.clone();
    let b_rect2 = rect_btn.clone();
    arrow_btn.connect_clicked(move |_| {
        *tool_arrow.borrow_mut() = AnnotationTool::Arrow;
        b_arrow2.add_css_class("active");
        b_pen2.remove_css_class("active");
        b_rect2.remove_css_class("active");
    });

    let tool_rect = current_tool.clone();
    let b_pen3 = pen_btn.clone();
    let b_arrow3 = arrow_btn.clone();
    let b_rect3 = rect_btn.clone();
    rect_btn.connect_clicked(move |_| {
        *tool_rect.borrow_mut() = AnnotationTool::Rect;
        b_rect3.add_css_class("active");
        b_pen3.remove_css_class("active");
        b_arrow3.remove_css_class("active");
    });

    let clear_annots = annotations.clone();
    let clear_active = active_drawing.clone();
    let da_clear = da.clone();
    clear_btn.connect_clicked(move |_| {
        clear_annots.borrow_mut().clear();
        clear_active.borrow_mut().take();
        da_clear.queue_draw();
    });

    let draw_gesture = gtk4::GestureDrag::new();
    let tool_drag = current_tool.clone();
    let active_start = active_drawing.clone();
    let da_start = da.clone();
    draw_gesture.connect_drag_begin(move |_, x, y| {
        let w = da_start.width() as f64;
        let h = da_start.height() as f64;
        if w <= 0.0 || h <= 0.0 {
            return;
        }
        let nx = (x / w).clamp(0.0, 1.0);
        let ny = (y / h).clamp(0.0, 1.0);

        let annot = match *tool_drag.borrow() {
            AnnotationTool::Pen => Annotation::Pen(vec![(nx, ny)]),
            AnnotationTool::Arrow => Annotation::Arrow {
                start: (nx, ny),
                end: (nx, ny),
            },
            AnnotationTool::Rect => Annotation::Rect {
                start: (nx, ny),
                end: (nx, ny),
            },
        };
        *active_start.borrow_mut() = Some(annot);
        da_start.queue_draw();
    });

    let active_update = active_drawing.clone();
    let da_update = da.clone();
    draw_gesture.connect_drag_update(move |gesture, offset_x, offset_y| {
        let Some((start_x, start_y)) = gesture.start_point() else {
            return;
        };
        let w = da_update.width() as f64;
        let h = da_update.height() as f64;
        if w <= 0.0 || h <= 0.0 {
            return;
        }

        let cur_x = start_x + offset_x;
        let cur_y = start_y + offset_y;
        let nx = (cur_x / w).clamp(0.0, 1.0);
        let ny = (cur_y / h).clamp(0.0, 1.0);

        let mut active = active_update.borrow_mut();
        if let Some(ref mut annot) = *active {
            match annot {
                Annotation::Pen(points) => {
                    let should_push = points
                        .last()
                        .map(|last| (last.0 - nx).hypot(last.1 - ny) > 0.005)
                        .unwrap_or(true);
                    if should_push {
                        points.push((nx, ny));
                    }
                }
                Annotation::Arrow { end, .. } => {
                    *end = (nx, ny);
                }
                Annotation::Rect { end, .. } => {
                    *end = (nx, ny);
                }
            }
        }
        da_update.queue_draw();
    });

    let active_end = active_drawing.clone();
    let annots_end = annotations.clone();
    let da_end = da.clone();
    draw_gesture.connect_drag_end(move |_, _, _| {
        if let Some(annot) = active_end.borrow_mut().take() {
            annots_end.borrow_mut().push(annot);
        }
        da_end.queue_draw();
    });
    da.add_controller(draw_gesture);

    canvas_overlay.add_overlay(&da);
    canvas_container.append(&canvas_overlay);
    modal_card.append(&canvas_container);
    modal_window.set_child(Some(&modal_card));

    let on_done_cell = Rc::new(RefCell::new(Some(on_done)));
    let win_weak = modal_window.downgrade();
    let annots_done = annotations.clone();

    let finish_modal = Rc::new(move || {
        if let Some(win) = win_weak.upgrade() {
            win.close();
        }
        if let Some(cb) = on_done_cell.borrow_mut().take() {
            let res = annots_done.borrow().clone();
            cb(res);
        }
    });

    let finish_btn = finish_modal.clone();
    done_btn.connect_clicked(move |_| finish_btn());

    let key_controller = gtk4::EventControllerKey::new();
    let finish_key = finish_modal.clone();
    let annots_key = annotations.clone();
    let da_key = da.clone();
    key_controller.connect_key_pressed(move |_, key, _, state| {
        if key == gdk::Key::Escape || key == gdk::Key::Return || key == gdk::Key::KP_Enter {
            finish_key();
            glib::Propagation::Stop
        } else if key == gdk::Key::z && state.contains(gdk::ModifierType::CONTROL_MASK) {
            let mut list = annots_key.borrow_mut();
            list.pop();
            da_key.queue_draw();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    modal_window.add_controller(key_controller);

    modal_window.present();
}
