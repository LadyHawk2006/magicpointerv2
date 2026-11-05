use anyhow::{anyhow, Result};
use arboard::Clipboard;
use tracing::info;

pub fn copy_to_clipboard(text: &str) -> Result<()> {
    let mut clipboard = Clipboard::new()
        .map_err(|e| anyhow!("Failed to access clipboard: {}", e))?;
    clipboard.set_text(text)
        .map_err(|e| anyhow!("Failed to set clipboard text: {}", e))?;
    info!("Successfully copied {} bytes to Wayland clipboard", text.len());
    Ok(())
}
