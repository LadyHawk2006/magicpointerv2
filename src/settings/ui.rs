use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::settings::{self, ApiKeyRecord, AppSettings, WindowPosition};

const AI_STUDIO_URL: &str = "https://aistudio.google.com/";

struct KeyRow {
    id: String,
    name: gtk4::Entry,
    primary: gtk4::CheckButton,
}

pub fn show_settings(app: &gtk4::Application, shared: Rc<RefCell<AppSettings>>) {
    let window = gtk4::ApplicationWindow::builder()
        .application(app)
        .title("Magic Pointer Settings")
        .default_width(620)
        .default_height(760)
        .build();
    window.set_resizable(false);
    window.add_css_class("settings-window");

    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    root.add_css_class("settings-content");
    let header = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    header.add_css_class("settings-header");
    let close_button = gtk4::Button::from_icon_name("window-close-symbolic");
    close_button.add_css_class("window-close-button");
    close_button.set_tooltip_text(Some("Close settings"));
    header.append(&close_button);
    let title = gtk4::Label::new(Some("Settings"));
    title.add_css_class("title-3");
    title.set_xalign(0.0);
    header.append(&title);
    root.append(&header);

    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    scrolled.set_propagate_natural_width(false);
    scrolled.set_propagate_natural_height(false);
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 18);
    content.set_margin_top(24);
    content.set_margin_bottom(20);
    content.set_margin_start(24);
    content.set_margin_end(24);
    scrolled.set_child(Some(&content));
    root.append(&scrolled);

    let status = gtk4::Label::new(None);
    status.set_wrap(true);
    status.set_xalign(0.0);
    status.add_css_class("settings-status");
    content.append(&status);

    let (request_section, request_content) = collapsible_section("Gemini request", false);
    let (model_section, model_content) = collapsible_section("Gemini model management", false);
    let model_note = gtk4::Label::new(Some(
        "Drag models to change fallback order. Models are tried in this order when the previous model is unavailable, rate-limited, overloaded, times out, or returns no text.",
    ));
    model_note.set_xalign(0.0);
    model_note.set_wrap(true);
    model_note.add_css_class("dim-label");
    model_content.append(&model_note);

    let model_order = Rc::new(RefCell::new(shared.borrow().model_order.clone()));
    let model_rows = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    let model_rows_scroll = gtk4::ScrolledWindow::new();
    model_rows_scroll.set_min_content_height(120);
    model_rows_scroll.set_max_content_height(220);
    model_rows_scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    model_rows_scroll.set_child(Some(&model_rows));
    model_content.append(&model_rows_scroll);
    rebuild_model_rows(&model_order, &model_rows, &status);

    let model_catalog = match settings::available_models() {
        Ok(models) => models,
        Err(error) => {
            status.set_text(&format!("Could not load models.txt catalog: {error}"));
            Vec::new()
        }
    };
    let model_picker_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let model_picker = gtk4::ComboBoxText::new();
    model_picker.set_hexpand(true);
    for (model_id, display_name) in &model_catalog {
        model_picker.append(
            Some(model_id),
            &format!(
                "{} ({})",
                compact_model_label(display_name, 32),
                compact_model_label(model_id, 28)
            ),
        );
    }
    if let Some((model_id, _)) = model_catalog.first() {
        model_picker.set_active_id(Some(model_id));
    }
    let add_model = gtk4::Button::with_label("Add model");
    add_model.set_sensitive(!model_catalog.is_empty());
    let refresh_catalog = gtk4::Button::with_label("Refresh catalog");
    refresh_catalog.set_tooltip_text(Some("Query Gemini API for live available models"));
    model_picker_row.append(&model_picker);
    model_picker_row.append(&add_model);
    model_picker_row.append(&refresh_catalog);
    model_content.append(&model_picker_row);

    let refresh_status = status.clone();
    let refresh_picker = model_picker.clone();
    let refresh_add_btn = add_model.clone();
    let refresh_btn_self = refresh_catalog.clone();
    refresh_catalog.connect_clicked(move |_| {
        let status_ui = refresh_status.clone();
        let picker_ui = refresh_picker.clone();
        let add_btn_ui = refresh_add_btn.clone();
        let btn_ui = refresh_btn_self.clone();

        btn_ui.set_sensitive(false);
        status_ui.set_text("Fetching live Gemini models from API...");

        gtk4::glib::spawn_future_local(async move {
            match settings::fetch_live_models().await {
                Ok(models) => {
                    picker_ui.remove_all();
                    for (model_id, display_name) in &models {
                        picker_ui.append(
                            Some(model_id),
                            &format!(
                                "{} ({})",
                                compact_model_label(display_name, 32),
                                compact_model_label(model_id, 28)
                            ),
                        );
                    }
                    if let Some((model_id, _)) = models.first() {
                        picker_ui.set_active_id(Some(model_id));
                    }
                    add_btn_ui.set_sensitive(!models.is_empty());
                    status_ui.set_text(&format!(
                        "Successfully refreshed {} models from Gemini API.",
                        models.len()
                    ));
                }
                Err(err) => {
                    tracing::warn!("Could not refresh models from API: {err}");
                    status_ui.set_text(&format!(
                        "Live model refresh failed ({err}); using catalog."
                    ));
                }
            }
            btn_ui.set_sensitive(true);
        });
    });

    let add_model_order = model_order.clone();
    let add_model_rows = model_rows.clone();
    let add_model_status = status.clone();
    let add_model_picker = model_picker.clone();
    add_model.connect_clicked(move |_| {
        let Some(model_id) = add_model_picker.active_id().map(|id| id.to_string()) else {
            add_model_status.set_text("Choose a model to add.");
            return;
        };
        let mut order = add_model_order.borrow_mut();
        if order.contains(&model_id) {
            add_model_status.set_text("That model is already in the attempt order.");
            return;
        }
        order.push(model_id.clone());
        drop(order);
        rebuild_model_rows(&add_model_order, &add_model_rows, &add_model_status);
        add_model_status.set_text("Model added. Save settings to use the new order.");
    });

    let max_output_tokens = gtk4::SpinButton::with_range(1.0, 65_536.0, 1.0);
    max_output_tokens.set_value(shared.borrow().max_output_tokens as f64);
    request_content.append(&labeled_row(
        "Maximum output tokens",
        max_output_tokens.clone().upcast(),
    ));

    let temperature = gtk4::SpinButton::with_range(0.0, 2.0, 0.05);
    temperature.set_digits(2);
    temperature.set_value(shared.borrow().temperature);
    request_content.append(&labeled_row(
        "Temperature (0–2)",
        temperature.clone().upcast(),
    ));

    let connection_timeout = gtk4::SpinButton::with_range(1.0, 120.0, 1.0);
    connection_timeout.set_value(shared.borrow().connection_timeout_secs as f64);
    request_content.append(&labeled_row(
        "Connection timeout (seconds)",
        connection_timeout.clone().upcast(),
    ));

    let response_timeout = gtk4::SpinButton::with_range(1.0, 300.0, 1.0);
    response_timeout.set_value(shared.borrow().response_timeout_secs as f64);
    request_content.append(&labeled_row(
        "Initial response timeout (seconds)",
        response_timeout.clone().upcast(),
    ));

    let stream_idle_timeout = gtk4::SpinButton::with_range(1.0, 300.0, 1.0);
    stream_idle_timeout.set_value(shared.borrow().stream_idle_timeout_secs as f64);
    request_content.append(&labeled_row(
        "Stream idle timeout (seconds)",
        stream_idle_timeout.clone().upcast(),
    ));
    content.append(&request_section);

    let (key_section, key_content) = collapsible_section("Gemini API keys", false);
    let key_note = gtk4::Label::new(Some(
        "Keys are stored in the system keyring. The primary key is tried first; the others are fallbacks.",
    ));
    key_note.set_xalign(0.0);
    key_note.set_wrap(true);
    key_note.add_css_class("dim-label");
    key_content.append(&key_note);
    let key_rows_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    key_content.append(&key_rows_box);
    content.append(&key_section);

    let rows = Rc::new(RefCell::new(Vec::<KeyRow>::new()));
    rebuild_key_rows(&shared, &key_rows_box, &rows, &status);

    let key_entry_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let new_key_name = gtk4::Entry::new();
    new_key_name.set_placeholder_text(Some("Key name"));
    new_key_name.set_width_chars(12);
    let new_key_value = gtk4::Entry::new();
    new_key_value.set_visibility(false);
    new_key_value.set_input_purpose(gtk4::InputPurpose::Password);
    new_key_value.set_placeholder_text(Some("Paste API key"));
    new_key_value.set_hexpand(true);
    let add_key = gtk4::Button::with_label("Add key");
    key_entry_row.append(&new_key_name);
    key_entry_row.append(&new_key_value);
    key_entry_row.append(&add_key);
    key_content.append(&key_entry_row);
    let open_studio = gtk4::Button::with_label("Open Google AI Studio");
    open_studio.set_halign(gtk4::Align::Start);
    key_content.append(&open_studio);

    let (appearance_section, appearance_content) = collapsible_section("Appearance", false);
    let font_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
    font_row.append(&gtk4::Label::new(Some("Font family")));
    let font_family = gtk4::Entry::new();
    font_family.set_hexpand(true);
    font_family.set_text(&shared.borrow().font_family);
    font_row.append(&font_family);
    appearance_content.append(&font_row);

    let font_size = gtk4::SpinButton::with_range(10.0, 40.0, 1.0);
    font_size.set_value(shared.borrow().font_size);
    appearance_content.append(&labeled_row(
        "Response font size (pt)",
        font_size.clone().upcast(),
    ));

    let opacity = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 0.25, 1.0, 0.01);
    opacity.set_value(shared.borrow().overlay_opacity);
    opacity.set_draw_value(true);
    opacity.set_digits(2);
    appearance_content.append(&labeled_row("Overlay opacity", opacity.clone().upcast()));

    let background_color = gtk4::Entry::new();
    background_color.set_placeholder_text(Some("System theme default (or e.g. #202124)"));
    background_color.set_text(&shared.borrow().background_color);
    appearance_content.append(&labeled_row(
        "Chat background",
        background_color.clone().upcast(),
    ));

    let css_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let custom_css = gtk4::Entry::new();
    custom_css.set_placeholder_text(Some("Path to a GTK CSS file (optional)"));
    custom_css.set_text(&shared.borrow().custom_css_path);
    custom_css.set_hexpand(true);
    let load_css = gtk4::Button::with_label("Choose…");
    css_row.append(&custom_css);
    css_row.append(&load_css);
    appearance_content.append(&labeled_row("Custom GTK CSS", css_row.upcast()));
    let export_css = gtk4::Button::with_label("Export CSS theme files…");
    export_css.set_halign(gtk4::Align::Start);
    appearance_content.append(&export_css);
    content.append(&appearance_section);

    let (hyprland_section, hyprland_content) = collapsible_section("Hyprland chat window", true);
    let hyprland_note = gtk4::Label::new(Some(
        "These options manage the regular response window through ~/.config/hypr/magicpointerv2.lua. Position presets are calculated from the focused monitor via Hyprland IPC when settings are saved; the config is then reloaded.",
    ));
    hyprland_note.set_xalign(0.0);
    hyprland_note.set_wrap(true);
    hyprland_note.add_css_class("dim-label");
    hyprland_content.append(&hyprland_note);

    let initial_window_settings = shared.borrow().hyprland_window.clone();
    let window_width = gtk4::SpinButton::with_range(200.0, 7680.0, 10.0);
    window_width.set_value(initial_window_settings.width as f64);
    hyprland_content.append(&labeled_row(
        "Window width (px)",
        window_width.clone().upcast(),
    ));
    let window_height = gtk4::SpinButton::with_range(200.0, 4320.0, 10.0);
    window_height.set_value(initial_window_settings.height as f64);
    hyprland_content.append(&labeled_row(
        "Window height (px)",
        window_height.clone().upcast(),
    ));

    let window_position = gtk4::ComboBoxText::new();
    for (id, label) in [
        ("center", "Center"),
        ("top-left", "Top left"),
        ("top-right", "Top right"),
        ("bottom-left", "Bottom left"),
        ("bottom-right", "Bottom right"),
    ] {
        window_position.append(Some(id), label);
    }
    window_position.set_active_id(Some(match initial_window_settings.position {
        WindowPosition::Center => "center",
        WindowPosition::TopLeft => "top-left",
        WindowPosition::TopRight => "top-right",
        WindowPosition::BottomLeft => "bottom-left",
        WindowPosition::BottomRight => "bottom-right",
    }));
    hyprland_content.append(&labeled_row(
        "Open position",
        window_position.clone().upcast(),
    ));

    let show_border = gtk4::CheckButton::with_label("Show Hyprland border");
    show_border.set_active(initial_window_settings.show_border);
    hyprland_content.append(&show_border);
    let border_size = gtk4::SpinButton::with_range(1.0, 10.0, 1.0);
    border_size.set_value(initial_window_settings.border_size as f64);
    border_size.set_sensitive(show_border.is_active());
    hyprland_content.append(&labeled_row(
        "Border width (px)",
        border_size.clone().upcast(),
    ));
    let border_size_sensitive = border_size.clone();
    show_border.connect_toggled(move |button| {
        border_size_sensitive.set_sensitive(button.is_active());
    });

    let show_shadow = gtk4::CheckButton::with_label("Enable Hyprland shadow");
    show_shadow.set_active(initial_window_settings.show_shadow);
    hyprland_content.append(&show_shadow);
    let blur = gtk4::CheckButton::with_label("Enable Hyprland blur");
    blur.set_active(initial_window_settings.blur);
    hyprland_content.append(&blur);
    let glow = gtk4::CheckButton::with_label("Enable Hyprland glow");
    glow.set_active(initial_window_settings.glow);
    hyprland_content.append(&glow);

    let hyprland_opacity = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 25.0, 100.0, 1.0);
    hyprland_opacity.set_value(initial_window_settings.opacity_percent as f64);
    hyprland_opacity.set_draw_value(true);
    hyprland_opacity.set_digits(0);
    hyprland_content.append(&labeled_row(
        "Hyprland opacity (%)",
        hyprland_opacity.clone().upcast(),
    ));
    content.append(&hyprland_section);

    let (spatial_section, spatial_content) = collapsible_section("Spatial intelligence", false);
    let spatial_enabled = gtk4::CheckButton::with_label("Enable spatial intelligence");
    spatial_enabled.set_active(shared.borrow().spatial.enabled);
    spatial_content.append(&spatial_enabled);
    let spatial_options = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    spatial_options.set_margin_start(20);
    let hardware = gtk4::CheckButton::with_label("Attach hardware information");
    hardware.set_active(shared.borrow().spatial.attach_hardware_info);
    let active_title = gtk4::CheckButton::with_label("Attach active window title");
    active_title.set_active(shared.borrow().spatial.attach_active_window_title);
    let workspace_windows =
        gtk4::CheckButton::with_label("Attach open workspace windows (window tree context)");
    workspace_windows.set_active(shared.borrow().spatial.attach_workspace_windows);
    spatial_options.append(&hardware);
    spatial_options.append(&active_title);
    spatial_options.append(&workspace_windows);
    spatial_options.append(&gtk4::Label::new(Some(
        "Additional context to include with every request",
    )));
    let custom_context = gtk4::TextView::new();
    custom_context.set_wrap_mode(gtk4::WrapMode::WordChar);
    custom_context.set_height_request(90);
    custom_context
        .buffer()
        .set_text(&shared.borrow().spatial.custom_text);
    let context_scroll = gtk4::ScrolledWindow::new();
    context_scroll.set_min_content_height(90);
    context_scroll.set_child(Some(&custom_context));
    spatial_options.append(&context_scroll);
    spatial_content.append(&spatial_options);
    spatial_options.set_visible(spatial_enabled.is_active());
    let reveal_options = spatial_options.clone();
    spatial_enabled.connect_toggled(move |button| {
        reveal_options.set_visible(button.is_active());
    });
    content.append(&spatial_section);

    let (preset_section, preset_content) =
        collapsible_section("System instructions & Presets", false);
    let sys_label = gtk4::Label::new(Some(
        "Persistent system prompt (e.g. 'Always answer concisely with Arch/Linux-specific commands'):",
    ));
    sys_label.set_xalign(0.0);
    sys_label.set_wrap(true);
    sys_label.add_css_class("dim-label");
    preset_content.append(&sys_label);

    let sys_instruction_view = gtk4::TextView::new();
    sys_instruction_view.set_wrap_mode(gtk4::WrapMode::WordChar);
    sys_instruction_view.set_height_request(70);
    sys_instruction_view
        .buffer()
        .set_text(&shared.borrow().system_instruction);
    let sys_scroll = gtk4::ScrolledWindow::new();
    sys_scroll.set_min_content_height(70);
    sys_scroll.set_child(Some(&sys_instruction_view));
    preset_content.append(&sys_scroll);

    let chips_label = gtk4::Label::new(Some(
        "Custom quick-action chips (available alongside default prompt chips):",
    ));
    chips_label.set_xalign(0.0);
    chips_label.set_wrap(true);
    chips_label.add_css_class("dim-label");
    chips_label.set_margin_top(8);
    preset_content.append(&chips_label);

    let custom_chips_state = Rc::new(RefCell::new(shared.borrow().custom_chips.clone()));
    let chips_container = gtk4::Box::new(gtk4::Orientation::Vertical, 4);
    preset_content.append(&chips_container);
    rebuild_chips_ui(&custom_chips_state, &chips_container);

    let add_chip_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
    let new_chip_label = gtk4::Entry::new();
    new_chip_label.set_placeholder_text(Some("Chip Label (e.g. Arch Fix)"));
    new_chip_label.set_width_chars(14);

    let new_chip_prompt = gtk4::Entry::new();
    new_chip_prompt.set_placeholder_text(Some("Prompt (e.g. Provide Arch Linux fix)"));
    new_chip_prompt.set_hexpand(true);

    let add_chip_btn = gtk4::Button::with_label("Add chip");
    add_chip_row.append(&new_chip_label);
    add_chip_row.append(&new_chip_prompt);
    add_chip_row.append(&add_chip_btn);
    preset_content.append(&add_chip_row);

    let add_chips_state = custom_chips_state.clone();
    let add_chips_box = chips_container.clone();
    let add_lbl_entry = new_chip_label.clone();
    let add_prompt_entry = new_chip_prompt.clone();
    add_chip_btn.connect_clicked(move |_| {
        let label = add_lbl_entry.text().trim().to_string();
        let prompt = add_prompt_entry.text().trim().to_string();
        if !label.is_empty() && !prompt.is_empty() {
            add_chips_state.borrow_mut().push((label, prompt));
            add_lbl_entry.set_text("");
            add_prompt_entry.set_text("");
            rebuild_chips_ui(&add_chips_state, &add_chips_box);
        }
    });

    content.append(&preset_section);
    content.append(&model_section);

    let footer = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    footer.set_margin_top(8);
    footer.set_margin_bottom(16);
    footer.set_margin_start(24);
    footer.set_margin_end(24);
    footer.set_halign(gtk4::Align::Fill);
    let reset = gtk4::Button::with_label("Reset app data…");
    reset.add_css_class("destructive-action");
    let close = gtk4::Button::with_label("Close");
    let save = gtk4::Button::with_label("Save settings");
    save.add_css_class("suggested-action");
    footer.append(&reset);
    let spacer = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    spacer.set_hexpand(true);
    footer.append(&spacer);
    footer.append(&close);
    footer.append(&save);
    root.append(&footer);
    window.set_child(Some(&root));

    let reset_window = window.clone();
    let reset_app = app.clone();
    let reset_shared = shared.clone();
    let reset_status = status.clone();
    reset.connect_clicked(move |_| {
        let dialog = gtk4::Dialog::builder()
            .title("Reset Magic Pointer")
            .transient_for(&reset_window)
            .modal(true)
            .build();
        let message = gtk4::Label::new(Some(
            "This permanently removes Magic Pointer settings and API keys from the system keyring, deletes its managed Hyprland Lua module and require line, then closes the app. Your other Hyprland settings and unrelated files are kept.",
        ));
        message.set_wrap(true);
        message.set_xalign(0.0);
        message.set_margin_top(16);
        message.set_margin_bottom(16);
        message.set_margin_start(16);
        message.set_margin_end(16);
        dialog.content_area().append(&message);
        dialog.add_button("Cancel", gtk4::ResponseType::Cancel);
        let confirm = dialog.add_button("Reset and close", gtk4::ResponseType::Accept);
        confirm.add_css_class("destructive-action");

        let settings_state = reset_shared.clone();
        let status_label = reset_status.clone();
        let settings_window = reset_window.clone();
        let application = reset_app.clone();
        dialog.connect_response(move |dialog, response| {
            if response == gtk4::ResponseType::Accept {
                match settings::reset_app_data() {
                    Ok(()) => {
                        *settings_state.borrow_mut() = AppSettings::default();
                        settings_window.close();
                        application.quit();
                    }
                    Err(error) => {
                        tracing::error!("Could not completely reset application data: {error:#}");
                        status_label.set_text(&format!("Reset was incomplete: {error:#}"));
                    }
                }
            }
            dialog.close();
        });
        dialog.present();
    });

    let open_studio_status = status.clone();
    open_studio.connect_clicked(move |_| {
        if let Err(error) = gtk4::gio::AppInfo::launch_default_for_uri(
            AI_STUDIO_URL,
            None::<&gtk4::gio::AppLaunchContext>,
        ) {
            open_studio_status.set_text(&format!("Could not open AI Studio: {error}"));
        }
    });

    let chooser_window = window.clone();
    let chooser_css = custom_css.clone();
    let chooser_status = status.clone();
    load_css.connect_clicked(move |_| {
        let dialog = gtk4::FileChooserNative::new(
            Some("Choose GTK CSS file"),
            Some(&chooser_window),
            gtk4::FileChooserAction::Open,
            Some("Open"),
            Some("Cancel"),
        );
        let path_entry = chooser_css.clone();
        let status_label = chooser_status.clone();
        dialog.connect_response(move |dialog, response| {
            if response == gtk4::ResponseType::Accept
                && let Some(file) = dialog.file()
                && let Some(path) = file.path()
            {
                path_entry.set_text(&path.to_string_lossy());
                status_label.set_text("CSS path selected. Save settings to load it.");
            }
            dialog.destroy();
        });
        dialog.show();
    });

    let export_window = window.clone();
    let export_status = status.clone();
    let export_shared = shared.clone();
    let export_font_family = font_family.clone();
    let export_font_size = font_size.clone();
    let export_background = background_color.clone();
    let export_custom_css = custom_css.clone();
    export_css.connect_clicked(move |_| {
        let dialog = gtk4::FileChooserNative::new(
            Some("Choose a folder for the CSS theme files"),
            Some(&export_window),
            gtk4::FileChooserAction::SelectFolder,
            Some("Export"),
            Some("Cancel"),
        );
        let status_label = export_status.clone();
        let settings_state = export_shared.clone();
        let font_family = export_font_family.clone();
        let font_size = export_font_size.clone();
        let background = export_background.clone();
        let custom_css = export_custom_css.clone();
        dialog.connect_response(move |dialog, response| {
            if response == gtk4::ResponseType::Accept {
                let result = (|| {
                    let file = dialog
                        .file()
                        .ok_or_else(|| anyhow::anyhow!("No export folder was selected"))?;
                    let directory = file.path().ok_or_else(|| {
                        anyhow::anyhow!("The export folder is not a local directory")
                    })?;
                    let mut current_settings = settings_state.borrow().clone();
                    current_settings.font_family = font_family.text().trim().to_string();
                    current_settings.font_size = font_size.value();
                    current_settings.background_color = background.text().trim().to_string();
                    current_settings.custom_css_path = custom_css.text().trim().to_string();
                    settings::appearance::write_css_export(&directory, &current_settings)
                })();

                match result {
                    Ok((template_path, example_path)) => status_label.set_text(&format!(
                        "Exported {} and {}.",
                        template_path.display(),
                        example_path.display()
                    )),
                    Err(error) => {
                        status_label.set_text(&format!("Could not export CSS files: {error:#}"))
                    }
                }
            }
            dialog.destroy();
        });
        dialog.show();
    });

    let add_shared = shared.clone();
    let add_status = status.clone();
    let add_name = new_key_name.clone();
    let add_value = new_key_value.clone();
    let add_container = key_rows_box.clone();
    let add_rows = rows.clone();
    add_key.connect_clicked(move |_| {
        let name = add_name.text().trim().to_string();
        let key = add_value.text().trim().to_string();
        if name.is_empty() || key.is_empty() {
            add_status.set_text("Enter a name and API key before adding it.");
            return;
        }
        let result = { add_shared.borrow_mut().add_api_key(&name, &key) };
        match result {
            Ok(()) => {
                if add_shared.borrow().api_keys.len() == 1 {
                    while let Some(child) = add_container.first_child() {
                        add_container.remove(&child);
                    }
                }
                if let Some(record) = add_shared.borrow().api_keys.last().cloned() {
                    append_api_key_row(&add_shared, &add_container, &add_rows, &add_status, record);
                }
                add_name.set_text("");
                add_value.set_text("");
                add_status.set_text("API key added.");
            }
            Err(error) => add_status.set_text(&format!("Could not add API key: {error}")),
        }
    });

    close_button.connect_clicked({
        let window = window.clone();
        move |_| window.close()
    });
    close.connect_clicked({
        let window = window.clone();
        move |_| window.close()
    });

    let save_shared = shared.clone();
    let save_rows = rows.clone();
    let save_model_order = model_order.clone();
    let save_app = app.clone();
    let save_status = status.clone();
    save.connect_clicked(move |_| {
        let mut draft = save_shared.borrow().clone();
        for row in save_rows.borrow().iter() {
            let name = row.name.text();
            if name.trim().is_empty() {
                save_status.set_text("Each saved API key needs a name.");
                return;
            }
            let Some(record) = draft.api_keys.iter_mut().find(|key| key.id == row.id) else {
                save_status.set_text(
                    "An API key changed while this panel was open. Reopen settings and try again.",
                );
                return;
            };
            record.name = name.trim().to_string();
            if row.primary.is_active() {
                draft.primary_api_key_id = Some(row.id.clone());
            }
        }

        draft.model_order = save_model_order.borrow().clone();
        if draft.model_order.is_empty() {
            save_status.set_text("Keep at least one Gemini model in the attempt order.");
            return;
        }
        draft.max_output_tokens = max_output_tokens.value_as_int().max(1) as u32;
        draft.temperature = temperature.value();
        draft.connection_timeout_secs = connection_timeout.value_as_int().max(1) as u64;
        draft.response_timeout_secs = response_timeout.value_as_int().max(1) as u64;
        draft.stream_idle_timeout_secs = stream_idle_timeout.value_as_int().max(1) as u64;
        draft.hyprland_window.width = window_width.value_as_int().clamp(200, 7680) as u32;
        draft.hyprland_window.height = window_height.value_as_int().clamp(200, 4320) as u32;
        let Some(position) = window_position.active_id() else {
            save_status.set_text("Choose a Hyprland chat-window position.");
            return;
        };
        draft.hyprland_window.position = match position.as_str() {
            "center" => WindowPosition::Center,
            "top-left" => WindowPosition::TopLeft,
            "top-right" => WindowPosition::TopRight,
            "bottom-left" => WindowPosition::BottomLeft,
            "bottom-right" => WindowPosition::BottomRight,
            _ => {
                save_status.set_text("Choose a valid Hyprland chat-window position.");
                return;
            }
        };
        draft.hyprland_window.show_border = show_border.is_active();
        draft.hyprland_window.border_size = border_size.value_as_int().clamp(1, 10) as u32;
        draft.hyprland_window.show_shadow = show_shadow.is_active();
        draft.hyprland_window.blur = blur.is_active();
        draft.hyprland_window.glow = glow.is_active();
        draft.hyprland_window.opacity_percent =
            hyprland_opacity.value().round().clamp(25.0, 100.0) as u32;
        draft.font_family = font_family.text().trim().to_string();
        if draft.font_family.is_empty() {
            save_status.set_text("Font family cannot be empty.");
            return;
        }
        draft.font_size = font_size.value();
        draft.overlay_opacity = opacity.value();
        draft.background_color = background_color.text().trim().to_string();
        draft.custom_css_path = custom_css.text().trim().to_string();
        draft.spatial.enabled = spatial_enabled.is_active();
        draft.spatial.attach_hardware_info = hardware.is_active();
        draft.spatial.attach_active_window_title = active_title.is_active();
        draft.spatial.attach_workspace_windows = workspace_windows.is_active();
        let buffer = custom_context.buffer();
        let (start, end) = buffer.bounds();
        draft.spatial.custom_text = buffer.text(&start, &end, true).to_string();

        let sys_buf = sys_instruction_view.buffer();
        let (sys_start, sys_end) = sys_buf.bounds();
        draft.system_instruction = sys_buf.text(&sys_start, &sys_end, true).to_string();
        draft.custom_chips = custom_chips_state.borrow().clone();

        match settings::apply_appearance(&save_app, &draft).and_then(|()| draft.save()) {
            Ok(()) => {
                *save_shared.borrow_mut() = draft.clone();
                match settings::hyprland::install(&draft) {
                    Ok(()) => save_status.set_text("Settings saved; Hyprland integration is up to date."),
                    Err(error) => save_status.set_text(&format!(
                        "Settings were saved, but Hyprland integration could not be applied: {error:#}"
                    )),
                }
            }
            Err(error) => {
                let restore_error =
                    settings::apply_appearance(&save_app, &save_shared.borrow()).err();
                if let Some(restore_error) = restore_error {
                    tracing::error!("Could not restore the previous appearance: {restore_error}");
                    save_status.set_text(&format!(
                        "Could not save settings: {error}. Restoring the previous appearance also failed: {restore_error}"
                    ));
                } else {
                    save_status.set_text(&format!("Could not save settings: {error}"));
                }
            }
        }
    });

    window.present();
}

