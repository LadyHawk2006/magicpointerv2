use gtk4::cairo;
use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use image::DynamicImage;
use std::cell::RefCell;
use std::rc::Rc;

const PROMPT_WIDTH: i32 = 450;
const PROMPT_HEIGHT: i32 = 380;
const CURSOR_GAP: i32 = 18;
const EDGE_PADDING: i32 = 12;

#[derive(Clone, Debug, PartialEq)]
pub enum Annotation {
    Pen(Vec<(f64, f64)>),
    Arrow { start: (f64, f64), end: (f64, f64) },
    Rect { start: (f64, f64), end: (f64, f64) },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnnotationTool {
    Pen,
    Arrow,
    Rect,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PromptPosition {
    left: i32,
    top: i32,
}

fn position_prompt(
    cursor: (i32, i32),
    monitor: gdk::Rectangle,
    prompt_size: (i32, i32),
) -> PromptPosition {
    let cursor_x = cursor.0 - monitor.x();
    let cursor_y = cursor.1 - monitor.y();
    let available_width = monitor.width().max(0);
    let available_height = monitor.height().max(0);

    let left = if cursor_x + CURSOR_GAP + prompt_size.0 <= available_width - EDGE_PADDING {
        cursor_x + CURSOR_GAP
    } else {
        cursor_x - CURSOR_GAP - prompt_size.0
    };
    let top = if cursor_y + CURSOR_GAP + prompt_size.1 <= available_height - EDGE_PADDING {
        cursor_y + CURSOR_GAP
    } else {
        cursor_y - CURSOR_GAP - prompt_size.1
    };

    PromptPosition {
        left: left.clamp(
            EDGE_PADDING,
            (available_width - prompt_size.0 - EDGE_PADDING).max(EDGE_PADDING),
        ),
        top: top.clamp(
            EDGE_PADDING,
            (available_height - prompt_size.1 - EDGE_PADDING).max(EDGE_PADDING),
        ),
    }
}

pub fn apply_annotations_to_image(
    image: &DynamicImage,
    annotations: &[Annotation],
) -> DynamicImage {
    if annotations.is_empty() {
        return image.clone();
    }

    let width = image.width() as i32;
    let height = image.height() as i32;

    let mut surface = match cairo::ImageSurface::create(cairo::Format::ARgb32, width, height) {
        Ok(s) => s,
        Err(_) => return image.clone(),
    };

    let cr = match cairo::Context::new(&surface) {
        Ok(c) => c,
        Err(_) => return image.clone(),
    };

    let scale_x = width as f64;
    let scale_y = height as f64;

    cr.set_source_rgba(0.95, 0.22, 0.28, 0.95);
    cr.set_line_width((width.min(height) as f64 * 0.005).max(3.5));
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);

    for annot in annotations {
        match annot {
            Annotation::Pen(points) => {
                if points.len() < 2 {
                    if let Some(p) = points.first() {
                        cr.arc(p.0 * scale_x, p.1 * scale_y, 4.0, 0.0, std::f64::consts::TAU);
                        let _ = cr.fill();
                    }
                    continue;
                }
                cr.move_to(points[0].0 * scale_x, points[0].1 * scale_y);
                for p in &points[1..] {
                    cr.line_to(p.0 * scale_x, p.1 * scale_y);
                }
                let _ = cr.stroke();
            }
            Annotation::Arrow { start, end } => {
                let x0 = start.0 * scale_x;
                let y0 = start.1 * scale_y;
                let x1 = end.0 * scale_x;
                let y1 = end.1 * scale_y;

                cr.move_to(x0, y0);
                cr.line_to(x1, y1);
                let _ = cr.stroke();

                let angle = (y1 - y0).atan2(x1 - x0);
                let head_len = (width.min(height) as f64 * 0.025).clamp(16.0, 36.0);
                let arrow_angle = std::f64::consts::PI / 6.0;

                let p1_x = x1 - head_len * (angle - arrow_angle).cos();
                let p1_y = y1 - head_len * (angle - arrow_angle).sin();
                let p2_x = x1 - head_len * (angle + arrow_angle).cos();
                let p2_y = y1 - head_len * (angle + arrow_angle).sin();

                cr.move_to(x1, y1);
                cr.line_to(p1_x, p1_y);
                cr.line_to(p2_x, p2_y);
                cr.close_path();
                let _ = cr.fill();
            }
            Annotation::Rect { start, end } => {
                let rx = start.0.min(end.0) * scale_x;
                let ry = start.1.min(end.1) * scale_y;
                let rw = (end.0 - start.0).abs() * scale_x;
                let rh = (end.1 - start.1).abs() * scale_y;

                cr.rectangle(rx, ry, rw, rh);
                let _ = cr.stroke();
            }
        }
    }

    drop(cr);

    let mut rgba_image = image.to_rgba8();
    if let Ok(data) = surface.data() {
        for (orig, annot_chunk) in rgba_image.chunks_exact_mut(4).zip(data.chunks_exact(4)) {
            let a = annot_chunk[3] as u32;
            if a > 0 {
                let inv_a = 255 - a;
                let r = annot_chunk[2] as u32; // In Cairo ARGB32 little endian: [B, G, R, A]
                let g = annot_chunk[1] as u32;
                let b = annot_chunk[0] as u32;
                orig[0] = ((orig[0] as u32 * inv_a + r * 255) / 255).min(255) as u8;
                orig[1] = ((orig[1] as u32 * inv_a + g * 255) / 255).min(255) as u8;
                orig[2] = ((orig[2] as u32 * inv_a + b * 255) / 255).min(255) as u8;
                orig[3] = (orig[3] as u32 + a).min(255) as u8;
            }
        }
    }

    DynamicImage::ImageRgba8(rgba_image)
}

pub fn show_prompt_overlay<F, D>(
    app: &gtk4::Application,
    image: DynamicImage,
    cursor: (i32, i32),
    opacity: f64,
    on_submit: F,
    on_dismiss: D,
) where
    F: FnOnce(String, DynamicImage) + 'static,
    D: FnOnce() + 'static,
{
    let Some(display) = gdk::Display::default() else {
        tracing::error!("Cannot show prompt composer: no GDK display is available");
        on_dismiss();
        return;
    };

    let Some((monitor, monitor_geometry)) = find_monitor(&display, cursor) else {
        tracing::error!("Cannot show prompt composer: no display monitor is available");
        on_dismiss();
        return;
    };

    let prompt_width = PROMPT_WIDTH.min((monitor_geometry.width() - 2 * EDGE_PADDING).max(1));
    let prompt_height = PROMPT_HEIGHT.min((monitor_geometry.height() - 2 * EDGE_PADDING).max(1));
    let prompt_size = (prompt_width, prompt_height);
    let position = position_prompt(cursor, monitor_geometry, prompt_size);
    let window = gtk4::ApplicationWindow::new(app);
    window.init_layer_shell();
    window.set_title(Some("Ask about this capture"));
    window.set_default_size(prompt_width, prompt_height);
    window.set_decorated(false);
    window.set_opacity(opacity);
    window.add_css_class("prompt-overlay");
    window.set_layer(Layer::Overlay);
    window.set_monitor(Some(&monitor));
    window.set_keyboard_mode(KeyboardMode::Exclusive);
    window.set_exclusive_zone(0);
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Left, true);
    window.set_margin(Edge::Top, position.top);
    window.set_margin(Edge::Left, position.left);

