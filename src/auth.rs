use crate::keyring_store;
use anyhow::Result;
use std::env;

pub fn get_api_key() -> Result<Option<String>> {
    for variable in ["GEMINI_API_KEY", "GOOGLE_API_KEY"] {
        if let Ok(key) = env::var(variable) {
            let key = key.trim();
            if !key.is_empty() {
                return Ok(Some(key.to_owned()));
            }
        }
    }

    keyring_store::get_api_key()
}

pub fn store_api_key(api_key: &str) -> Result<()> {
    keyring_store::store_api_key(api_key.trim())
}
