use anyhow::Result;
use hyprland::data::{Client, Clients, CursorPosition, Monitors};
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
    pub address: String,
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
        address: c.address.to_string(),
    });

    Ok(HyprState {
        cursor: (cursor_pos.x as i32, cursor_pos.y as i32),
        active_window,
    })
}

pub fn get_visible_windows() -> Result<Vec<WindowGeometry>> {
    let clients = Clients::get()?;
    let monitors = Monitors::get().ok();

    let active_workspaces: Vec<i32> = monitors
        .map(|mons| {
            mons.iter()
                .flat_map(|m| {
                    let mut ids = vec![m.active_workspace.id];
                    if m.special_workspace.id != 0 {
                        ids.push(m.special_workspace.id);
                    }
                    ids
                })
                .collect()
        })
        .unwrap_or_default();

    let mut visible_clients: Vec<_> = clients
        .into_iter()
        .filter(|c| {
            if !c.mapped || c.size.0 <= 0 || c.size.1 <= 0 {
                return false;
            }
            // Skip magicpointer's own surfaces
            if c.class.starts_with("com.magicpointerv2") || c.class.starts_with("magicpointerv2") {
                return false;
            }
            if active_workspaces.is_empty() {
                true
            } else {
                c.pinned || active_workspaces.contains(&c.workspace.id)
            }
        })
        .collect();

    // Sort: floating windows first, then by focus_history_id ascending (topmost first)
    visible_clients.sort_by(|a, b| {
        b.floating
            .cmp(&a.floating)
            .then_with(|| a.focus_history_id.cmp(&b.focus_history_id))
    });

    Ok(visible_clients
        .into_iter()
        .map(|c| WindowGeometry {
            x: c.at.0 as i32,
            y: c.at.1 as i32,
            width: c.size.0 as i32,
            height: c.size.1 as i32,
            title: c.title,
            class: c.class,
            address: c.address.to_string(),
        })
        .collect())
}