    let card = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    card.add_css_class("prompt-card");

    // --- Lightweight Annotation State & Setup ---
    let annotations: Rc<RefCell<Vec<Annotation>>> = Rc::new(RefCell::new(Vec::new()));
    let current_tool: Rc<RefCell<AnnotationTool>> = Rc::new(RefCell::new(AnnotationTool::Pen));
    let active_drawing: Rc<RefCell<Option<Annotation>>> = Rc::new(RefCell::new(None));

    let preview_container = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    preview_container.add_css_class("prompt-preview-container");

    // Mini annotation toolbar
    let toolbar = gtk4::Box::new(gtk4::Orientation::Horizontal, 4);
    toolbar.add_css_class("annotation-toolbar");

    let pen_btn = gtk4::Button::with_label("✏️ Pen");
    pen_btn.add_css_class("annotation-tool-btn");
    pen_btn.add_css_class("active");
    pen_btn.set_tooltip_text(Some("Draw freehand circles or underlines"));

    let arrow_btn = gtk4::Button::with_label("↗️ Arrow");
    arrow_btn.add_css_class("annotation-tool-btn");
    arrow_btn.set_tooltip_text(Some("Draw pointing arrow"));

    let rect_btn = gtk4::Button::with_label("▢ Box");
    rect_btn.add_css_class("annotation-tool-btn");
    rect_btn.set_tooltip_text(Some("Draw bounding box"));