fn rebuild_model_rows(
    model_order: &Rc<RefCell<Vec<String>>>,
    container: &gtk4::Box,
    status: &gtk4::Label,
) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }

    let models = model_order.borrow().clone();
    for (index, model_id) in models.iter().enumerate() {
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
        row.add_css_class("model-order-row");

        let drag_handle = gtk4::Label::new(Some("↕"));
        drag_handle.set_tooltip_text(Some("Drag to reorder"));
        drag_handle.set_cursor_from_name(Some("grab"));
        row.append(&drag_handle);

        let model_label = gtk4::Label::new(Some(&format!("{}. {}", index + 1, model_id)));
        model_label.set_xalign(0.0);
        model_label.set_selectable(true);
        model_label.set_hexpand(true);
        row.append(&model_label);

        let remove = gtk4::Button::from_icon_name("list-remove-symbolic");
        remove.set_tooltip_text(Some("Remove model from attempt order"));
        remove.set_sensitive(models.len() > 1);
        row.append(&remove);
        container.append(&row);

        let drag_source = gtk4::DragSource::new();
        drag_source.set_actions(gtk4::gdk::DragAction::MOVE);
        let dragged_model = model_id.clone();
        drag_source.connect_prepare(move |_, _, _| {
            let value = dragged_model.to_value();
            Some(gtk4::gdk::ContentProvider::for_value(&value))
        });
        drag_handle.add_controller(drag_source);

        let drop_target = gtk4::DropTarget::new(String::static_type(), gtk4::gdk::DragAction::MOVE);
        let order_on_drop = model_order.clone();
        let rows_on_drop = container.downgrade();
        let status_on_drop = status.clone();
        let target_row = row.downgrade();
        let target_model = model_id.clone();
        drop_target.connect_drop(move |_, value, _, y| {
            let Ok(dragged_model) = value.get::<String>() else {
                return false;
            };
            let Some(container) = rows_on_drop.upgrade() else {
                return false;
            };
            let mut order = order_on_drop.borrow_mut();
            let Some(from) = order.iter().position(|model| model == &dragged_model) else {
                return false;
            };
            let Some(to) = order.iter().position(|model| model == &target_model) else {
                return false;
            };

            let drop_after = target_row
                .upgrade()
                .is_some_and(|row| row.height() > 0 && y >= row.height() as f64 / 2.0);
            let mut destination = to + usize::from(drop_after);
            let moved_model = order.remove(from);
            if from < destination {
                destination -= 1;
            }
            destination = destination.min(order.len());
            order.insert(destination, moved_model);
            drop(order);

            rebuild_model_rows(&order_on_drop, &container, &status_on_drop);
            status_on_drop.set_text("Model order changed. Save settings to apply it.");
            true
        });
        row.add_controller(drop_target);

        let order_on_remove = model_order.clone();
        let rows_on_remove = container.downgrade();
        let status_on_remove = status.clone();
        let removed_model = model_id.clone();
        remove.connect_clicked(move |_| {
            let mut order = order_on_remove.borrow_mut();
            if order.len() <= 1 {
                status_on_remove.set_text("Keep at least one Gemini model in the attempt order.");
                return;
            }
            order.retain(|model| model != &removed_model);
            drop(order);
            if let Some(container) = rows_on_remove.upgrade() {
                rebuild_model_rows(&order_on_remove, &container, &status_on_remove);
            }
            status_on_remove.set_text("Model removed. Save settings to apply the change.");
        });
    }
}

