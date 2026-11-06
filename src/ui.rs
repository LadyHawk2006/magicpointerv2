use std::sync::Arc;

use crate::ai_agent::ConversationSession;
use crate::executor;
use crate::screencapture;
use crate::selection_overlay::{self, Rect};
use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use image::DynamicImage;
use std::cell::RefCell;
use std::rc::Rc;
use tokio::sync::mpsc;
use tracing::error;

pub fn build_floating_card(
    app: &gtk4::Application,
    captured_image: DynamicImage,
    captured_context: Option<String>,
    initial_prompt: String,
    session: ConversationSession,
    tokio_rt: Arc<tokio::runtime::Runtime>,
) -> gtk4::ApplicationWindow {
    let window = gtk4::ApplicationWindow::new(app);
    window.set_title(Some("Magic Pointer"));
    window.set_default_size(537, 763);
    window.set_decorated(false);

    window.add_css_class("card-window");

    // Outer card
    let card = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    card.add_css_class("magic-card");
    card.set_hexpand(true);
    card.set_vexpand(true);

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
    let attachment_status = gtk4::Label::new(None);
    attachment_status.add_css_class("attachment-status");
    attachment_row.append(&attachment_previews);
    attachment_row.append(&attachment_status);
    attachment_row.set_visible(false);

    let bottom_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 9);
    bottom_row.add_css_class("chat-input-bar");

    let entry = gtk4::Entry::new();
    entry.add_css_class("chat-entry");
    entry.set_placeholder_text(Some("Ask about this..."));
    entry.set_width_chars(1);
    entry.set_hexpand(true);

    let entry_shell = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    entry_shell.add_css_class("chat-entry-shell");
    entry_shell.set_hexpand(true);
    entry_shell.append(&entry);

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

    window.set_child(Some(&card));

    let capture_app = app.clone();
    let capture_pending_images = pending_images.clone();
    let capture_attachment_status = attachment_status.clone();
    let capture_attachment_previews = attachment_previews.clone();
    let capture_attachment_row = attachment_row.clone();
    capture_btn.connect_clicked(move |_| {
        let pending_images = capture_pending_images.clone();
        let attachment_status = capture_attachment_status.clone();
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
                    let count = images.len();
                    attachment_row.set_visible(true);
                    let attachment_message = if count == 1 {
                        "capture attached".to_string()
                    } else {
                        format!("{count} captures attached")
                    };
                    attachment_status.set_text(&attachment_message);
                    refresh_attachment_previews(&attachment_previews, &images);
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
    let last_ai_for_copy = last_ai_text.clone();
    let scrolled_weak = scrolled.downgrade();
    let attachment_preview_submit = attachment_previews.clone();
    let attachment_status_submit = attachment_status.clone();
    let attachment_row_submit = attachment_row.clone();

    let submit_action: Rc<dyn Fn(String)> = Rc::new(move |text: String| {
        if text.trim().is_empty() {
            return;
        }

        on_submit_entry.set_text("");

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
            if !is_initial_turn {
                let image_count = gtk4::Label::new(Some(&format!(
                    "{} capture{} attached",
                    captured_images.len(),
                    if captured_images.len() == 1 { "" } else { "s" }
                )));
                image_count.add_css_class("message-attachment");
                user_bubble.append(&image_count);
            }
            attachment_status_submit.set_text("");
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
        let (token_tx, token_rx) = mpsc::channel::<String>(64);
        on_submit_attach(token_rx, ai_response);
        let error_tx = token_tx.clone();

        let session_clone = on_submit_session.clone();
        on_submit_rt.spawn(async move {
            if let Err(e) = session_clone.add_user_turn(&text, &captured_images).await {
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

    let submit_clone1 = submit_action.clone();
    entry.connect_activate(move |entry| {
        submit_clone1(entry.text().to_string());
    });

    let send_submit = submit_action.clone();
    let send_entry = entry.clone();
    send_btn.connect_clicked(move |_| {
        send_submit(send_entry.text().to_string());
    });

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
