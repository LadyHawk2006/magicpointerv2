use crate::auth::AuthManager;
use anyhow::{anyhow, Result};
use base64::{engine::general_purpose::STANDARD, Engine as _};
use futures_util::StreamExt;
use image::DynamicImage;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{json, Value};
use std::env;
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, Mutex};
use tracing::{info, warn};

pub fn encode_image_png_base64(image: &DynamicImage) -> Result<String> {
    let mut bytes: Vec<u8> = Vec::new();
    let mut cursor = Cursor::new(&mut bytes);
    image.write_to(&mut cursor, image::ImageFormat::Png)?;
    Ok(STANDARD.encode(bytes))
}

#[derive(Debug, Deserialize)]
struct GeminiStreamChunk {
    candidates: Option<Vec<Candidate>>,
    error: Option<GeminiError>,
}

#[derive(Debug, Deserialize)]
struct GeminiError {
    message: String,
}

#[derive(Debug, Deserialize)]
struct Candidate {
    content: Option<CandidateContent>,
}

#[derive(Debug, Deserialize)]
struct CandidateContent {
    parts: Option<Vec<CandidatePart>>,
}

#[derive(Debug, Deserialize)]
struct CandidatePart {
    text: Option<String>,
}

const MODELS_TO_TRY: &[&str] = &[
    "gemini-flash-lite-latest",
    "gemini-3.5-flash-lite",
    "gemini-3.1-flash-lite",
    "gemini-2.5-flash-lite",
    "gemini-flash-latest",
    "gemini-3.8-flash",
    "gemini-3.7-flash",
    "gemini-3.6-flash",
    "gemini-3.5-flash",
    "gemini-2.5-flash",
];

/// Maintains multi-turn conversation history
#[derive(Clone)]
pub struct ConversationSession {
    contents: Arc<Mutex<Vec<Value>>>,
}

impl ConversationSession {
    pub fn new() -> Self {
        Self {
            contents: Arc::new(Mutex::new(Vec::new())),
        }
    }

    /// Appends the initial user turn with the captured screenshot
    pub async fn add_user_initial_turn(&self, image: &DynamicImage, prompt: &str) -> Result<()> {
        let base64_image = encode_image_png_base64(image)?;
        let turn = json!({
            "role": "user",
            "parts": [
                { "text": prompt },
                {
                    "inline_data": {
                        "mime_type": "image/png",
                        "data": base64_image
                    }
                }
            ]
        });
        self.contents.lock().await.push(turn);
        Ok(())
    }

    /// Appends a follow-up text prompt from the user
    pub async fn add_user_follow_up(&self, text: &str) {
        let turn = json!({
            "role": "user",
            "parts": [{ "text": text }]
        });
        self.contents.lock().await.push(turn);
    }

    /// Appends model response to history once finished
    pub async fn add_model_response(&self, text: &str) {
        let turn = json!({
            "role": "model",
            "parts": [{ "text": text }]
        });
        self.contents.lock().await.push(turn);
    }

    /// Streams the next assistant turn to the UI
    pub async fn stream_next_turn(&self, token_tx: mpsc::Sender<String>) -> Result<()> {
        let history = self.contents.lock().await.clone();

        let body = json!({
            "contents": history,
            "generationConfig": {
                "temperature": 0.2,
                "maxOutputTokens": 2048
            }
        });

        let auth_manager = AuthManager::new();
        let (bearer_token, api_key) = if let Ok(token) = auth_manager.get_access_token().await {
            (Some(token), None)
        } else {
            let key = env::var("GEMINI_API_KEY").or_else(|_| env::var("GOOGLE_API_KEY")).ok();
            (None, key)
        };

        if bearer_token.is_none() && api_key.is_none() {
            return Err(anyhow!("No authentication available."));
        }

        // Fix SSE connection timeout: use connect_timeout instead of global request timeout
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(12))
            .build()?;

        for (i, &model_name) in MODELS_TO_TRY.iter().enumerate() {
            let mut retry_count = 0;
            let max_retries = 2;

            while retry_count <= max_retries {
                let url = if let Some(ref key) = api_key {
                    format!(
                        "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse&key={}",
                        model_name, key
                    )
                } else {
                    format!(
                        "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse",
                        model_name
                    )
                };

                info!("Connecting to model: {} (attempt {})...", model_name, retry_count + 1);

                let mut req = client
                    .post(&url)
                    .header("Content-Type", "application/json")
                    .json(&body);

                if let Some(ref bearer) = bearer_token {
                    req = req.header("Authorization", format!("Bearer {}", bearer));
                }

                let res = match req.send().await {
                    Ok(r) => r,
                    Err(e) => {
                        warn!("Connection error with {}: {}. Retrying...", model_name, e);
                        retry_count += 1;
                        tokio::time::sleep(Duration::from_millis(500 * (retry_count as u64))).await;
                        continue;
                    }
                };

                let status = res.status();
                if !status.is_success() {
                    let err_text = res.text().await.unwrap_or_default();
                    warn!("Model {} returned status {}: {}", model_name, status, err_text);

                    if status.as_u16() == 503 && retry_count < max_retries {
                        retry_count += 1;
                        let _ = token_tx.send(format!("[{} high demand, retrying...]\n", model_name)).await;
                        tokio::time::sleep(Duration::from_millis(1000 * (retry_count as u64))).await;
                        continue;
                    }

                    if (status.as_u16() == 404 || status.as_u16() == 503 || status.as_u16() == 429) && i + 1 < MODELS_TO_TRY.len() {
                        let _ = token_tx.send(format!("[Falling back to {}...]\n\n", MODELS_TO_TRY[i + 1])).await;
                        break;
                    }
                    return Err(anyhow!("Gemini API error ({}): {}", model_name, err_text));
                }

                let mut stream = res.bytes_stream();
                let mut buffer = String::new();
                let mut full_accumulated = String::new();

                while let Some(chunk_res) = stream.next().await {
                    let chunk = chunk_res?;
                    let text = String::from_utf8_lossy(&chunk);
                    buffer.push_str(&text);

                    while let Some(pos) = buffer.find("\n\n") {
                        let event_block = buffer[..pos].to_string();
                        buffer = buffer[pos + 2..].to_string();

                        for line in event_block.lines() {
                            if let Some(data) = line.strip_prefix("data: ") {
                                if let Ok(parsed) = serde_json::from_str::<GeminiStreamChunk>(data) {
                                    if let Some(err) = parsed.error {
                                        let _ = token_tx.send(format!("\n\n[API Error: {}]", err.message)).await;
                                        return Err(anyhow!("Gemini streaming error: {}", err.message));
                                    }

                                    if let Some(candidates) = parsed.candidates {
                                        for cand in candidates {
                                            if let Some(content) = cand.content {
                                                if let Some(parts) = content.parts {
                                                    for part in parts {
                                                        if let Some(t) = part.text {
                                                            if !t.is_empty() {
                                                                full_accumulated.push_str(&t);
                                                                let _ = token_tx.send(t).await;
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Save assistant answer in history for follow-ups
                self.add_model_response(&full_accumulated).await;
                info!("Finished response from {}", model_name);
                return Ok(());
            }
        }

        Err(anyhow!("All Gemini model endpoints failed"))
    }
}
