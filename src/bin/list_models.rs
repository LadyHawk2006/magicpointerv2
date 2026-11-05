use anyhow::Result;
use magicpointerv2::auth::AuthManager;
use reqwest::Client;
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    let auth_manager = AuthManager::new();
    let (bearer, api_key) = if let Ok(t) = auth_manager.get_access_token().await {
        (Some(t), None)
    } else {
        (
            None,
            env::var("GEMINI_API_KEY")
                .or_else(|_| env::var("GOOGLE_API_KEY"))
                .ok(),
        )
    };

    let client = Client::new();
    let url = if let Some(ref k) = api_key {
        format!(
            "https://generativelanguage.googleapis.com/v1beta/models?key={}",
            k
        )
    } else {
        "https://generativelanguage.googleapis.com/v1beta/models".to_string()
    };

    let mut req = client.get(&url);
    if let Some(ref b) = bearer {
        req = req.header("Authorization", format!("Bearer {}", b));
    }

    let res = req.send().await?;
    let text = res.text().await?;
    println!("Available Models:\n{}", text);
    Ok(())
}
