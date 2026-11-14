use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub mod appearance;
pub mod context;
pub mod hyprland;
pub mod ui;

pub use appearance::apply_appearance;
pub use context::compose_spatial_prompt;

const SERVICE_NAME: &str = "magicpointerv2";
const LEGACY_ACCOUNT_NAME: &str = "gemini_api_key";
const MODEL_CATALOG: &str = include_str!("../../models.txt");

pub const DEFAULT_MODEL_ORDER: &[&str] = &[
    "gemini-3.1-flash-lite",
    "gemini-flash-lite-latest",
    "gemini-3.5-flash-lite",
    "gemini-2.5-flash-lite",
    "gemini-flash-latest",
    "gemini-3.8-flash",
    "gemini-3.7-flash",
    "gemini-3.6-flash",
    "gemini-3.5-flash",
    "gemini-2.5-flash",
];

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
    pub attach_workspace_windows: bool,
    pub custom_text: String,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum WindowPosition {
    Center,
    TopLeft,
    TopRight,
    BottomLeft,
    #[default]
    BottomRight,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct HyprlandWindowSettings {
    pub width: u32,
    pub height: u32,
    pub position: WindowPosition,
    pub show_border: bool,
    pub border_size: u32,
    pub show_shadow: bool,
    pub blur: bool,
    pub glow: bool,
    pub opacity_percent: u32,
}

impl Default for HyprlandWindowSettings {
    fn default() -> Self {
        Self {
            width: 542,
            height: 668,
            position: WindowPosition::default(),
            show_border: false,
            border_size: 2,
            show_shadow: false,
            blur: true,
            glow: false,
            opacity_percent: 95,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default)]
pub struct AppSettings {
    pub api_keys: Vec<ApiKeyRecord>,
    pub primary_api_key_id: Option<String>,
    pub model_order: Vec<String>,
    pub max_output_tokens: u32,
    pub temperature: f64,
    pub connection_timeout_secs: u64,
    pub response_timeout_secs: u64,
    pub stream_idle_timeout_secs: u64,
    pub font_family: String,
    pub font_size: f64,
    pub overlay_opacity: f64,
    pub background_color: String,
    pub custom_css_path: String,
    pub system_instruction: String,
    pub custom_chips: Vec<(String, String)>,
    pub spatial: SpatialSettings,
    pub hyprland_window: HyprlandWindowSettings,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            api_keys: Vec::new(),
            primary_api_key_id: None,
            model_order: DEFAULT_MODEL_ORDER
                .iter()
                .map(|model| (*model).to_string())
                .collect(),
            max_output_tokens: 2048,
            temperature: 0.2,
            connection_timeout_secs: 12,
            response_timeout_secs: 20,
            stream_idle_timeout_secs: 20,
            font_family: "system-ui".to_string(),
            font_size: 13.0,
            overlay_opacity: 0.96,
            background_color: String::new(),
            custom_css_path: String::new(),
            system_instruction: String::new(),
            custom_chips: Vec::new(),
            spatial: SpatialSettings::default(),
            hyprland_window: HyprlandWindowSettings::default(),
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
        self.temperature = self.temperature.clamp(0.0, 2.0);
        self.max_output_tokens = self.max_output_tokens.clamp(1, 65_536);
        self.connection_timeout_secs = self.connection_timeout_secs.clamp(1, 120);
        self.response_timeout_secs = self.response_timeout_secs.clamp(1, 300);
        self.stream_idle_timeout_secs = self.stream_idle_timeout_secs.clamp(1, 300);
        self.hyprland_window.width = self.hyprland_window.width.clamp(200, 7680);
        self.hyprland_window.height = self.hyprland_window.height.clamp(200, 4320);
        self.hyprland_window.border_size = self.hyprland_window.border_size.clamp(1, 10);
        self.hyprland_window.opacity_percent = self.hyprland_window.opacity_percent.clamp(25, 100);

        let mut seen_models = HashSet::new();
        self.model_order = self
            .model_order
            .drain(..)
            .map(|model| model.trim().to_string())
            .filter(|model| !model.is_empty() && seen_models.insert(model.clone()))
            .collect();
        if self.model_order.is_empty() {
            self.model_order = DEFAULT_MODEL_ORDER
                .iter()
                .map(|model| (*model).to_string())
                .collect();
        }

        if !self
            .api_keys
            .iter()
            .any(|key| Some(key.id.as_str()) == self.primary_api_key_id.as_deref())
        {
            self.primary_api_key_id = self.api_keys.first().map(|key| key.id.clone());
        }

        self.custom_chips
            .retain(|(label, prompt)| !label.trim().is_empty() && !prompt.trim().is_empty());
    }
}

pub fn parse_model_catalog(json_str: &str) -> Result<Vec<(String, String)>> {
    let json_start = json_str
        .find('{')
        .ok_or_else(|| anyhow!("Model list does not contain JSON"))?;
    let catalog: serde_json::Value = serde_json::from_str(&json_str[json_start..])
        .context("Could not parse the model catalog JSON")?;
    let models = catalog
        .get("models")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| anyhow!("Model catalog has no models array"))?;

    let mut available = Vec::new();
    let mut seen = HashSet::new();
    for model in models {
        let supports_generation = model
            .get("supportedGenerationMethods")
            .and_then(serde_json::Value::as_array)
            .is_some_and(|methods| {
                methods
                    .iter()
                    .any(|method| method.as_str() == Some("generateContent"))
            });
        if !supports_generation {
            continue;
        }

        let Some(model_id) = model
            .get("name")
            .and_then(serde_json::Value::as_str)
            .and_then(|name| name.strip_prefix("models/"))
        else {
            continue;
        };
        if !seen.insert(model_id.to_string()) {
            continue;
        }
        let display_name = model
            .get("displayName")
            .and_then(serde_json::Value::as_str)
            .filter(|name| !name.trim().is_empty())
            .unwrap_or(model_id);
        available.push((model_id.to_string(), display_name.to_string()));
    }

    if available.is_empty() {
        return Err(anyhow!(
            "Catalog contains no models supporting generateContent"
        ));
    }
    Ok(available)
}

