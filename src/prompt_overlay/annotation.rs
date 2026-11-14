use gtk4::cairo;
use gtk4::gdk;
use gtk4::glib;
use image::DynamicImage;

#[derive(Clone, Debug, PartialEq)]
pub enum Annotation {
    Pen(Vec<(f64, f64)>),
    Arrow { start: (f64, f64), end: (f64, f64) },
    Rect { start: (f64, f64), end: (f64, f64) },
    Redact { start: (f64, f64), end: (f64, f64) },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnnotationTool {
    Pen,
    Arrow,
    Rect,
    Redact,
}

pub fn apply_annotations_to_image(
    image: &DynamicImage,
    annotations: &[Annotation],
) -> DynamicImage {
    if annotations.is_empty() {
        return image.clone();
    }

    let width = image.width() as i32;
    let height = image.height() as i32;

    let mut surface = match cairo::ImageSurface::create(cairo::Format::ARgb32, width, height) {
        Ok(s) => s,
        Err(_) => return image.clone(),
    };

    let cr = match cairo::Context::new(&surface) {
        Ok(c) => c,
        Err(_) => return image.clone(),
    };

    let scale_x = width as f64;
    let scale_y = height as f64;

    cr.set_source_rgba(0.95, 0.22, 0.28, 0.95);
    cr.set_line_width((width.min(height) as f64 * 0.005).max(3.5));
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);

    for annot in annotations {
        match annot {
            Annotation::Pen(points) => {
                if points.len() < 2 {
                    if let Some(p) = points.first() {
                        cr.arc(
                            p.0 * scale_x,
                            p.1 * scale_y,
                            4.0,
                            0.0,
                            std::f64::consts::TAU,
                        );
                        let _ = cr.fill();
                    }
                    continue;
                }
                cr.move_to(points[0].0 * scale_x, points[0].1 * scale_y);
                for p in &points[1..] {
                    cr.line_to(p.0 * scale_x, p.1 * scale_y);
                }
                let _ = cr.stroke();
            }
            Annotation::Arrow { start, end } => {
                let x0 = start.0 * scale_x;
                let y0 = start.1 * scale_y;
                let x1 = end.0 * scale_x;
                let y1 = end.1 * scale_y;

                cr.move_to(x0, y0);
                cr.line_to(x1, y1);
                let _ = cr.stroke();

                let angle = (y1 - y0).atan2(x1 - x0);
                let head_len = (width.min(height) as f64 * 0.025).clamp(16.0, 36.0);
                let arrow_angle = std::f64::consts::PI / 6.0;

                let p1_x = x1 - head_len * (angle - arrow_angle).cos();
                let p1_y = y1 - head_len * (angle - arrow_angle).sin();
                let p2_x = x1 - head_len * (angle + arrow_angle).cos();
                let p2_y = y1 - head_len * (angle + arrow_angle).sin();

                cr.move_to(x1, y1);
                cr.line_to(p1_x, p1_y);
                cr.line_to(p2_x, p2_y);
                cr.close_path();
                let _ = cr.fill();
            }
            Annotation::Rect { start, end } => {
                cr.set_source_rgba(0.95, 0.22, 0.28, 0.95);
                let rx = start.0.min(end.0) * scale_x;
                let ry = start.1.min(end.1) * scale_y;
                let rw = (end.0 - start.0).abs() * scale_x;
                let rh = (end.1 - start.1).abs() * scale_y;

                cr.rectangle(rx, ry, rw, rh);
                let _ = cr.stroke();
            }
            Annotation::Redact { start, end } => {
                let rx = start.0.min(end.0) * scale_x;
                let ry = start.1.min(end.1) * scale_y;
                let rw = (end.0 - start.0).abs() * scale_x;
                let rh = (end.1 - start.1).abs() * scale_y;

                cr.set_source_rgba(0.0, 0.0, 0.0, 1.0);
                cr.rectangle(rx, ry, rw, rh);
                let _ = cr.fill();
            }
        }
    }

    drop(cr);

