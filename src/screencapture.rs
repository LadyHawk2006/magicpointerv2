use anyhow::{Result, anyhow};
use image::DynamicImage;
use libwayshot::WayshotConnection;
use libwayshot::region::{LogicalRegion, Position, Size};
use tracing::info;

pub fn capture_screen_region(x: i32, y: i32, width: u32, height: u32) -> Result<DynamicImage> {
    if width == 0 || height == 0 {
        return Err(anyhow!("Cannot capture region with 0 dimensions"));
    }

    info!(
        "Capturing compositor buffer at ({}, {}) [{}x{}] via libwayshot",
        x, y, width, height
    );

    let wayshot = WayshotConnection::new()?;
    let region = LogicalRegion {
        inner: libwayshot::region::Region {
            position: Position { x, y },
            size: Size { width, height },
        },
    };

    let image = wayshot.screenshot_freeze(move |_| Ok(region), false)?;
    Ok(image)
}
