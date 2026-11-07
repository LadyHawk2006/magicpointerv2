use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;

use crate::settings::{self, ApiKeyRecord, AppSettings};

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
    window.add_css_class("settings-window");

    let root = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    root.add_css_class("settings-content");
    let scrolled = gtk4::ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 18);
    content.set_margin_top(24);
    content.set_margin_bottom(20);
    content.set_margin_start(24);
    content.set_margin_end(24);
    scrolled.set_child(Some(&content));
    root.append(&scrolled);

    let title = gtk4::Label::new(Some("Settings"));
    title.add_css_class("title-2");
    title.set_xalign(0.0);
    content.append(&title);

    let status = gtk4::Label::new(None);
    status.set_wrap(true);
    status.set_xalign(0.0);
    status.add_css_class("settings-status");
    content.append(&status);

    let request_section = section("Gemini request");
    let model_note = gtk4::Label::new(Some(
        "Drag models to change fallback order. Models are tried in this order when the previous model is unavailable, rate-limited, overloaded, times out, or returns no text.",
    ));
    model_note.set_xalign(0.0);
    model_note.set_wrap(true);
    model_note.add_css_class("dim-label");
    request_section.append(&model_note);

    let model_order = Rc::new(RefCell::new(shared.borrow().model_order.clone()));
    let model_rows = gtk4::Box::new(gtk4::Orientation::Vertical, 6);
    let model_rows_scroll = gtk4::ScrolledWindow::new();
    model_rows_scroll.set_min_content_height(120);
    model_rows_scroll.set_max_content_height(220);
    model_rows_scroll.set_policy(gtk4::PolicyType::Never, gtk4::PolicyType::Automatic);
    model_rows_scroll.set_child(Some(&model_rows));
    request_section.append(&model_rows_scroll);
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
        model_picker.append(Some(model_id), &format!("{display_name} ({model_id})"));
    }
    if let Some((model_id, _)) = model_catalog.first() {
        model_picker.set_active_id(Some(model_id));
    }
    let add_model = gtk4::Button::with_label("Add model");
    add_model.set_sensitive(!model_catalog.is_empty());
    model_picker_row.append(&model_picker);
    model_picker_row.append(&add_model);
    request_section.append(&model_picker_row);

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
    request_section.append(&labeled_row(
        "Maximum output tokens",
        max_output_tokens.clone().upcast(),
    ));

    let temperature = gtk4::SpinButton::with_range(0.0, 2.0, 0.05);
    temperature.set_digits(2);
    temperature.set_value(shared.borrow().temperature);
    request_section.append(&labeled_row(
        "Temperature (0–2)",
        temperature.clone().upcast(),
    ));

    let connection_timeout = gtk4::SpinButton::with_range(1.0, 120.0, 1.0);
    connection_timeout.set_value(shared.borrow().connection_timeout_secs as f64);
    request_section.append(&labeled_row(
        "Connection timeout (seconds)",
        connection_timeout.clone().upcast(),
    ));

    let response_timeout = gtk4::SpinButton::with_range(1.0, 300.0, 1.0);
    response_timeout.set_value(shared.borrow().response_timeout_secs as f64);
    request_section.append(&labeled_row(
        "Initial response timeout (seconds)",
        response_timeout.clone().upcast(),
    ));

    let stream_idle_timeout = gtk4::SpinButton::with_range(1.0, 300.0, 1.0);
    stream_idle_timeout.set_value(shared.borrow().stream_idle_timeout_secs as f64);
    request_section.append(&labeled_row(
        "Stream idle timeout (seconds)",
        stream_idle_timeout.clone().upcast(),
    ));
    content.append(&request_section);

    let key_section = section("Gemini API keys");
    let key_note = gtk4::Label::new(Some(
        "Keys are stored in the system keyring. The primary key is tried first; the others are fallbacks.",
    ));
    key_note.set_xalign(0.0);
    key_note.set_wrap(true);
    key_note.add_css_class("dim-label");
    key_section.append(&key_note);
    let key_rows_box = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    key_section.append(&key_rows_box);
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
    key_section.append(&key_entry_row);
    let open_studio = gtk4::Button::with_label("Open Google AI Studio");
    open_studio.set_halign(gtk4::Align::Start);
    key_section.append(&open_studio);

    let appearance_section = section("Appearance");
    let font_row = gtk4::Box::new(gtk4::Orientation::Horizontal, 10);
    font_row.append(&gtk4::Label::new(Some("Font family")));
    let font_family = gtk4::Entry::new();
    font_family.set_hexpand(true);
    font_family.set_text(&shared.borrow().font_family);
    font_row.append(&font_family);
    appearance_section.append(&font_row);

    let font_size = gtk4::SpinButton::with_range(10.0, 40.0, 1.0);
    font_size.set_value(shared.borrow().font_size);
    appearance_section.append(&labeled_row(
        "Response font size (pt)",
        font_size.clone().upcast(),
    ));

    let opacity = gtk4::Scale::with_range(gtk4::Orientation::Horizontal, 0.25, 1.0, 0.01);
    opacity.set_value(shared.borrow().overlay_opacity);
    opacity.set_draw_value(true);
    opacity.set_digits(2);
    appearance_section.append(&labeled_row("Overlay opacity", opacity.clone().upcast()));

    let background_color = gtk4::Entry::new();
    background_color.set_placeholder_text(Some("System theme default (or e.g. #202124)"));
    background_color.set_text(&shared.borrow().background_color);
    appearance_section.append(&labeled_row(
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
    appearance_section.append(&labeled_row("Custom GTK CSS", css_row.upcast()));
    content.append(&appearance_section);

    let spatial_section = section("Spatial intelligence");
    let spatial_enabled = gtk4::CheckButton::with_label("Enable spatial intelligence");
    spatial_enabled.set_active(shared.borrow().spatial.enabled);
    spatial_section.append(&spatial_enabled);
    let spatial_options = gtk4::Box::new(gtk4::Orientation::Vertical, 8);
    spatial_options.set_margin_start(20);
    let hardware = gtk4::CheckButton::with_label("Attach hardware information");
    hardware.set_active(shared.borrow().spatial.attach_hardware_info);
    let active_title = gtk4::CheckButton::with_label("Attach active window title");
    active_title.set_active(shared.borrow().spatial.attach_active_window_title);
    spatial_options.append(&hardware);
    spatial_options.append(&active_title);
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
    spatial_section.append(&spatial_options);
    spatial_options.set_visible(spatial_enabled.is_active());
    let reveal_options = spatial_options.clone();
    spatial_enabled.connect_toggled(move |button| {
        reveal_options.set_visible(button.is_active());
    });
    content.append(&spatial_section);

    let footer = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    footer.set_margin_top(8);
    footer.set_margin_bottom(16);
    footer.set_margin_start(24);
    footer.set_margin_end(24);
    footer.set_halign(gtk4::Align::End);
    let close = gtk4::Button::with_label("Close");
    let save = gtk4::Button::with_label("Save settings");
    save.add_css_class("suggested-action");
    footer.append(&close);
    footer.append(&save);
    root.append(&footer);
    window.set_child(Some(&root));

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
        let buffer = custom_context.buffer();
        let (start, end) = buffer.bounds();
        draft.spatial.custom_text = buffer.text(&start, &end, true).to_string();

        match settings::apply_appearance(&save_app, &draft).and_then(|()| draft.save()) {
            Ok(()) => {
                *save_shared.borrow_mut() = draft;
                save_status.set_text("Settings saved.");
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

fn section(title: &str) -> gtk4::Box {
    let section = gtk4::Box::new(gtk4::Orientation::Vertical, 10);
    section.add_css_class("settings-section");
    let heading = gtk4::Label::new(Some(title));
    heading.add_css_class("heading");
    heading.set_xalign(0.0);
    section.append(&heading);
    section
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
