use gtk4::gdk;
use gtk4::prelude::*;
use magicpointerv2::ai_agent::ConversationSession;
use magicpointerv2::hypr_ipc;
use magicpointerv2::prompt_overlay;
use magicpointerv2::screencapture;
use magicpointerv2::selection_overlay::{self, Rect};
use magicpointerv2::ui;
use std::sync::Arc;
use tracing::{error, info, warn};

fn load_custom_css() {
    let css_provider = gtk4::CssProvider::new();
    let css_data = include_str!("style.css");
    css_provider.load_from_data(css_data);

    if let Some(display) = gdk::Display::default() {
        gtk4::style_context_add_provider_for_display(
            &display,
            &css_provider,
            gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }
}

fn main() {
    tracing_subscriber::fmt::init();
    info!("Launching magicpointerv2 chat assistant...");

    let tokio_rt = Arc::new(
        tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("Failed to initialize Tokio runtime"),
    );

    let app = gtk4::Application::builder()
        .application_id("com.magicpointerv2.tool")
        .build();

    let rt = tokio_rt.clone();

    app.connect_startup(|_| {
        load_custom_css();
    });

    app.connect_activate(move |app| {
        info!("Activated: opening spatial selection overlay immediately...");

        let app_handle = app.clone();
        let app_dismiss = app.clone();
        let rt_handle = rt.clone();

        selection_overlay::show_selection_overlay(
            app,
            move |rect: Rect| {
                info!("Region selected: {:?}", rect);

                match screencapture::capture_screen_region(rect.x, rect.y, rect.width, rect.height)
                {
                    Ok(image) => {
                        info!(
                            "Captured screen buffer ({}x{}) into RAM",
                            image.width(),
                            image.height()
                        );

                        let session = ConversationSession::new();

                        let (cursor, context_label) = match hypr_ipc::get_hyprland_state() {
                            Ok(state) => (
                                state.cursor,
                                state
                                    .active_window
                                    .map(|window| format!("{} ({})", window.class, window.title)),
                            ),
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
                                )
                            }
                        };

                        let submit_app = app_handle.clone();
                        let submit_session = session.clone();
                        let submit_runtime = rt_handle.clone();
                        let dismiss_app = app_handle.clone();
                        prompt_overlay::show_prompt_overlay(
                            &app_handle,
                            image,
                            cursor,
                            move |prompt, image| {
                                let chat_window = ui::build_floating_card(
                                    &submit_app,
                                    image,
                                    context_label,
                                    prompt,
                                    submit_session,
                                    submit_runtime,
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
                    Err(e) => {
                        error!("Screen capture error: {}", e);
                        app_handle.quit();
                    }
                }
            },
            move || {
                info!("Selection dismissed/cancelled. Exiting process.");
                app_dismiss.quit();
            },
        );
    });

    app.run();
}
