use anyhow::{Context, Result, anyhow};
use gtk4::prelude::*;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const SERVICE_NAME: &str = "magicpointerv2";
const LEGACY_ACCOUNT_NAME: &str = "gemini_api_key";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ApiKeyRecord {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
pub struct SpatialSettings {
    pub enabled: bool,
    pub attach_hardware_info: bool,
    pub attach_active_window_title: bool,
    pub custom_text: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct AppSettings {
    pub api_keys: Vec<ApiKeyRecord>,
    pub primary_api_key_id: Option<String>,
    pub font_family: String,
    pub font_size: f64,
    pub overlay_opacity: f64,
    pub background_color: String,
    pub custom_css_path: String,
    pub spatial: SpatialSettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            api_keys: Vec::new(),
            primary_api_key_id: None,
            font_family: "system-ui".to_string(),
            font_size: 13.0,
            overlay_opacity: 0.96,
            background_color: String::new(),
            custom_css_path: String::new(),
            spatial: SpatialSettings::default(),
        }
    }
}

impl AppSettings {
    pub fn load() -> Result<Self> {
        let path = settings_path()?;
        if path.exists() {
            let contents = fs::read_to_string(&path)
                .with_context(|| format!("Could not read settings from {}", path.display()))?;
            let mut settings: Self = serde_json::from_str(&contents)
                .with_context(|| format!("Settings file {} is invalid", path.display()))?;
            settings.normalize();
            return Ok(settings);
        }

        let mut settings = Self::default();
        match get_secret(LEGACY_ACCOUNT_NAME) {
            Ok(Some(api_key)) => {
                let id = "legacy-primary".to_string();
                store_secret(&key_account(&id), &api_key, "Magic Pointer Gemini API key")?;
                settings.api_keys.push(ApiKeyRecord {
                    id: id.clone(),
                    name: "Primary".to_string(),
                });
                settings.primary_api_key_id = Some(id);
                settings.save()?;
            }
            Ok(None) => {}
            Err(error) => {
                tracing::warn!("Could not check for a legacy API key: {error}");
            }
        }
        Ok(settings)
    }

    pub fn save(&self) -> Result<()> {
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

        let path = settings_path()?;
        let directory = path
            .parent()
            .ok_or_else(|| anyhow!("Settings path has no parent directory"))?;
        fs::create_dir_all(directory)
            .with_context(|| format!("Could not create {}", directory.display()))?;

        let temporary_path = path.with_extension(format!("json.{}.tmp", std::process::id()));
        let contents = serde_json::to_vec_pretty(self)?;
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&temporary_path)
            .with_context(|| format!("Could not write {}", temporary_path.display()))?;
        file.set_permissions(fs::Permissions::from_mode(0o600))?;
        file.write_all(&contents)?;
        file.sync_all()?;
        fs::rename(&temporary_path, &path)
            .with_context(|| format!("Could not update {}", path.display()))?;
        Ok(())
    }

    pub fn add_api_key(&mut self, name: &str, api_key: &str) -> Result<()> {
        let name = name.trim();
        let api_key = api_key.trim();
        if name.is_empty() || api_key.is_empty() {
            return Err(anyhow!("API key name and value are required"));
        }

        let id = next_key_id()?;
        store_secret(
            &key_account(&id),
            api_key,
            &format!("Magic Pointer: {name}"),
        )?;
        let previous_primary = self.primary_api_key_id.clone();
        self.api_keys.push(ApiKeyRecord {
            id: id.clone(),
            name: name.to_string(),
        });
        if self.primary_api_key_id.is_none() {
            self.primary_api_key_id = Some(id.clone());
        }
        if let Err(error) = self.save() {
            self.api_keys.pop();
            self.primary_api_key_id = previous_primary;
            return match clear_secret(&key_account(&id)) {
                Ok(()) => Err(error),
                Err(cleanup_error) => Err(anyhow!(
                    "{error}; additionally, the unsaved API key could not be removed from Secret Service: {cleanup_error}"
                )),
            };
        }
        Ok(())
    }

    pub fn remove_api_key(&mut self, id: &str) -> Result<()> {
        let Some(index) = self.api_keys.iter().position(|key| key.id == id) else {
            return Err(anyhow!("API key was not found"));
        };
        let secret_exists = get_secret(&key_account(id))?.is_some();
        let removed = self.api_keys.remove(index);
        let previous_primary = self.primary_api_key_id.clone();
        if self.primary_api_key_id.as_deref() == Some(id) {
            self.primary_api_key_id = self.api_keys.first().map(|key| key.id.clone());
        }
        if let Err(error) = self.save() {
            self.api_keys.insert(index, removed);
            self.primary_api_key_id = previous_primary;
            return Err(error);
        }
        if !secret_exists {
            return Ok(());
        }
        if let Err(error) = clear_secret(&key_account(id)) {
            self.api_keys.insert(index, removed);
            self.primary_api_key_id = previous_primary;
            return match self.save() {
                Ok(()) => Err(error),
                Err(restore_error) => Err(anyhow!(
                    "{error}; additionally, restoring settings failed: {restore_error}"
                )),
            };
        }
        Ok(())
    }

    pub fn set_primary_api_key(&mut self, id: &str) -> Result<()> {
        if !self.api_keys.iter().any(|key| key.id == id) {
            return Err(anyhow!("Selected primary API key was not found"));
        }
        let previous = self.primary_api_key_id.replace(id.to_string());
        if let Err(error) = self.save() {
            self.primary_api_key_id = previous;
            return Err(error);
        }
        Ok(())
    }

    pub fn rename_api_key(&mut self, id: &str, name: &str) -> Result<()> {
        let name = name.trim();
        if name.is_empty() {
            return Err(anyhow!("API key name cannot be empty"));
        }
        let index = self
            .api_keys
            .iter()
            .position(|key| key.id == id)
            .ok_or_else(|| anyhow!("API key was not found"))?;
        let previous = std::mem::replace(&mut self.api_keys[index].name, name.to_string());
        if let Err(error) = self.save() {
            self.api_keys[index].name = previous;
            return Err(error);
        }
        Ok(())
    }

    pub fn api_keys_in_order(&self) -> Result<Vec<(String, String)>> {
        let mut keys = Vec::with_capacity(self.api_keys.len());
        for record in self.ordered_api_key_records() {
            match get_secret(&key_account(&record.id))? {
                Some(secret) => keys.push((record.name.clone(), secret)),
                None => tracing::warn!(
                    "Saved API key '{}' is missing from Secret Service; skipping it",
                    record.name
                ),
            }
        }
        Ok(keys)
    }

    fn ordered_api_key_records(&self) -> Vec<&ApiKeyRecord> {
        let mut records = self.api_keys.iter().collect::<Vec<_>>();
        records.sort_by_key(|key| key.id != self.primary_api_key_id.as_deref().unwrap_or_default());
        records
    }

    fn normalize(&mut self) {
        self.overlay_opacity = self.overlay_opacity.clamp(0.25, 1.0);
        self.font_size = self.font_size.clamp(10.0, 40.0);
        if !self
            .api_keys
            .iter()
            .any(|key| Some(key.id.as_str()) == self.primary_api_key_id.as_deref())
        {
            self.primary_api_key_id = self.api_keys.first().map(|key| key.id.clone());
        }
    }
}

