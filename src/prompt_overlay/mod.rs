pub mod annotation;
pub mod annotation_modal;
pub mod position;

pub use annotation::{
    Annotation, AnnotationTool, apply_annotations_to_image, draw_annotations_on_context,
    large_image_texture,
};
pub use annotation_modal::show_annotation_modal;
pub use position::{
    CURSOR_GAP, EDGE_PADDING, PROMPT_HEIGHT, PROMPT_WIDTH, PromptPosition, find_monitor,
    position_prompt,
};

use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use image::DynamicImage;
use std::cell::RefCell;
use std::rc::Rc;

pub fn show_prompt_overlay<F, D>(
    app: &gtk4::Application,
    image: DynamicImage,
    cursor: (i32, i32),
    opacity: f64,
    custom_chips: Vec<(String, String)>,
    on_submit: F,
    on_dismiss: D,
) where
    F: FnOnce(String, DynamicImage, bool) + 'static,
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

    let annotations: Rc<RefCell<Vec<Annotation>>> = Rc::new(RefCell::new(Vec::new()));

    // --- Header Row: Thumbnail + Info & Annotate button ---
    let header_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    header_row.add_css_class("prompt-header-row");

    let texture = image_texture(&image);

    let thumb_overlay = gtk4::Overlay::new();
    thumb_overlay.add_css_class("prompt-thumb-box");
    thumb_overlay.set_cursor_from_name(Some("pointer"));

    let preview = gtk4::Picture::for_paintable(&texture);
    preview.set_can_shrink(true);
    preview.set_keep_aspect_ratio(true);
    preview.set_size_request(100, 62);
    preview.add_css_class("prompt-preview");
    thumb_overlay.set_child(Some(&preview));

    let thumb_da = gtk4::DrawingArea::new();
    thumb_da.set_size_request(100, 62);
    let state_thumb_annots = annotations.clone();
    thumb_da.set_draw_func(move |_, cr, width, height| {
        let annots = state_thumb_annots.borrow();
        if !annots.is_empty() {
            draw_annotations_on_context(cr, &annots, None, width as f64, height as f64);
        }
    });
    thumb_overlay.add_overlay(&thumb_da);
    header_row.append(&thumb_overlay);

    let info_box = gtk4::Box::new(gtk4::Orientation::Vertical, 2);
    info_box.add_css_class("prompt-header-info");
    info_box.set_valign(gtk4::Align::Center);

    let cap_title = gtk4::Label::new(Some("Screen Capture"));
    cap_title.add_css_class("prompt-capture-title");
    cap_title.set_halign(gtk4::Align::Start);

    let cap_sub = gtk4::Label::new(Some(&format!("{} × {} px", image.width(), image.height())));
    cap_sub.add_css_class("prompt-capture-subtitle");
    cap_sub.set_halign(gtk4::Align::Start);

    let btns_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let annotate_btn = gtk4::Button::with_label("Annotate");
    annotate_btn.add_css_class("prompt-annotate-btn");
    annotate_btn.set_tooltip_text(Some("Open large canvas to draw arrows, boxes, or lines"));

    let clear_btn = gtk4::Button::with_label("Clear");
    clear_btn.add_css_class("prompt-clear-btn");
    clear_btn.set_visible(false);

    btns_row.append(&annotate_btn);
    btns_row.append(&clear_btn);

    info_box.append(&cap_title);
    info_box.append(&cap_sub);
    info_box.append(&btns_row);
    header_row.append(&info_box);

    card.append(&header_row);

    // Wire up Annotate Modal
    let app_modal = app.clone();
    let mon_modal = monitor.clone();
    let mon_geom = monitor_geometry;
    let img_modal = image.clone();
    let annots_modal = annotations.clone();
    let thumb_da_modal = thumb_da.clone();
    let btn_label = annotate_btn.clone();
    let clear_btn_ref = clear_btn.clone();

    let open_annotation_modal = Rc::new(move || {
        let app_handle = app_modal.clone();
        let mon = mon_modal.clone();
        let img = img_modal.clone();
        let annots_ref = annots_modal.clone();
        let da_refresh = thumb_da_modal.clone();
        let label_refresh = btn_label.clone();
        let clear_refresh = clear_btn_ref.clone();

        let current = annots_ref.borrow().clone();
        show_annotation_modal(&app_handle, &mon, mon_geom, &img, current, move |updated| {
            let has_items = !updated.is_empty();
            let count = updated.len();
            *annots_ref.borrow_mut() = updated;
            da_refresh.queue_draw();

            if has_items {
                label_refresh.set_label(&format!("Annotated ({})", count));
                clear_refresh.set_visible(true);
            } else {
                label_refresh.set_label("Annotate");
                clear_refresh.set_visible(false);
            }
        });
    });

    let open_btn = open_annotation_modal.clone();
    annotate_btn.connect_clicked(move |_| open_btn());

    let open_thumb = open_annotation_modal.clone();
    let click_gesture = gtk4::GestureClick::new();
    click_gesture.connect_pressed(move |_, _, _, _| {
        open_thumb();
    });
    thumb_overlay.add_controller(click_gesture);

    let annots_clear = annotations.clone();
    let thumb_da_clear = thumb_da.clone();
    let annotate_label_clear = annotate_btn.clone();
    let clear_btn_hide = clear_btn.clone();
    clear_btn.connect_clicked(move |_| {
        annots_clear.borrow_mut().clear();
        thumb_da_clear.queue_draw();
        annotate_label_clear.set_label("Annotate");
        clear_btn_hide.set_visible(false);
    });

    // --- Composer Input Row ---
    let composer = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    composer.add_css_class("prompt-composer");

    let entry_scroll = gtk4::ScrolledWindow::new();
    entry_scroll.add_css_class("prompt-entry-scroll");
    entry_scroll.set_min_content_height(44);
    entry_scroll.set_max_content_height(84);
    entry_scroll.set_propagate_natural_height(true);
    entry_scroll.set_hscrollbar_policy(gtk4::PolicyType::Never);
    entry_scroll.set_vscrollbar_policy(gtk4::PolicyType::Automatic);
    entry_scroll.set_overlay_scrolling(true);
    entry_scroll.set_hexpand(true);

    let entry = gtk4::TextView::new();
    entry.add_css_class("prompt-entry");
    entry.set_wrap_mode(gtk4::WrapMode::WordChar);
    entry.set_accepts_tab(false);
    entry.set_top_margin(6);
    entry.set_bottom_margin(6);
    entry.set_left_margin(0);
    entry.set_right_margin(0);

    let entry_overlay = gtk4::Overlay::new();
    entry_overlay.set_child(Some(&entry));
    let placeholder = gtk4::Label::new(Some("Ask anything about this capture..."));
    placeholder.add_css_class("prompt-entry-placeholder");
    placeholder.set_halign(gtk4::Align::Start);
    placeholder.set_valign(gtk4::Align::Start);
    placeholder.set_margin_top(6);
    placeholder.set_can_target(false);
    entry_overlay.add_overlay(&placeholder);
    entry_scroll.set_child(Some(&entry_overlay));
    let placeholder_changed = placeholder.clone();
    entry.buffer().connect_changed(move |buffer| {
        placeholder_changed.set_visible(buffer.char_count() == 0);
    });

    let send_icon = gtk4::Image::from_icon_name("document-send-symbolic");
    send_icon.set_pixel_size(20);
    let send_button = gtk4::Button::new();
    send_button.set_child(Some(&send_icon));
    send_button.add_css_class("prompt-send-button");
    send_button.set_tooltip_text(Some("Send question (Enter)"));
    composer.append(&entry_scroll);
    composer.append(&send_button);
    card.append(&composer);

    // --- Quick-Action Chips & Search Grounding ---
    let chip_bar = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    chip_bar.add_css_class("prompt-chips-bar");

    let search_toggle = gtk4::ToggleButton::with_label("Search Grounding");
    search_toggle.add_css_class("prompt-search-chip");
    search_toggle.set_tooltip_text(Some(
        "Enable Gemini google_search grounding for live web lookup per request",
    ));
    chip_bar.append(&search_toggle);

    let chips = [
        (
            "Explain",
            "Explain what is shown in this capture in detail.",
        ),
        (
            "Extract Code",
            "Extract all visible code from this capture verbatim with appropriate syntax highlighting.",
        ),
        (
            "Debug Error",
            "Diagnose this error or bug, explain why it occurred, and provide the fix.",
        ),
        (
            "Summarize",
            "Summarize the key information and points visible here.",
        ),
        (
            "Translate",
            "Translate the visible text into English and explain key terms.",
        ),
    ];

    let image_holder = Rc::new(RefCell::new(Some(image)));
    let on_submit_holder = Rc::new(RefCell::new(Some(on_submit)));
    let submit_window = window.downgrade();
    let submit_entry = entry.clone();
    let submit_image = image_holder.clone();
    let submit_callback = on_submit_holder.clone();
    let submit_annots = annotations.clone();
    let submit_search_toggle = search_toggle.clone();

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

        let search_grounding = submit_search_toggle.is_active();

        if let Some(window) = submit_window.upgrade() {
            window.close();
        }

        let final_image = apply_annotations_to_image(&raw_image, &submit_annots.borrow());
        callback(question, final_image, search_grounding);
    });

    for (label, prompt_text) in chips {
        let chip_btn = gtk4::Button::with_label(label);
        chip_btn.add_css_class("prompt-chip");
        chip_btn.set_tooltip_text(Some("Quick prompt"));

        let submit_chip = submit.clone();
        chip_btn.connect_clicked(move |_| {
            submit_chip(Some(prompt_text.to_string()));
        });
        chip_bar.append(&chip_btn);
    }

    for (label, prompt_text) in &custom_chips {
        let chip_btn = gtk4::Button::with_label(label);
        chip_btn.add_css_class("prompt-chip");
        chip_btn.add_css_class("custom-prompt-chip");
        chip_btn.set_tooltip_text(Some(&format!("Custom prompt: {prompt_text}")));

        let submit_chip = submit.clone();
        let prompt_clone = prompt_text.clone();
        chip_btn.connect_clicked(move |_| {
            submit_chip(Some(prompt_clone.clone()));
        });
        chip_bar.append(&chip_btn);
    }
    card.append(&chip_bar);
    window.set_child(Some(&card));

    let submit_button = submit.clone();
    send_button.connect_clicked(move |_| submit_button(None));

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

fn image_texture(image: &DynamicImage) -> gdk::MemoryTexture {
    let thumbnail = image.thumbnail(200, 130).to_rgba8();
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