    let clear_btn = gtk4::Button::with_label("↺ Clear");
    clear_btn.add_css_class("annotation-tool-btn");
    clear_btn.set_tooltip_text(Some("Clear annotations"));

    toolbar.append(&pen_btn);
    toolbar.append(&arrow_btn);
    toolbar.append(&rect_btn);
    toolbar.append(&clear_btn);
    preview_container.append(&toolbar);

    // Thumbnail preview overlay with interactive drawing area
    let preview_overlay = gtk4::Overlay::new();
    let texture = image_texture(&image);
    let preview = gtk4::Picture::for_paintable(&texture);
    preview.set_can_shrink(true);
    preview.set_keep_aspect_ratio(true);
    let preview_w = 200.min(prompt_width - 48);
    let preview_h = 110.min(prompt_height - 180);
    preview.set_size_request(preview_w, preview_h);
    preview.add_css_class("prompt-preview");
    preview.set_overflow(gtk4::Overflow::Hidden);
    preview.set_halign(gtk4::Align::Start);
    preview_overlay.set_child(Some(&preview));

    let da = gtk4::DrawingArea::new();
    da.set_cursor_from_name(Some("crosshair"));
    da.set_size_request(preview_w, preview_h);
    da.set_halign(gtk4::Align::Start);
    da.set_valign(gtk4::Align::Start);

