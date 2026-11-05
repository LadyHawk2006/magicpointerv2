use gtk4::gdk;
use gtk4::prelude::*;
use magicpointerv2::ai_agent::ConversationSession;
use magicpointerv2::hypr_ipc;
use magicpointerv2::overlay_ui;
use magicpointerv2::screencapture;
use magicpointerv2::selection_overlay::{self, Rect};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{error, info};

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

                match screencapture::capture_screen_region(rect.x, rect.y, rect.width, rect.height) {
                    Ok(image) => {
                        info!("Captured screen buffer ({}x{}) into RAM", image.width(), image.height());

                        let session = ConversationSession::new();
                        let (token_tx, token_rx) = mpsc::channel::<String>(64);

                        let context_label = hypr_ipc::get_hyprland_state().ok().and_then(|s| {
                            s.active_window.map(|w| format!("{} ({})", w.class, w.title))
                        });

                        let card_x = rect.x + (rect.width as i32 / 2);
                        let card_y = rect.y + rect.height as i32;

                        let card_window = overlay_ui::build_floating_card(
                            &app_handle,
                            card_x,
                            card_y,
                            context_label,
                            token_rx,
                            session.clone(),
                            rt_handle.clone(),
                        );

                        let app_exit = app_handle.clone();
                        card_window.connect_destroy(move |_| {
                            info!("Chat closed. Exiting process.");
                            app_exit.quit();
                        });

                        card_window.present();

                        let session_spawn = session.clone();
                        rt_handle.spawn(async move {
                            let initial_prompt = "Analyze this UI element / screen capture concisely. If it contains text or code, transcribe and explain it. If it contains a button or actionable item, provide context and quick next steps.";
                            if let Err(e) = session_spawn.add_user_initial_turn(&image, initial_prompt).await {
                                error!("Failed to create initial turn: {}", e);
                                return;
                            }
                            if let Err(e) = session_spawn.stream_next_turn(token_tx).await {
                                error!("AI multimodal stream error: {}", e);
                            }
                        });
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
