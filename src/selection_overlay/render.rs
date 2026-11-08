use super::geometry::{SelectionMode, SnappedWindow};
use gtk4::cairo;

pub fn draw_mode_pill(cr: &cairo::Context, mode: SelectionMode, screen_w: f64) {
    let mode_str = mode.title();
    let text = match mode {
        SelectionMode::WindowSnap => {
            format!(
                "[Space] Mode: {}  •  Click to capture, Drag for box",
                mode_str
            )
        }
        SelectionMode::RegionSelection => {
            format!("[Space] Mode: {}  •  Drag custom crop box", mode_str)
        }
        SelectionMode::FullMonitor => {
            format!(
                "[Space] Mode: {}  •  Click anywhere to capture full screen",
                mode_str
            )
        }
    };

    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    cr.set_font_size(12.0);

    let extents = match cr.text_extents(&text) {
        Ok(ext) => ext,
        Err(_) => return,
    };

    let pill_w = extents.width() + 28.0;
    let pill_h = 28.0;
    let pill_x = (screen_w - pill_w) / 2.0;
    let pill_y = 14.0;

    cr.set_source_rgba(0.08, 0.10, 0.14, 0.92);
    draw_rounded_rect(cr, pill_x, pill_y, pill_w, pill_h, 7.0);
    let _ = cr.fill();

    cr.set_source_rgba(0.23, 0.51, 0.96, 0.80);
    cr.set_line_width(1.5);
    draw_rounded_rect(cr, pill_x, pill_y, pill_w, pill_h, 7.0);
    let _ = cr.stroke();

    cr.set_source_rgba(0.95, 0.96, 0.99, 1.0);
    let text_x = pill_x + 14.0;
    let text_y = pill_y + (pill_h + extents.height()) / 2.0;
    cr.move_to(text_x, text_y);
    let _ = cr.show_text(&text);
}

pub fn draw_fullscreen_hint(cr: &cairo::Context, screen_w: f64, screen_h: f64) {
    let text = format!(
        "Full Screen ({} × {})  •  Click to capture",
        screen_w.round() as i32,
        screen_h.round() as i32
    );
    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    cr.set_font_size(14.0);

    let extents = match cr.text_extents(&text) {
        Ok(ext) => ext,
        Err(_) => return,
    };

    let pill_w = extents.width() + 32.0;
    let pill_h = 36.0;
    let pill_x = (screen_w - pill_w) / 2.0;
    let pill_y = (screen_h - pill_h) / 2.0;

    cr.set_source_rgba(0.08, 0.10, 0.14, 0.94);
    draw_rounded_rect(cr, pill_x, pill_y, pill_w, pill_h, 9.0);
    let _ = cr.fill();

    cr.set_source_rgba(0.23, 0.51, 0.96, 0.90);
    cr.set_line_width(2.0);
    draw_rounded_rect(cr, pill_x, pill_y, pill_w, pill_h, 9.0);
    let _ = cr.stroke();

    cr.set_source_rgba(0.95, 0.96, 0.99, 1.0);
    let text_x = pill_x + 16.0;
    let text_y = pill_y + (pill_h + extents.height()) / 2.0;
    cr.move_to(text_x, text_y);
    let _ = cr.show_text(&text);
}

pub fn draw_corner_brackets(
    cr: &cairo::Context,
    rx: f64,
    ry: f64,
    rw: f64,
    rh: f64,
    max_len: f64,
    line_width: f64,
) {
    cr.set_source_rgba(0.55, 0.75, 1.0, 1.0);
    cr.set_line_width(line_width);
    let corner_len = max_len.min(rw / 3.0).min(rh / 3.0);

    // Top-left
    cr.move_to(rx, ry + corner_len);
    cr.line_to(rx, ry);
    cr.line_to(rx + corner_len, ry);

    // Top-right
    cr.move_to(rx + rw - corner_len, ry);
    cr.line_to(rx + rw, ry);
    cr.line_to(rx + rw, ry + corner_len);

    // Bottom-left
    cr.move_to(rx, ry + rh - corner_len);
    cr.line_to(rx, ry + rh);
    cr.line_to(rx + corner_len, ry + rh);

    // Bottom-right
    cr.move_to(rx + rw - corner_len, ry + rh);
    cr.line_to(rx + rw, ry + rh);
    cr.line_to(rx + rw, ry + rh - corner_len);
    let _ = cr.stroke();
}