    let mut rgba_image = image.to_rgba8();
    if let Ok(data) = surface.data() {
        for (orig, annot_chunk) in rgba_image.chunks_exact_mut(4).zip(data.chunks_exact(4)) {
            let a = annot_chunk[3] as u32;
            if a > 0 {
                let inv_a = 255 - a;
                let r = annot_chunk[2] as u32;
                let g = annot_chunk[1] as u32;
                let b = annot_chunk[0] as u32;
                orig[0] = ((orig[0] as u32 * inv_a + r * 255) / 255).min(255) as u8;
                orig[1] = ((orig[1] as u32 * inv_a + g * 255) / 255).min(255) as u8;
                orig[2] = ((orig[2] as u32 * inv_a + b * 255) / 255).min(255) as u8;
                orig[3] = (orig[3] as u32 + a).min(255) as u8;
            }
        }
    }

    DynamicImage::ImageRgba8(rgba_image)
}

pub fn draw_annotations_on_context(
    cr: &cairo::Context,
    annotations: &[Annotation],
    active_drawing: Option<&Annotation>,
    width: f64,
    height: f64,
) {
    cr.set_source_rgba(0.95, 0.22, 0.28, 0.95);
    cr.set_line_width(3.0);
    cr.set_line_cap(cairo::LineCap::Round);
    cr.set_line_join(cairo::LineJoin::Round);

    for annot in annotations {
        match annot {
            Annotation::Pen(points) => {
                if points.len() < 2 {
                    continue;
                }
                cr.move_to(points[0].0 * width, points[0].1 * height);
                for p in &points[1..] {
                    cr.line_to(p.0 * width, p.1 * height);
                }
                let _ = cr.stroke();
            }
            Annotation::Arrow { start, end } => {
                let x0 = start.0 * width;
                let y0 = start.1 * height;
                let x1 = end.0 * width;
                let y1 = end.1 * height;

                cr.move_to(x0, y0);
                cr.line_to(x1, y1);
                let _ = cr.stroke();

                let angle = (y1 - y0).atan2(x1 - x0);
                let head_len = 14.0;
                let arrow_angle = std::f64::consts::PI / 6.0;

                let p1_x = x1 - head_len * (angle - arrow_angle).cos();
                let p1_y = y1 - head_len * (angle - arrow_angle).sin();
                let p2_x = x1 - head_len * (angle + arrow_angle).cos();
                let p2_y = y1 - head_len * (angle + arrow_angle).sin();

                cr.move_to(x1, y1);
                cr.line_to(p1_x, p1_y);
                cr.line_to(p2_x, p2_y);
                cr.close_path();
                let _ = cr.fill();
            }
            Annotation::Rect { start, end } => {
                cr.set_source_rgba(0.95, 0.22, 0.28, 0.95);
                let rx = start.0.min(end.0) * width;
                let ry = start.1.min(end.1) * height;
                let rw = (end.0 - start.0).abs() * width;
                let rh = (end.1 - start.1).abs() * height;

                cr.rectangle(rx, ry, rw, rh);
                let _ = cr.stroke();
            }
            Annotation::Redact { start, end } => {
                let rx = start.0.min(end.0) * width;
                let ry = start.1.min(end.1) * height;
                let rw = (end.0 - start.0).abs() * width;
                let rh = (end.1 - start.1).abs() * height;

                cr.set_source_rgba(0.04, 0.04, 0.04, 0.98);
                cr.rectangle(rx, ry, rw, rh);
                let _ = cr.fill();

                cr.set_source_rgba(0.3, 0.3, 0.3, 0.6);
                cr.set_line_width(1.0);
                cr.rectangle(rx, ry, rw, rh);
                let _ = cr.stroke();
            }
        }
    }

    if let Some(active) = active_drawing {
        match active {
            Annotation::Pen(points) => {
                cr.set_source_rgba(0.95, 0.22, 0.28, 0.95);
                if points.len() >= 2 {
                    cr.move_to(points[0].0 * width, points[0].1 * height);
                    for p in &points[1..] {
                        cr.line_to(p.0 * width, p.1 * height);
                    }
                    let _ = cr.stroke();
                }
            }
            Annotation::Arrow { start, end } => {
                cr.set_source_rgba(0.95, 0.22, 0.28, 0.95);
                let x0 = start.0 * width;
                let y0 = start.1 * height;
                let x1 = end.0 * width;
                let y1 = end.1 * height;

                cr.move_to(x0, y0);
                cr.line_to(x1, y1);
                let _ = cr.stroke();

                let angle = (y1 - y0).atan2(x1 - x0);
                let head_len = 14.0;
                let arrow_angle = std::f64::consts::PI / 6.0;

                let p1_x = x1 - head_len * (angle - arrow_angle).cos();
                let p1_y = y1 - head_len * (angle - arrow_angle).sin();
                let p2_x = x1 - head_len * (angle + arrow_angle).cos();
                let p2_y = y1 - head_len * (angle + arrow_angle).sin();

                cr.move_to(x1, y1);
                cr.line_to(p1_x, p1_y);
                cr.line_to(p2_x, p2_y);
                cr.close_path();
                let _ = cr.fill();
            }
            Annotation::Rect { start, end } => {
                cr.set_source_rgba(0.95, 0.22, 0.28, 0.95);
                let rx = start.0.min(end.0) * width;
                let ry = start.1.min(end.1) * height;
                let rw = (end.0 - start.0).abs() * width;
                let rh = (end.1 - start.1).abs() * height;

                cr.rectangle(rx, ry, rw, rh);
                let _ = cr.stroke();
            }
            Annotation::Redact { start, end } => {
                let rx = start.0.min(end.0) * width;
                let ry = start.1.min(end.1) * height;
                let rw = (end.0 - start.0).abs() * width;
                let rh = (end.1 - start.1).abs() * height;

                cr.set_source_rgba(0.08, 0.08, 0.08, 0.85);
                cr.rectangle(rx, ry, rw, rh);
                let _ = cr.fill();

                cr.set_source_rgba(1.0, 0.35, 0.35, 0.9);
                cr.set_line_width(1.5);
                cr.rectangle(rx, ry, rw, rh);
                let _ = cr.stroke();
            }
        }
    }
}

