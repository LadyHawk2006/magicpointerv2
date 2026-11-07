//! Application GTK stylesheets bundled into a single binary constant.

pub const APP_CSS: &str = concat!(
    include_str!("base.css"),
    include_str!("settings.css"),
    include_str!("prompt.css"),
    include_str!("annotation.css"),
    include_str!("chat.css"),
    include_str!("markdown.css"),
);

#[cfg(test)]
mod tests {
    use super::APP_CSS;

    #[test]
    fn app_css_is_non_empty_and_contains_expected_classes() {
        assert!(!APP_CSS.is_empty());
        assert!(APP_CSS.contains(".prompt-card"));
        assert!(APP_CSS.contains(".annotation-modal-card"));
        assert!(APP_CSS.contains(".magic-card"));
        assert!(APP_CSS.contains(".settings-window"));
        assert!(APP_CSS.contains(".markdown-content"));
    }
}
