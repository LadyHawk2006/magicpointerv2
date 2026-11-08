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
}

impl CaptureTarget {
    pub fn rect(&self) -> Rect {
        match self {
            CaptureTarget::Region(r) => *r,
            CaptureTarget::Window { rect, .. } => *rect,
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
    }
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
