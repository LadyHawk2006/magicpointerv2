use std::sync::Arc;

use crate::ai_agent::ConversationSession;
use crate::executor;
use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use pulldown_cmark::{Event, Options, Parser, Tag, TagEnd};
use std::cell::RefCell;
use std::rc::Rc;
use tokio::sync::mpsc;
use tracing::error;

pub fn build_floating_card(
    app: &gtk4::Application,
    captured_context: Option<String>,
    initial_token_rx: mpsc::Receiver<String>,
    session: ConversationSession,
    tokio_rt: Arc<tokio::runtime::Runtime>,
) -> gtk4::ApplicationWindow {
    let window = gtk4::ApplicationWindow::new(app);
    window.set_title(Some("Magic Pointer"));
    window.set_default_size(520, 460);

    window.add_css_class("card-window");

    // Outer card
    let card = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    card.add_css_class("magic-card");
    card.set_hexpand(true);
    card.set_vexpand(true);

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

    let initial_response = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    initial_response.add_css_class("markdown-content");
    let placeholder = gtk4::Label::new(Some("Analyzing screen capture..."));
    placeholder.add_css_class("markdown-paragraph");
    placeholder.set_wrap(true);
    placeholder.set_xalign(0.0);
    initial_response.append(&placeholder);
    initial_bubble.append(&initial_response);
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

                    render_markdown(&response, &raw_text);

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
                    render_markdown(&response, "No response received.");
                }

                if let Some(status) = status_weak.upgrade() {
                    status.set_text("Ready");
                }
            });
        }
    };

    // Connect initial screenshot turn stream
    attach_streaming_receiver(initial_token_rx, initial_response);

    // --- Interactive Chat Input Bar + Resize Corner ---
    let bottom_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    bottom_row.add_css_class("chat-input-bar");

    let entry = gtk4::Entry::new();
    entry.add_css_class("chat-entry");
    entry.set_placeholder_text(Some("Ask a follow-up question..."));
    entry.set_hexpand(true);

    let send_btn = gtk4::Button::with_label("Send");
    send_btn.add_css_class("chat-send-btn");

    bottom_row.append(&entry);
    bottom_row.append(&send_btn);
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

        let ai_response = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
        ai_response.add_css_class("markdown-content");
        ai_bubble.append(&ai_response);
        on_submit_msg_box.append(&ai_bubble);

        // 3. Dispatch follow-up to Gemini over Tokio session
        let (token_tx, token_rx) = mpsc::channel::<String>(64);
        on_submit_attach(token_rx, ai_response);
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

