use anyhow::{Result, anyhow};
use magicpointerv2::auth;
use reqwest::Client;

#[tokio::main]
async fn main() -> Result<()> {
    let api_keys = auth::get_api_keys()?;
    if api_keys.is_empty() {
        return Err(anyhow!("No Gemini API key is configured."));
    }

    let client = Client::new();
    let mut last_error = None;
    for (name, api_key) in api_keys {
        let response = client
            .get("https://generativelanguage.googleapis.com/v1beta/models")
            .header("x-goog-api-key", api_key)
            .send()
            .await?;
        let status = response.status();
        let text = response.text().await?;
        if status.is_success() {
            println!("Available Models:\n{}", text);
            return Ok(());
        }
        last_error = Some(anyhow!("API key '{name}' returned {status}: {text}"));
    }

    Err(last_error.unwrap_or_else(|| anyhow!("All configured Gemini API keys failed.")))
}
