use crate::hypr_ipc::WindowGeometry;

pub const SNAP_DISTANCE: f64 = 10.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectionMode {
    WindowSnap,
    RegionSelection,
    FullMonitor,
}

impl SelectionMode {
    pub fn next(self) -> Self {
        match self {
            Self::WindowSnap => Self::RegionSelection,
            Self::RegionSelection => Self::FullMonitor,
            Self::FullMonitor => Self::WindowSnap,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::WindowSnap => "Window Snap",
            Self::RegionSelection => "Region Selection",
            Self::FullMonitor => "Full Monitor",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl Rect {
    pub fn contains(&self, px: f64, py: f64) -> bool {
        let x = self.x as f64;
        let y = self.y as f64;
        let w = self.width as f64;
        let h = self.height as f64;
        px >= x && px <= x + w && py >= y && py <= y + h
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SnappedWindow {
    pub compositor_rect: Rect,
    pub local_rect: Rect,
    pub title: String,
    pub class: String,
}

pub fn convert_windows_to_local(
    windows: &[WindowGeometry],
    monitor_origin: (i32, i32),
    monitor_size: (i32, i32),
) -> Vec<SnappedWindow> {
    windows
        .iter()
        .filter_map(|w| {
            let local_x = w.x - monitor_origin.0;
            let local_y = w.y - monitor_origin.1;
            let local_w = w.width;
            let local_h = w.height;

            if local_w <= 0 || local_h <= 0 {
                return None;
            }

            if local_x + local_w <= 0
                || local_y + local_h <= 0
                || local_x >= monitor_size.0
                || local_y >= monitor_size.1
            {
                return None;
            }

            let clamped_x = local_x.max(0);
            let clamped_y = local_y.max(0);
            let clamped_right = (local_x + local_w).min(monitor_size.0);
            let clamped_bottom = (local_y + local_h).min(monitor_size.1);
            let clamped_w = clamped_right - clamped_x;
            let clamped_h = clamped_bottom - clamped_y;

            if clamped_w < 8 || clamped_h < 8 {
                return None;
            }

            Some(SnappedWindow {
                compositor_rect: Rect {
                    x: clamped_x + monitor_origin.0,
                    y: clamped_y + monitor_origin.1,
                    width: clamped_w as u32,
                    height: clamped_h as u32,
                },
                local_rect: Rect {
                    x: clamped_x,
                    y: clamped_y,
                    width: clamped_w as u32,
                    height: clamped_h as u32,
                },
                title: w.title.clone(),
                class: w.class.clone(),
            })
        })
        .collect()
}

pub fn find_all_snapped_windows(
    windows: &[SnappedWindow],
    cursor_x: f64,
    cursor_y: f64,
) -> Vec<SnappedWindow> {
    windows
        .iter()
        .filter(|w| w.local_rect.contains(cursor_x, cursor_y))
        .cloned()
        .collect()
}

pub fn find_snapped_window(
    windows: &[SnappedWindow],
    cursor_x: f64,
    cursor_y: f64,
) -> Option<&SnappedWindow> {
    windows
        .iter()
        .find(|w| w.local_rect.contains(cursor_x, cursor_y))
}

pub fn cycle_window_index(current: usize, total: usize, forward: bool) -> usize {
    if total == 0 {
        return 0;
    }
    if forward {
        (current + 1) % total
    } else {
        (current + total - 1) % total
    }
}

pub fn collect_guides(
    windows: &[SnappedWindow],
    monitor_width: i32,
    monitor_height: i32,
) -> (Vec<f64>, Vec<f64>) {
    let mut x_guides = vec![0.0, monitor_width as f64];
    let mut y_guides = vec![0.0, monitor_height as f64];

    for w in windows {
        let rx = w.local_rect.x as f64;
        let ry = w.local_rect.y as f64;
        let rw = w.local_rect.width as f64;
        let rh = w.local_rect.height as f64;
        x_guides.push(rx);
        x_guides.push(rx + rw);
        y_guides.push(ry);
        y_guides.push(ry + rh);
    }

    x_guides.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    x_guides.dedup();
    y_guides.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    y_guides.dedup();

    (x_guides, y_guides)
}

pub fn find_nearest_guide(guides: &[f64], val: f64, threshold: f64) -> Option<f64> {
    guides
        .iter()
        .copied()
        .filter(|&g| (g - val).abs() <= threshold)
        .min_by(|a, b| {
            let diff_a = (a - val).abs();
            let diff_b = (b - val).abs();
            diff_a
                .partial_cmp(&diff_b)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_contains_point() {
        let rect = Rect {
            x: 100,
            y: 100,
            width: 200,
            height: 150,
        };
        assert!(rect.contains(100.0, 100.0));
        assert!(rect.contains(150.0, 150.0));
        assert!(rect.contains(300.0, 250.0));
        assert!(!rect.contains(99.0, 100.0));
        assert!(!rect.contains(301.0, 150.0));
        assert!(!rect.contains(150.0, 251.0));
    }

    #[test]
    fn convert_windows_to_local_clamps_and_translates() {
        let windows = vec![
            WindowGeometry {
                x: 100,
                y: 200,
                width: 800,
                height: 600,
                title: "Editor".to_string(),
                class: "code".to_string(),
            },
            WindowGeometry {
                x: -50,
                y: 100,
                width: 400,
                height: 300,
                title: "Terminal".to_string(),
                class: "alacritty".to_string(),
            },
            WindowGeometry {
                x: 2500,
                y: 100,
                width: 500,
                height: 400,
                title: "Offscreen".to_string(),
                class: "other".to_string(),
            },
        ];

        let monitor_origin = (0, 0);
        let monitor_size = (1920, 1080);
        let snapped = convert_windows_to_local(&windows, monitor_origin, monitor_size);

        assert_eq!(snapped.len(), 2);
        assert_eq!(snapped[0].local_rect.x, 100);
        assert_eq!(snapped[0].local_rect.y, 200);
        assert_eq!(snapped[0].local_rect.width, 800);
        assert_eq!(snapped[0].local_rect.height, 600);

        assert_eq!(snapped[1].local_rect.x, 0);
        assert_eq!(snapped[1].local_rect.y, 100);
        assert_eq!(snapped[1].local_rect.width, 350);
        assert_eq!(snapped[1].local_rect.height, 300);
    }

    #[test]
    fn convert_windows_supports_multi_monitor_offset() {
        let windows = vec![WindowGeometry {
            x: 2020,
            y: 150,
            width: 1000,
            height: 700,
            title: "Browser".to_string(),
            class: "firefox".to_string(),
        }];

        let monitor_origin = (1920, 0);
        let monitor_size = (1920, 1080);
        let snapped = convert_windows_to_local(&windows, monitor_origin, monitor_size);

        assert_eq!(snapped.len(), 1);
        assert_eq!(snapped[0].local_rect.x, 100);
        assert_eq!(snapped[0].local_rect.y, 150);
        assert_eq!(snapped[0].compositor_rect.x, 2020);
        assert_eq!(snapped[0].compositor_rect.y, 150);
    }

    #[test]
    fn mode_switcher_cycles_correctly() {
        let mode = SelectionMode::WindowSnap;
        assert_eq!(mode.next(), SelectionMode::RegionSelection);
        assert_eq!(mode.next().next(), SelectionMode::FullMonitor);
        assert_eq!(mode.next().next().next(), SelectionMode::WindowSnap);
    }

    #[test]
    fn overlapping_windows_cycle_index_forward_and_backward() {
        assert_eq!(cycle_window_index(0, 3, true), 1);
        assert_eq!(cycle_window_index(1, 3, true), 2);
        assert_eq!(cycle_window_index(2, 3, true), 0);
        assert_eq!(cycle_window_index(0, 3, false), 2);
        assert_eq!(cycle_window_index(2, 3, false), 1);
        assert_eq!(cycle_window_index(0, 0, true), 0);
    }

    #[test]
    fn edge_snapping_finds_nearest_guide() {
        let guides = vec![0.0, 100.0, 500.0, 1920.0];

        assert_eq!(find_nearest_guide(&guides, 98.0, 10.0), Some(100.0));
        assert_eq!(find_nearest_guide(&guides, 105.0, 10.0), Some(100.0));
        assert_eq!(find_nearest_guide(&guides, 120.0, 10.0), None);
        assert_eq!(find_nearest_guide(&guides, 500.0, 10.0), Some(500.0));
    }

    #[test]
    fn find_all_overlapping_windows() {
        let windows = vec![
            SnappedWindow {
                compositor_rect: Rect {
                    x: 100,
                    y: 100,
                    width: 200,
                    height: 200,
                },
                local_rect: Rect {
                    x: 100,
                    y: 100,
                    width: 200,
                    height: 200,
                },
                title: "Top Window".to_string(),
                class: "floating".to_string(),
            },
            SnappedWindow {
                compositor_rect: Rect {
                    x: 50,
                    y: 50,
                    width: 500,
                    height: 500,
                },
                local_rect: Rect {
                    x: 50,
                    y: 50,
                    width: 500,
                    height: 500,
                },
                title: "Bottom Window".to_string(),
                class: "tiled".to_string(),
            },
        ];

        let matched = find_all_snapped_windows(&windows, 150.0, 150.0);
        assert_eq!(matched.len(), 2);
        assert_eq!(matched[0].class, "floating");
        assert_eq!(matched[1].class, "tiled");

        let single = find_all_snapped_windows(&windows, 60.0, 60.0);
        assert_eq!(single.len(), 1);
        assert_eq!(single[0].class, "tiled");
    }
}