fn rebuild_key_rows(
    shared: &Rc<RefCell<AppSettings>>,
    container: &gtk4::Box,
    rows: &Rc<RefCell<Vec<KeyRow>>>,
    status: &gtk4::Label,
) {
    rows.borrow_mut().clear();
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }

    let records = shared.borrow().api_keys.clone();
    if records.is_empty() {
        let empty = gtk4::Label::new(Some("No saved keys. Add a key below to get started."));
        empty.set_xalign(0.0);
        empty.add_css_class("dim-label");
        container.append(&empty);
        return;
    }

    for record in records {
        append_api_key_row(shared, container, rows, status, record);
    }
}

fn append_api_key_row(
    shared: &Rc<RefCell<AppSettings>>,
    container: &gtk4::Box,
    rows: &Rc<RefCell<Vec<KeyRow>>>,
    status: &gtk4::Label,
    record: ApiKeyRecord,
) {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    let name = gtk4::Entry::new();
    name.set_text(&record.name);
    name.set_hexpand(true);
    let primary = gtk4::CheckButton::with_label("Primary");
    if let Some(group) = rows.borrow().first() {
        primary.set_group(Some(&group.primary));
    } else {
        primary.set_active(true);
    }
    primary.set_active(shared.borrow().primary_api_key_id.as_deref() == Some(&record.id));
    let delete = gtk4::Button::from_icon_name("user-trash-symbolic");
    delete.add_css_class("destructive-action");
    delete.set_tooltip_text(Some("Delete this API key"));
    row.append(&name);
    row.append(&primary);
    row.append(&delete);
    container.append(&row);
    rows.borrow_mut().push(KeyRow {
        id: record.id.clone(),
        name,
        primary,
    });

    let delete_shared = shared.clone();
    let delete_rows = Rc::downgrade(rows);
    let delete_container = container.downgrade();
    let delete_row = row.downgrade();
    let delete_id = record.id;
    let delete_status = status.clone();
    delete.connect_clicked(move |_| {
        let staged_primary = delete_rows.upgrade().and_then(|rows| {
            rows.borrow()
                .iter()
                .find(|row| row.primary.is_active() && row.id != delete_id)
                .map(|row| row.id.clone())
        });
        let result = delete_shared.borrow_mut().remove_api_key(&delete_id);
        match result {
            Ok(()) => {
                let mut primary_error = None;
                if let Some(primary_id) = staged_primary
                    && let Err(error) = delete_shared.borrow_mut().set_primary_api_key(&primary_id)
                {
                    primary_error = Some(error);
                }
                if let (Some(container), Some(row)) =
                    (delete_container.upgrade(), delete_row.upgrade())
                {
                    container.remove(&row);
                }
                if let Some(rows) = delete_rows.upgrade() {
                    rows.borrow_mut().retain(|row| row.id != delete_id);
                    if let Some(primary_id) = delete_shared.borrow().primary_api_key_id.as_ref() {
                        for row in rows.borrow().iter() {
                            row.primary.set_active(&row.id == primary_id);
                        }
                    }
                }
                if delete_shared.borrow().api_keys.is_empty()
                    && let Some(container) = delete_container.upgrade()
                {
                    let empty =
                        gtk4::Label::new(Some("No saved keys. Add a key below to get started."));
                    empty.set_xalign(0.0);
                    empty.add_css_class("dim-label");
                    container.append(&empty);
                }
                if let Some(error) = primary_error {
                    delete_status.set_text(&format!(
                        "Key deleted, but could not save the selected primary key: {error}"
                    ));
                } else {
                    delete_status.set_text("API key deleted.");
                }
            }
            Err(error) => {
                tracing::error!("Could not delete API key: {error}");
                delete_status.set_text(&format!("Could not delete API key: {error}"));
            }
        }
    });
}