    let state_annots = annotations.clone();
    let state_active = active_drawing.clone();
    da.set_draw_func(move |_, cr, width, height| {
        let scale_x = width as f64;
        let scale_y = height as f64;

        cr.set_source_rgba(0.95, 0.22, 0.28, 0.95);
        cr.set_line_width(2.5);
        cr.set_line_cap(cairo::LineCap::Round);
        cr.set_line_join(cairo::LineJoin::Round);

        let annots = state_annots.borrow();
        for annot in annots.iter() {
            match annot {
                Annotation::Pen(points) => {
                    if points.len() < 2 {
                        continue;
                    }
                    cr.move_to(points[0].0 * scale_x, points[0].1 * scale_y);
                    for p in &points[1..] {
                        cr.line_to(p.0 * scale_x, p.1 * scale_y);
                    }
                    let _ = cr.stroke();
                }
                Annotation::Arrow { start, end } => {
                    let x0 = start.0 * scale_x;
                    let y0 = start.1 * scale_y;
                    let x1 = end.0 * scale_x;
                    let y1 = end.1 * scale_y;

                    cr.move_to(x0, y0);
                    cr.line_to(x1, y1);
                    let _ = cr.stroke();

                    let angle = (y1 - y0).atan2(x1 - x0);
                    let head_len = 10.0;
                    let arrow_angle = std::f64::consts::PI / 6.0;

                    let p1_x = x1 - head_len * (angle - arrow_angle).cos();
                    let p1_y = y1 - head_len * (angle - arrow_angle).sin();
                    let p2_x = x1 - head_len * (angle + arrow_angle).cos();
                    let p2_y = y1 - head_len * (angle + arrow_angle).sin();

                    cr.move_to(x1, y1);
                    cr.line_to(p1_x, p1_y);
                    cr.line_to(p2_x, p2_y);
                    cr.close_path();
                    let _ = cr.fill();
                }
                Annotation::Rect { start, end } => {
                    let rx = start.0.min(end.0) * scale_x;
                    let ry = start.1.min(end.1) * scale_y;
                    let rw = (end.0 - start.0).abs() * scale_x;
                    let rh = (end.1 - start.1).abs() * scale_y;

                    cr.rectangle(rx, ry, rw, rh);
                    let _ = cr.stroke();
                }
            }
        }

        if let Some(ref active) = *state_active.borrow() {
            match active {
                Annotation::Pen(points) => {
                    if points.len() >= 2 {
                        cr.move_to(points[0].0 * scale_x, points[0].1 * scale_y);
                        for p in &points[1..] {
                            cr.line_to(p.0 * scale_x, p.1 * scale_y);
                        }
                        let _ = cr.stroke();
                    }
                }
                Annotation::Arrow { start, end } => {
                    let x0 = start.0 * scale_x;
                    let y0 = start.1 * scale_y;
                    let x1 = end.0 * scale_x;
                    let y1 = end.1 * scale_y;

                    cr.move_to(x0, y0);
                    cr.line_to(x1, y1);
                    let _ = cr.stroke();

                    let angle = (y1 - y0).atan2(x1 - x0);
                    let head_len = 10.0;
                    let arrow_angle = std::f64::consts::PI / 6.0;

                    let p1_x = x1 - head_len * (angle - arrow_angle).cos();
                    let p1_y = y1 - head_len * (angle - arrow_angle).sin();
                    let p2_x = x1 - head_len * (angle + arrow_angle).cos();
                    let p2_y = y1 - head_len * (angle + arrow_angle).sin();

                    cr.move_to(x1, y1);
                    cr.line_to(p1_x, p1_y);
                    cr.line_to(p2_x, p2_y);
                    cr.close_path();
                    let _ = cr.fill();
                }
                Annotation::Rect { start, end } => {
                    let rx = start.0.min(end.0) * scale_x;
                    let ry = start.1.min(end.1) * scale_y;
                    let rw = (end.0 - start.0).abs() * scale_x;
                    let rh = (end.1 - start.1).abs() * scale_y;

                    cr.rectangle(rx, ry, rw, rh);
                    let _ = cr.stroke();
                }
            }
        }
    });

    // Wire up tool selection buttons
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

    // Gesture drag for drawing on the preview
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
                        .map(|last| (last.0 - nx).hypot(last.1 - ny) > 0.008)
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

    preview_overlay.add_overlay(&da);
    preview_container.append(&preview_overlay);
    card.append(&preview_container);

    let composer = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    composer.add_css_class("prompt-composer");

    let entry_scroll = gtk4::ScrolledWindow::new();
    entry_scroll.add_css_class("prompt-entry-scroll");
    entry_scroll.set_min_content_height(48);
    entry_scroll.set_max_content_height(96);
    entry_scroll.set_propagate_natural_height(true);
    entry_scroll.set_hscrollbar_policy(gtk4::PolicyType::Never);
    entry_scroll.set_vscrollbar_policy(gtk4::PolicyType::Automatic);
    entry_scroll.set_overlay_scrolling(true);
    entry_scroll.set_hexpand(true);

    let entry = gtk4::TextView::new();
    entry.add_css_class("prompt-entry");
    entry.set_wrap_mode(gtk4::WrapMode::WordChar);
    entry.set_accepts_tab(false);
    entry.set_top_margin(8);
    entry.set_bottom_margin(8);
    entry.set_left_margin(0);
    entry.set_right_margin(0);

