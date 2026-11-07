use std::cell::RefCell;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result, anyhow};
use gtk4::prelude::*;

use crate::settings::AppSettings;

thread_local! {
    static APPEARANCE_PROVIDERS: RefCell<Option<(gtk4::CssProvider, Option<gtk4::CssProvider>)>> = const { RefCell::new(None) };
}

pub fn apply_appearance(app: &gtk4::Application, settings: &AppSettings) -> Result<()> {
    let Some(display) = gtk4::gdk::Display::default() else {
        return Err(anyhow!("No display available while applying settings"));
    };

    let font_family = settings
        .font_family
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace(['\n', '\r'], " ");
    let mut css = format!(
        ".markdown-content, .chat-message-text, .markdown-paragraph, .markdown-list-item, .markdown-heading, .markdown-h1, .markdown-h2, .markdown-h3, .markdown-h4, .markdown-h5, .markdown-h6 {{ font-family: \"{font_family}\"; font-size: {}pt; }} .chat-entry, .chat-entry placeholder {{ font-family: \"{font_family}\"; }}",
        settings.font_size
    );
    if !settings.background_color.trim().is_empty() {
        let color = settings.background_color.trim();
        gtk4::gdk::RGBA::parse(color)
            .map_err(|error| anyhow!("Invalid background color: {error}"))?;
        css.push_str(&format!(
            "window.background.card-window, .magic-card, .chat-history, .chat-history viewport {{ background-color: {color}; }}"
        ));
    }

    let appearance = provider_from_css(&css)?;

    let custom_css = if settings.custom_css_path.trim().is_empty() {
        None
    } else {
        let path = Path::new(settings.custom_css_path.trim());
        let css = fs::read_to_string(path)
            .with_context(|| format!("Could not read custom GTK CSS from {}", path.display()))?;
        Some(provider_from_css(&css).with_context(|| {
            format!(
                "Custom GTK CSS at {} contains invalid syntax",
                path.display()
            )
        })?)
    };

    APPEARANCE_PROVIDERS.with(|providers| {
        if let Some((old_appearance, old_custom)) = providers.borrow_mut().take() {
            gtk4::style_context_remove_provider_for_display(&display, &old_appearance);
            if let Some(old_custom) = old_custom {
                gtk4::style_context_remove_provider_for_display(&display, &old_custom);
            }
        }
        gtk4::style_context_add_provider_for_display(
            &display,
            &appearance,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION + 1,
        );
        if let Some(provider) = custom_css.as_ref() {
            gtk4::style_context_add_provider_for_display(
                &display,
                provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION + 2,
            );
        }
        *providers.borrow_mut() = Some((appearance, custom_css));
    });

    for window in app.windows() {
        if window.has_css_class("prompt-overlay") {
            window.set_opacity(settings.overlay_opacity);
        }
    }
    Ok(())
}

pub(crate) fn provider_from_css(css: &str) -> Result<gtk4::CssProvider> {
    let provider = gtk4::CssProvider::new();
    let parse_error = std::rc::Rc::new(RefCell::new(None::<String>));
    let parse_error_handler = parse_error.clone();
    provider.connect_parsing_error(move |_, section, error| {
        *parse_error_handler.borrow_mut() = Some(format!(
            "{} at {}:{}",
            error,
            section.start_location().lines(),
            section.start_location().chars()
        ));
    });
    provider.load_from_data(css);
    if let Some(error) = parse_error.borrow().as_ref() {
        return Err(anyhow!("{error}"));
    }
    Ok(provider)
}
