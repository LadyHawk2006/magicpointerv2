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