pub fn settings_path() -> Result<PathBuf> {
    if let Some(config_dir) = std::env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(config_dir)
            .join("magicpointerv2")
            .join("settings.json"));
    }
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("HOME is not set"))?;
    Ok(Path::new(&home)
        .join(".config")
        .join("magicpointerv2")
        .join("settings.json"))
}

fn key_account(id: &str) -> String {
    format!("gemini_api_key_{id}")
}

fn next_key_id() -> Result<String> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .context("System clock is earlier than the Unix epoch")?
        .as_nanos();
    Ok(format!("key-{timestamp}-{}", std::process::id()))
}

fn get_secret(account: &str) -> Result<Option<String>> {
    let output = std::process::Command::new("secret-tool")
        .args(["lookup", "service", SERVICE_NAME, "account", account])
        .output()
        .context("Could not run secret-tool; install libsecret's command-line utilities")?;

    if output.status.success() {
        let secret = String::from_utf8(output.stdout)
            .context("Secret Service returned a key that is not valid UTF-8")?;
        let secret = secret.trim();
        return Ok((!secret.is_empty()).then(|| secret.to_string()));
    }

    let error = String::from_utf8_lossy(&output.stderr);
    if output.status.code() == Some(1) && error.trim().is_empty() {
        Ok(None)
    } else {
        Err(anyhow!("Secret Service lookup failed: {}", error.trim()))
    }
}

