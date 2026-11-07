use std::cell::RefCell;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, anyhow};
use gtk4::prelude::*;

use crate::settings::AppSettings;
use crate::styles::APP_CSS;

const CUSTOM_CSS_TEMPLATE: &str = include_str!("../styles/custom.css.template");
const EXAMPLE_CSS: &str = include_str!("../styles/examplecss.css");

thread_local! {
    static APPEARANCE_PROVIDERS: RefCell<Option<(gtk4::CssProvider, Option<gtk4::CssProvider>)>> = const { RefCell::new(None) };
}

pub fn apply_appearance(app: &gtk4::Application, settings: &AppSettings) -> Result<()> {
    let Some(display) = gtk4::gdk::Display::default() else {
        return Err(anyhow!("No display available while applying settings"));
    };

    let css = active_appearance_css(settings)?;

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

pub fn export_css_template(settings: &AppSettings) -> Result<String> {
    let active_appearance = active_appearance_css(settings)?;
    let custom_css = if settings.custom_css_path.trim().is_empty() {
        String::new()
    } else {
        let path = Path::new(settings.custom_css_path.trim());
        fs::read_to_string(path)
            .with_context(|| format!("Could not read custom GTK CSS from {}", path.display()))?
    };

    Ok(CUSTOM_CSS_TEMPLATE
        .replace("{{BUILT_IN_STYLES}}", &effective_builtin_css())
        .replace("{{ACTIVE_APPEARANCE}}", &active_appearance)
        .replace("{{EXISTING_CUSTOM_STYLES}}", &custom_css))
}

pub fn export_css_files(settings: &AppSettings) -> Result<(String, String)> {
    Ok((export_css_template(settings)?, EXAMPLE_CSS.to_string()))
}

pub fn write_css_export(directory: &Path, settings: &AppSettings) -> Result<(PathBuf, PathBuf)> {
    let (template, example) = export_css_files(settings)?;
    let template_path = directory.join("magicpointerv2-custom.css");
    let example_path = directory.join("examplecss.css");

    write_new_css_file(&template_path, template.as_bytes())?;
    if let Err(error) = write_new_css_file(&example_path, example.as_bytes()) {
        fs::remove_file(&template_path).with_context(|| {
            format!(
                "Could not clean up partial CSS export {} after the example file failed",
                template_path.display()
            )
        })?;
        return Err(error);
    }

    Ok((template_path, example_path))
}

fn write_new_css_file(path: &Path, contents: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .with_context(|| format!("Could not create {} without overwriting", path.display()))?;
    if let Err(error) = file.write_all(contents) {
        drop(file);
        fs::remove_file(path).with_context(|| {
            format!(
                "Could not remove incomplete CSS export {} after write failure",
                path.display()
            )
        })?;
        return Err(error).with_context(|| format!("Could not write {}", path.display()));
    }
    Ok(())
}

fn effective_builtin_css() -> String {
    APP_CSS
        .lines()
        .filter(|line| {
            let declaration = line.trim();
            !declaration.starts_with("overflow:") && !declaration.starts_with("cursor:")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn active_appearance_css(settings: &AppSettings) -> Result<String> {
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
    Ok(css)
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

#[cfg(test)]
mod tests {
    use super::{export_css_files, export_css_template, provider_from_css};
    use crate::settings::AppSettings;

    #[test]
    fn exported_template_contains_effective_styles_and_active_appearance() {
        let mut settings = AppSettings::default();
        settings.font_family = "Inter".to_string();
        settings.font_size = 19.0;
        settings.background_color = "#202124".to_string();

        let template = export_css_template(&settings).unwrap();
        assert!(template.contains("exact built-in CSS bundled by this"));
        assert!(template.contains(".prompt-card"));
        assert!(template.contains(".settings-expander > title"));
        assert!(template.contains("@define-color mp-accent"));
        assert!(template.contains("font-family: \"Inter\"; font-size: 19pt"));
        assert!(template.contains("background-color: #202124"));
        assert!(!template.contains("{{BUILT_IN_STYLES}}"));
        assert!(!template.contains("overflow:"));
        assert!(!template.contains("cursor:"));
    }

    #[test]
    fn exported_template_includes_selected_custom_css() {
        let path = std::env::temp_dir().join(format!(
            "magicpointerv2-custom-css-{}.css",
            std::process::id()
        ));
        let custom_css = ".magic-card { border-radius: 17px; }";
        std::fs::write(&path, custom_css).unwrap();
        let settings = AppSettings {
            custom_css_path: path.to_string_lossy().to_string(),
            ..AppSettings::default()
        };

        let template = export_css_template(&settings).unwrap();
        std::fs::remove_file(path).unwrap();

        assert!(template.contains(custom_css));
    }

    #[test]
    fn export_bundle_contains_a_ready_to_load_example_theme() {
        let (_, example_css) = export_css_files(&AppSettings::default()).unwrap();

        assert!(example_css.contains("Aurora Night"));
        assert!(example_css.contains(".prompt-card"));
        assert!(example_css.contains(".magic-card"));
        assert!(example_css.contains("@define-color mp-example-accent"));
        assert!(!example_css.contains("{{"));
    }

    #[test]
    fn export_writes_both_named_files_without_overwriting_existing_files() {
        let directory = std::env::temp_dir().join(format!(
            "magicpointerv2-css-export-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();

        let (template_path, example_path) =
            super::write_css_export(&directory, &AppSettings::default()).unwrap();

        assert_eq!(
            template_path.file_name().unwrap(),
            "magicpointerv2-custom.css"
        );
        assert_eq!(example_path.file_name().unwrap(), "examplecss.css");
        assert!(
            std::fs::read_to_string(&template_path)
                .unwrap()
                .contains(".prompt-card")
        );
        assert!(
            std::fs::read_to_string(&example_path)
                .unwrap()
                .contains("Aurora Night")
        );
        assert!(super::write_css_export(&directory, &AppSettings::default()).is_err());

        std::fs::remove_dir_all(directory).unwrap();
    }

    #[test]
    fn exported_template_css_is_accepted_by_gtk_when_a_display_is_available() {
        if gtk4::init().is_err() {
            return;
        }

        let (template, example_css) = export_css_files(&AppSettings::default()).unwrap();
        assert!(provider_from_css(&template).is_ok());
        assert!(provider_from_css(&example_css).is_ok());
    }
}
