use crate::auth;
use gtk4::prelude::*;
use std::cell::Cell;
use std::rc::Rc;

const AI_STUDIO_URL: &str = "https://aistudio.google.com/";

pub fn show_api_key_setup(
    app: &gtk4::Application,
    initial_error: Option<String>,
    on_saved: impl Fn() + 'static,
    on_cancel: impl Fn() + 'static,
) {
    let window = gtk4::ApplicationWindow::builder()
        .application(app)
        .title("Set up Magic Pointer")
        .default_width(440)
        .resizable(false)
        .modal(true)
        .build();
    window.add_css_class("auth-setup-window");

    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 16);
    content.add_css_class("auth-setup-content");
    content.set_margin_top(24);
    content.set_margin_bottom(24);
    content.set_margin_start(24);
    content.set_margin_end(24);

    let heading = gtk4::Label::new(Some("Connect to Gemini"));
    heading.add_css_class("title-2");
    heading.set_xalign(0.0);
    content.append(&heading);

    let instructions = gtk4::Label::new(Some(
        "To get started, sign in to or create a Google account in AI Studio. Choose “Develop with Gemini API” and create an API key. Complete any account or project setup steps AI Studio requests, then paste the key below.",
    ));
    instructions.set_wrap(true);
    instructions.set_xalign(0.0);
    content.append(&instructions);

    let browser_button = gtk4::Button::with_label("Open Google AI Studio");
    browser_button.set_halign(gtk4::Align::Start);
    content.append(&browser_button);

    let api_key_entry = gtk4::Entry::new();
    api_key_entry.set_visibility(false);
    api_key_entry.set_input_purpose(gtk4::InputPurpose::Password);
    api_key_entry.set_placeholder_text(Some("Paste Gemini API key"));
    api_key_entry.set_hexpand(true);
    content.append(&api_key_entry);

    let storage_note = gtk4::Label::new(Some(
        "The key is saved in your system keyring. No Google OAuth login is needed.",
    ));
    storage_note.set_wrap(true);
    storage_note.set_xalign(0.0);
    storage_note.add_css_class("dim-label");
    content.append(&storage_note);

    let status = gtk4::Label::new(None);
    status.set_wrap(true);
    status.set_xalign(0.0);
    status.add_css_class("error");
    if let Some(error) = initial_error {
        status.set_text(&format!(
            "Could not check the system keyring: {error}. You can still enter the key and try saving it."
        ));
    }
    content.append(&status);

    let actions = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    actions.set_halign(gtk4::Align::End);
    let cancel_button = gtk4::Button::with_label("Cancel");
    let save_button = gtk4::Button::with_label("Save key and continue");
    save_button.add_css_class("suggested-action");
    actions.append(&cancel_button);
    actions.append(&save_button);
    content.append(&actions);

    window.set_child(Some(&content));

    let saved = Rc::new(Cell::new(false));
    let on_cancel: Rc<dyn Fn()> = Rc::new(on_cancel);
    let close_cancel = on_cancel.clone();
    let close_saved = saved.clone();
    window.connect_close_request(move |_| {
        if !close_saved.get() {
            close_cancel();
        }
        gtk4::glib::Propagation::Proceed
    });

    let cancel_window = window.clone();
    cancel_button.connect_clicked(move |_| cancel_window.close());

    let save_window = window.clone();
    let save_status = status.clone();
    let save_entry = api_key_entry.clone();
    let save_saved = saved.clone();
    let on_saved: Rc<dyn Fn()> = Rc::new(on_saved);
    save_button.connect_clicked(move |_| {
        let api_key = save_entry.text().trim().to_owned();
        if api_key.is_empty() {
            save_status.set_text("Paste your Gemini API key to continue.");
            return;
        }

        match auth::store_api_key(&api_key) {
            Ok(()) => {
                save_saved.set(true);
                save_window.close();
                on_saved();
            }
            Err(error) => {
                save_status.set_text(&format!(
                    "Could not save the key in your system keyring: {error}"
                ));
            }
        }
    });

    let browser_status = status.clone();
    browser_button.connect_clicked(move |_| {
        if let Err(error) = gtk4::gio::AppInfo::launch_default_for_uri(
            AI_STUDIO_URL,
            None::<&gtk4::gio::AppLaunchContext>,
        ) {
            browser_status.set_text(&format!("Could not open Google AI Studio: {error}"));
        }
    });

    if let Err(error) = gtk4::gio::AppInfo::launch_default_for_uri(
        AI_STUDIO_URL,
        None::<&gtk4::gio::AppLaunchContext>,
    ) {
        status.set_text(&format!(
            "Could not open Google AI Studio automatically: {error}. Use the button to retry."
        ));
    }

    window.present();
}