pub fn available_models() -> Result<Vec<(String, String)>> {
    parse_model_catalog(MODEL_CATALOG)
}

pub async fn fetch_live_models() -> Result<Vec<(String, String)>> {
    let api_keys = crate::auth::get_api_keys()?;
    if api_keys.is_empty() {
        return Err(anyhow!("No Gemini API key is configured."));
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let mut last_error = None;

    for (name, api_key) in api_keys {
        let response = match client
            .get("https://generativelanguage.googleapis.com/v1beta/models")
            .header("x-goog-api-key", &api_key)
            .send()
            .await
        {
            Ok(res) => res,
            Err(err) => {
                last_error = Some(anyhow!("Network error with key '{name}': {err}"));
                continue;
            }
        };

        let status = response.status();
        let text = response.text().await?;
        if status.is_success() {
            return parse_model_catalog(&text);
        }
        last_error = Some(anyhow!("API key '{name}' returned {status}: {text}"));
    }

    Err(last_error.unwrap_or_else(|| anyhow!("All configured Gemini API keys failed.")))
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

pub fn reset_app_data() -> Result<()> {
    let mut failures = Vec::new();

    if let Err(error) = clear_app_secrets() {
        failures.push(format!("Secret Service credentials: {error:#}"));
    }
    if let Err(error) = remove_saved_settings() {
        failures.push(format!("saved settings: {error:#}"));
    }
    if let Err(error) = hyprland::reset() {
        failures.push(format!("Hyprland integration: {error:#}"));
    }

    if failures.is_empty() {
        Ok(())
    } else {
        Err(anyhow!(
            "Reset was incomplete:\n- {}",
            failures.join("\n- ")
        ))
    }
}

fn run_keyring_async<F, T>(future: F) -> Result<T>
where
    F: std::future::Future<Output = Result<T>>,
{
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        tokio::task::block_in_place(|| handle.block_on(future))
    } else {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .context("Could not build Tokio runtime for Secret Service keyring access")?;
        rt.block_on(future)
    }
}

async fn open_keyring() -> Result<oo7::Keyring> {
    let keyring = oo7::Keyring::new()
        .await
        .map_err(|err| anyhow!("Could not connect to Secret Service: {err}"))?;
    if keyring.is_locked().await.unwrap_or(false) {
        if let Err(unlock_err) = keyring.unlock().await {
            tracing::warn!("Failed to unlock keyring automatically: {unlock_err}");
            return Err(anyhow!("Secret Service keyring is locked: {unlock_err}"));
        }
    }
    Ok(keyring)
}

fn clear_app_secrets() -> Result<()> {
    run_keyring_async(async {
        let keyring = open_keyring().await?;
        keyring
            .delete(&[("service", SERVICE_NAME)])
            .await
            .map_err(|err| anyhow!("Secret Service rejected credential removal: {err}"))?;
        Ok(())
    })
}

fn remove_saved_settings() -> Result<()> {
    let path = settings_path()?;
    match fs::remove_file(&path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(error).with_context(|| format!("Could not remove {}", path.display()));
        }
    }

    let Some(directory) = path.parent() else {
        return Err(anyhow!("Settings path has no parent directory"));
    };
    let entries = match fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Could not inspect {}", directory.display()));
        }
    };

    for entry in entries {
        let entry = entry.with_context(|| format!("Could not inspect {}", directory.display()))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with("settings.json.") && name.ends_with(".tmp") {
            fs::remove_file(entry.path())
                .with_context(|| format!("Could not remove {}", entry.path().display()))?;
        }
    }
    match fs::remove_dir(directory) {
        Ok(()) => {}
        Err(error)
            if matches!(
                error.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::DirectoryNotEmpty
            ) => {}
        Err(error) => {
            return Err(error).with_context(|| format!("Could not remove {}", directory.display()));
        }
    }
    Ok(())
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
    run_keyring_async(async {
        let keyring = open_keyring().await?;
        let items = keyring
            .search_items(&[("service", SERVICE_NAME), ("account", account)])
            .await
            .map_err(|err| anyhow!("Secret Service lookup failed: {err}"))?;

        for item in items {
            let secret_bytes = item
                .secret()
                .await
                .map_err(|err| anyhow!("Could not read secret from Secret Service: {err}"))?;
            let secret = String::from_utf8(secret_bytes.to_vec())
                .context("Secret Service returned a key that is not valid UTF-8")?;
            let secret = secret.trim();
            if !secret.is_empty() {
                return Ok(Some(secret.to_string()));
            }
        }
        Ok(None)
    })
}

