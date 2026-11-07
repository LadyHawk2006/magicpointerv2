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
    let distribution = fs::read_to_string("/etc/os-release")
        .or_else(|_| fs::read_to_string("/usr/lib/os-release"))
        .ok()
        .and_then(|contents| parse_os_release(&contents))
        .unwrap_or_else(|| "unavailable".to_string());
    let kernel = fs::read_to_string("/proc/sys/kernel/osrelease")
        .map(|release| release.trim().to_string())
        .ok()
        .filter(|release| !release.is_empty())
        .unwrap_or_else(|| "unavailable".to_string());
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
        "System: {}, Kernel: {}, Architecture: {}, CPU: {}, Memory: {}",
        distribution,
        kernel,
        std::env::consts::ARCH,
        cpu,
        memory
    )
}

fn parse_os_release(contents: &str) -> Option<String> {
    let mut name = None;
    let mut version = None;
    let mut pretty_name = None;
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let Some((key, raw_value)) = line.split_once('=') else {
            continue;
        };
        let value = unquote_os_release_value(raw_value.trim());
        match key {
            "PRETTY_NAME" => pretty_name = Some(value),
            "NAME" => name = Some(value),
            "VERSION_ID" => version = Some(value),
            _ => {}
        }
    }

    pretty_name.filter(|value| !value.is_empty()).or_else(|| {
        let name = name.filter(|value| !value.is_empty())?;
        match version.filter(|value| !value.is_empty()) {
            Some(version) => Some(format!("{name} {version}")),
            None => Some(name),
        }
    })
}

fn unquote_os_release_value(value: &str) -> String {
    if let Some(value) = value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
    {
        return value.replace("\\\"", "\"").replace("\\\\", "\\");
    }
    if let Some(value) = value
        .strip_prefix('\'')
        .and_then(|value| value.strip_suffix('\''))
    {
        return value.to_string();
    }
    value.to_string()
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
        assert!(prompt.contains("System:"));
        assert!(prompt.contains("Kernel:"));
        assert!(prompt.contains(std::env::consts::ARCH));
        assert!(prompt.ends_with("User request:\nQuestion"));
    }

    #[test]
    fn os_release_uses_actual_pretty_name_instead_of_a_distribution_assumption() {
        assert_eq!(
            parse_os_release(
                "NAME=\"Fedora Linux\"\nVERSION_ID=43\nPRETTY_NAME=\"Fedora Linux 43 (Workstation Edition)\"\n"
            ),
            Some("Fedora Linux 43 (Workstation Edition)".to_string())
        );
    }

    #[test]
    fn os_release_falls_back_to_name_and_version() {
        assert_eq!(
            parse_os_release("NAME='Debian GNU/Linux'\nVERSION_ID=\"13\"\n"),
            Some("Debian GNU/Linux 13".to_string())
        );
    }

    #[test]
    fn os_release_unquotes_escaped_values() {
        assert_eq!(
            parse_os_release("PRETTY_NAME=\"Linux \\\"Example\\\" \\\\ OS\"\n"),
            Some("Linux \"Example\" \\ OS".to_string())
        );
    }
}
