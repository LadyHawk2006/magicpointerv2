use anyhow::Result;
use hyprland::data::{Client, CursorPosition};
use hyprland::shared::{HyprData, HyprDataActiveOptional};

#[derive(Debug, Clone)]
pub struct HyprState {
    pub cursor: (i32, i32),
    pub active_window: Option<WindowGeometry>,
}

#[derive(Debug, Clone)]
pub struct WindowGeometry {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub title: String,
    pub class: String,
}

pub fn get_hyprland_state() -> Result<HyprState> {
    let cursor_pos = CursorPosition::get()?;
    let client = Client::get_active()?;

    let active_window = client.map(|c| WindowGeometry {
        x: c.at.0 as i32,
        y: c.at.1 as i32,
        width: c.size.0 as i32,
        height: c.size.1 as i32,
        title: c.title,
        class: c.class,
    });

    Ok(HyprState {
        cursor: (cursor_pos.x as i32, cursor_pos.y as i32),
        active_window,
    })
}
