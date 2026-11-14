use anyhow::{Result, anyhow};
use image::DynamicImage;
use libwayshot::WayshotConnection;
use libwayshot::region::{LogicalRegion, Position, Size, TopLevel};
use tracing::{info, warn};

use crate::selection_overlay::geometry::Rect;

#[derive(Debug, Clone, PartialEq)]
pub enum CaptureTarget {
    Region(Rect),
    Window {
        rect: Rect,
        title: String,
        class: String,
        address: String,
    },
    Montage(Vec<CaptureTarget>),
}

impl CaptureTarget {
    pub fn rect(&self) -> Rect {
        match self {
            CaptureTarget::Region(r) => *r,
            CaptureTarget::Window { rect, .. } => *rect,
            CaptureTarget::Montage(targets) => {
                if targets.is_empty() {
                    Rect {
                        x: 0,
                        y: 0,
                        width: 1,
                        height: 1,
                    }
                } else {
                    let mut min_x = i32::MAX;
                    let mut min_y = i32::MAX;
                    let mut max_x = i32::MIN;
                    let mut max_y = i32::MIN;
                    for t in targets {
                        let r = t.rect();
                        min_x = min_x.min(r.x);
                        min_y = min_y.min(r.y);
                        max_x = max_x.max(r.x + r.width as i32);
                        max_y = max_y.max(r.y + r.height as i32);
                    }
                    Rect {
                        x: min_x,
                        y: min_y,
                        width: (max_x - min_x).max(1) as u32,
                        height: (max_y - min_y).max(1) as u32,
                    }
                }
            }
        }
    }
}

pub fn capture_target(target: &CaptureTarget) -> Result<DynamicImage> {
    match target {
        CaptureTarget::Region(rect) => {
            capture_screen_region(rect.x, rect.y, rect.width, rect.height)
        }
        CaptureTarget::Window {
            rect,
            title,
            class,
            address,
        } => capture_window_or_fallback(*rect, title, class, address),
        CaptureTarget::Montage(targets) => capture_montage(targets),
    }
}

pub fn capture_montage(targets: &[CaptureTarget]) -> Result<DynamicImage> {
    if targets.is_empty() {
        return Err(anyhow!("Cannot capture montage with empty targets"));
    }
    let mut captured_images = Vec::new();
    for target in targets {
        let label = match target {
            CaptureTarget::Window { class, title, .. } => format!("{class}: {title}"),
            _ => "Region".to_string(),
        };
        let img = capture_target(target)?;
        captured_images.push((label, img));
    }
    create_montage_composite(&captured_images)
}

pub fn create_montage_composite(images: &[(String, DynamicImage)]) -> Result<DynamicImage> {
    if images.is_empty() {
        return Err(anyhow!("Cannot create montage with 0 images"));
    }
    if images.len() == 1 {
        return Ok(images[0].1.clone());
    }

    let header_height = 28u32;
    let separator = 8u32;

    // Total width is sum of widths plus separators between them
    let total_width: u32 = images.iter().map(|(_, img)| img.width()).sum::<u32>()
        + separator * (images.len() as u32 - 1);
    let max_img_height: u32 = images
        .iter()
        .map(|(_, img)| img.height())
        .max()
        .unwrap_or(0);
    let total_height = max_img_height + header_height;

    let mut composite = image::RgbaImage::from_pixel(
        total_width,
        total_height,
        image::Rgba([18, 22, 28, 255]), // Dark background
    );

    let mut current_x = 0u32;
    for (_idx, (_label, img)) in images.iter().enumerate() {
        let rgba = img.to_rgba8();
        let img_w = rgba.width();
        let img_h = rgba.height();

        // Overlay window image at (current_x, header_height)
        for y in 0..img_h {
            for x in 0..img_w {
                if current_x + x < total_width && header_height + y < total_height {
                    let pixel = rgba.get_pixel(x, y);
                    composite.put_pixel(current_x + x, header_height + y, *pixel);
                }
            }
        }

        // Draw top header bar for each panel
        for y in 0..header_height {
            for x in 0..img_w {
                if current_x + x < total_width {
                    let header_color = if y == header_height - 1 {
                        image::Rgba([70, 95, 130, 255]) // Border line
                    } else {
                        image::Rgba([30, 36, 46, 255]) // Header bar background
                    };
                    composite.put_pixel(current_x + x, y, header_color);
                }
            }
        }

        current_x += img_w + separator;
    }

    Ok(DynamicImage::ImageRgba8(composite))
}

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

    #[allow(deprecated)]
    let image = wayshot.screenshot(region, false)?;
    Ok(image)
}