    let entry_overlay = gtk4::Overlay::new();
    entry_overlay.set_child(Some(&entry));
    let placeholder = gtk4::Label::new(Some("Ask anything about this capture"));
    placeholder.add_css_class("prompt-entry-placeholder");
    placeholder.set_halign(gtk4::Align::Start);
    placeholder.set_valign(gtk4::Align::Start);
    placeholder.set_margin_top(8);
    placeholder.set_can_target(false);
    entry_overlay.add_overlay(&placeholder);
    entry_scroll.set_child(Some(&entry_overlay));
    let placeholder_changed = placeholder.clone();
    entry.buffer().connect_changed(move |buffer| {
        placeholder_changed.set_visible(buffer.char_count() == 0);
    });

    let send_icon = gtk4::Image::from_icon_name("document-send");
    send_icon.set_pixel_size(32);
    let send_button = gtk4::Button::new();
    send_button.set_child(Some(&send_icon));
    send_button.add_css_class("prompt-send-button");
    send_button.set_tooltip_text(Some("Send question (Enter)"));
    composer.append(&entry_scroll);
    composer.append(&send_button);
    card.append(&composer);

    // --- Quick-Action Chips (Zero-Typing Prompts) ---
    let chip_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    chip_bar.add_css_class("prompt-chips-bar");

    let chips = [
        ("Explain this", "Explain what is shown in this capture in detail."),
        ("Extract text / Code", "Extract all visible text and code from this image verbatim."),
        ("Debug error", "Diagnose this error or bug, explain why it occurred, and provide the fix."),
        ("Summarize", "Summarize the key information and points visible here."),
        ("Translate", "Translate the visible text into English and explain key terms."),
    ];

    let image_holder = Rc::new(RefCell::new(Some(image)));
    let on_submit_holder = Rc::new(RefCell::new(Some(on_submit)));
    let submit_window = window.downgrade();
    let submit_entry = entry.clone();
    let submit_image = image_holder.clone();
    let submit_callback = on_submit_holder.clone();
    let submit_annots = annotations.clone();

    let submit = Rc::new(move |prompt_override: Option<String>| {
        let question = if let Some(override_text) = prompt_override {
            override_text
        } else {
            let buffer = submit_entry.buffer();
            let (start, end) = buffer.bounds();
            buffer.text(&start, &end, true).trim().to_string()
        };

        if question.is_empty() {
            return;
        }

        let (Some(raw_image), Some(callback)) = (
            submit_image.borrow_mut().take(),
            submit_callback.borrow_mut().take(),
        ) else {
            return;
        };

        if let Some(window) = submit_window.upgrade() {
            window.close();
        }

        // Apply lightweight in-memory annotations before dispatching
        let final_image = apply_annotations_to_image(&raw_image, &submit_annots.borrow());
        callback(question, final_image);
    });

    for (label, prompt_text) in chips {
        let chip_btn = gtk4::Button::with_label(label);
        chip_btn.add_css_class("prompt-chip");
        chip_btn.set_tooltip_text(Some("Quickly ask without typing"));

        let submit_chip = submit.clone();
        chip_btn.connect_clicked(move |_| {
            submit_chip(Some(prompt_text.to_string()));
        });
        chip_bar.append(&chip_btn);
    }
    card.append(&chip_bar);
    window.set_child(Some(&card));

    let submit_button = submit.clone();
    send_button.connect_clicked(move |_| submit_button(None));

