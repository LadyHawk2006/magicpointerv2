use crate::settings::AppSettings;
use anyhow::Result;
use std::env;

pub fn get_api_keys() -> Result<Vec<(String, String)>> {
    let environment_key = ["GEMINI_API_KEY", "GOOGLE_API_KEY"]
        .into_iter()
        .find_map(|variable| {
            env::var(variable).ok().and_then(|key| {
                let key = key.trim();
                (!key.is_empty()).then(|| (format!("Environment ({variable})"), key.to_owned()))
            })
        });

    let mut api_keys = match AppSettings::load().and_then(|settings| settings.api_keys_in_order()) {
        Ok(api_keys) => api_keys,
        Err(error) if environment_key.is_some() => {
            tracing::warn!(
                "Could not load saved fallback API keys; using the environment key only: {error}"
            );
            Vec::new()
        }
        Err(error) => return Err(error),
    };

    if let Some(environment_key) = environment_key {
        api_keys.insert(0, environment_key);
        api_keys.dedup_by(|current, previous| current.1 == previous.1);
    }

    Ok(api_keys)
}

pub fn get_api_key() -> Result<Option<String>> {
    Ok(get_api_keys()?.into_iter().next().map(|(_, key)| key))
}

pub fn store_api_key(api_key: &str) -> Result<()> {
    let mut settings = AppSettings::load()?;
    let name = if settings.api_keys.is_empty() {
        "Primary"
    } else {
        "API key"
    };
    settings.add_api_key(name, api_key)?;
    Ok(())
}