pub fn large_image_texture(image: &DynamicImage, max_w: u32, max_h: u32) -> gdk::MemoryTexture {
    let scaled = image.thumbnail(max_w, max_h).to_rgba8();
    let width = scaled.width();
    let height = scaled.height();
    let bytes = glib::Bytes::from_owned(scaled.into_raw());
    gdk::MemoryTexture::new(
        width as i32,
        height as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &bytes,
        width as usize * 4,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{DynamicImage, RgbaImage};

    #[test]
    fn apply_annotations_renders_onto_image_in_memory() {
        let img = DynamicImage::ImageRgba8(RgbaImage::new(100, 100));
        let annots = vec![
            Annotation::Pen(vec![(0.1, 0.1), (0.5, 0.5)]),
            Annotation::Arrow {
                start: (0.2, 0.2),
                end: (0.8, 0.8),
            },
            Annotation::Rect {
                start: (0.3, 0.3),
                end: (0.7, 0.7),
            },
        ];

        let result = apply_annotations_to_image(&img, &annots);
        assert_eq!(result.width(), 100);
        assert_eq!(result.height(), 100);

        let rgba = result.to_rgba8();
        let modified = rgba.pixels().any(|p| p[3] > 0);
        assert!(modified, "Annotations should modify pixels");
    }

    #[test]
    fn empty_annotations_return_untouched_image() {
        let img = DynamicImage::ImageRgba8(RgbaImage::new(50, 50));
        let result = apply_annotations_to_image(&img, &[]);
        assert_eq!(result.width(), 50);
        assert_eq!(result.height(), 50);
    }

    #[test]
    fn redaction_annotation_blacks_out_target_pixels_in_memory() {
        let mut img = RgbaImage::new(100, 100);
        for p in img.pixels_mut() {
            *p = image::Rgba([255, 255, 255, 255]);
        }
        let dynamic_img = DynamicImage::ImageRgba8(img);
        let annots = vec![Annotation::Redact {
            start: (0.2, 0.2),
            end: (0.8, 0.8),
        }];

        let result = apply_annotations_to_image(&dynamic_img, &annots);
        let rgba = result.to_rgba8();
        let center = rgba.get_pixel(50, 50);
        assert_eq!(center[0], 0);
        assert_eq!(center[1], 0);
        assert_eq!(center[2], 0);
        assert_eq!(center[3], 255);
    }
}
