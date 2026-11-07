use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context, Result, anyhow};

use super::{AppSettings, HyprlandWindowSettings, WindowPosition};

const APP_REQUIRE: &str = "require(\"magicpointerv2\")";
const EDGE_PADDING: i32 = 24;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MonitorBounds {
    width: i32,
    height: i32,
    top: i32,
    right: i32,
    bottom: i32,
    left: i32,
}

#[derive(serde::Deserialize)]
struct MonitorInfo {
    width: i32,
    height: i32,
    #[serde(default = "default_scale")]
    scale: f64,
    #[serde(default)]
    focused: bool,
    #[serde(default)]
    disabled: bool,
    #[serde(default)]
    reserved: [i32; 4],
}

pub fn install(settings: &AppSettings) -> Result<()> {
    let bounds = focused_monitor_bounds()?;
    let config_dir = hyprland_config_dir()?;
    fs::create_dir_all(&config_dir)
        .with_context(|| format!("Could not create {}", config_dir.display()))?;
    let module_path = config_dir.join("magicpointerv2.lua");
    let rendered = render_config(&settings.hyprland_window, bounds);
    let module_changed = match fs::read_to_string(&module_path) {
        Ok(existing) if existing == rendered => false,
        Ok(_) => {
            write_atomic(&module_path, &rendered)
                .with_context(|| format!("Could not update {}", module_path.display()))?;
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            write_atomic(&module_path, &rendered)
                .with_context(|| format!("Could not create {}", module_path.display()))?;
            true
        }
        Err(error) => {
            return Err(error).with_context(|| format!("Could not read {}", module_path.display()));
        }
    };

    let main_config = config_dir.join("hyprland.lua");
    let require_added = ensure_module_required(&main_config)
        .with_context(|| format!("Could not add Magic Pointer to {}", main_config.display()))?;
    if !module_changed && !require_added {
        return Ok(());
    }

    let output = Command::new("hyprctl")
        .arg("reload")
        .output()
        .context("Could not run `hyprctl reload` to apply Magic Pointer window rules")?;
    if !output.status.success() {
        return Err(anyhow!(
            "Magic Pointer's Hyprland files were written, but Hyprland rejected the reload: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(())
}

pub fn reset() -> Result<()> {
    let config_dir = hyprland_config_dir()?;
    let main_config = config_dir.join("hyprland.lua");
    let module_path = config_dir.join("magicpointerv2.lua");

    let require_removed = remove_module_require(&main_config).with_context(|| {
        format!(
            "Could not remove Magic Pointer from {}",
            main_config.display()
        )
    })?;
    let module_removed = match fs::remove_file(&module_path) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            return Err(error)
                .with_context(|| format!("Could not remove {}", module_path.display()));
        }
    };

    if require_removed || module_removed {
        let output = Command::new("hyprctl").arg("reload").output().context(
            "Could not run `hyprctl reload` to remove Magic Pointer's Hyprland integration",
        )?;
        if !output.status.success() {
            return Err(anyhow!(
                "Magic Pointer's Hyprland files were removed, but Hyprland rejected the reload: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
    }

    tracing::info!(
        require_removed,
        module_removed,
        "Removed Magic Pointer Hyprland integration"
    );
    Ok(())
}

fn remove_module_require(path: &Path) -> Result<bool> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(error) => {
            return Err(error).with_context(|| {
                format!(
                    "Could not read Hyprland Lua configuration {}",
                    path.display()
                )
            });
        }
    };
    let filtered = contents
        .split_inclusive('\n')
        .filter(|line| line.trim() != APP_REQUIRE)
        .collect::<String>();
    if filtered == contents {
        return Ok(false);
    }
    write_atomic(path, &filtered)?;
    Ok(true)
}

fn focused_monitor_bounds() -> Result<MonitorBounds> {
    let output = Command::new("hyprctl")
        .args(["monitors", "-j"])
        .output()
        .context("Could not query Hyprland monitor geometry")?;
    if !output.status.success() {
        return Err(anyhow!(
            "Hyprland monitor query failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let monitors: Vec<MonitorInfo> = serde_json::from_slice(&output.stdout)
        .context("Could not parse Hyprland monitor geometry")?;
    let monitor = monitors
        .into_iter()
        .find(|monitor| monitor.focused && !monitor.disabled)
        .ok_or_else(|| anyhow!("Hyprland did not report a focused, active monitor"))?;
    let scale = if monitor.scale.is_finite() && monitor.scale > 0.0 {
        monitor.scale
    } else {
        1.0
    };
    let [top, right, bottom, left] = monitor.reserved;
    Ok(MonitorBounds {
        width: (f64::from(monitor.width) / scale).round() as i32,
        height: (f64::from(monitor.height) / scale).round() as i32,
        top: (f64::from(top) / scale).round() as i32,
        right: (f64::from(right) / scale).round() as i32,
        bottom: (f64::from(bottom) / scale).round() as i32,
        left: (f64::from(left) / scale).round() as i32,
    })
}

fn default_scale() -> f64 {
    1.0
}

fn render_config(settings: &HyprlandWindowSettings, monitor: MonitorBounds) -> String {
    let width = settings.width as i32;
    let height = settings.height as i32;
    let border_size = if settings.show_border {
        settings.border_size
    } else {
        0
    };
    let opacity = f64::from(settings.opacity_percent) / 100.0;
    let mut rule = format!(
        r#"-- Managed by Magic Pointer v2. Edit window options in the application's Settings.
hl.window_rule({{
    name = "magicpointerv2-chat-window",
    match = {{
        class = [[^com\.magicpointerv2\.tool$]],
        initial_title = [[^Magic Pointer$]],
    }},
    float = true,
    size = {{ {width}, {height} }},
    border_size = {border_size},
    no_shadow = {no_shadow},
    no_glow = {no_glow},
    no_blur = {no_blur},
    opacity = "{opacity:.2} override",
    no_anim = false,
    no_dim = true,
    decorate = true,
    focus_on_activate = true,
"#,
        no_shadow = !settings.show_shadow,
        no_glow = !settings.glow,
        no_blur = !settings.blur,
    );

    match settings.position {
        WindowPosition::Center => rule.push_str("    center = true,\n"),
        position => {
            let (x, y) = position_coordinates(position, width, height, monitor);
            rule.push_str(&format!("    move = {{ {x}, {y} }},\n"));
        }
    }
    rule.push_str(
        "})\n\nhl.window_rule({\n    name = \"magicpointerv2-settings-window\",\n    match = {\n        class = [[^com\\.magicpointerv2\\.tool$]],\n        initial_title = [[^Magic Pointer Settings$]],\n    },\n    float = true,\n    center = true,\n})\n\nhl.bind(\"Home\", hl.dsp.exec_cmd(\"magicpointerv2\"))\n",
    );
    rule
}

fn position_coordinates(
    position: WindowPosition,
    width: i32,
    height: i32,
    monitor: MonitorBounds,
) -> (i32, i32) {
    let available_width = (monitor.width - monitor.left - monitor.right).max(0);
    let available_height = (monitor.height - monitor.top - monitor.bottom).max(0);
    let max_x = (monitor.width - monitor.right - width - EDGE_PADDING).max(monitor.left);
    let max_y = (monitor.height - monitor.bottom - height - EDGE_PADDING).max(monitor.top);
    let min_x = (monitor.left + EDGE_PADDING).min(max_x);
    let min_y = (monitor.top + EDGE_PADDING).min(max_y);

    let x = match position {
        WindowPosition::TopLeft | WindowPosition::BottomLeft => min_x,
        WindowPosition::TopRight | WindowPosition::BottomRight => max_x,
        WindowPosition::Center => {
            (monitor.left + (available_width - width) / 2).clamp(min_x, max_x)
        }
    };
    let y = match position {
        WindowPosition::TopLeft | WindowPosition::TopRight => min_y,
        WindowPosition::BottomLeft | WindowPosition::BottomRight => max_y,
        WindowPosition::Center => {
            (monitor.top + (available_height - height) / 2).clamp(min_y, max_y)
        }
    };
    (x, y)
}

fn ensure_module_required(path: &Path) -> Result<bool> {
    let mut contents = fs::read_to_string(path).with_context(|| {
        format!(
            "Could not read Hyprland Lua configuration {}",
            path.display()
        )
    })?;
    if contents.lines().any(|line| line.trim() == APP_REQUIRE) {
        return Ok(false);
    }

    if !contents.is_empty() && !contents.ends_with('\n') {
        contents.push('\n');
    }
    contents.push_str(APP_REQUIRE);
    contents.push('\n');
    write_atomic(path, &contents).with_context(|| {
        format!(
            "Could not update Hyprland Lua configuration {}",
            path.display()
        )
    })?;
    Ok(true)
}

fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    use std::io::Write;

    let target_path = match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => fs::canonicalize(path)
            .with_context(|| format!("Could not resolve symlink {}", path.display()))?,
        Ok(_) => path.to_path_buf(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => path.to_path_buf(),
        Err(error) => {
            return Err(error).with_context(|| format!("Could not inspect {}", path.display()));
        }
    };
    let permissions = match fs::metadata(&target_path) {
        Ok(metadata) => Some(metadata.permissions()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(error).with_context(|| format!("Could not inspect {}", path.display()));
        }
    };
    let temporary_path = target_path.with_extension(format!("lua.{}.tmp", std::process::id()));
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(&temporary_path)
        .with_context(|| format!("Could not write {}", temporary_path.display()))?;
    file.write_all(contents.as_bytes())
        .with_context(|| format!("Could not write {}", temporary_path.display()))?;
    if let Some(permissions) = permissions {
        file.set_permissions(permissions)
            .with_context(|| format!("Could not preserve permissions for {}", path.display()))?;
    }
    file.sync_all()
        .with_context(|| format!("Could not sync {}", temporary_path.display()))?;
    fs::rename(&temporary_path, &target_path)
        .with_context(|| format!("Could not replace {}", path.display()))?;
    Ok(())
}

fn hyprland_config_dir() -> Result<PathBuf> {
    if let Some(config_home) = std::env::var_os("XDG_CONFIG_HOME") {
        return Ok(PathBuf::from(config_home).join("hypr"));
    }
    let home = std::env::var_os("HOME").ok_or_else(|| anyhow!("HOME is not set"))?;
    Ok(Path::new(&home).join(".config").join("hypr"))
}

#[cfg(test)]
mod tests {
    use super::{MonitorBounds, position_coordinates, remove_module_require, render_config};
    use crate::settings::{HyprlandWindowSettings, WindowPosition};
    use std::fs;

    fn monitor() -> MonitorBounds {
        MonitorBounds {
            width: 1920,
            height: 1080,
            top: 36,
            right: 2,
            bottom: 2,
            left: 2,
        }
    }

    #[test]
    fn position_presets_keep_the_window_inside_monitor_edges() {
        assert_eq!(
            position_coordinates(WindowPosition::BottomLeft, 542, 668, monitor()),
            (26, 386)
        );
        assert_eq!(
            position_coordinates(WindowPosition::BottomRight, 542, 668, monitor()),
            (1352, 386)
        );
        assert_eq!(
            position_coordinates(WindowPosition::TopLeft, 542, 668, monitor()),
            (26, 60)
        );
        assert_eq!(
            position_coordinates(WindowPosition::TopRight, 542, 668, monitor()),
            (1352, 60)
        );
    }

    #[test]
    fn generated_rules_reflect_window_settings_and_home_keybind() {
        let config = render_config(&HyprlandWindowSettings::default(), monitor());
        assert!(config.contains("size = { 542, 668 }"));
        assert!(config.contains("border_size = 0"));
        assert!(config.contains("no_shadow = true"));
        assert!(config.contains("no_glow = true"));
        assert!(config.contains("no_blur = false"));
        assert!(config.contains("opacity = \"0.95 override\""));
        assert!(config.contains("move = { 1352, 386 }"));
        assert!(config.contains("initial_title = [[^Magic Pointer$]]"));
        assert!(config.contains("initial_title = [[^Magic Pointer Settings$]]"));
        assert!(config.contains("name = \"magicpointerv2-settings-window\""));
        assert!(config.contains("    center = true,"));
        assert!(config.contains("hl.bind(\"Home\", hl.dsp.exec_cmd(\"magicpointerv2\"))"));
    }

    #[test]
    fn reset_removes_only_the_exact_managed_require_line() {
        let path = std::env::temp_dir().join(format!(
            "magicpointerv2-hyprland-reset-{}-{}.lua",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let original =
            "-- keep this comment\nrequire(\"magicpointerv2\")\nrequire(\"other-module\")\n";
        fs::write(&path, original).unwrap();

        assert!(remove_module_require(&path).unwrap());
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "-- keep this comment\nrequire(\"other-module\")\n"
        );
        assert!(!remove_module_require(&path).unwrap());

        fs::remove_file(path).unwrap();
    }

    #[test]
    fn monitor_ipc_json_supplies_focused_display_geometry() {
        let monitors: Vec<super::MonitorInfo> = serde_json::from_str(
            r#"[{"width":1920,"height":1080,"scale":1.0,"focused":true,"disabled":false,"reserved":[36,2,2,2]}]"#,
        )
        .unwrap();
        let focused = monitors
            .into_iter()
            .find(|monitor| monitor.focused && !monitor.disabled)
            .unwrap();
        assert_eq!(focused.width, 1920);
        assert_eq!(focused.height, 1080);
        assert_eq!(focused.reserved, [36, 2, 2, 2]);
    }
}
