use gtk4::gdk;
use gtk4::prelude::*;

pub const PROMPT_WIDTH: i32 = 440;
pub const PROMPT_HEIGHT: i32 = 280;
pub const CURSOR_GAP: i32 = 18;
pub const EDGE_PADDING: i32 = 12;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PromptPosition {
    pub left: i32,
    pub top: i32,
}

pub fn position_prompt(
    cursor: (i32, i32),
    monitor: gdk::Rectangle,
    prompt_size: (i32, i32),
) -> PromptPosition {
    let cursor_x = cursor.0 - monitor.x();
    let cursor_y = cursor.1 - monitor.y();
    let available_width = monitor.width().max(0);
    let available_height = monitor.height().max(0);

    let left = if cursor_x + CURSOR_GAP + prompt_size.0 <= available_width - EDGE_PADDING {
        cursor_x + CURSOR_GAP
    } else {
        cursor_x - CURSOR_GAP - prompt_size.0
    };
    let top = if cursor_y + CURSOR_GAP + prompt_size.1 <= available_height - EDGE_PADDING {
        cursor_y + CURSOR_GAP
    } else {
        cursor_y - CURSOR_GAP - prompt_size.1
    };

    PromptPosition {
        left: left.clamp(
            EDGE_PADDING,
            (available_width - prompt_size.0 - EDGE_PADDING).max(EDGE_PADDING),
        ),
        top: top.clamp(
            EDGE_PADDING,
            (available_height - prompt_size.1 - EDGE_PADDING).max(EDGE_PADDING),
        ),
    }
}

pub fn find_monitor(
    display: &gdk::Display,
    cursor: (i32, i32),
) -> Option<(gdk::Monitor, gdk::Rectangle)> {
    let monitors = display.monitors();
    let mut first_monitor = None;
    for index in 0..monitors.n_items() {
        let monitor = monitors.item(index)?.downcast::<gdk::Monitor>().ok()?;
        let geometry = monitor.geometry();
        if first_monitor.is_none() {
            first_monitor = Some((monitor.clone(), geometry));
        }

        if cursor.0 >= geometry.x()
            && cursor.1 >= geometry.y()
            && cursor.0 < geometry.x() + geometry.width()
            && cursor.1 < geometry.y() + geometry.height()
        {
            return Some((monitor, geometry));
        }
    }

    first_monitor
}

#[cfg(test)]
mod tests {
    use super::*;
    use gtk4::gdk;

    #[test]
    fn prompt_is_placed_beside_cursor_when_room_is_available() {
        let monitor = gdk::Rectangle::new(0, 0, 1920, 1080);
        assert_eq!(
            position_prompt((400, 300), monitor, (380, 220)),
            PromptPosition {
                left: 418,
                top: 318
            }
        );
    }

    #[test]
    fn prompt_flips_and_clamps_near_monitor_edges() {
        let monitor = gdk::Rectangle::new(0, 0, 1920, 1080);
        assert_eq!(
            position_prompt((1900, 1060), monitor, (380, 220)),
            PromptPosition {
                left: 1502,
                top: 822
            }
        );
    }

    #[test]
    fn prompt_uses_monitor_local_coordinates() {
        let monitor = gdk::Rectangle::new(-1920, 0, 1920, 1080);
        assert_eq!(
            position_prompt((-1800, 300), monitor, (380, 220)),
            PromptPosition {
                left: 138,
                top: 318
            }
        );
    }
}
