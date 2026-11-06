use anyhow::{Result, anyhow};
use std::io::Write;
use std::process::{Command, Stdio};
use tracing::{debug, info};

const SERVICE_NAME: &str = "magicpointerv2";
const ACCOUNT_NAME: &str = "gemini_api_key";

pub fn get_api_key() -> Result<Option<String>> {
    let output = Command::new("secret-tool")
        .args(["lookup", "service", SERVICE_NAME, "account", ACCOUNT_NAME])
        .output()?;

    if output.status.success() {
        let api_key = String::from_utf8(output.stdout)
            .map_err(|error| anyhow!("API key in system keyring is not valid UTF-8: {error}"))?;
        let api_key = api_key.trim();
        if !api_key.is_empty() {
            debug!("Retrieved Gemini API key from system keyring");
            return Ok(Some(api_key.to_owned()));
        }
        return Ok(None);
    }

    let error = String::from_utf8_lossy(&output.stderr);
    if output.status.code() == Some(1) && error.trim().is_empty() {
        return Ok(None);
    }

    Err(anyhow!(
        "Failed to read Gemini API key from system keyring: {}",
        error.trim()
    ))
}

pub fn store_api_key(api_key: &str) -> Result<()> {
    if api_key.is_empty() {
        return Err(anyhow!("Gemini API key cannot be empty"));
    }

    let mut child = Command::new("secret-tool")
        .args([
            "store",
            "--label=Magic Pointer Gemini API key",
            "service",
            SERVICE_NAME,
            "account",
            ACCOUNT_NAME,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()?;

    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow!("Could not open input for the system keyring process"))?;
    stdin.write_all(api_key.as_bytes())?;
    drop(stdin);

    let output = child.wait_with_output()?;
    if output.status.success() {
        info!("Saved Gemini API key securely in system keyring");
        Ok(())
    } else {
        Err(anyhow!(
            "Failed to save Gemini API key in system keyring: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ))
    }
}
