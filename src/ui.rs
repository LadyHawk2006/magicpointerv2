use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use crate::ai_agent::ConversationSession;
use crate::executor;
use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use std::cell::RefCell;
use std::rc::Rc;
use tokio::sync::mpsc;
use tracing::error;

pub fn build_floating_card(
    app: &gtk4::Application,
    cursor_x: i32,
    cursor_y: i32,
    captured_context: Option<String>,
    initial_token_rx: mpsc::Receiver<String>,
    session: ConversationSession,
    tokio_rt: Arc<tokio::runtime::Runtime>,
) -> gtk4::ApplicationWindow {
    let window = gtk4::ApplicationWindow::new(app);

    window.init_layer_shell();
    window.set_layer(Layer::Overlay);
    window.set_keyboard_mode(KeyboardMode::OnDemand);
    window.set_exclusive_zone(-1);

    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Left, true);

    let current_x = Arc::new(AtomicI32::new((cursor_x + 16).max(10)));
    let current_y = Arc::new(AtomicI32::new((cursor_y + 16).max(10)));
    let current_width = Arc::new(AtomicI32::new(420));
    let current_height = Arc::new(AtomicI32::new(320));

    window.set_margin(Edge::Left, current_x.load(Ordering::Relaxed));
    window.set_margin(Edge::Top, current_y.load(Ordering::Relaxed));

    window.add_css_class("card-window");

    // Outer card
    let card = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    card.add_css_class("magic-card");
    card.set_size_request(
        current_width.load(Ordering::Relaxed),
        current_height.load(Ordering::Relaxed),
    );

    // Header container (Acts as a Drag Handle)
    let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    header.add_css_class("magic-header");

    let title_icon = gtk4::Image::from_icon_name("sparkles-symbolic");
    let title_label = gtk4::Label::new(Some("Magic Pointer"));
    title_label.add_css_class("magic-title");

    let status_label = gtk4::Label::new(Some(if let Some(ref ctx) = captured_context {
        ctx.as_str()
    } else {
        "Thinking..."
    }));
    status_label.add_css_class("magic-status");
    status_label.set_hexpand(true);
    status_label.set_halign(gtk4::Align::Start);

    let copy_btn = gtk4::Button::from_icon_name("edit-copy-symbolic");
    copy_btn.add_css_class("magic-btn-close");
    copy_btn.set_tooltip_text(Some("Copy last response"));

    let close_btn = gtk4::Button::from_icon_name("window-close-symbolic");
    close_btn.add_css_class("magic-btn-close");
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

    // --- Drag Handle Logic for Moving Card ---
    let drag_gesture = gtk4::GestureDrag::new();
    let win_weak_drag = window.downgrade();
    let drag_start_x = Rc::new(RefCell::new(0));
    let drag_start_y = Rc::new(RefCell::new(0));
    let pos_x = current_x.clone();
    let pos_y = current_y.clone();

    let sx = drag_start_x.clone();
    let sy = drag_start_y.clone();
    let px = pos_x.clone();
    let py = pos_y.clone();
    drag_gesture.connect_drag_begin(move |_, _, _| {
        *sx.borrow_mut() = px.load(Ordering::Relaxed);
        *sy.borrow_mut() = py.load(Ordering::Relaxed);
    });

    let sx2 = drag_start_x.clone();
    let sy2 = drag_start_y.clone();
    drag_gesture.connect_drag_update(move |_gesture, offset_x, offset_y| {
        let new_x = *sx2.borrow() + offset_x.round() as i32;
        let new_y = *sy2.borrow() + offset_y.round() as i32;
        pos_x.store(new_x, Ordering::Relaxed);
        pos_y.store(new_y, Ordering::Relaxed);

        if let Some(win) = win_weak_drag.upgrade() {
            win.set_margin(Edge::Left, new_x.max(0));
            win.set_margin(Edge::Top, new_y.max(0));
        }
    });
    header.add_controller(drag_gesture);

    // --- Chat Thread View ---
    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_hexpand(true);
    scrolled.set_hscrollbar_policy(gtk4::PolicyType::Never);

    let messages_box = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    scrolled.set_child(Some(&messages_box));
    card.append(&scrolled);

    // Function to add a styled AI message bubble
    let initial_bubble = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    initial_bubble.add_css_class("chat-bubble-ai");

    let initial_ai_label = gtk4::Label::new(Some("Analyzing screen capture..."));
    initial_ai_label.set_wrap(true);
    initial_ai_label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
    initial_ai_label.set_selectable(true);
    initial_ai_label.set_xalign(0.0);
    initial_bubble.append(&initial_ai_label);
    messages_box.append(&initial_bubble);

    let last_ai_text = Rc::new(RefCell::new(String::new()));
    let last_ai_clone = last_ai_text.clone();

    copy_btn.connect_clicked(move |btn| {
        let text = last_ai_clone.borrow().clone();
        if !text.is_empty() {
            let _ = executor::copy_to_clipboard(&text);
            btn.set_icon_name("object-select-symbolic");
        }
    });

    // Helper to stream tokens into a label
    let attach_streaming_receiver = {
        let status_weak = status_label.downgrade();
        let scrolled_weak = scrolled.downgrade();
        let last_ai_store = last_ai_text.clone();

        move |mut rx: mpsc::Receiver<String>, label: gtk4::Label| {
            let status_weak = status_weak.clone();
            let scrolled_weak = scrolled_weak.clone();
            let accumulated = Rc::new(RefCell::new(String::new()));
            let last_store = last_ai_store.clone();

            glib::spawn_future_local(async move {
                while let Some(token) = rx.recv().await {
                    accumulated.borrow_mut().push_str(&token);
                    let raw_text = accumulated.borrow().clone();
                    *last_store.borrow_mut() = raw_text.clone();

                    let pango_text = markdown_to_pango(&raw_text);
                    label.set_markup(&pango_text);

                    if let Some(status) = status_weak.upgrade() {
                        status.set_text("Streaming...");
                    }

                    // Auto-scroll to bottom
                    if let Some(scr) = scrolled_weak.upgrade() {
                        let adj = scr.vadjustment();
                        adj.set_value(adj.upper() - adj.page_size());
                    }
                }

                if accumulated.borrow().is_empty() {
                    label.set_text("No response received.");
                }

                if let Some(status) = status_weak.upgrade() {
                    status.set_text("Ready");
                }
            });
        }
    };

    // Connect initial screenshot turn stream
    attach_streaming_receiver(initial_token_rx, initial_ai_label);

    // --- Interactive Chat Input Bar + Resize Corner ---
    let bottom_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    bottom_row.add_css_class("chat-input-bar");

    let entry = gtk4::Entry::new();
    entry.add_css_class("chat-entry");
    entry.set_placeholder_text(Some("Ask a follow-up question..."));
    entry.set_hexpand(true);

    let send_btn = gtk4::Button::with_label("Send");
    send_btn.add_css_class("chat-send-btn");

    // Bottom-right resize handle
    let resize_handle = gtk4::Image::from_icon_name("view-fullscreen-symbolic");
    resize_handle.add_css_class("resize-handle");
    resize_handle.set_tooltip_text(Some("Drag to resize window"));

    // --- Resize Drag Logic ---
    let resize_gesture = gtk4::GestureDrag::new();
    let card_clone = card.clone();
    let w_store = current_width.clone();
    let h_store = current_height.clone();
    let start_w = Rc::new(RefCell::new(0));
    let start_h = Rc::new(RefCell::new(0));

    let sw = start_w.clone();
    let sh = start_h.clone();
    let w_val = w_store.clone();
    let h_val = h_store.clone();
    resize_gesture.connect_drag_begin(move |_, _, _| {
        *sw.borrow_mut() = w_val.load(Ordering::Relaxed);
        *sh.borrow_mut() = h_val.load(Ordering::Relaxed);
    });

    let sw2 = start_w.clone();
    let sh2 = start_h.clone();
    resize_gesture.connect_drag_update(move |_gesture, offset_x, offset_y| {
        let new_w = (*sw2.borrow() + offset_x.round() as i32).max(340);
        let new_h = (*sh2.borrow() + offset_y.round() as i32).max(240);
        w_store.store(new_w, Ordering::Relaxed);
        h_store.store(new_h, Ordering::Relaxed);

        card_clone.set_size_request(new_w, new_h);
    });
    resize_handle.add_controller(resize_gesture);

    bottom_row.append(&entry);
    bottom_row.append(&send_btn);
    bottom_row.append(&resize_handle);
    card.append(&bottom_row);

    window.set_child(Some(&card));

    // Handle User Submitting a Follow-Up Turn
    let on_submit_entry = entry.clone();
    let on_submit_msg_box = messages_box.clone();
    let on_submit_session = session.clone();
    let on_submit_rt = tokio_rt.clone();
    let on_submit_attach = attach_streaming_receiver.clone();

    let submit_action = move || {
        let text = on_submit_entry.text().to_string();
        if text.trim().is_empty() {
            return;
        }

        on_submit_entry.set_text("");

        // 1. Add user message bubble
        let user_bubble = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        user_bubble.add_css_class("chat-bubble-user");

        let user_label = gtk4::Label::new(Some(&text));
        user_label.set_wrap(true);
        user_label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        user_label.set_selectable(true);
        user_label.set_xalign(1.0);
        user_bubble.append(&user_label);
        on_submit_msg_box.append(&user_bubble);

        // 2. Add assistant response placeholder bubble
        let ai_bubble = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
        ai_bubble.add_css_class("chat-bubble-ai");

        let ai_label = gtk4::Label::new(Some("..."));
        ai_label.set_wrap(true);
        ai_label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        ai_label.set_selectable(true);
        ai_label.set_xalign(0.0);
        ai_bubble.append(&ai_label);
        on_submit_msg_box.append(&ai_bubble);

        // 3. Dispatch follow-up to Gemini over Tokio session
        let (token_tx, token_rx) = mpsc::channel::<String>(64);
        on_submit_attach(token_rx, ai_label);
        let error_tx = token_tx.clone();

        let session_clone = on_submit_session.clone();
        on_submit_rt.spawn(async move {
            session_clone.add_user_follow_up(&text).await;
            if let Err(e) = session_clone.stream_next_turn(token_tx).await {
                error!("Error streaming follow up: {}", e);
                let _ = error_tx.send(format!("[Error: {e}]")).await;
            }
        });
    };

    let submit_clone1 = submit_action.clone();
    entry.connect_activate(move |_| {
        submit_clone1();
    });

    send_btn.connect_clicked(move |_| {
        submit_action();
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

/// Convert supported Markdown to valid Pango markup, including while a code fence is streaming.
fn markdown_to_pango(input: &str) -> String {
    let escaped = glib::markup_escape_text(input);
    let mut out = String::new();

    let mut in_code_block = false;
    for line in escaped.lines() {
        if line.starts_with("```") {
            in_code_block = !in_code_block;
            if in_code_block {
                out.push_str("<tt>");
            } else {
                out.push_str("</tt>\n");
            }
            continue;
        }

        if in_code_block {
            out.push_str(line);
            out.push('\n');
            continue;
        }

        let mut line_str = line.to_string();

        // Bold: **text**
        while let Some(start) = line_str.find("**") {
            if let Some(end) = line_str[start + 2..].find("**") {
                let actual_end = start + 2 + end;
                let bold_text = &line_str[start + 2..actual_end];
                line_str = format!(
                    "{}<b>{}</b>{}",
                    &line_str[..start],
                    bold_text,
                    &line_str[actual_end + 2..]
                );
            } else {
                break;
            }
        }

        // Inline code: `text`
        while let Some(start) = line_str.find('`') {
            if let Some(end) = line_str[start + 1..].find('`') {
                let actual_end = start + 1 + end;
                let code_text = &line_str[start + 1..actual_end];
                line_str = format!(
                    "{}<tt>{}</tt>{}",
                    &line_str[..start],
                    code_text,
                    &line_str[actual_end + 1..]
                );
            } else {
                break;
            }
        }

        out.push_str(&line_str);
        out.push('\n');
    }

    if in_code_block {
        out.push_str("</tt>");
    }

    out
}

#[cfg(test)]
mod tests {
    use super::markdown_to_pango;

    #[test]
    fn closes_an_incomplete_streaming_code_fence() {
        assert_eq!(
            markdown_to_pango("### Transcription\n\n```text\ncargo check"),
            "### Transcription\n\n<tt>cargo check\n</tt>"
        );
    }

    #[test]
    fn escapes_markup_inside_code_blocks() {
        assert_eq!(
            markdown_to_pango("```\n<widget>\n```"),
            "<tt>&lt;widget&gt;\n</tt>\n"
        );
    }
}
