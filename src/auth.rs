use crate::keyring_store;
use anyhow::{anyhow, Result};
use reqwest::Client;
use serde::Deserialize;
use std::env;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::process::Command;
use tracing::{error, info};

// Google OAuth endpoints
const GOOGLE_AUTH_URL: &str = "https://accounts.google.com/o/oauth2/v2/auth";
const GOOGLE_TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

// Standard Google Cloud Project client credentials (can be overridden via env)
const DEFAULT_CLIENT_ID: &str = "689012915683-aueabgqe56pa81pu6i54stesq8gf0ejl.apps.googleusercontent.com";
const DEFAULT_CLIENT_SECRET: &str = "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";
const OAUTH_SCOPES: &str = "https://www.googleapis.com/auth/generative-language.retriever https://www.googleapis.com/auth/cloud-platform";

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: Option<String>,
    #[allow(dead_code)]
    expires_in: Option<u64>,
}

pub struct AuthManager {
    client: Client,
    client_id: String,
    client_secret: Option<String>,
}

impl AuthManager {
    pub fn new() -> Self {
        let client_id = env::var("GOOGLE_CLIENT_ID").unwrap_or_else(|_| DEFAULT_CLIENT_ID.to_string());
        let client_secret = env::var("GOOGLE_CLIENT_SECRET").ok().or_else(|| Some(DEFAULT_CLIENT_SECRET.to_string()));

        Self {
            client: Client::new(),
            client_id,
            client_secret,
        }
    }

    /// Obtains an active bearer access token, refreshing if necessary,
    /// or initiating the browser login if no credentials exist.
    pub async fn get_access_token(&self) -> Result<String> {
        // 1. Check if a refresh token is saved in the system keyring
        if let Some(refresh_token) = keyring_store::get_refresh_token() {
            info!("Found existing credentials in system keyring. Refreshing access token...");
            match self.refresh_access_token(&refresh_token).await {
                Ok(token) => return Ok(token),
                Err(e) => {
                    error!("Stored refresh token invalid or expired: {}. Re-authenticating...", e);
                }
            }
        }

        // 2. No valid stored token: trigger interactive browser login
        info!("No stored credentials found. Starting Google browser login flow...");
        let refresh_token = self.login_via_browser().await?;
        keyring_store::store_refresh_token(&refresh_token)?;

        self.refresh_access_token(&refresh_token).await
    }

    async fn refresh_access_token(&self, refresh_token: &str) -> Result<String> {
        let mut params = vec![
            ("client_id", self.client_id.as_str()),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ];

        let secret = self.client_secret.clone().unwrap_or_default();
        if !secret.is_empty() {
            params.push(("client_secret", &secret));
        }

        let res = self.client
            .post(GOOGLE_TOKEN_URL)
            .form(&params)
            .send()
            .await?;

        if !res.status().is_success() {
            let err = res.text().await.unwrap_or_default();
            return Err(anyhow!("Failed to refresh access token: {}", err));
        }

        let token_data: TokenResponse = res.json().await?;
        Ok(token_data.access_token)
    }

    async fn login_via_browser(&self) -> Result<String> {
        // Desktop OAuth apps with Google redirect to http://localhost:<port>/callback
        let listener = TcpListener::bind("127.0.0.1:8989")
            .or_else(|_| TcpListener::bind("127.0.0.1:0"))?;
        let port = listener.local_addr()?.port();
        let redirect_uri = format!("http://localhost:{}/callback", port);

        let auth_url = format!(
            "{}?client_id={}&redirect_uri={}&response_type=code&scope={}&access_type=offline&prompt=consent",
            GOOGLE_AUTH_URL,
            urlencoding(&self.client_id),
            urlencoding(&redirect_uri),
            urlencoding(OAUTH_SCOPES),
        );

        info!("Opening Google login in your default browser...");
        let _ = Command::new("xdg-open").arg(&auth_url).spawn();

        println!("\n=======================================================");
        println!("Please log in with your Google Account in the browser window.");
        println!("If the browser did not open automatically, open this URL:");
        println!("{}", auth_url);
        println!("=======================================================\n");

        // Wait for incoming callback on the local server
        let (mut stream, _) = listener.accept()?;
        let mut reader = BufReader::new(&mut stream);
        let mut request_line = String::new();
        reader.read_line(&mut request_line)?;

        let auth_code = extract_code_from_request(&request_line)
            .ok_or_else(|| anyhow!("OAuth authorization code was not received"))?;

        // Send a pleasant HTML response to the user's browser
        let html_response = "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\n\r\n\
            <html><body style='font-family:sans-serif; text-align:center; padding-top:60px; background:#181825; color:#cdd6f4;'>\
            <h2>✓ Magic Pointer Authorized!</h2>\
            <p style='color:#a6adc8;'>Credentials saved securely to your system keyring.<br>You can close this tab and return to the application.</p>\
            </body></html>";
        let _ = stream.write_all(html_response.as_bytes());

        // Exchange authorization code for refresh & access tokens
        let mut params = vec![
            ("code", auth_code.as_str()),
            ("client_id", self.client_id.as_str()),
            ("redirect_uri", redirect_uri.as_str()),
            ("grant_type", "authorization_code"),
        ];

        let secret = self.client_secret.clone().unwrap_or_default();
        if !secret.is_empty() {
            params.push(("client_secret", &secret));
        }

        let res = self.client
            .post(GOOGLE_TOKEN_URL)
            .form(&params)
            .send()
            .await?;

        if !res.status().is_success() {
            let err = res.text().await.unwrap_or_default();
            return Err(anyhow!("Failed to exchange authorization code: {}", err));
        }

        let token_data: TokenResponse = res.json().await?;
        token_data.refresh_token
            .ok_or_else(|| anyhow!("No refresh token returned by Google (was consent granted?)"))
    }
}

fn extract_code_from_request(request_line: &str) -> Option<String> {
    // Looks like: "GET /callback?code=4/0A... HTTP/1.1"
    let path = request_line.split_whitespace().nth(1)?;
    let query = path.split('?').nth(1)?;
    for pair in query.split('&') {
        let mut parts = pair.split('=');
        if parts.next()? == "code" {
            return parts.next().map(|s| s.to_string());
        }
    }
    None
}

fn urlencoding(input: &str) -> String {
    let mut encoded = String::new();
    for b in input.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                encoded.push(b as char);
            }
            _ => {
                encoded.push_str(&format!("%{:02X}", b));
            }
        }
    }
    encoded
}

