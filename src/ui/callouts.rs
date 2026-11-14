use gtk4::cairo;
use serde::Deserialize;

#[derive(Clone, Debug, PartialEq)]
pub struct VisualCallout {
    pub ymin: f64,
    pub xmin: f64,
    pub ymax: f64,
    pub xmax: f64,
    pub label: String,
}

#[derive(Deserialize)]
struct RawBox2d {
    box_2d: Option<Vec<f64>>,
    label: Option<String>,
}

pub fn parse_visual_callouts(text: &str) -> Vec<VisualCallout> {
    let mut callouts = Vec::new();

    let mut cursor = 0;
    while let Some(idx) = text[cursor..].find("box_2d") {
        let abs_idx = cursor + idx;
        let start = text[..abs_idx].rfind('{').unwrap_or(abs_idx);
        if let Some(end_offset) = text[abs_idx..].find('}') {
            let end = abs_idx + end_offset + 1;
            let candidate = &text[start..end];
            if let Ok(raw) = serde_json::from_str::<RawBox2d>(candidate) {
                if let Some(coords) = raw.box_2d {
                    if coords.len() == 4 {
                        let ymin = (coords[0] / 1000.0).clamp(0.0, 1.0);
                        let xmin = (coords[1] / 1000.0).clamp(0.0, 1.0);
                        let ymax = (coords[2] / 1000.0).clamp(0.0, 1.0);
                        let xmax = (coords[3] / 1000.0).clamp(0.0, 1.0);
                        if ymax > ymin && xmax > xmin {
                            let label = raw.label.unwrap_or_default().trim().to_string();
                            callouts.push(VisualCallout {
                                ymin,
                                xmin,
                                ymax,
                                xmax,
                                label,
                            });
                        }
                    }
                }
            }
        }
        cursor = abs_idx + 6;
    }

    callouts
}

pub fn draw_callouts_on_context(
    cr: &cairo::Context,
    callouts: &[VisualCallout],
    img_w: f64,
    img_h: f64,
    alloc_w: f64,
    alloc_h: f64,
) {
    if callouts.is_empty() || img_w <= 0.0 || img_h <= 0.0 || alloc_w <= 0.0 || alloc_h <= 0.0 {
        return;
    }

    let aspect_img = img_w / img_h;
    let aspect_alloc = alloc_w / alloc_h;
    let (dw, dh, dx, dy) = if aspect_img > aspect_alloc {
        let dw = alloc_w;
        let dh = dw / aspect_img;
        let dy = (alloc_h - dh) / 2.0;
        (dw, dh, 0.0, dy)
    } else {
        let dh = alloc_h;
        let dw = dh * aspect_img;
        let dx = (alloc_w - dw) / 2.0;
        (dw, dh, dx, 0.0)
    };

    for callout in callouts {
        let bx = dx + callout.xmin * dw;
        let by = dy + callout.ymin * dh;
        let bw = (callout.xmax - callout.xmin) * dw;
        let bh = (callout.ymax - callout.ymin) * dh;

        // Subtle translucent fill
        cr.set_source_rgba(0.0, 0.82, 1.0, 0.16);
        cr.rectangle(bx, by, bw, bh);
        let _ = cr.fill();

        // Bright border
        cr.set_source_rgba(0.0, 0.82, 1.0, 0.95);
        cr.set_line_width(2.0);
        cr.rectangle(bx, by, bw, bh);
        let _ = cr.stroke();

        if !callout.label.is_empty() {
            cr.set_font_size(11.0);
            if let Ok(extents) = cr.text_extents(&callout.label) {
                let pad_x = 5.0;
                let pad_y = 3.0;
                let badge_w = extents.width() + pad_x * 2.0;
                let badge_h = extents.height() + pad_y * 2.0;

                let badge_x = bx.clamp(dx, (dx + dw - badge_w).max(dx));
                let badge_y = if by - badge_h >= dy { by - badge_h } else { by };

                cr.set_source_rgba(0.0, 0.55, 0.85, 0.92);
                cr.rectangle(badge_x, badge_y, badge_w, badge_h);
                let _ = cr.fill();

                cr.set_source_rgba(1.0, 1.0, 1.0, 1.0);
                cr.move_to(badge_x + pad_x, badge_y + badge_h - pad_y);
                let _ = cr.show_text(&callout.label);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_box_2d_json_objects() {
        let text =
            r#"Here is the button: {"box_2d": [150, 200, 250, 400], "label": "Save Button"}"#;
        let callouts = parse_visual_callouts(text);
        assert_eq!(callouts.len(), 1);
        assert_eq!(callouts[0].ymin, 0.15);
        assert_eq!(callouts[0].xmin, 0.20);
        assert_eq!(callouts[0].ymax, 0.25);
        assert_eq!(callouts[0].xmax, 0.40);
        assert_eq!(callouts[0].label, "Save Button");
    }

    #[test]
    fn ignores_malformed_coordinates() {
        let text = r#"{"box_2d": [500, 500, 200, 200], "label": "inverted"}"#;
        let callouts = parse_visual_callouts(text);
        assert!(callouts.is_empty());
    }
}