fn render_markdown(container: &gtk4::Box, markdown: &str) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }

    let mut class = "markdown-paragraph".to_string();
    let mut content = String::new();
    let mut is_code_block = false;
    let mut quote_depth = 0usize;
    let mut inline_tags = Vec::new();
    let mut list_depth = 0usize;

    let flush = |container: &gtk4::Box, class: &str, content: &mut String, is_code_block: bool| {
        if content.is_empty() {
            return;
        }

        let label = gtk4::Label::new(None);
        label.add_css_class(class);
        label.set_wrap(true);
        label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        label.set_selectable(true);
        label.set_xalign(0.0);
        if is_code_block {
            label.set_text(content);
        } else {
            label.set_markup(content);
        }
        container.append(&label);
        content.clear();
    };

    for event in Parser::new_ext(markdown, Options::all()) {
        match event {
            Event::Start(Tag::Paragraph) => {
                if content.is_empty() {
                    class = if quote_depth > 0 {
                        "markdown-blockquote".to_string()
                    } else {
                        "markdown-paragraph".to_string()
                    };
                }
            }
            Event::End(TagEnd::Paragraph) => {
                flush(container, &class, &mut content, is_code_block);
            }
            Event::Start(Tag::Heading { level, .. }) => {
                flush(container, &class, &mut content, is_code_block);
                class = format!("markdown-heading markdown-h{}", level as u8);
            }
            Event::End(TagEnd::Heading(_)) => {
                flush(container, &class, &mut content, false);
            }
            Event::Start(Tag::CodeBlock(_)) => {
                flush(container, &class, &mut content, is_code_block);
                class = "markdown-code".to_string();
                is_code_block = true;
            }
            Event::End(TagEnd::CodeBlock) => {
                flush(container, &class, &mut content, true);
                is_code_block = false;
            }
            Event::Start(Tag::BlockQuote(_)) => quote_depth += 1,
            Event::End(TagEnd::BlockQuote(_)) => {
                quote_depth = quote_depth.saturating_sub(1);
            }
            Event::Start(Tag::List(_)) => list_depth += 1,
            Event::End(TagEnd::List(_)) => list_depth = list_depth.saturating_sub(1),
            Event::Start(Tag::Item) => {
                flush(container, &class, &mut content, is_code_block);
                class = "markdown-list-item".to_string();
                content.push_str(&format!("{}• ", "  ".repeat(list_depth.saturating_sub(1))));
            }
            Event::End(TagEnd::Item) => {
                flush(container, &class, &mut content, false);
            }
            Event::Start(Tag::Emphasis) => {
                content.push_str("<i>");
                inline_tags.push("</i>");
            }
            Event::Start(Tag::Strong) => {
                content.push_str("<b>");
                inline_tags.push("</b>");
            }
            Event::Start(Tag::Strikethrough) => {
                content.push_str("<s>");
                inline_tags.push("</s>");
            }
            Event::Start(Tag::Link { dest_url, .. }) => {
                content.push_str("<a href=\"");
                content.push_str(&glib::markup_escape_text(&dest_url));
                content.push_str("\">");
                inline_tags.push("</a>");
            }
            Event::End(
                TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough | TagEnd::Link,
            ) => {
                if let Some(tag) = inline_tags.pop() {
                    content.push_str(tag);
                }
            }
            Event::Text(text) | Event::Html(text) | Event::InlineHtml(text) => {
                if is_code_block {
                    content.push_str(&text);
                } else {
                    content.push_str(&glib::markup_escape_text(&text));
                }
            }
            Event::Code(code) => {
                content.push_str("<tt>");
                content.push_str(&glib::markup_escape_text(&code));
                content.push_str("</tt>");
            }
            Event::SoftBreak => content.push(' '),
            Event::HardBreak => content.push('\n'),
            Event::Rule => {
                flush(container, &class, &mut content, is_code_block);
                let rule = gtk4::Label::new(Some(" "));
                rule.add_css_class("markdown-rule");
                container.append(&rule);
            }
            Event::TaskListMarker(checked) => {
                content.push_str(if checked { "[x] " } else { "[ ] " });
            }
            Event::FootnoteReference(label) => {
                content.push_str(&glib::markup_escape_text(&format!("[^{label}]")));
            }
            Event::DisplayMath(math) | Event::InlineMath(math) => {
                content.push_str(&glib::markup_escape_text(&math));
            }
            Event::Start(_) | Event::End(_) => {}
        }
    }

    flush(container, &class, &mut content, is_code_block);
}

#[cfg(test)]
mod tests {
    use super::{Event, Options, Parser, Tag};

    #[test]
    fn pulldown_cmark_recognizes_markdown_structures() {
        let events: Vec<_> = Parser::new_ext(
            "## Heading\n\nA **bold** phrase and `code`.\n\n```rust\nfn main() {}\n```",
            Options::all(),
        )
        .collect();

        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::Start(Tag::Heading { .. })))
        );
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::Start(Tag::Strong)))
        );
        assert!(events.iter().any(|event| matches!(event, Event::Code(_))));
        assert!(
            events
                .iter()
                .any(|event| matches!(event, Event::Start(Tag::CodeBlock(_))))
        );
    }

    #[test]
    fn escapes_html_in_markdown_as_text() {
        let events: Vec<_> = Parser::new_ext("<widget>", Options::all()).collect();
        assert!(events.iter().any(|event| {
            matches!(event, Event::Html(text) | Event::InlineHtml(text) if text.as_ref() == "<widget>")
        }));
    }
}
