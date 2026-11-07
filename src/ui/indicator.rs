use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::time::Duration;

use gtk4::glib;
use gtk4::prelude::*;

pub fn begin_fetch_indicator(
    fetch_count: &Rc<Cell<usize>>,
    glow: &gtk4::DrawingArea,
    animation: &Rc<RefCell<Option<glib::SourceId>>>,
    phase: &Rc<Cell<f64>>,
) {
    fetch_count.set(fetch_count.get() + 1);
    glow.set_visible(true);

    if animation.borrow().is_none() {
        let is_fetching = fetch_count.clone();
        let glow = glow.downgrade();
        let phase = phase.clone();
        let source = glib::timeout_add_local(Duration::from_millis(40), move || {
            if is_fetching.get() == 0 {
                return glib::ControlFlow::Break;
            }

            phase.set((phase.get() + 0.035) % std::f64::consts::TAU);
            if let Some(glow) = glow.upgrade() {
                glow.queue_draw();
            }
            glib::ControlFlow::Continue
        });
        *animation.borrow_mut() = Some(source);
    }
}

pub fn finish_fetch_indicator(
    fetch_count: &Rc<Cell<usize>>,
    glow: &gtk4::DrawingArea,
    animation: &Rc<RefCell<Option<glib::SourceId>>>,
) {
    let remaining = fetch_count.get().saturating_sub(1);
    fetch_count.set(remaining);
    if remaining == 0 {
        glow.set_visible(false);
        if let Some(source) = animation.borrow_mut().take() {
            source.remove();
        }
    }
}

pub fn draw_fetch_glow(
    context: &gtk4::cairo::Context,
    width: i32,
    height: i32,
    phase: f64,
) -> Result<(), gtk4::cairo::Error> {
    if width <= 0 || height <= 0 {
        return Ok(());
    }

    let pulse = 0.4 + 0.6 * ((phase.sin() + 1.0) / 2.0);
    let phase_turns = phase / std::f64::consts::TAU;
    let colors = [
        (1.0, 0.08, 0.20),
        (1.0, 0.48, 0.05),
        (0.95, 0.92, 0.12),
        (0.05, 0.88, 0.42),
        (0.12, 0.48, 1.0),
        (0.72, 0.18, 1.0),
    ];
    for (line_width, alpha) in [(12.0, 0.08), (7.0, 0.16), (2.0, 0.82)] {
        let gradient = gtk4::cairo::LinearGradient::new(0.0, 0.5, width as f64, 0.5);
        for step in 0..=60 {
            let offset = step as f64 / 60.0;
            let color_position = (offset + phase_turns).rem_euclid(1.0);
            let palette_position = color_position * colors.len() as f64;
            let color_index = palette_position.floor() as usize % colors.len();
            let next_color = (color_index + 1) % colors.len();
            let blend = palette_position.fract();
            let color = (
                colors[color_index].0 * (1.0 - blend) + colors[next_color].0 * blend,
                colors[color_index].1 * (1.0 - blend) + colors[next_color].1 * blend,
                colors[color_index].2 * (1.0 - blend) + colors[next_color].2 * blend,
            );
            gradient.add_color_stop_rgba(offset, color.0, color.1, color.2, alpha * pulse);
        }

        context.set_line_width(line_width);
        context.set_line_cap(gtk4::cairo::LineCap::Round);
        context.set_source(&gradient)?;
        context.move_to(0.0, 0.5);
        context.line_to(width as f64, 0.5);
        context.stroke()?;
    }
    Ok(())
}
