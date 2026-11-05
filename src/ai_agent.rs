use crate::auth::AuthManager;
use anyhow::{Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::StreamExt;
use image::DynamicImage;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use std::env;
use std::io::Cursor;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Mutex, mpsc};
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

fn take_sse_event(buffer: &mut String, end_of_stream: bool) -> Option<String> {
    let lf_boundary = buffer.find("\n\n").map(|index| (index, 2));
    let crlf_boundary = buffer.find("\r\n\r\n").map(|index| (index, 4));
    let boundary = match (lf_boundary, crlf_boundary) {
        (Some(lf), Some(crlf)) => Some(if lf.0 <= crlf.0 { lf } else { crlf }),
        (Some(lf), None) => Some(lf),
        (None, Some(crlf)) => Some(crlf),
        (None, None) => None,
    };

    if let Some((index, boundary_len)) = boundary {
        let event = buffer[..index].to_string();
        buffer.drain(..index + boundary_len);
        return Some(event);
    }

    if end_of_stream && !buffer.trim().is_empty() {
        return Some(std::mem::take(buffer));
    }

    None
}

fn sse_event_data(event: &str) -> Option<String> {
    let data = event
        .lines()
        .filter_map(|line| {
            line.strip_prefix("data:")
                .map(|value| value.strip_prefix(' ').unwrap_or(value))
        })
        .collect::<Vec<_>>()
        .join("\n");

    (!data.is_empty()).then_some(data)
}

fn parse_gemini_event(event: &str) -> Result<Vec<String>> {
    let Some(data) = sse_event_data(event) else {
        return Ok(Vec::new());
    };
    if data == "[DONE]" {
        return Ok(Vec::new());
    }

    let parsed = serde_json::from_str::<GeminiStreamChunk>(&data)
        .map_err(|e| anyhow!("Invalid Gemini SSE response: {e}"))?;
    if let Some(err) = parsed.error {
        return Err(anyhow!("Gemini streaming error: {}", err.message));
    }

    Ok(parsed
        .candidates
        .unwrap_or_default()
        .into_iter()
        .filter_map(|candidate| candidate.content)
        .flat_map(|content| content.parts.unwrap_or_default())
        .filter_map(|part| part.text)
        .filter(|text| !text.is_empty())
        .collect())
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
            let key = env::var("GEMINI_API_KEY")
                .or_else(|_| env::var("GOOGLE_API_KEY"))
                .ok();
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

                info!(
                    "Connecting to model: {} (attempt {})...",
                    model_name,
                    retry_count + 1
                );

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
                    warn!(
                        "Model {} returned status {}: {}",
                        model_name, status, err_text
                    );

                    if status.as_u16() == 503 && retry_count < max_retries {
                        retry_count += 1;
                        let _ = token_tx
                            .send(format!("[{} high demand, retrying...]\n", model_name))
                            .await;
                        tokio::time::sleep(Duration::from_millis(1000 * (retry_count as u64)))
                            .await;
                        continue;
                    }

                    if (status.as_u16() == 404 || status.as_u16() == 503 || status.as_u16() == 429)
                        && i + 1 < MODELS_TO_TRY.len()
                    {
                        let _ = token_tx
                            .send(format!("[Falling back to {}...]\n\n", MODELS_TO_TRY[i + 1]))
                            .await;
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

                    while let Some(event) = take_sse_event(&mut buffer, false) {
                        for text in parse_gemini_event(&event)? {
                            full_accumulated.push_str(&text);
                            let _ = token_tx.send(text).await;
                        }
                    }
                }

                if let Some(event) = take_sse_event(&mut buffer, true) {
                    for text in parse_gemini_event(&event)? {
                        full_accumulated.push_str(&text);
                        let _ = token_tx.send(text).await;
                    }
                }

                if full_accumulated.is_empty() {
                    return Err(anyhow!(
                        "Gemini completed the stream without returning text."
                    ));
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

#[cfg(test)]
mod tests {
    use super::{parse_gemini_event, sse_event_data, take_sse_event};

    #[test]
    fn parses_lf_and_crlf_sse_boundaries() {
        for separator in ["\n\n", "\r\n\r\n"] {
            let mut buffer = format!("data: {{\"text\":\"hello\"}}{separator}data: [DONE]");
            assert_eq!(
                take_sse_event(&mut buffer, false).as_deref(),
                Some("data: {\"text\":\"hello\"}")
            );
            assert_eq!(
                sse_event_data("data: {\"text\":\"hello\"}").as_deref(),
                Some("{\"text\":\"hello\"}")
            );
            assert_eq!(
                take_sse_event(&mut buffer, true).as_deref(),
                Some("data: [DONE]")
            );
        }
    }

    #[test]
    fn preserves_event_delimiter_split_across_chunks() {
        let mut buffer = "data: {\"text\":\"hello\"}\r\n".to_string();
        assert_eq!(take_sse_event(&mut buffer, false), None);
        buffer.push_str("\r\n");
        assert_eq!(
            take_sse_event(&mut buffer, false).as_deref(),
            Some("data: {\"text\":\"hello\"}")
        );
    }

    #[test]
    fn parses_gemini_text_response() {
        assert_eq!(
            parse_gemini_event(
                "data: {\"candidates\":[{\"content\":{\"parts\":[{\"text\":\"Hello\"}]}}]}"
            )
            .unwrap(),
            vec!["Hello"]
        );
    }
}
