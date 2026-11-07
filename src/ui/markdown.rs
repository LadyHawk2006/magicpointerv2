use crate::executor;
use gtk4::glib;
use gtk4::prelude::*;
use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use tracing::error;

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
    let hash_comments = language.is_some_and(|language| {
        [
            "bash", "lua", "sh", "shell", "python", "py", "yaml", "yml", "toml", "ruby", "rb",
            "makefile", "r", "perl", "pl", "elisp", "lisp", "scheme", "clojure", "nim", "nix",
            "julia",
        ]
        .contains(&language.to_ascii_lowercase().as_str())
    });
    let chars: Vec<char> = code.chars().collect();
    let mut markup = String::with_capacity(code.len() + 64);
    let mut index = 0;
    while index < chars.len() {
        let start = index;
        let style_tag = if chars[index] == '/'
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
            Some("span style=\"italic\"")
        } else if hash_comments && chars[index] == '#' {
            index += 1;
            while index < chars.len() && chars[index] != '\n' {
                index += 1;
            }
            Some("span style=\"italic\"")
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
            None
        } else if chars[index].is_ascii_digit() {
            index += 1;
            while index < chars.len()
                && (chars[index].is_ascii_alphanumeric() || matches!(chars[index], '.' | '_'))
            {
                index += 1;
            }
            None
        } else if chars[index].is_ascii_alphabetic() || chars[index] == '_' {
            index += 1;
            while index < chars.len()
                && (chars[index].is_ascii_alphanumeric() || chars[index] == '_')
            {
                index += 1;
            }
            let word: String = chars[start..index].iter().collect();
            let keywords = [
                "as", "async", "await", "break", "const", "continue", "else", "enum", "fn", "for",
                "function", "if", "impl", "import", "in", "let", "match", "mut", "new", "pub",
                "return", "self", "static", "struct", "trait", "use", "var", "while", "class",
                "def", "from", "print", "try", "except",
            ];
            let booleans = ["true", "false", "null", "None", "True", "False"];

            if keywords.contains(&word.as_str()) {
                Some("span weight=\"bold\"")
            } else if booleans.contains(&word.as_str()) {
                Some("span weight=\"bold\"")
            } else {
                None
            }
        } else {
            index += 1;
            None
        };

        let token: String = chars[start..index].iter().collect();
        let escaped = glib::markup_escape_text(&token);
        if let Some(style) = style_tag {
            if style.starts_with("span ") {
                markup.push_str(&format!("<{style}>{escaped}</span>"));
            } else {
                markup.push_str(&format!("<{style}>{escaped}</{style}>"));
            }
        } else {
            markup.push_str(&escaped);
        }
    }

    markup
}

pub fn render_markdown(container: &gtk4::Box, markdown: &str) {
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
    fn code_styling_escapes_markup_and_marks_keywords_and_comments() {
        let markup = highlight_code_markup("let message = \"<hello>\"; // note", Some("rust"));
        assert!(markup.contains("<span weight=\"bold\">let</span>"));
        assert!(markup.contains("&quot;&lt;hello&gt;&quot;"));
        assert!(markup.contains("<span style=\"italic\">// note</span>"));
        assert!(markup.contains("&lt;hello&gt;"));
    }

    #[test]
    fn hash_comments_are_only_highlighted_for_hash_comment_languages() {
        let rust = highlight_code_markup("let value = #tag;", Some("rust"));
        let python = highlight_code_markup("value = 1 # note", Some("python"));
        assert!(!rust.contains("<span style=\"italic\">#tag;</span>"));
        assert!(python.contains("<span style=\"italic\"># note</span>"));
    }
}
