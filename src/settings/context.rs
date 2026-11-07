use std::fs;

use crate::settings::AppSettings;

pub fn compose_spatial_prompt(
    settings: &AppSettings,
    active_window_title: Option<&str>,
    prompt: &str,
) -> String {
    if !settings.spatial.enabled {
        return prompt.to_string();
    }

    let mut context = Vec::new();
    if settings.spatial.attach_hardware_info {
        context.push(hardware_summary());
    }
    if settings.spatial.attach_active_window_title
        && let Some(title) = active_window_title.filter(|title| !title.trim().is_empty())
    {
        context.push(format!("Active window: {}", title.trim()));
    }
    if !settings.spatial.custom_text.trim().is_empty() {
        context.push(format!(
            "User-provided context: {}",
            settings.spatial.custom_text.trim()
        ));
    }
    if context.is_empty() {
        return prompt.to_string();
    }

    format!(
        "Additional context:\n{}\n\nUser request:\n{}",
        context.join("\n"),
        prompt.trim()
    )
}

fn hardware_summary() -> String {
    let cpu = fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|contents| {
            contents.lines().find_map(|line| {
                line.split_once(':')
                    .filter(|(key, _)| matches!(key.trim(), "model name" | "Hardware"))
                    .map(|(_, value)| value.trim().to_string())
            })
        })
        .unwrap_or_else(|| "unavailable".to_string());
    let memory = fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|contents| {
            contents.lines().find_map(|line| {
                line.strip_prefix("MemTotal:")
                    .and_then(|value| value.split_whitespace().next())
                    .and_then(|value| value.parse::<u64>().ok())
                    .map(|kilobytes| format!("{} GiB", kilobytes / 1_048_576))
            })
        })
        .unwrap_or_else(|| "unavailable".to_string());

    format!(
        "Hardware: {} {}, CPU: {}, Memory: {}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        cpu,
        memory
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spatial_context_is_disabled_by_default() {
        let settings = AppSettings::default();
        assert_eq!(
            compose_spatial_prompt(&settings, Some("Terminal"), "Explain this"),
            "Explain this"
        );
    }

    #[test]
    fn spatial_context_attaches_selected_data_to_each_prompt() {
        let mut settings = AppSettings::default();
        settings.spatial.enabled = true;
        settings.spatial.attach_active_window_title = true;
        settings.spatial.custom_text = "  Project: Magic Pointer  ".to_string();

        let prompt = compose_spatial_prompt(&settings, Some(" Terminal "), "  Explain this  ");
        assert!(prompt.contains("Active window: Terminal"));
        assert!(prompt.contains("User-provided context: Project: Magic Pointer"));
        assert!(prompt.ends_with("User request:\nExplain this"));
    }

    #[test]
    fn spatial_hardware_toggle_adds_local_hardware_summary() {
        let mut settings = AppSettings::default();
        settings.spatial.enabled = true;
        settings.spatial.attach_hardware_info = true;

        let prompt = compose_spatial_prompt(&settings, None, "Question");
        assert!(prompt.contains("Hardware:"));
        assert!(prompt.contains(std::env::consts::ARCH));
        assert!(prompt.ends_with("User request:\nQuestion"));
    }
}
