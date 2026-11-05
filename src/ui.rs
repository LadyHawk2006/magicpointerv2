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
    session: ConversationSession,
    tokio_rt: Arc<tokio::runtime::Runtime>,
) -> gtk4::ApplicationWindow {
    let window = gtk4::ApplicationWindow::new(app);
    window.set_title(Some("Magic Pointer"));
    window.set_default_size(560, 600);

    window.add_css_class("card-window");

    // Outer card
    let card = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    card.add_css_class("magic-card");
    card.set_hexpand(true);
    card.set_vexpand(true);

    // Header container (Acts as a Drag Handle)
    let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
    header.add_css_class("magic-header");

    let title_icon = gtk4::Image::from_icon_name("sparkles-symbolic");
    title_icon.add_css_class("brand-icon");
    let title_label = gtk4::Label::new(Some("Magic Pointer"));
    title_label.add_css_class("magic-title");

    let status_label = gtk4::Label::new(Some("Ready"));
    status_label.add_css_class("magic-status");
    status_label.set_hexpand(true);
    status_label.set_halign(gtk4::Align::Start);

    let copy_btn = gtk4::Button::from_icon_name("edit-copy-symbolic");
    copy_btn.add_css_class("header-action");
    copy_btn.set_tooltip_text(Some("Copy last response"));

    let close_btn = gtk4::Button::from_icon_name("window-close-symbolic");
    close_btn.add_css_class("header-action");
    let win_weak_close = window.downgrade();
    close_btn.connect_clicked(move |_| {
        if let Some(win) = win_weak_close.upgrade() {
            win.close();
        }
    });

    header.append(&title_icon);
    header.append(&title_label);
    header.append(&status_label);
    header.append(&copy_btn);
    header.append(&close_btn);
    card.append(&header);

    // --- Chat Thread View ---
    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.add_css_class("chat-history");
    scrolled.set_vexpand(true);
    scrolled.set_hexpand(true);
    scrolled.set_hscrollbar_policy(gtk4::PolicyType::Never);

    let messages_box = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    messages_box.add_css_class("chat-messages");
    scrolled.set_child(Some(&messages_box));
    card.append(&scrolled);

    let pending_images = Rc::new(RefCell::new(vec![captured_image]));

    // Function to add a styled AI message bubble
    let selection_notice = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
    selection_notice.add_css_class("selection-notice");
    let selection_icon = gtk4::Image::from_icon_name("image-x-generic-symbolic");
    selection_icon.add_css_class("selection-notice-icon");
    let selection_copy = gtk4::Box::new(gtk4::Orientation::Vertical, 3);
    selection_copy.set_hexpand(true);
    let selection_title = gtk4::Label::new(Some("Your screen, ready to explore"));
    selection_title.add_css_class("selection-notice-title");
    selection_title.set_xalign(0.0);
    let selection_message = match captured_context.as_deref() {
        Some(context) => format!("Captured from {context}"),
        None => "Ask about anything in the captured area.".to_string(),
    };
    let selection_label = gtk4::Label::new(Some(&selection_message));
    selection_label.add_css_class("selection-notice-label");
    selection_label.set_wrap(true);
    selection_label.set_xalign(0.0);
    selection_notice.append(&selection_icon);
    selection_copy.append(&selection_title);
    selection_copy.append(&selection_label);
    selection_notice.append(&selection_copy);
    messages_box.append(&selection_notice);

    let last_ai_text = Rc::new(RefCell::new(String::new()));
    let last_ai_clone = last_ai_text.clone();
    let copy_status_weak = status_label.downgrade();

    copy_btn.connect_clicked(move |btn| {
        let text = last_ai_clone.borrow().clone();
        if !text.is_empty() {
            match executor::copy_to_clipboard(&text) {
                Ok(()) => {
                    btn.set_icon_name("object-select-symbolic");
                    if let Some(status) = copy_status_weak.upgrade() {
                        status.set_text("Copied");
                        status.set_tooltip_text(None);
                    }
                }
                Err(error) => {
                    error!("Clipboard copy failed: {error:#}");
                    if let Some(status) = copy_status_weak.upgrade() {
                        status.set_text("Copy failed");
                        status.set_tooltip_text(Some(&error.to_string()));
                    }
                }
            }
        }
    });

    // Helper to stream tokens into a label
    let attach_streaming_receiver = {
        let status_weak = status_label.downgrade();
        let scrolled_weak = scrolled.downgrade();
        let last_ai_store = last_ai_text.clone();

        move |mut rx: mpsc::Receiver<String>, response: gtk4::Box| {
            let status_weak = status_weak.clone();
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

                    if let Some(status) = status_weak.upgrade() {
                        status.set_text("Streaming...");
                    }

                    // Auto-scroll to bottom
                    if let Some(scr) = scrolled_weak.upgrade() {
                        scroll_to_bottom(&scr);
                    }
                }

                if accumulated.borrow().is_empty() {
                    crate::markdown::render_markdown(&response, "No response received.");
                }

                if let Some(status) = status_weak.upgrade() {
                    status.set_text("Ready");
                }
            });
        }
    };

    // --- Interactive Chat Input Bar + Resize Corner ---
    let composer = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
    composer.add_css_class("chat-composer");

    let attachment_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    attachment_row.add_css_class("attachment-row");
    let attachment_previews = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    attachment_previews.add_css_class("attachment-previews");
    let attachment_status = gtk4::Label::new(Some("1 capture"));
    attachment_status.add_css_class("attachment-status");
    attachment_row.append(&attachment_previews);
    attachment_row.append(&attachment_status);
    refresh_attachment_previews(&attachment_previews, &pending_images.borrow());

    let bottom_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 9);
    bottom_row.add_css_class("chat-input-bar");

    let entry = gtk4::Entry::new();
    entry.add_css_class("chat-entry");
    entry.set_placeholder_text(Some("Ask a question or follow up..."));
    entry.set_width_chars(1);
    entry.set_hexpand(true);

    let preset_grid = gtk4::Grid::new();
    preset_grid.add_css_class("prompt-presets");
    preset_grid.set_column_spacing(8);
    preset_grid.set_row_spacing(8);
    preset_grid.set_column_homogeneous(true);

    let prompt_presets = [
        (
            "Explain error",
            "dialog-warning-symbolic",
            "Explain the error visible in the attached capture, identify the likely cause, and suggest practical steps to fix it. If no error is visible, say so.",
        ),
        (
            "Summarize",
            "view-list-symbolic",
            "Summarize the selected content concisely. Highlight only the most important points.",
        ),
        (
            "Extract text",
            "edit-find-symbolic",
            "Transcribe all legible text from the attached capture, preserving its structure where practical. Mark anything uncertain.",
        ),
        (
            "Describe UI",
            "applications-graphics-symbolic",
            "Describe the visible interface, explain the purpose of its important elements, and suggest useful next steps.",
        ),
        (
            "Translate",
            "preferences-desktop-locale-symbolic",
            "Translate the visible text into English. Preserve the original meaning and briefly identify the source language if possible.",
        ),
    ];
    let mut preset_buttons = Vec::with_capacity(prompt_presets.len());
    for (index, (label, icon, prompt)) in prompt_presets.into_iter().enumerate() {
        let button = gtk4::Button::new();
        button.add_css_class("prompt-preset-btn");
        let content = gtk4::Box::new(gtk4::Orientation::Horizontal, 7);
        let image = gtk4::Image::from_icon_name(icon);
        image.add_css_class("preset-icon");
        let title = gtk4::Label::new(Some(label));
        content.append(&image);
        content.append(&title);
        button.set_child(Some(&content));
        preset_grid.attach(&button, (index % 3) as i32, (index / 3) as i32, 1, 1);
        preset_buttons.push((button, prompt));
    }

    let capture_btn = gtk4::Button::from_icon_name("list-add-symbolic");
    capture_btn.add_css_class("capture-image-btn");
    capture_btn.set_tooltip_text(Some("Capture another image"));

    let send_btn = gtk4::Button::from_icon_name("go-up-symbolic");
    send_btn.add_css_class("chat-send-btn");
    send_btn.set_tooltip_text(Some("Send message"));
    card.append(&preset_grid);

    bottom_row.append(&capture_btn);
    bottom_row.append(&entry);
    bottom_row.append(&send_btn);
    composer.append(&attachment_row);
    composer.append(&bottom_row);
    card.append(&composer);

    window.set_child(Some(&card));

    let capture_app = app.clone();
    let capture_pending_images = pending_images.clone();
    let capture_status = status_label.downgrade();
    let capture_attachment_status = attachment_status.clone();
    let capture_attachment_previews = attachment_previews.clone();
    let capture_attachment_row = attachment_row.clone();
    capture_btn.connect_clicked(move |_| {
        if let Some(status) = capture_status.upgrade() {
            status.set_text("Select a region...");
        }

        let pending_images = capture_pending_images.clone();
        let status_weak = capture_status.clone();
        let dismiss_status = capture_status.clone();
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
                    attachment_status.set_text(&format!(
                        "{} capture{}",
                        count,
                        if count == 1 { "" } else { "s" }
                    ));
                    refresh_attachment_previews(&attachment_previews, &images);
                    if let Some(status) = status_weak.upgrade() {
                        status.set_text("Capture attached");
                    }
                }
                Err(error) => {
                    error!("Additional screen capture failed: {}", error);
                    if let Some(status) = status_weak.upgrade() {
                        status.set_text("Capture failed");
                    }
                }
            },
            move || {
                if let Some(status) = dismiss_status.upgrade() {
                    status.set_text("Ready");
                }
            },
        );
    });

    // Handle User Submitting a Follow-Up Turn
    let on_submit_entry = entry.clone();
    let on_submit_msg_box = messages_box.clone();
    let on_submit_session = session.clone();
    let on_submit_rt = tokio_rt.clone();
    let on_submit_attach = attach_streaming_receiver.clone();
    let on_submit_images = pending_images.clone();
    let status_weak = status_label.downgrade();
    let scrolled_weak = scrolled.downgrade();
    let preset_grid_submit = preset_grid.clone();
    let attachment_preview_submit = attachment_previews.clone();
    let attachment_status_submit = attachment_status.clone();
    let attachment_row_submit = attachment_row.clone();

    let submit_action: Rc<dyn Fn(String)> = Rc::new(move |text: String| {
        if text.trim().is_empty() {
            return;
        }

        preset_grid_submit.set_visible(false);
        on_submit_entry.set_text("");

        // 1. Add user message bubble
        let user_bubble = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        user_bubble.add_css_class("chat-bubble-user");
        user_bubble.set_halign(gtk4::Align::End);

        let captured_images = std::mem::take(&mut *on_submit_images.borrow_mut());
        if !captured_images.is_empty() {
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
            let image_count = gtk4::Label::new(Some(&format!(
                "{} capture{} attached",
                captured_images.len(),
                if captured_images.len() == 1 { "" } else { "s" }
            )));
            image_count.add_css_class("message-attachment");
            user_bubble.append(&image_count);
            attachment_status_submit.set_text("No captures");
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
        on_submit_msg_box.append(&ai_bubble);
        if let Some(scrolled) = scrolled_weak.upgrade() {
            scroll_to_bottom(&scrolled);
        }

        // 3. Dispatch follow-up to Gemini over Tokio session
        let (token_tx, token_rx) = mpsc::channel::<String>(64);
        on_submit_attach(token_rx, ai_response);
        let error_tx = token_tx.clone();
        if let Some(status) = status_weak.upgrade() {
            status.set_text("Thinking...");
        }

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

    for (button, prompt) in preset_buttons {
        let submit = submit_action.clone();
        button.connect_clicked(move |_| {
            submit(prompt.to_string());
        });
    }

    send_btn.connect_clicked(move |_| {
        submit_action(entry.text().to_string());
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

    window
}

fn refresh_attachment_previews(container: &gtk4::Box, images: &[DynamicImage]) {
    const PREVIEW_MIN_WIDTH: i32 = 96;
    const PREVIEW_MIN_HEIGHT: i32 = 72;

    while let Some(child) = container.first_child() {
        container.remove(&child);
    }

    for image in images {
        let rgba = image.thumbnail(192, 144).to_rgba8();
        let width = rgba.width();
        let height = rgba.height();
        let bytes = glib::Bytes::from_owned(rgba.into_raw());
        let texture = gdk::MemoryTexture::new(
            width as i32,
            height as i32,
            gdk::MemoryFormat::R8g8b8a8,
            &bytes,
            width as usize * 4,
        );
        let preview = gtk4::Image::from_paintable(Some(&texture));
        preview.set_size_request(PREVIEW_MIN_WIDTH, PREVIEW_MIN_HEIGHT);
        preview.add_css_class("attachment-preview");
        container.append(&preview);
    }
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
