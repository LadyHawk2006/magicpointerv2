use std::sync::Arc;

use crate::ai_agent::ConversationSession;
use crate::executor;
use crate::screencapture;
use crate::selection_overlay::{self, Rect};
use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use image::DynamicImage;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
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

    let status_label = gtk4::Label::new(Some("Ready"));
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
    let selection_message = match captured_context.as_deref() {
        Some(context) => format!("Capture ready from {context}. Ask a question to get started."),
        None => "Screenshot attached. Ask a question to get started.".to_string(),
    };
    let selection_label = gtk4::Label::new(Some(&selection_message));
    selection_label.add_css_class("selection-notice-label");
    selection_label.set_wrap(true);
    selection_label.set_xalign(0.0);
    selection_notice.append(&selection_icon);
    selection_notice.append(&selection_label);
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

                    render_markdown(&response, &raw_text);

                    if let Some(status) = status_weak.upgrade() {
                        status.set_text("Streaming...");
                    }

                    // Auto-scroll to bottom
                    if let Some(scr) = scrolled_weak.upgrade() {
                        scroll_to_bottom(&scr);
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

    // --- Interactive Chat Input Bar + Resize Corner ---
    let bottom_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
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
            "Explain the error visible in the attached capture, identify the likely cause, and suggest practical steps to fix it. If no error is visible, say so.",
        ),
        (
            "Summarize",
            "Summarize the selected content concisely. Highlight only the most important points.",
        ),
        (
            "Extract text",
            "Transcribe all legible text from the attached capture, preserving its structure where practical. Mark anything uncertain.",
        ),
        (
            "Describe UI",
            "Describe the visible interface, explain the purpose of its important elements, and suggest useful next steps.",
        ),
        (
            "Translate",
            "Translate the visible text into English. Preserve the original meaning and briefly identify the source language if possible.",
        ),
    ];
    let mut preset_buttons = Vec::with_capacity(prompt_presets.len());
    for (index, (label, prompt)) in prompt_presets.into_iter().enumerate() {
        let button = gtk4::Button::with_label(label);
        button.add_css_class("prompt-preset-btn");
        preset_grid.attach(&button, (index % 3) as i32, (index / 3) as i32, 1, 1);
        preset_buttons.push((button, prompt));
    }

    let capture_btn = gtk4::Button::with_label("Capture");
    capture_btn.add_css_class("capture-btn");

    let send_btn = gtk4::Button::with_label("Send");
    send_btn.add_css_class("chat-send-btn");

    let attachment_status = gtk4::Label::new(Some("1 image attached"));
    attachment_status.add_css_class("attachment-status");

    let attachment_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    attachment_row.add_css_class("attachment-row");
    attachment_row.append(&attachment_status);
    card.append(&attachment_row);
    card.append(&preset_grid);

    bottom_row.append(&entry);
    bottom_row.append(&capture_btn);
    bottom_row.append(&send_btn);
    card.append(&bottom_row);

    window.set_child(Some(&card));

    let capture_app = app.clone();
    let capture_pending_images = pending_images.clone();
    let capture_status = status_label.downgrade();
    let capture_attachment_status = attachment_status.clone();
    capture_btn.connect_clicked(move |_| {
        if let Some(status) = capture_status.upgrade() {
            status.set_text("Select a region...");
        }

        let pending_images = capture_pending_images.clone();
        let status_weak = capture_status.clone();
        let dismiss_status = capture_status.clone();
        let attachment_status = capture_attachment_status.clone();
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
                    attachment_status.set_text(&format!(
                        "{} image{} attached",
                        count,
                        if count == 1 { "" } else { "s" }
                    ));
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

        let user_label = gtk4::Label::new(Some(&text));
        user_label.add_css_class("chat-message-text");
        user_label.set_wrap(true);
        user_label.set_wrap_mode(gtk4::pango::WrapMode::WordChar);
        user_label.set_max_width_chars(64);
        user_label.set_selectable(true);
        user_label.set_xalign(1.0);
        user_bubble.append(&user_label);
        let captured_images = std::mem::take(&mut *on_submit_images.borrow_mut());
        if !captured_images.is_empty() {
            let image_count = gtk4::Label::new(Some(&format!(
                "{} image{} attached",
                captured_images.len(),
                if captured_images.len() == 1 { "" } else { "s" }
            )));
            image_count.add_css_class("message-attachment");
            user_bubble.append(&image_count);
            attachment_status.set_text("No images attached");
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

fn append_code_block(container: &gtk4::Box, code: &str, language: Option<&str>) {
    let code_card = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    code_card.add_css_class("code-card");
    code_card.set_hexpand(true);

    let toolbar = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    toolbar.add_css_class("code-toolbar");

    let language_label = gtk4::Label::new(Some(
        language
            .filter(|language| !language.is_empty())
            .unwrap_or("CODE"),
    ));
    language_label.add_css_class("code-language");
    language_label.set_xalign(0.0);
    language_label.set_hexpand(true);

    let copy_button = gtk4::Button::from_icon_name("edit-copy-symbolic");
    copy_button.add_css_class("code-copy-btn");
    copy_button.set_tooltip_text(Some("Copy code"));
    let copied_code = code.to_string();
    copy_button.connect_clicked(
        move |button| match executor::copy_to_clipboard(&copied_code) {
            Ok(()) => {
                button.set_icon_name("object-select-symbolic");
                button.set_tooltip_text(Some("Code copied"));
            }
            Err(error) => {
                error!("Code clipboard copy failed: {error:#}");
                button.set_icon_name("dialog-warning-symbolic");
                button.set_tooltip_text(Some(&error.to_string()));
            }
        },
    );

    toolbar.append(&language_label);
    toolbar.append(&copy_button);
    code_card.append(&toolbar);

    let source = gtk4::Label::new(None);
    source.add_css_class("code-block-source");
    source.set_markup(&highlight_code_markup(code, language));
    source.set_wrap(true);
    source.set_wrap_mode(gtk4::pango::WrapMode::Char);
    source.set_selectable(true);
    source.set_xalign(0.0);
    source.set_hexpand(true);
    code_card.append(&source);
    container.append(&code_card);
}

fn highlight_code_markup(code: &str, language: Option<&str>) -> String {
    const KEYWORD_COLOR: &str = "#c792ea";
    const STRING_COLOR: &str = "#c3e88d";
    const COMMENT_COLOR: &str = "#7f8c9f";
    const NUMBER_COLOR: &str = "#f78c6c";

    let hash_comments = language.is_some_and(|language| {
        [
            "bash", "sh", "shell", "python", "py", "yaml", "yml", "toml", "ruby", "rb", "makefile",
            "r",
        ]
        .contains(&language.to_ascii_lowercase().as_str())
    });
    let chars: Vec<char> = code.chars().collect();
    let mut markup = String::with_capacity(code.len() + 64);
    let mut index = 0;
    while index < chars.len() {
        let start = index;
        let color = if chars[index] == '/'
            && chars
                .get(index + 1)
                .is_some_and(|next| *next == '/' || *next == '*')
        {
            let block_comment = chars[index + 1] == '*';
            index += 2;
            if block_comment {
                while index + 1 < chars.len() && !(chars[index] == '*' && chars[index + 1] == '/') {
                    index += 1;
                }
                index = (index + 2).min(chars.len());
            } else {
                while index < chars.len() && chars[index] != '\n' {
                    index += 1;
                }
            }
            Some(COMMENT_COLOR)
        } else if hash_comments && chars[index] == '#' {
            index += 1;
            while index < chars.len() && chars[index] != '\n' {
                index += 1;
            }
            Some(COMMENT_COLOR)
        } else if matches!(chars[index], '"' | '\'' | '`') {
            let quote = chars[index];
            index += 1;
            while index < chars.len() {
                if chars[index] == '\\' {
                    index = (index + 2).min(chars.len());
                } else if chars[index] == quote {
                    index += 1;
                    break;
                } else {
                    index += 1;
                }
            }
            Some(STRING_COLOR)
        } else if chars[index].is_ascii_digit() {
            index += 1;
            while index < chars.len()
                && (chars[index].is_ascii_alphanumeric() || matches!(chars[index], '.' | '_'))
            {
                index += 1;
            }
            Some(NUMBER_COLOR)
        } else if chars[index].is_ascii_alphabetic() || chars[index] == '_' {
            index += 1;
            while index < chars.len()
                && (chars[index].is_ascii_alphanumeric() || chars[index] == '_')
            {
                index += 1;
            }
            let word: String = chars[start..index].iter().collect();
            if [
                "as", "async", "await", "break", "const", "continue", "else", "enum", "fn", "for",
                "function", "if", "impl", "import", "in", "let", "match", "mut", "new", "pub",
                "return", "self", "static", "struct", "trait", "use", "var", "while", "class",
                "def", "from", "print", "try", "except", "true", "false", "null", "None", "True",
                "False",
            ]
            .contains(&word.as_str())
            {
                Some(KEYWORD_COLOR)
            } else {
                None
            }
        } else {
            index += 1;
            None
        };

        let token: String = chars[start..index].iter().collect();
        let escaped = glib::markup_escape_text(&token);
        if let Some(color) = color {
            markup.push_str(&format!("<span foreground=\"{color}\">{escaped}</span>"));
        } else {
            markup.push_str(&escaped);
        }
    }

    markup
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
    let mut code_language = None;

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
        label.set_hexpand(true);
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
            Event::Start(Tag::CodeBlock(kind)) => {
                flush(container, &class, &mut content, is_code_block);
                class = "markdown-code".to_string();
                code_language = match kind {
                    CodeBlockKind::Fenced(info) => info
                        .split_whitespace()
                        .next()
                        .filter(|language| !language.is_empty())
                        .map(str::to_string),
                    CodeBlockKind::Indented => None,
                };
                is_code_block = true;
            }
            Event::End(TagEnd::CodeBlock) => {
                append_code_block(container, &content, code_language.as_deref());
                content.clear();
                is_code_block = false;
                code_language = None;
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
                content.push_str("<span foreground=\"#a5d6ff\"><tt>");
                content.push_str(&glib::markup_escape_text(&code));
                content.push_str("</tt></span>");
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
    use super::{Event, Options, Parser, Tag, highlight_code_markup};

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

    #[test]
    fn code_highlighting_escapes_markup_and_colors_tokens() {
        let markup = highlight_code_markup("let message = \"<hello>\"; // note", Some("rust"));
        assert!(markup.contains("foreground=\"#c792ea\""));
        assert!(markup.contains("foreground=\"#c3e88d\""));
        assert!(markup.contains("foreground=\"#7f8c9f\""));
        assert!(markup.contains("&lt;hello&gt;"));
    }

    #[test]
    fn hash_comments_are_only_highlighted_for_hash_comment_languages() {
        let rust = highlight_code_markup("let value = #tag;", Some("rust"));
        let python = highlight_code_markup("value = 1 # note", Some("python"));
        assert!(!rust.contains("foreground=\"#7f8c9f\""));
        assert!(python.contains("foreground=\"#7f8c9f\""));
    }
}
