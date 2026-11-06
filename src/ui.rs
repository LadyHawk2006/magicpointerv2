use std::sync::Arc;

use crate::ai_agent::ConversationSession;
use crate::executor;
use crate::screencapture;
use crate::selection_overlay::{self, Rect};
use crate::settings::{self, AppSettings};
use crate::settings_ui;
use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use image::DynamicImage;
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::error;

pub struct ChatPreferences {
    pub active_window_title: Option<String>,
    pub settings: Rc<RefCell<AppSettings>>,
}

pub fn build_floating_card(
    app: &gtk4::Application,
    captured_image: DynamicImage,
    captured_context: Option<String>,
    initial_prompt: String,
    session: ConversationSession,
    tokio_rt: Arc<tokio::runtime::Runtime>,
    preferences: ChatPreferences,
) -> gtk4::ApplicationWindow {
    let window = gtk4::ApplicationWindow::new(app);
    window.set_title(Some("Magic Pointer"));
    window.set_default_size(537, 763);
    window.set_decorated(false);
    window.set_opacity(preferences.settings.borrow().overlay_opacity);

    window.add_css_class("card-window");

    let card_overlay = gtk4::Overlay::new();
    card_overlay.set_hexpand(true);
    card_overlay.set_vexpand(true);

    // Outer card
    let card = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    card.add_css_class("magic-card");
    card.set_hexpand(true);
    card.set_vexpand(true);
    card_overlay.set_child(Some(&card));

    let fetch_glow = gtk4::DrawingArea::new();
    fetch_glow.add_css_class("chat-fetch-glow");
    fetch_glow.set_hexpand(true);
    fetch_glow.set_vexpand(true);
    fetch_glow.set_can_target(false);
    fetch_glow.set_visible(false);
    let glow_phase = Rc::new(Cell::new(0.0));
    let glow_phase_draw = glow_phase.clone();
    fetch_glow.set_draw_func(move |_, context, width, height| {
        draw_fetch_glow(context, width, height, glow_phase_draw.get());
    });
    card_overlay.add_overlay(&fetch_glow);
    let fetch_count = Rc::new(Cell::new(0));
    let fetch_animation = Rc::new(RefCell::new(None));

    let toolbar = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    toolbar.add_css_class("chat-toolbar");
    let toolbar_spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    toolbar_spacer.set_hexpand(true);
    toolbar.append(&toolbar_spacer);
    let settings_button = gtk4::Button::from_icon_name("preferences-system-symbolic");
    settings_button.add_css_class("chat-settings-button");
    settings_button.set_tooltip_text(Some("Settings"));
    toolbar.append(&settings_button);
    card.append(&toolbar);

    // --- Chat Thread View ---
    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.add_css_class("chat-history");
    scrolled.set_vexpand(true);
    scrolled.set_hexpand(true);
    scrolled.set_hscrollbar_policy(gtk4::PolicyType::Never);
    scrolled.set_vscrollbar_policy(gtk4::PolicyType::Automatic);
    scrolled.set_overlay_scrolling(true);

    let messages_box = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    messages_box.add_css_class("chat-messages");
    scrolled.set_child(Some(&messages_box));
    card.append(&scrolled);

    let initial_capture = picture_for_image(&captured_image, 500, 240);
    initial_capture.add_css_class("initial-capture");
    initial_capture.set_overflow(gtk4::Overflow::Hidden);
    if let Some(context) = captured_context.as_deref() {
        initial_capture.set_tooltip_text(Some(context));
    }
    messages_box.append(&initial_capture);

    let pending_images = Rc::new(RefCell::new(vec![captured_image]));
    let initial_turn = Rc::new(RefCell::new(true));

    let last_ai_text = Rc::new(RefCell::new(String::new()));

    // Helper to stream tokens into a label
    let attach_streaming_receiver = {
        let scrolled_weak = scrolled.downgrade();
        let last_ai_store = last_ai_text.clone();

        move |mut rx: mpsc::Receiver<String>, response: gtk4::Box| {
            let scrolled_weak = scrolled_weak.clone();
            let fetch_count = fetch_count.clone();
            let fetch_glow = fetch_glow.clone();
            let fetch_animation = fetch_animation.clone();
            let accumulated = Rc::new(RefCell::new(String::new()));
            let last_store = last_ai_store.clone();
            let response = response.clone();

            glib::spawn_future_local(async move {
                while let Some(token) = rx.recv().await {
                    accumulated.borrow_mut().push_str(&token);
                    let raw_text = accumulated.borrow().clone();
                    *last_store.borrow_mut() = raw_text.clone();

                    crate::markdown::render_markdown(&response, &raw_text);

                    // Auto-scroll to bottom
                    if let Some(scr) = scrolled_weak.upgrade() {
                        scroll_to_bottom(&scr);
                    }
                }

                if accumulated.borrow().is_empty() {
                    crate::markdown::render_markdown(&response, "No response received.");
                }
                finish_fetch_indicator(&fetch_count, &fetch_glow, &fetch_animation);
            });
        }
    };

    // --- Interactive Chat Input Bar + Resize Corner ---
    let composer = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    composer.add_css_class("chat-composer");

    let attachment_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    attachment_row.add_css_class("attachment-row");
    let attachment_previews = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    attachment_previews.add_css_class("attachment-previews");
    attachment_row.append(&attachment_previews);
    attachment_row.set_visible(false);

    let bottom_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 9);
    bottom_row.add_css_class("chat-input-bar");

    let entry_scroll = gtk4::ScrolledWindow::new();
    entry_scroll.add_css_class("chat-entry-scroll");
    entry_scroll.set_min_content_height(48);
    entry_scroll.set_max_content_height(104);
    entry_scroll.set_propagate_natural_height(true);
    entry_scroll.set_hscrollbar_policy(gtk4::PolicyType::Never);
    entry_scroll.set_vscrollbar_policy(gtk4::PolicyType::Automatic);
    entry_scroll.set_overlay_scrolling(true);
    entry_scroll.set_hexpand(true);

    let entry = gtk4::TextView::new();
    entry.add_css_class("chat-entry");
    entry.set_wrap_mode(gtk4::WrapMode::WordChar);
    entry.set_accepts_tab(false);
    entry.set_top_margin(7);
    entry.set_bottom_margin(7);
    entry.set_left_margin(6);
    entry.set_right_margin(6);

    let entry_overlay = gtk4::Overlay::new();
    entry_overlay.set_child(Some(&entry));
    let input_placeholder = gtk4::Label::new(Some("Ask about this..."));
    input_placeholder.add_css_class("chat-entry-placeholder");
    input_placeholder.set_halign(gtk4::Align::Start);
    input_placeholder.set_valign(gtk4::Align::Start);
    input_placeholder.set_margin_top(7);
    input_placeholder.set_margin_start(6);
    input_placeholder.set_can_target(false);
    entry_overlay.add_overlay(&input_placeholder);
    entry_scroll.set_child(Some(&entry_overlay));
    let input_placeholder_changed = input_placeholder.clone();
    entry.buffer().connect_changed(move |buffer| {
        input_placeholder_changed.set_visible(buffer.char_count() == 0);
    });
    let entry_weak = entry.downgrade();
    entry.buffer().connect_mark_set(move |buffer, _, mark| {
        if mark == &buffer.get_insert() {
            if let Some(entry) = entry_weak.upgrade() {
                entry.scroll_to_mark(mark, 0.1, false, 0.0, 0.0);
            }
        }
    });

    let entry_shell = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    entry_shell.add_css_class("chat-entry-shell");
    entry_shell.set_hexpand(true);
    entry_shell.append(&entry_scroll);

    let capture_btn = gtk4::Button::from_icon_name("list-add-symbolic");
    capture_btn.add_css_class("capture-image-btn");
    capture_btn.set_tooltip_text(Some("Capture another image"));

    let send_icon = gtk4::Image::from_icon_name("document-send");
    send_icon.set_pixel_size(32);
    let send_btn = gtk4::Button::new();
    send_btn.set_child(Some(&send_icon));
    send_btn.add_css_class("chat-send-btn");
    send_btn.set_tooltip_text(Some("Send message"));

    bottom_row.append(&capture_btn);
    bottom_row.append(&entry_shell);
    bottom_row.append(&send_btn);
    composer.append(&attachment_row);
    composer.append(&bottom_row);
    card.append(&composer);

    window.set_child(Some(&card_overlay));

    let settings_app = app.clone();
    let settings_state = preferences.settings.clone();
    settings_button.connect_clicked(move |_| {
        settings_ui::show_settings(&settings_app, settings_state.clone());
    });

    let capture_app = app.clone();
    let capture_pending_images = pending_images.clone();
    let capture_attachment_previews = attachment_previews.clone();
    let capture_attachment_row = attachment_row.clone();
    capture_btn.connect_clicked(move |_| {
        let pending_images = capture_pending_images.clone();
        let attachment_previews = capture_attachment_previews.clone();
        let attachment_row = capture_attachment_row.clone();
        selection_overlay::show_selection_overlay(
            &capture_app,
            move |rect: Rect| match screencapture::capture_screen_region(
                rect.x,
                rect.y,
                rect.width,
                rect.height,
            ) {
                Ok(image) => {
                    let mut images = pending_images.borrow_mut();
                    images.push(image);
                    drop(images);
                    attachment_row.set_visible(true);
                    refresh_pending_attachment_previews(
                        &attachment_previews,
                        pending_images.clone(),
                        attachment_row.clone(),
                    );
                }
                Err(error) => {
                    error!("Additional screen capture failed: {}", error);
                }
            },
            || {},
        );
    });

    // Handle User Submitting a Follow-Up Turn
    let on_submit_entry = entry.clone();
    let on_submit_msg_box = messages_box.clone();
    let on_submit_session = session.clone();
    let on_submit_rt = tokio_rt.clone();
    let on_submit_attach = attach_streaming_receiver.clone();
    let on_submit_images = pending_images.clone();
    let on_submit_initial_turn = initial_turn.clone();
    let on_submit_settings = preferences.settings.clone();
    let on_submit_active_title = preferences.active_window_title.clone();
    let last_ai_for_copy = last_ai_text.clone();
    let scrolled_weak = scrolled.downgrade();
    let attachment_preview_submit = attachment_previews.clone();
    let attachment_row_submit = attachment_row.clone();
    let submit_fetch_count = fetch_count.clone();
    let submit_fetch_glow = fetch_glow.clone();
    let submit_fetch_animation = fetch_animation.clone();
    let submit_glow_phase = glow_phase.clone();

    let submit_action: Rc<dyn Fn(String)> = Rc::new(move |text: String| {
        if text.trim().is_empty() {
            return;
        }

        let model_text = settings::compose_spatial_prompt(
            &on_submit_settings.borrow(),
            on_submit_active_title.as_deref(),
            &text,
        );
        on_submit_entry.buffer().set_text("");

        let is_initial_turn = std::mem::replace(&mut *on_submit_initial_turn.borrow_mut(), false);
        let captured_images = std::mem::take(&mut *on_submit_images.borrow_mut());

        // 1. Add the user prompt below the initial capture.
        let user_bubble = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        user_bubble.add_css_class("chat-bubble-user");
        user_bubble.set_halign(gtk4::Align::End);

        if !is_initial_turn && !captured_images.is_empty() {
            let image_previews = gtk4::Box::new(gtk4::Orientation::Horizontal, 7);
            image_previews.add_css_class("message-image-previews");
            refresh_attachment_previews(&image_previews, &captured_images);
            user_bubble.append(&image_previews);
        }

        let user_label = gtk4::Label::new(Some(&text));
        user_label.add_css_class("chat-message-text");
        user_label.set_wrap(true);
        user_label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        user_label.set_max_width_chars(64);
        user_label.set_selectable(true);
        user_label.set_xalign(1.0);
        user_bubble.append(&user_label);
        if !captured_images.is_empty() {
            refresh_attachment_previews(&attachment_preview_submit, &[]);
            attachment_row_submit.set_visible(false);
        }
        on_submit_msg_box.append(&user_bubble);

        // 2. Add assistant response placeholder bubble
        let ai_bubble = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        ai_bubble.add_css_class("chat-bubble-ai");

        let ai_response = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        ai_response.add_css_class("markdown-content");
        ai_response.set_hexpand(true);
        ai_bubble.append(&ai_response);

        let response_actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
        response_actions.add_css_class("response-actions");
        response_actions.set_halign(gtk4::Align::End);
        let response_copy = gtk4::Button::from_icon_name("edit-copy-symbolic");
        response_copy.add_css_class("response-copy");
        response_copy.set_tooltip_text(Some("Copy response"));
        let copy_text = last_ai_for_copy.clone();
        response_copy.connect_clicked(move |button| {
            let text = copy_text.borrow().clone();
            if text.is_empty() {
                return;
            }
            match executor::copy_to_clipboard(&text) {
                Ok(()) => {
                    button.set_icon_name("object-select-symbolic");
                }
                Err(error) => {
                    error!("Clipboard copy failed: {error:#}");
                    button.set_tooltip_text(Some(&format!("Copy failed: {error}")));
                }
            }
        });
        response_actions.append(&response_copy);
        ai_bubble.append(&response_actions);
        on_submit_msg_box.append(&ai_bubble);
        if let Some(scrolled) = scrolled_weak.upgrade() {
            scroll_to_bottom(&scrolled);
        }

        // 3. Dispatch follow-up to Gemini over Tokio session
        begin_fetch_indicator(
            &submit_fetch_count,
            &submit_fetch_glow,
            &submit_fetch_animation,
            &submit_glow_phase,
        );
        let (token_tx, token_rx) = mpsc::channel::<String>(64);
        on_submit_attach(token_rx, ai_response);
        let error_tx = token_tx.clone();

        let session_clone = on_submit_session.clone();
        on_submit_rt.spawn(async move {
            if let Err(e) = session_clone
                .add_user_turn(&model_text, &captured_images)
                .await
            {
                error!("Failed to create user turn: {}", e);
                let _ = error_tx.send(format!("[Error: {e}]")).await;
                return;
            }

            if let Err(e) = session_clone.stream_next_turn(token_tx).await {
                error!("Error streaming follow up: {}", e);
                let _ = error_tx.send(format!("[Error: {e}]")).await;
            }
        });
    });

    let send_submit = submit_action.clone();
    let send_entry = entry.clone();
    send_btn.connect_clicked(move |_| {
        send_submit(text_view_contents(&send_entry));
    });

    let submit_key = submit_action.clone();
    let submit_key_entry = entry.clone();
    let submit_controller = gtk4::EventControllerKey::new();
    submit_controller.connect_key_pressed(move |_, key, _, state| {
        if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter)
            && state.contains(gdk::ModifierType::CONTROL_MASK)
        {
            submit_key(text_view_contents(&submit_key_entry));
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    entry.add_controller(submit_controller);

    // Escape key closes window
    let key_controller = gtk4::EventControllerKey::new();
    let win_weak_key = window.downgrade();
    key_controller.connect_key_pressed(move |_, key, _, _| {
        if key == gdk::Key::Escape {
            if let Some(win) = win_weak_key.upgrade() {
                win.close();
            }
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(key_controller);

    submit_action(initial_prompt);
    window
}

fn begin_fetch_indicator(
    fetch_count: &Rc<Cell<usize>>,
    glow: &gtk4::DrawingArea,
    animation: &Rc<RefCell<Option<glib::SourceId>>>,
    phase: &Rc<Cell<f64>>,
) {
    fetch_count.set(fetch_count.get() + 1);
    glow.set_visible(true);

    if animation.borrow().is_none() {
        let is_fetching = fetch_count.clone();
        let glow = glow.downgrade();
        let phase = phase.clone();
        let source = glib::timeout_add_local(Duration::from_millis(40), move || {
            if is_fetching.get() == 0 {
                return glib::ControlFlow::Break;
            }

            phase.set((phase.get() + 0.035) % std::f64::consts::TAU);
            if let Some(glow) = glow.upgrade() {
                glow.queue_draw();
            }
            glib::ControlFlow::Continue
        });
        *animation.borrow_mut() = Some(source);
    }
}

fn finish_fetch_indicator(
    fetch_count: &Rc<Cell<usize>>,
    glow: &gtk4::DrawingArea,
    animation: &Rc<RefCell<Option<glib::SourceId>>>,
) {
    let remaining = fetch_count.get().saturating_sub(1);
    fetch_count.set(remaining);
    if remaining == 0 {
        glow.set_visible(false);
        if let Some(source) = animation.borrow_mut().take() {
            source.remove();
        }
    }
}

fn draw_fetch_glow(
    context: &gtk4::cairo::Context,
    width: i32,
    height: i32,
    phase: f64,
) -> Result<(), gtk4::cairo::Error> {
    if width <= 0 || height <= 0 {
        return Ok(());
    }

    let pulse = 0.45 + 0.55 * ((phase.sin() + 1.0) / 2.0);
    for (line_width, alpha) in [(14.0, 0.09), (8.0, 0.13), (2.5, 0.72)] {
        let gradient =
            gtk4::cairo::LinearGradient::new(0.0, 0.0, width as f64, height as f64 * 0.45);
        for (offset, color) in [
            (0.0, (1.0, 0.08, 0.20)),
            (0.2, (1.0, 0.48, 0.05)),
            (0.4, (0.95, 0.92, 0.12)),
            (0.6, (0.05, 0.88, 0.42)),
            (0.8, (0.12, 0.48, 1.0)),
            (1.0, (0.72, 0.18, 1.0)),
        ] {
            gradient.add_color_stop_rgba(
                offset,
                color.0,
                color.1,
                color.2,
                alpha * pulse,
            );
        }

        context.set_line_width(line_width);
        context.set_source(&gradient)?;
        rounded_rectangle_path(
            context,
            9.0,
            9.0,
            width as f64 - 18.0,
            height as f64 - 18.0,
            16.0,
        );
        context.stroke()?;
    }
    Ok(())
}

fn rounded_rectangle_path(
    context: &gtk4::cairo::Context,
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    radius: f64,
) {
    let radius = radius.min(width / 2.0).min(height / 2.0);
    context.new_sub_path();
    context.arc(
        x + width - radius,
        y + radius,
        radius,
        -std::f64::consts::FRAC_PI_2,
        0.0,
    );
    context.arc(
        x + width - radius,
        y + height - radius,
        radius,
        0.0,
        std::f64::consts::FRAC_PI_2,
    );
    context.arc(
        x + radius,
        y + height - radius,
        radius,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
    );
    context.arc(
        x + radius,
        y + radius,
        radius,
        std::f64::consts::PI,
        3.0 * std::f64::consts::FRAC_PI_2,
    );
    context.close_path();
}

fn refresh_attachment_previews(container: &gtk4::Box, images: &[DynamicImage]) {
    const PREVIEW_MIN_WIDTH: i32 = 96;
    const PREVIEW_MIN_HEIGHT: i32 = 72;

    while let Some(child) = container.first_child() {
        container.remove(&child);
    }

    for image in images {
        let preview = picture_for_image(image, PREVIEW_MIN_WIDTH, PREVIEW_MIN_HEIGHT);
        preview.add_css_class("attachment-preview");
        container.append(&preview);
    }
}

fn refresh_pending_attachment_previews(
    container: &gtk4::Box,
    images: Rc<RefCell<Vec<DynamicImage>>>,
    attachment_row: gtk4::Box,
) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }

    let image_count = images.borrow().len();
    for index in 0..image_count {
        let preview = gtk4::Overlay::new();
        preview.add_css_class("pending-attachment-preview");
        preview.set_child(Some(&picture_for_image(&images.borrow()[index], 96, 72)));

        let remove_button = gtk4::Button::from_icon_name("window-close-symbolic");
        remove_button.add_css_class("remove-attachment-button");
        remove_button.set_tooltip_text(Some("Remove capture"));
        remove_button.set_halign(gtk4::Align::End);
        remove_button.set_valign(gtk4::Align::Start);
        preview.add_overlay(&remove_button);

        let pending_images = images.clone();
        let previews = container.clone();
        let attachment_row = attachment_row.clone();
        remove_button.connect_clicked(move |_| {
            let mut images = pending_images.borrow_mut();
            if index < images.len() {
                images.remove(index);
            }
            let is_empty = images.is_empty();
            drop(images);
            attachment_row.set_visible(!is_empty);
            refresh_pending_attachment_previews(
                &previews,
                pending_images.clone(),
                attachment_row.clone(),
            );
        });

        container.append(&preview);
    }
}

fn text_view_contents(text_view: &gtk4::TextView) -> String {
    let buffer = text_view.buffer();
    let (start, end) = buffer.bounds();
    buffer.text(&start, &end, true).to_string()
}

fn picture_for_image(image: &DynamicImage, max_width: i32, max_height: i32) -> gtk4::Picture {
    let thumbnail = image
        .thumbnail(max_width as u32, max_height as u32)
        .to_rgba8();
    let width = thumbnail.width();
    let height = thumbnail.height();
    let bytes = glib::Bytes::from_owned(thumbnail.into_raw());
    let texture = gdk::MemoryTexture::new(
        width as i32,
        height as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &bytes,
        width as usize * 4,
    );
    let picture = gtk4::Picture::for_paintable(&texture);
    picture.set_can_shrink(true);
    picture.set_keep_aspect_ratio(true);
    picture.set_size_request(max_width, max_height);
    picture.set_overflow(gtk4::Overflow::Hidden);
    picture
}

fn scroll_to_bottom(scrolled: &gtk4::ScrolledWindow) {
    let scrolled = scrolled.downgrade();
    glib::idle_add_local_once(move || {
        if let Some(scrolled) = scrolled.upgrade() {
            let adjustment = scrolled.vadjustment();
            adjustment
                .set_value((adjustment.upper() - adjustment.page_size()).max(adjustment.lower()));
        }
    });
}
