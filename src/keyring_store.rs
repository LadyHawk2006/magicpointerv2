use anyhow::{anyhow, Result};
use std::process::Command;
use tracing::{debug, info};

const SERVICE_NAME: &str = "magicpointerv2";
const ACCOUNT_NAME: &str = "google_oauth_refresh_token";

pub fn get_refresh_token() -> Option<String> {
    let output = Command::new("secret-tool")
        .args(["lookup", "service", SERVICE_NAME, "account", ACCOUNT_NAME])
        .output()
        .ok()?;

    if output.status.success() {
        let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !token.is_empty() {
            debug!("Retrieved OAuth refresh token from system keyring");
            return Some(token);
        }
    }
    None
}

pub fn store_refresh_token(token: &str) -> Result<()> {
    use std::io::Write;
    let mut child = Command::new("secret-tool")
        .args([
            "store",
            "--label=MagicPointer Google OAuth",
            "service",
            SERVICE_NAME,
            "account",
            ACCOUNT_NAME,
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()?;

    if let Some(mut stdin) = child.stdin.take() {
        stdin.write_all(token.as_bytes())?;
    }

    let status = child.wait()?;
    if status.success() {
        info!("Saved Google OAuth refresh token securely in system keyring");
        Ok(())
    } else {
        Err(anyhow!("Failed to store credentials in system keyring via secret-tool"))
    }
}

pub fn clear_credentials() -> Result<()> {
    let status = Command::new("secret-tool")
        .args(["clear", "service", SERVICE_NAME, "account", ACCOUNT_NAME])
        .status()?;
    if status.success() {
        info!("Cleared credentials from system keyring");
        Ok(())
    } else {
        Err(anyhow!("Failed to clear credentials from system keyring"))
    }
}
