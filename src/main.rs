use gtk4::gdk;
use gtk4::prelude::*;
use magicpointerv2::ai_agent::ConversationSession;
use magicpointerv2::auth;
use magicpointerv2::auth_setup;
use magicpointerv2::hypr_ipc;
use magicpointerv2::prompt_overlay;
use magicpointerv2::screencapture;
use magicpointerv2::selection_overlay;
use magicpointerv2::settings::{self, AppSettings};
use magicpointerv2::settings_ui;
use magicpointerv2::ui::{self, ChatPreferences};
use std::cell::RefCell;
use std::io::Write;
use std::rc::Rc;
use std::sync::Arc;
use tracing::{error, info, warn};

fn load_custom_css() {
    let css_provider = gtk4::CssProvider::new();
    let css_data = magicpointerv2::styles::APP_CSS;
    css_provider.load_from_data(css_data);

    if let Some(display) = gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &css_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

fn start_selection(
    app: gtk4::Application,
    rt: Arc<tokio::runtime::Runtime>,
    settings_state: Rc<RefCell<AppSettings>>,
) {
    info!("Opening spatial selection overlay...");

    let app_handle = app.clone();
    let app_dismiss = app.clone();
    let rt_handle = rt.clone();

    selection_overlay::show_selection_overlay(
        &app,
        move |target: screencapture::CaptureTarget| {
            let rect = target.rect();
            info!("Target selected: {:?} (bounding rect: {:?})", target, rect);

            match screencapture::capture_target(&target) {
                Ok(image) => {
                    info!(
                        "Captured screen buffer ({}x{}) into RAM",
                        image.width(),
                        image.height()
                    );

                    let session = ConversationSession::new();

                    let (cursor, active_window_title, context_label) = match &target {
                        screencapture::CaptureTarget::Window { title, class, .. } => {
                            let cursor =
                                hypr_ipc::get_hyprland_state().map(|s| s.cursor).unwrap_or((
                                    rect.x.saturating_add(rect.width as i32),
                                    rect.y.saturating_add(rect.height as i32),
                                ));
                            (
                                cursor,
                                Some(title.clone()),
                                Some(format!("{} ({})", class, title)),
                            )
                        }
                        screencapture::CaptureTarget::Region(_) => {
                            match hypr_ipc::get_hyprland_state() {
                                Ok(state) => {
                                    let matching_window =
                                        hypr_ipc::get_visible_windows().ok().and_then(|windows| {
                                            windows.into_iter().find(|w| {
                                                w.x == rect.x
                                                    && w.y == rect.y
                                                    && (w.width as u32) == rect.width
                                                    && (w.height as u32) == rect.height
                                            })
                                        });

                                    if let Some(target_w) = matching_window {
                                        (
                                            state.cursor,
                                            Some(target_w.title.clone()),
                                            Some(format!(
                                                "{} ({})",
                                                target_w.class, target_w.title
                                            )),
                                        )
                                    } else {
                                        (
                                            state.cursor,
                                            state
                                                .active_window
                                                .as_ref()
                                                .map(|window| window.title.clone()),
                                            state.active_window.map(|window| {
                                                format!("{} ({})", window.class, window.title)
                                            }),
                                        )
                                    }
                                }
                                Err(error) => {
                                    warn!(
                                        "Could not query Hyprland cursor; placing prompt near selected region: {error}"
                                    );
                                    (
                                        (
                                            rect.x.saturating_add(rect.width as i32),
                                            rect.y.saturating_add(rect.height as i32),
                                        ),
                                        None,
                                        None,
                                    )
                                }
                            }
                        }
                    };

                    let submit_app = app_handle.clone();
                    let submit_session = session.clone();
                    let submit_runtime = rt_handle.clone();
                    let submit_settings = settings_state.clone();
                    let submit_active_window_title = active_window_title.clone();
                    let dismiss_app = app_handle.clone();
                    let overlay_opacity = settings_state.borrow().overlay_opacity;
                    prompt_overlay::show_prompt_overlay(
                        &app_handle,
                        image,
                        cursor,
                        overlay_opacity,
                        move |prompt, image| {
                            let chat_window = ui::build_floating_card(
                                &submit_app,
                                image,
                                context_label,
                                prompt,
                                submit_session,
                                submit_runtime,
                                ChatPreferences {
                                    active_window_title: submit_active_window_title,
                                    settings: submit_settings,
                                },
                            );

                            let app_exit = submit_app.clone();
                            chat_window.connect_destroy(move |_| {
                                info!("Chat closed. Exiting process.");
                                app_exit.quit();
                            });
                            chat_window.present();
                        },
                        move || dismiss_app.quit(),
                    );
                }
                Err(error) => {
                    error!("Screen capture error: {}", error);
                    app_handle.quit();
                }
            }
        },
        move || {
            info!("Selection dismissed/cancelled. Exiting process.");
            app_dismiss.quit();
        },
    );
}

fn main() {
    tracing_subscriber::fmt::init();
    if std::env::args_os().any(|arg| arg == "--reset") {
        eprintln!(
            "This removes Magic Pointer settings, its Secret Service credentials, and its managed Hyprland integration."
        );
        eprint!("Type RESET to continue: ");
        if let Err(error) = std::io::stderr().flush() {
            eprintln!("Could not display confirmation prompt: {error}");
            std::process::exit(1);
        }
        let mut confirmation = String::new();
        if let Err(error) = std::io::stdin().read_line(&mut confirmation) {
            eprintln!("Could not read confirmation: {error}");
            std::process::exit(1);
        }
        if confirmation.trim() != "RESET" {
            println!("Reset cancelled.");
            return;
        }
        match settings::reset_app_data() {
            Ok(()) => println!("Magic Pointer data and Hyprland integration were reset."),
            Err(error) => {
                eprintln!("{error:#}");
                std::process::exit(1);
            }
        }
        return;
    }

    info!("Launching magicpointerv2 chat assistant...");

    let tokio_rt = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to initialize Tokio runtime"),
    );

    let settings_mode = std::env::args_os().any(|arg| arg == "--settings");
    let app = gtk4::Application::builder()
        .application_id("com.magicpointerv2.tool")
        .flags(if settings_mode {
            gtk4::gio::ApplicationFlags::NON_UNIQUE
        } else {
            gtk4::gio::ApplicationFlags::default()
        })
        .build();
    app.add_main_option(
        "settings",
        gtk4::glib::Char(0),
        gtk4::glib::OptionFlags::NONE,
        gtk4::glib::OptionArg::None,
        "Open the settings panel",
        None,
    );

    let rt = tokio_rt.clone();

    app.connect_startup(|_| {
        load_custom_css();
    });

    app.connect_activate(move |app| {
        let current_settings = match AppSettings::load() {
            Ok(settings) => settings,
            Err(error) => {
                error!("Could not load application settings: {error:#}");
                app.quit();
                return;
            }
        };
        let settings_state = Rc::new(RefCell::new(current_settings));
        if let Err(error) = settings::apply_appearance(app, &settings_state.borrow()) {
            warn!("Could not apply all appearance settings: {error:#}");
        }
        if let Err(error) = settings::hyprland::install(&settings_state.borrow()) {
            warn!("Could not apply Hyprland window rules: {error:#}");
        }

        if settings_mode {
            settings_ui::show_settings(app, settings_state);
            return;
        }

        match auth::get_api_key() {
            Ok(Some(_)) => start_selection(app.clone(), rt.clone(), settings_state),
            Ok(None) => {
                info!("No Gemini API key configured; opening first-run setup.");
                let app_saved = app.clone();
                let runtime_saved = rt.clone();
                let settings_saved = settings_state.clone();
                let app_cancelled = app.clone();
                auth_setup::show_api_key_setup(
                    app,
                    None,
                    move || match AppSettings::load() {
                        Ok(updated_settings) => {
                            *settings_saved.borrow_mut() = updated_settings;
                            start_selection(
                                app_saved.clone(),
                                runtime_saved.clone(),
                                settings_saved.clone(),
                            );
                        }
                        Err(error) => {
                            error!("Could not reload settings after saving the API key: {error:#}");
                            app_saved.quit();
                        }
                    },
                    move || app_cancelled.quit(),
                );
            }
            Err(error) => {
                warn!("Could not access the system keyring: {error}");
                let app_saved = app.clone();
                let runtime_saved = rt.clone();
                let settings_saved = settings_state.clone();
                let app_cancelled = app.clone();
                auth_setup::show_api_key_setup(
                    app,
                    Some(format!("{error:#}")),
                    move || match AppSettings::load() {
                        Ok(updated_settings) => {
                            *settings_saved.borrow_mut() = updated_settings;
                            start_selection(
                                app_saved.clone(),
                                runtime_saved.clone(),
                                settings_saved.clone(),
                            );
                        }
                        Err(error) => {
                            error!("Could not reload settings after saving the API key: {error:#}");
                            app_saved.quit();
                        }
                    },
                    move || app_cancelled.quit(),
                );
            }
        }
    });

    app.run();
}