fn store_secret(account: &str, secret: &str, label: &str) -> Result<()> {
    run_keyring_async(async {
        let keyring = open_keyring().await?;
        keyring
            .create_item(
                label,
                &[("service", SERVICE_NAME), ("account", account)],
                secret.as_bytes(),
                true,
            )
            .await
            .map_err(|err| anyhow!("Could not store API key in Secret Service: {err}"))?;
        Ok(())
    })
}

fn clear_secret(account: &str) -> Result<()> {
    run_keyring_async(async {
        let keyring = open_keyring().await?;
        keyring
            .delete(&[("service", SERVICE_NAME), ("account", account)])
            .await
            .map_err(|err| anyhow!("Could not remove API key from Secret Service: {err}"))?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::{ApiKeyRecord, AppSettings, DEFAULT_MODEL_ORDER, available_models};

    #[test]
    fn settings_normalize_invalid_ranges() {
        let mut settings = AppSettings {
            font_size: 100.0,
            overlay_opacity: 0.0,
            max_output_tokens: 100_000,
            temperature: 3.0,
            connection_timeout_secs: 0,
            response_timeout_secs: 500,
            stream_idle_timeout_secs: 0,
            model_order: vec![
                " gemini-custom ".to_string(),
                "gemini-custom".to_string(),
                " ".to_string(),
            ],
            ..AppSettings::default()
        };
        settings.normalize();
        assert_eq!(settings.font_size, 40.0);
        assert_eq!(settings.overlay_opacity, 0.25);
        assert_eq!(settings.max_output_tokens, 65_536);
        assert_eq!(settings.temperature, 2.0);
        assert_eq!(settings.connection_timeout_secs, 1);
        assert_eq!(settings.response_timeout_secs, 300);
        assert_eq!(settings.stream_idle_timeout_secs, 1);
        assert_eq!(settings.model_order, ["gemini-custom"]);
    }

    #[test]
    fn empty_model_order_restores_default_models() {
        let mut settings = AppSettings {
            model_order: vec![" ".to_string()],
            ..AppSettings::default()
        };
        settings.normalize();
        assert_eq!(
            settings.model_order,
            DEFAULT_MODEL_ORDER
                .iter()
                .map(|model| (*model).to_string())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn older_settings_files_receive_defaults_for_new_request_options() {
        let settings: AppSettings = serde_json::from_str(r#"{"api_keys":[]}"#).unwrap();
        assert_eq!(settings.model_order.len(), DEFAULT_MODEL_ORDER.len());
        assert_eq!(settings.max_output_tokens, 2048);
        assert_eq!(settings.temperature, 0.2);
        assert_eq!(settings.connection_timeout_secs, 12);
        assert_eq!(settings.response_timeout_secs, 20);
        assert_eq!(settings.stream_idle_timeout_secs, 20);
        assert_eq!(settings.hyprland_window.width, 542);
        assert_eq!(settings.hyprland_window.height, 668);
        assert_eq!(settings.hyprland_window.opacity_percent, 95);
        assert!(settings.hyprland_window.blur);
        assert!(!settings.hyprland_window.show_border);
        assert!(!settings.hyprland_window.show_shadow);
        assert!(!settings.hyprland_window.glow);
    }

    #[test]
    fn catalog_exposes_models_supporting_generate_content() {
        let models = available_models().unwrap();
        assert!(models.iter().any(|(id, _)| id == "gemini-2.5-pro"));
        assert!(models.iter().all(|(id, _)| !id.starts_with("models/")));
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