    // Enter to submit; Shift+Enter to insert a newline
    let submit_key = submit.clone();
    let submit_controller = gtk4::EventControllerKey::new();
    submit_controller.connect_key_pressed(move |_, key, _, state| {
        if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter) {
            if state.contains(gdk::ModifierType::SHIFT_MASK) {
                glib::Propagation::Proceed
            } else {
                submit_key(None);
                glib::Propagation::Stop
            }
        } else {
            glib::Propagation::Proceed
        }
    });
    entry.add_controller(submit_controller);

    let key_controller = gtk4::EventControllerKey::new();
    let key_window = window.downgrade();
    let dismiss_callback = Rc::new(RefCell::new(Some(on_dismiss)));
    let escape_callback = dismiss_callback.clone();
    key_controller.connect_key_pressed(move |_, key, _, _| {
        if key == gdk::Key::Escape {
            if let Some(window) = key_window.upgrade() {
                window.close();
            }
            if let Some(callback) = escape_callback.borrow_mut().take() {
                callback();
            }
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(key_controller);

    window.present();
    entry.grab_focus();
}

pub fn find_monitor(
    display: &gdk::Display,
    cursor: (i32, i32),
) -> Option<(gdk::Monitor, gdk::Rectangle)> {
    let monitors = display.monitors();
    let mut first_monitor = None;
    for index in 0..monitors.n_items() {
        let monitor = monitors.item(index)?.downcast::<gdk::Monitor>().ok()?;
        let geometry = monitor.geometry();
        if first_monitor.is_none() {
            first_monitor = Some((monitor.clone(), geometry));
        }

        if cursor.0 >= geometry.x()
            && cursor.1 >= geometry.y()
            && cursor.0 < geometry.x() + geometry.width()
            && cursor.1 < geometry.y() + geometry.height()
        {
            return Some((monitor, geometry));
        }
    }

    first_monitor
}

fn image_texture(image: &DynamicImage) -> gdk::MemoryTexture {
    let thumbnail = image.thumbnail(288, 192).to_rgba8();
    let width = thumbnail.width();
    let height = thumbnail.height();
    let bytes = glib::Bytes::from_owned(thumbnail.into_raw());
    gdk::MemoryTexture::new(
        width as i32,
        height as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &bytes,
        width as usize * 4,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk4::gdk;
    use image::{DynamicImage, RgbaImage};

    #[test]
    fn prompt_is_placed_beside_cursor_when_room_is_available() {
        let monitor = gdk::Rectangle::new(0, 0, 1920, 1080);
        assert_eq!(
            position_prompt((400, 300), monitor, (380, 220)),
            PromptPosition {
                left: 418,
                top: 318
            }
        );
    }

    #[test]
    fn prompt_flips_and_clamps_near_monitor_edges() {
        let monitor = gdk::Rectangle::new(0, 0, 1920, 1080);
        assert_eq!(
            position_prompt((1900, 1060), monitor, (380, 220)),
            PromptPosition {
                left: 1502,
                top: 822
            }
        );
    }

    #[test]
    fn prompt_uses_monitor_local_coordinates() {
        let monitor = gdk::Rectangle::new(-1920, 0, 1920, 1080);
        assert_eq!(
            position_prompt((-1800, 300), monitor, (380, 220)),
            PromptPosition {
                left: 138,
                top: 318
            }
        );
    }

    #[test]
    fn apply_annotations_renders_onto_image_in_memory() {
        let img = DynamicImage::ImageRgba8(RgbaImage::new(100, 100));
        let annots = vec![
            Annotation::Pen(vec![(0.1, 0.1), (0.5, 0.5)]),
            Annotation::Arrow {
                start: (0.2, 0.2),
                end: (0.8, 0.8),
            },
            Annotation::Rect {
                start: (0.3, 0.3),
                end: (0.7, 0.7),
            },
        ];

        let result = apply_annotations_to_image(&img, &annots);
        assert_eq!(result.width(), 100);
        assert_eq!(result.height(), 100);

        // Verify that some pixels are no longer 0 alpha / 0 color (annotations drawn)
        let rgba = result.to_rgba8();
        let modified = rgba.pixels().any(|p| p[3] > 0);
        assert!(modified, "Annotations should modify pixels");
    }

    #[test]
    fn empty_annotations_return_untouched_image() {
        let img = DynamicImage::ImageRgba8(RgbaImage::new(50, 50));
        let result = apply_annotations_to_image(&img, &[]);
        assert_eq!(result.width(), 50);
        assert_eq!(result.height(), 50);
    }
}