fn capture_window_or_fallback(
    rect: Rect,
    title: &str,
    class: &str,
    address: &str,
) -> Result<DynamicImage> {
    if rect.width == 0 || rect.height == 0 {
        return Err(anyhow!("Cannot capture window with 0 dimensions"));
    }

    match WayshotConnection::new() {
        Ok(wayshot) => {
            if wayshot.toplevel_capture_support() {
                let toplevels = wayshot.get_all_toplevels();
                if let Some(tl) = find_matching_toplevel(toplevels, title, class, address) {
                    info!(
                        "Capturing isolated toplevel window '{}' ({}) via libwayshot",
                        tl.title, tl.app_id
                    );
                    match wayshot.screenshot_toplevel(tl, false) {
                        Ok(image) => return Ok(image),
                        Err(err) => {
                            warn!(
                                "Failed to capture toplevel window directly ({err}); falling back to screen region"
                            );
                        }
                    }
                } else {
                    warn!(
                        "No matching toplevel found for class='{class}', title='{title}', address='{address}'; falling back to screen region"
                    );
                }
            } else {
                info!("Toplevel capture protocol not supported by compositor; using screen region");
            }
        }
        Err(err) => {
            warn!(
                "Failed to initialize WayshotConnection for toplevel capture ({err}); falling back to screen region"
            );
        }
    }

    capture_screen_region(rect.x, rect.y, rect.width, rect.height)
}

pub fn toplevel_match_score(
    tl_title: &str,
    tl_app_id: &str,
    tl_identifier: &str,
    target_title: &str,
    target_class: &str,
    target_address: &str,
) -> u32 {
    let mut score = 0;

    let clean_target_addr = target_address
        .trim()
        .trim_start_matches("0x")
        .to_lowercase();
    let clean_tl_id = tl_identifier.trim().trim_start_matches("0x").to_lowercase();
    if !clean_target_addr.is_empty() && !clean_tl_id.is_empty() {
        if clean_tl_id == clean_target_addr {
            score += 100;
        } else if clean_tl_id.contains(&clean_target_addr)
            || clean_target_addr.contains(&clean_tl_id)
        {
            score += 50;
        }
    }

    if !target_class.is_empty() && !tl_app_id.is_empty() {
        if tl_app_id.eq_ignore_ascii_case(target_class) {
            score += 40;
        } else if tl_app_id
            .to_lowercase()
            .contains(&target_class.to_lowercase())
            || target_class
                .to_lowercase()
                .contains(&tl_app_id.to_lowercase())
        {
            score += 20;
        }
    }

    if !target_title.is_empty() && !tl_title.is_empty() {
        if tl_title == target_title {
            score += 40;
        } else if tl_title.contains(target_title) || target_title.contains(tl_title) {
            score += 15;
        }
    }

    score
}

pub fn find_matching_toplevel<'a>(
    toplevels: &'a [TopLevel],
    title: &str,
    class: &str,
    address: &str,
) -> Option<&'a TopLevel> {
    toplevels
        .iter()
        .map(|tl| {
            (
                tl,
                toplevel_match_score(&tl.title, &tl.app_id, &tl.identifier, title, class, address),
            )
        })
        .filter(|(_, score)| *score > 0)
        .max_by_key(|(_, score)| *score)
        .map(|(tl, _)| tl)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_target_returns_correct_rect() {
        let rect = Rect {
            x: 10,
            y: 20,
            width: 100,
            height: 200,
        };
        let target_region = CaptureTarget::Region(rect);
        assert_eq!(target_region.rect(), rect);

        let target_window = CaptureTarget::Window {
            rect,
            title: "Test".to_string(),
            class: "app".to_string(),
            address: "0x123".to_string(),
        };
        assert_eq!(target_window.rect(), rect);
    }

    #[test]
    fn toplevel_match_score_prioritizes_address_and_class() {
        let score_exact = toplevel_match_score(
            "Terminal",
            "foot",
            "0x55aa1234",
            "Terminal",
            "foot",
            "0x55aa1234",
        );
        assert_eq!(score_exact, 100 + 40 + 40);

        let score_class_and_title =
            toplevel_match_score("cava", "cava", "", "cava", "cava", "0xdead");
        assert_eq!(score_class_and_title, 40 + 40);

        let score_mismatch =
            toplevel_match_score("firefox", "firefox", "0x1", "cava", "cava", "0x2");
        assert_eq!(score_mismatch, 0);
    }

    #[test]
    fn zero_dimension_capture_errors() {
        assert!(capture_screen_region(0, 0, 0, 100).is_err());
        assert!(capture_screen_region(0, 0, 100, 0).is_err());
    }
}
