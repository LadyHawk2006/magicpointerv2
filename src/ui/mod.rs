use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use image::DynamicImage;
use tokio::sync::mpsc;
use tracing::error;

use crate::ai_agent::ConversationSession;
use crate::executor;
use crate::screencapture;
use crate::selection_overlay;
use crate::settings::{self, AppSettings};
use crate::settings_ui;

pub mod attachments;
pub mod callouts;
pub mod indicator;
pub mod markdown;

use attachments::{
    picture_for_image, refresh_attachment_previews, refresh_pending_attachment_previews,
};
use indicator::{begin_fetch_indicator, draw_fetch_glow, finish_fetch_indicator};
pub use markdown::render_markdown;

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
    {
        let window_settings = &preferences.settings.borrow().hyprland_window;
        window.set_default_size(window_settings.width as i32, window_settings.height as i32);
    }
    window.set_decorated(false);
    window.set_opacity(1.0);

    window.add_css_class("card-window");

    // Outer card
    let card = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    card.add_css_class("magic-card");
    card.set_hexpand(true);
    card.set_vexpand(true);

    let fetch_glow = gtk4::DrawingArea::new();
    fetch_glow.add_css_class("chat-fetch-glow");
    fetch_glow.set_hexpand(true);
    fetch_glow.set_can_target(false);
    fetch_glow.set_visible(false);
    fetch_glow.set_valign(gtk4::Align::Start);
    fetch_glow.set_size_request(-1, 2);
    let glow_phase = Rc::new(Cell::new(0.0));
    let glow_phase_draw = glow_phase.clone();
    fetch_glow.set_draw_func(move |_, context, width, height| {
        if let Err(error) = draw_fetch_glow(context, width, height, glow_phase_draw.get()) {
            tracing::warn!("Failed to draw response activity glow: {error}");
        }
    });
    let fetch_count = Rc::new(Cell::new(0));
    let fetch_animation = Rc::new(RefCell::new(None));

    let toolbar = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    toolbar.add_css_class("chat-toolbar");
    let close_button = gtk4::Button::from_icon_name("window-close-symbolic");
    close_button.add_css_class("window-close-button");
    close_button.set_tooltip_text(Some("Close chat and end this session"));
    let close_window = window.clone();
    close_button.connect_clicked(move |_| close_window.close());
    toolbar.append(&close_button);

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

    let capture_overlay = gtk4::Overlay::new();
    capture_overlay.add_css_class("initial-capture-overlay");
    capture_overlay.set_child(Some(&initial_capture));

    let callouts_da = gtk4::DrawingArea::new();
    callouts_da.set_can_target(false);
    let detected_callouts: Rc<RefCell<Vec<callouts::VisualCallout>>> =
        Rc::new(RefCell::new(Vec::new()));
    let callouts_draw = detected_callouts.clone();
    let img_w = captured_image.width() as f64;
    let img_h = captured_image.height() as f64;
    callouts_da.set_draw_func(move |_, cr, width, height| {
        let list = callouts_draw.borrow();
        callouts::draw_callouts_on_context(cr, &list, img_w, img_h, width as f64, height as f64);
    });
    capture_overlay.add_overlay(&callouts_da);
    messages_box.append(&capture_overlay);

    let pending_images = Rc::new(RefCell::new(vec![captured_image]));
    let initial_turn = Rc::new(RefCell::new(true));

    let last_ai_text = Rc::new(RefCell::new(String::new()));

    // Helper to stream tokens into a label
    let attach_streaming_receiver = {
        let scrolled_weak = scrolled.downgrade();
        let last_ai_store = last_ai_text.clone();
        let fetch_count = fetch_count.clone();
        let fetch_glow = fetch_glow.clone();
        let fetch_animation = fetch_animation.clone();
        let callouts_store = detected_callouts.clone();
        let callouts_da_ref = callouts_da.clone();

        move |mut rx: mpsc::Receiver<String>, response: gtk4::Box| {
            let scrolled_weak = scrolled_weak.clone();
            let fetch_count = fetch_count.clone();
            let fetch_glow = fetch_glow.clone();
            let fetch_animation = fetch_animation.clone();
            let accumulated = Rc::new(RefCell::new(String::new()));
            let last_store = last_ai_store.clone();
            let response = response.clone();
            let callouts_target = callouts_store.clone();
            let da_refresh = callouts_da_ref.clone();

            glib::spawn_future_local(async move {
                while let Some(token) = rx.recv().await {
                    accumulated.borrow_mut().push_str(&token);
                    let raw_text = accumulated.borrow().clone();
                    *last_store.borrow_mut() = raw_text.clone();

                    markdown::render_markdown(&response, &raw_text);

                    let parsed = callouts::parse_visual_callouts(&raw_text);
                    if *callouts_target.borrow() != parsed {
                        *callouts_target.borrow_mut() = parsed;
                        da_refresh.queue_draw();
                    }

                    // Auto-scroll to bottom
                    if let Some(scr) = scrolled_weak.upgrade() {
                        scroll_to_bottom(&scr);
                    }
                }

                if accumulated.borrow().is_empty() {
                    markdown::render_markdown(&response, "No response received.");
                } else {
                    let raw_text = accumulated.borrow().clone();
                    let parsed = callouts::parse_visual_callouts(&raw_text);
                    if *callouts_target.borrow() != parsed {
                        *callouts_target.borrow_mut() = parsed;
                        da_refresh.queue_draw();
                    }
                }
                finish_fetch_indicator(&fetch_count, &fetch_glow, &fetch_animation);
            });
        }
    };

    // --- Interactive Chat Input Bar + Resize Corner ---
    let composer_overlay = gtk4::Overlay::new();
    composer_overlay.add_css_class("chat-composer-overlay");
    composer_overlay.set_hexpand(true);
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
    entry.set_top_margin(10);
    entry.set_bottom_margin(10);
    entry.set_left_margin(6);
    entry.set_right_margin(6);

    let entry_overlay = gtk4::Overlay::new();
    entry_overlay.set_child(Some(&entry));
    let input_placeholder = gtk4::Label::new(Some("Ask about this..."));
    input_placeholder.add_css_class("chat-entry-placeholder");
    input_placeholder.set_halign(gtk4::Align::Start);
    input_placeholder.set_valign(gtk4::Align::Center);
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
    composer_overlay.set_child(Some(&composer));
    composer_overlay.add_overlay(&fetch_glow);
    card.append(&composer_overlay);

    window.set_child(Some(&card));

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
        let capture_app = capture_app.clone();
        let callback_app = capture_app.clone();
        let pending_images = capture_pending_images.clone();
        let attachment_previews = capture_attachment_previews.clone();
        let attachment_row = capture_attachment_row.clone();
        selection_overlay::show_selection_overlay(
            &capture_app,
            move |target: screencapture::CaptureTarget| match screencapture::capture_target(&target)
            {
                Ok(image) => {
                    let mut images = pending_images.borrow_mut();
                    images.push(image);
                    drop(images);
                    attachment_row.set_visible(true);
                    refresh_pending_attachment_previews(
                        &callback_app,
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
        if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter) {
            if state.contains(gdk::ModifierType::SHIFT_MASK) {
                glib::Propagation::Proceed
            } else {
                let text = text_view_contents(&submit_key_entry);
                if !text.trim().is_empty() {
                    submit_key(text);
                }
                glib::Propagation::Stop
            }
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

fn text_view_contents(text_view: &gtk4::TextView) -> String {
    let buffer = text_view.buffer();
    let (start, end) = buffer.bounds();
    buffer.text(&start, &end, true).to_string()
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