fn store_secret(account: &str, secret: &str, label: &str) -> Result<()> {
    use std::io::Write;
    let mut child = std::process::Command::new("secret-tool")
        .args([
            "store",
            &format!("--label={label}"),
            "service",
            SERVICE_NAME,
            "account",
            account,
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("Could not run secret-tool; install libsecret's command-line utilities")?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("Could not write API key to Secret Service"))?;
    stdin.write_all(secret.as_bytes())?;
    drop(stdin);
    let output = child.wait_with_output()?;
    if output.status.success() {
        Ok(())
    } else {
        Err(anyhow!(
            "Could not store API key in Secret Service: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

fn clear_secret(account: &str) -> Result<()> {
    let output = std::process::Command::new("secret-tool")
        .args(["clear", "service", SERVICE_NAME, "account", account])
        .output()
        .context("Could not run secret-tool")?;
    if output.status.success() {
        Ok(())
    } else {
        Err(anyhow!(
            "Could not remove API key from Secret Service: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}

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
        if window.has_css_class("card-window") || window.has_css_class("prompt-overlay") {
            window.set_opacity(settings.overlay_opacity);
        }
    }
    Ok(())
}

fn provider_from_css(css: &str) -> Result<gtk4::CssProvider> {
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

pub fn compose_spatial_prompt(
    settings: &AppSettings,
    active_window_title: Option<&str>,
    prompt: &str,
) -> String {
    if !settings.spatial.enabled {
        return prompt.to_string();
    }

    let mut context = Vec::new();
    if settings.spatial.attach_hardware_info {
        context.push(hardware_summary());
    }
    if settings.spatial.attach_active_window_title
        && let Some(title) = active_window_title.filter(|title| !title.trim().is_empty())
    {
        context.push(format!("Active window: {}", title.trim()));
    }
    if !settings.spatial.custom_text.trim().is_empty() {
        context.push(format!(
            "User-provided context: {}",
            settings.spatial.custom_text.trim()
        ));
    }
    if context.is_empty() {
        return prompt.to_string();
    }

    format!(
        "Additional context:\n{}\n\nUser request:\n{}",
        context.join("\n"),
        prompt.trim()
    )
}

fn hardware_summary() -> String {
    let cpu = fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|contents| {
            contents.lines().find_map(|line| {
                line.split_once(':')
                    .filter(|(key, _)| matches!(key.trim(), "model name" | "Hardware"))
                    .map(|(_, value)| value.trim().to_string())
            })
        })
        .unwrap_or_else(|| "unavailable".to_string());
    let memory = fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|contents| {
            contents.lines().find_map(|line| {
                line.strip_prefix("MemTotal:")
                    .and_then(|value| value.split_whitespace().next())
                    .and_then(|value| value.parse::<u64>().ok())
                    .map(|kilobytes| format!("{} GiB", kilobytes / 1_048_576))
            })
        })
        .unwrap_or_else(|| "unavailable".to_string());

    format!(
        "Hardware: {} {}, CPU: {}, Memory: {}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        cpu,
        memory
    )
}

#[cfg(test)]
mod tests {
    use super::{ApiKeyRecord, AppSettings, compose_spatial_prompt};

    #[test]
    fn spatial_context_is_disabled_by_default() {
        let settings = AppSettings::default();
        assert_eq!(
            compose_spatial_prompt(&settings, Some("Terminal"), "Explain this"),
            "Explain this"
        );
    }

    #[test]
    fn spatial_context_attaches_selected_data_to_each_prompt() {
        let mut settings = AppSettings::default();
        settings.spatial.enabled = true;
        settings.spatial.attach_active_window_title = true;
        settings.spatial.custom_text = "  Project: Magic Pointer  ".to_string();

        let prompt = compose_spatial_prompt(&settings, Some(" Terminal "), "  Explain this  ");
        assert!(prompt.contains("Active window: Terminal"));
        assert!(prompt.contains("User-provided context: Project: Magic Pointer"));
        assert!(prompt.ends_with("User request:\nExplain this"));
    }

    #[test]
    fn spatial_hardware_toggle_adds_local_hardware_summary() {
        let mut settings = AppSettings::default();
        settings.spatial.enabled = true;
        settings.spatial.attach_hardware_info = true;

        let prompt = compose_spatial_prompt(&settings, None, "Question");
        assert!(prompt.contains("Hardware:"));
        assert!(prompt.contains(std::env::consts::ARCH));
        assert!(prompt.ends_with("User request:\nQuestion"));
    }

    #[test]
    fn settings_normalize_invalid_ranges() {
        let mut settings = AppSettings {
            font_size: 100.0,
            overlay_opacity: 0.0,
            ..AppSettings::default()
        };
        settings.normalize();
        assert_eq!(settings.font_size, 40.0);
        assert_eq!(settings.overlay_opacity, 0.25);
    }

    #[test]
    fn primary_api_key_precedes_fallback_keys() {
        let settings = AppSettings {
            api_keys: vec![
                ApiKeyRecord {
                    id: "fallback".to_string(),
                    name: "Fallback".to_string(),
                },
                ApiKeyRecord {
                    id: "primary".to_string(),
                    name: "Primary".to_string(),
                },
            ],
            primary_api_key_id: Some("primary".to_string()),
            ..AppSettings::default()
        };
        let ordered = settings
            .ordered_api_key_records()
            .into_iter()
            .map(|record| record.name.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ordered, ["Primary", "Fallback"]);
    }
}
