use anyhow::{Result, anyhow};
use magicpointerv2::auth;
use reqwest::Client;

#[tokio::main]
async fn main() -> Result<()> {
    let api_key =
        auth::get_api_key()?.ok_or_else(|| anyhow!("No Gemini API key is configured."))?;

    let response = Client::new()
        .get("https://generativelanguage.googleapis.com/v1beta/models")
        .header("x-goog-api-key", api_key)
        .send()
        .await?;

    let text = response.text().await?;
    println!("Available Models:\n{}", text);
    Ok(())
}