pub fn draw_rounded_rect(cr: &cairo::Context, x: f64, y: f64, w: f64, h: f64, r: f64) {
    let r = r.min(w / 2.0).min(h / 2.0);
    cr.new_sub_path();
    cr.arc(x + w - r, y + r, r, -std::f64::consts::FRAC_PI_2, 0.0);
    cr.arc(x + w - r, y + h - r, r, 0.0, std::f64::consts::FRAC_PI_2);
    cr.arc(
        x + r,
        y + h - r,
        r,
        std::f64::consts::FRAC_PI_2,
        std::f64::consts::PI,
    );
    cr.arc(
        x + r,
        y + r,
        r,
        std::f64::consts::PI,
        3.0 * std::f64::consts::FRAC_PI_2,
    );
    cr.close_path();
}

pub fn draw_snap_badge(
    cr: &cairo::Context,
    snapped: &SnappedWindow,
    cycle_index: usize,
    total_overlapping: usize,
    screen_w: f64,
) {
    let rx = snapped.local_rect.x as f64;
    let ry = snapped.local_rect.y as f64;
    let rw = snapped.local_rect.width as f64;

    let display_title = if !snapped.title.trim().is_empty() {
        let trimmed = snapped.title.trim();
        if trimmed.chars().count() > 30 {
            let truncated: String = trimmed.chars().take(28).collect();
            format!("{}…", truncated)
        } else {
            trimmed.to_string()
        }
    } else if !snapped.class.trim().is_empty() {
        snapped.class.trim().to_string()
    } else {
        "Window".to_string()
    };

    let badge_text = if total_overlapping > 1 {
        format!(
            "[{}/{}] {} • {}×{}  (Tab/Scroll to cycle • Click to capture)",
            cycle_index + 1,
            total_overlapping,
            display_title,
            snapped.local_rect.width,
            snapped.local_rect.height
        )
    } else {
        format!(
            "{} • {}×{}  (Click to capture • Drag for custom)",
            display_title, snapped.local_rect.width, snapped.local_rect.height
        )
    };

    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    cr.set_font_size(12.0);

    let extents = match cr.text_extents(&badge_text) {
        Ok(ext) => ext,
        Err(_) => return,
    };

    let pill_w = extents.width() + 28.0;
    let pill_h = 28.0;

    let pill_y = if ry >= pill_h + 8.0 {
        ry - pill_h - 6.0
    } else {
        (ry + 10.0).min(screen_w)
    };
    let pill_x = (rx + (rw - pill_w) / 2.0).clamp(12.0, (screen_w - pill_w - 12.0).max(12.0));

    cr.set_source_rgba(0.08, 0.10, 0.14, 0.92);
    draw_rounded_rect(cr, pill_x, pill_y, pill_w, pill_h, 6.0);
    let _ = cr.fill();

    cr.set_source_rgba(0.35, 0.60, 1.0, 0.75);
    cr.set_line_width(1.5);
    draw_rounded_rect(cr, pill_x, pill_y, pill_w, pill_h, 6.0);
    let _ = cr.stroke();

    cr.set_source_rgba(0.95, 0.96, 0.99, 1.0);
    let text_x = pill_x + 14.0;
    let text_y = pill_y + (pill_h + extents.height()) / 2.0;
    cr.move_to(text_x, text_y);
    let _ = cr.show_text(&badge_text);
}

pub fn draw_dimension_badge(
    cr: &cairo::Context,
    rx: f64,
    ry: f64,
    rw: f64,
    rh: f64,
    screen_w: f64,
    screen_h: f64,
) {
    if rw < 40.0 || rh < 24.0 {
        return;
    }

    let dim_text = format!("{} × {}", rw.round() as i32, rh.round() as i32);
    cr.select_font_face("Sans", cairo::FontSlant::Normal, cairo::FontWeight::Bold);
    cr.set_font_size(11.0);

    let extents = match cr.text_extents(&dim_text) {
        Ok(ext) => ext,
        Err(_) => return,
    };

    let pill_w = extents.width() + 16.0;
    let pill_h = 22.0;

    let pill_y = if ry + rh + pill_h + 8.0 <= screen_h {
        ry + rh + 6.0
    } else if ry >= pill_h + 8.0 {
        ry - pill_h - 6.0
    } else {
        ry + rh - pill_h - 6.0
    };
    let pill_x = (rx + rw - pill_w).clamp(6.0, screen_w - pill_w - 6.0);

    cr.set_source_rgba(0.08, 0.10, 0.14, 0.90);
    draw_rounded_rect(cr, pill_x, pill_y, pill_w, pill_h, 5.0);
    let _ = cr.fill();

    cr.set_source_rgba(0.35, 0.60, 1.0, 0.60);
    cr.set_line_width(1.0);
    draw_rounded_rect(cr, pill_x, pill_y, pill_w, pill_h, 5.0);
    let _ = cr.stroke();

    cr.set_source_rgba(0.92, 0.94, 0.98, 1.0);
    let text_x = pill_x + 8.0;
    let text_y = pill_y + (pill_h + extents.height()) / 2.0;
    cr.move_to(text_x, text_y);
    let _ = cr.show_text(&dim_text);
}
