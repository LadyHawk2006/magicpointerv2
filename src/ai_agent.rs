use crate::auth;
use crate::settings::AppSettings;
use anyhow::{Result, anyhow};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use futures_util::StreamExt;
use image::DynamicImage;
use reqwest::Client;
use serde::Deserialize;
use serde_json::{Value, json};
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tokio::sync::{Mutex, mpsc};
use tracing::{info, warn};

const MAX_IMAGE_DIMENSION: u32 = 2048;
const JPEG_QUALITY: u8 = 82;

pub fn encode_image_jpeg_base64(image: &DynamicImage) -> Result<String> {
    let image = image.thumbnail(MAX_IMAGE_DIMENSION, MAX_IMAGE_DIMENSION);
    let mut bytes = Vec::new();
    let mut encoder = image::codecs::jpeg::JpegEncoder::new_with_quality(&mut bytes, JPEG_QUALITY);
    encoder.encode_image(&image)?;
    Ok(STANDARD.encode(bytes))
}

#[derive(Debug, Deserialize)]
struct GeminiStreamChunk {
    candidates: Option<Vec<Candidate>>,
    error: Option<GeminiError>,
}

#[derive(Debug, Deserialize)]
struct GeminiError {
    code: Option<u16>,
    message: String,
    status: Option<String>,
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

#[derive(Debug)]
struct GeminiOverloadedError {
    message: String,
}

impl fmt::Display for GeminiOverloadedError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for GeminiOverloadedError {}

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
        if err.code == Some(503)
            || err.status.as_deref() == Some("UNAVAILABLE")
            || err.message.to_ascii_lowercase().contains("high demand")
        {
            return Err(anyhow::Error::new(GeminiOverloadedError {
                message: err.message,
            }));
        }
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
        self.add_user_turn(prompt, std::slice::from_ref(image))
            .await
    }

    /// Appends a user turn with zero or more captured images.
    pub async fn add_user_turn(&self, text: &str, images: &[DynamicImage]) -> Result<()> {
        let mut parts = vec![json!({ "text": text })];
        for image in images {
            parts.push(json!({
                "inline_data": {
                    "mime_type": "image/jpeg",
                    "data": encode_image_jpeg_base64(image)?
                }
            }));
        }

        self.contents.lock().await.push(json!({
            "role": "user",
            "parts": parts
        }));
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
        let settings = AppSettings::load()?;
        let api_keys = auth::get_api_keys()?.into_iter().collect::<Vec<_>>();
        if api_keys.is_empty() {
            return Err(anyhow!("No Gemini API key is configured."));
        }

        let mut last_error = None;
        let response_started = Arc::new(AtomicBool::new(false));
        for (index, (name, api_key)) in api_keys.iter().enumerate() {
            match self
                .stream_with_key(
                    &history,
                    api_key,
                    token_tx.clone(),
                    response_started.clone(),
                    &settings,
                )
                .await
            {
                Ok(()) => return Ok(()),
                Err(error) => {
                    warn!("Gemini API key '{name}' failed: {error}");
                    if response_started.load(Ordering::Relaxed) {
                        return Err(error);
                    }
                    last_error = Some(error);
                    if index + 1 < api_keys.len() {
                        info!("Trying fallback API key '{}'", api_keys[index + 1].0);
                    }
                }
            }
        }

        Err(anyhow!(
            "All configured Gemini API keys failed: {}",
            last_error
                .map(|error| error.to_string())
                .unwrap_or_else(|| "no API keys are available".to_string())
        ))
    }

    async fn stream_with_key(
        &self,
        history: &[Value],
        api_key: &str,
        token_tx: mpsc::Sender<String>,
        response_started: Arc<AtomicBool>,
        settings: &AppSettings,
    ) -> Result<()> {
        let body = json!({
            "contents": history,
            "generationConfig": {
                "temperature": settings.temperature,
                "maxOutputTokens": settings.max_output_tokens
            }
        });

        let client = Client::builder()
            .connect_timeout(Duration::from_secs(settings.connection_timeout_secs))
            .build()?;

        let mut partial_response = String::new();

        for (i, model_name) in settings.model_order.iter().enumerate() {
            let url = format!(
                "https://generativelanguage.googleapis.com/v1beta/models/{}:streamGenerateContent?alt=sse",
                model_name
            );

            info!("Connecting to model: {}...", model_name);

            let req = client
                .post(&url)
                .header("Content-Type", "application/json")
                .header("x-goog-api-key", api_key)
                .json(&body);

            let response_timeout = Duration::from_secs(settings.response_timeout_secs);
            let res = match tokio::time::timeout(response_timeout, req.send()).await {
                Err(_) => {
                    let reason = format!(
                        "timed out waiting for a response after {} seconds",
                        response_timeout.as_secs()
                    );
                    warn!("Model {} {}", model_name, reason);
                    if let Some(next_model) = settings.model_order.get(i + 1) {
                        info!(
                            "Switching from {} to {} after timeout",
                            model_name, next_model
                        );
                        continue;
                    }
                    return Err(anyhow!("Model {} {}", model_name, reason));
                }
                Ok(Err(e)) => {
                    warn!(
                        "Connection error with {}: {}. Trying the next model.",
                        model_name, e
                    );
                    if let Some(next_model) = settings.model_order.get(i + 1) {
                        info!(
                            "Switching from {} to {} after connection failure",
                            model_name, next_model
                        );
                        continue;
                    }
                    return Err(anyhow!("Connection to {} failed: {}", model_name, e));
                }
                Ok(Ok(response)) => response,
            };

            let status = res.status();
            if !status.is_success() {
                let err_text = match tokio::time::timeout(Duration::from_secs(5), res.text()).await
                {
                    Ok(Ok(text)) => text,
                    Ok(Err(error)) => format!("Could not read error response: {error}"),
                    Err(_) => "Timed out reading error response".to_string(),
                };
                warn!(
                    "Model {} returned status {}: {}",
                    model_name, status, err_text
                );

                if (status.as_u16() == 404 || status.as_u16() == 503 || status.as_u16() == 429)
                    && i + 1 < settings.model_order.len()
                {
                    let reason = match status.as_u16() {
                        404 => "is unavailable",
                        429 => "is rate limited",
                        _ => "is overloaded",
                    };
                    info!(
                        "Switching from {} to {} because it {}",
                        model_name,
                        settings.model_order[i + 1],
                        reason
                    );
                    continue;
                }
                return Err(anyhow!("Gemini API error ({}): {}", model_name, err_text));
            }

            let mut stream = res.bytes_stream();
            let mut buffer = String::new();
            let mut full_accumulated = String::new();
            let mut overload_error = None;

            'read_stream: loop {
                let stream_idle_timeout = Duration::from_secs(settings.stream_idle_timeout_secs);
                let chunk_res = match tokio::time::timeout(stream_idle_timeout, stream.next()).await
                {
                    Ok(Some(chunk_res)) => chunk_res,
                    Ok(None) => break,
                    Err(_) => {
                        overload_error = Some(format!(
                            "stopped responding for {} seconds",
                            stream_idle_timeout.as_secs()
                        ));
                        break;
                    }
                };
                let chunk = match chunk_res {
                    Ok(chunk) => chunk,
                    Err(error) => {
                        overload_error = Some(format!("stream failed: {error}"));
                        break;
                    }
                };
                let text = String::from_utf8_lossy(&chunk);
                buffer.push_str(&text);

                while let Some(event) = take_sse_event(&mut buffer, false) {
                    let texts = match parse_gemini_event(&event) {
                        Ok(texts) => texts,
                        Err(error) => {
                            if let Some(overloaded) = error.downcast_ref::<GeminiOverloadedError>()
                            {
                                overload_error =
                                    Some(format!("is overloaded: {}", overloaded.message));
                                break 'read_stream;
                            }
                            return Err(error);
                        }
                    };
                    for text in texts {
                        full_accumulated.push_str(&text);
                        response_started.store(true, Ordering::Relaxed);
                        let _ = token_tx.send(text).await;
                    }
                }
            }

            if overload_error.is_none() {
                if let Some(event) = take_sse_event(&mut buffer, true) {
                    let texts = match parse_gemini_event(&event) {
                        Ok(texts) => texts,
                        Err(error) => {
                            if let Some(overloaded) = error.downcast_ref::<GeminiOverloadedError>()
                            {
                                overload_error =
                                    Some(format!("is overloaded: {}", overloaded.message));
                                Vec::new()
                            } else {
                                return Err(error);
                            }
                        }
                    };
                    for text in texts {
                        full_accumulated.push_str(&text);
                        response_started.store(true, Ordering::Relaxed);
                        let _ = token_tx.send(text).await;
                    }
                }
            }

            if let Some(message) = overload_error {
                warn!("Model {} {}; moving to fallback", model_name, message);
                if let Some(next_model) = settings.model_order.get(i + 1) {
                    partial_response.push_str(&full_accumulated);
                    info!(
                        "Switching from {} to {} after stream failure",
                        model_name, next_model
                    );
                    continue;
                }

                return Err(anyhow!(
                    "Gemini model {} {} and no fallback models remain",
                    model_name,
                    message
                ));
            }

            if full_accumulated.is_empty() {
                let reason = "completed without returning text";
                if let Some(next_model) = settings.model_order.get(i + 1) {
                    warn!("Model {} {}; trying {}", model_name, reason, next_model);
                    info!(
                        "Switching from {} to {} after empty response",
                        model_name, next_model
                    );
                    continue;
                }
                return Err(anyhow!("Gemini model {} {}", model_name, reason));
            }

            // Save assistant answer in history for follow-ups
            partial_response.push_str(&full_accumulated);
            self.add_model_response(&partial_response).await;
            info!("Finished response from {}", model_name);
            return Ok(());
        }

        Err(anyhow!("All configured Gemini model endpoints failed"))
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ConversationSession, GeminiOverloadedError, parse_gemini_event, sse_event_data,
        take_sse_event,
    };
    use image::DynamicImage;

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

    #[test]
    fn identifies_gemini_503_stream_errors_as_retryable() {
        let error = parse_gemini_event(
            "data: {\"error\":{\"code\":503,\"message\":\"Service unavailable\",\"status\":\"UNAVAILABLE\"}}",
        )
        .unwrap_err();

        assert!(error.downcast_ref::<GeminiOverloadedError>().is_some());
    }

    #[tokio::test]
    async fn user_turn_includes_all_attached_images() {
        let session = ConversationSession::new();
        let images = [DynamicImage::new_rgba8(1, 1), DynamicImage::new_rgba8(2, 2)];

        session
            .add_user_turn("Compare these", &images)
            .await
            .unwrap();

        let contents = session.contents.lock().await;
        let parts = contents[0]["parts"].as_array().unwrap();
        assert_eq!(parts.len(), 3);
        assert_eq!(parts[0]["text"], "Compare these");
        assert_eq!(parts[1]["inline_data"]["mime_type"], "image/jpeg");
        assert_eq!(parts[2]["inline_data"]["mime_type"], "image/jpeg");
    }

    #[test]
    fn image_payload_is_jpeg_and_resized_to_the_maximum_dimension() {
        use super::{MAX_IMAGE_DIMENSION, encode_image_jpeg_base64};
        use base64::Engine as _;

        let image = DynamicImage::new_rgb8(MAX_IMAGE_DIMENSION + 100, 100);
        let encoded = encode_image_jpeg_base64(&image).unwrap();
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .unwrap();
        let decoded = image::load_from_memory(&bytes).unwrap();

        assert_eq!(decoded.width(), MAX_IMAGE_DIMENSION);
        assert!(decoded.height() < 100);
    }
}