fn rebuild_chips_ui(chips_state: &Rc<RefCell<Vec<(String, String)>>>, container: &gtk4::Box) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }
    let chips = chips_state.borrow().clone();
    for (idx, (label, prompt_text)) in chips.into_iter().enumerate() {
        let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 6);
        let lbl = gtk4::Label::new(Some(&format!("{label}: \"{prompt_text}\"")));
        lbl.set_xalign(0.0);
        lbl.set_hexpand(true);
        lbl.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        lbl.add_css_class("dim-label");

        let remove_btn = gtk4::Button::from_icon_name("window-close-symbolic");
        remove_btn.add_css_class("flat");
        remove_btn.set_tooltip_text(Some("Delete custom chip"));

        let state_remove = chips_state.clone();
        let box_remove = container.clone();
        remove_btn.connect_clicked(move |_| {
            let mut list = state_remove.borrow_mut();
            if idx < list.len() {
                list.remove(idx);
            }
            drop(list);
            rebuild_chips_ui(&state_remove, &box_remove);
        });

        row.append(&lbl);
        row.append(&remove_btn);
        container.append(&row);
    }
}

fn collapsible_section(title: &str, expanded: bool) -> (gtk4::Expander, gtk4::Box) {
    let expander = gtk4::Expander::new(Some(title));
    expander.add_css_class("settings-expander");
    expander.set_expanded(expanded);
    let section = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
    section.add_css_class("settings-section");
    expander.set_child(Some(&section));
    (expander, section)
}

fn labeled_row(label: &str, widget: gtk4::Widget) -> gtk4::Box {
    let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
    let caption = gtk4::Label::new(Some(label));
    caption.set_xalign(0.0);
    caption.set_width_chars(22);
    row.append(&caption);
    widget.set_hexpand(true);
    row.append(&widget);
    row
}

fn compact_model_label(label: &str, max_chars: usize) -> String {
    let mut chars = label.chars();
    let compact = chars.by_ref().take(max_chars).collect::<String>();
    if chars.next().is_some() {
        format!("{compact}…")
    } else {
        compact
    }
}

#[cfg(test)]
mod tests {
    use super::compact_model_label;

    #[test]
    fn model_picker_labels_are_compact() {
        assert_eq!(compact_model_label("A longer model name", 8), "A longer…");
        assert_eq!(compact_model_label("Short", 8), "Short");
        assert_eq!(compact_model_label("Gemini", 0), "…");
    }
}
