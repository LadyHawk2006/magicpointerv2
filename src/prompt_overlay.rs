use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use gtk4_layer_shell::{Edge, KeyboardMode, Layer, LayerShell};
use image::DynamicImage;
use std::cell::RefCell;
use std::rc::Rc;

const PROMPT_WIDTH: i32 = 380;
const PROMPT_HEIGHT: i32 = 300;
const CURSOR_GAP: i32 = 18;
const EDGE_PADDING: i32 = 12;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct PromptPosition {
    left: i32,
    top: i32,
}

fn position_prompt(
    cursor: (i32, i32),
    monitor: gdk::Rectangle,
    prompt_size: (i32, i32),
) -> PromptPosition {
    let cursor_x = cursor.0 - monitor.x();
    let cursor_y = cursor.1 - monitor.y();
    let available_width = monitor.width().max(0);
    let available_height = monitor.height().max(0);

    let left = if cursor_x + CURSOR_GAP + prompt_size.0 <= available_width - EDGE_PADDING {
        cursor_x + CURSOR_GAP
    } else {
        cursor_x - CURSOR_GAP - prompt_size.0
    };
    let top = if cursor_y + CURSOR_GAP + prompt_size.1 <= available_height - EDGE_PADDING {
        cursor_y + CURSOR_GAP
    } else {
        cursor_y - CURSOR_GAP - prompt_size.1
    };

    PromptPosition {
        left: left.clamp(
            EDGE_PADDING,
            (available_width - prompt_size.0 - EDGE_PADDING).max(EDGE_PADDING),
        ),
        top: top.clamp(
            EDGE_PADDING,
            (available_height - prompt_size.1 - EDGE_PADDING).max(EDGE_PADDING),
        ),
    }
}

pub fn show_prompt_overlay<F, D>(
    app: &gtk4::Application,
    image: DynamicImage,
    cursor: (i32, i32),
    opacity: f64,
    on_submit: F,
    on_dismiss: D,
) where
    F: FnOnce(String, DynamicImage) + 'static,
    D: FnOnce() + 'static,
{
    let Some(display) = gdk::Display::default() else {
        tracing::error!("Cannot show prompt composer: no GDK display is available");
        on_dismiss();
        return;
    };

    let Some((monitor, monitor_geometry)) = find_monitor(&display, cursor) else {
        tracing::error!("Cannot show prompt composer: no display monitor is available");
        on_dismiss();
        return;
    };

    let prompt_width = PROMPT_WIDTH.min((monitor_geometry.width() - 2 * EDGE_PADDING).max(1));
    let prompt_height = PROMPT_HEIGHT.min((monitor_geometry.height() - 2 * EDGE_PADDING).max(1));
    let prompt_size = (prompt_width, prompt_height);
    let position = position_prompt(cursor, monitor_geometry, prompt_size);
    let window = gtk4::ApplicationWindow::new(app);
    window.init_layer_shell();
    window.set_title(Some("Ask about this capture"));
    window.set_default_size(prompt_width, prompt_height);
    window.set_decorated(false);
    window.set_opacity(opacity);
    window.add_css_class("prompt-overlay");
    window.set_layer(Layer::Overlay);
    window.set_monitor(Some(&monitor));
    window.set_keyboard_mode(KeyboardMode::Exclusive);
    window.set_exclusive_zone(0);
    window.set_anchor(Edge::Top, true);
    window.set_anchor(Edge::Left, true);
    window.set_margin(Edge::Top, position.top);
    window.set_margin(Edge::Left, position.left);

    let card = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    card.add_css_class("prompt-card");

    let texture = image_texture(&image);
    let preview = gtk4::Picture::for_paintable(&texture);
    preview.set_can_shrink(true);
    preview.set_keep_aspect_ratio(true);
    preview.set_size_request(112.min(prompt_width - 48), 104.min(prompt_height - 112));
    preview.add_css_class("prompt-preview");
    preview.set_overflow(gtk4::Overflow::Hidden);
    preview.set_halign(gtk4::Align::Start);
    card.append(&preview);

    let composer = gtk4::Box::new(gtk4::Orientation::Horizontal, 8);
    composer.add_css_class("prompt-composer");

    let entry_scroll = gtk4::ScrolledWindow::new();
    entry_scroll.add_css_class("prompt-entry-scroll");
    entry_scroll.set_min_content_height(48);
    entry_scroll.set_max_content_height(96);
    entry_scroll.set_propagate_natural_height(true);
    entry_scroll.set_hscrollbar_policy(gtk4::PolicyType::Never);
    entry_scroll.set_vscrollbar_policy(gtk4::PolicyType::Automatic);
    entry_scroll.set_overlay_scrolling(true);
    entry_scroll.set_hexpand(true);

    let entry = gtk4::TextView::new();
    entry.add_css_class("prompt-entry");
    entry.set_wrap_mode(gtk4::WrapMode::WordChar);
    entry.set_accepts_tab(false);
    entry.set_top_margin(8);
    entry.set_bottom_margin(8);
    entry.set_left_margin(0);
    entry.set_right_margin(0);

    let entry_overlay = gtk4::Overlay::new();
    entry_overlay.set_child(Some(&entry));
    let placeholder = gtk4::Label::new(Some("Ask anything about this capture"));
    placeholder.add_css_class("prompt-entry-placeholder");
    placeholder.set_halign(gtk4::Align::Start);
    placeholder.set_valign(gtk4::Align::Start);
    placeholder.set_margin_top(8);
    placeholder.set_can_target(false);
    entry_overlay.add_overlay(&placeholder);
    entry_scroll.set_child(Some(&entry_overlay));
    let placeholder_changed = placeholder.clone();
    entry.buffer().connect_changed(move |buffer| {
        placeholder_changed.set_visible(buffer.char_count() == 0);
    });

    let send_icon = gtk4::Image::from_icon_name("document-send");
    send_icon.set_pixel_size(32);
    let send_button = gtk4::Button::new();
    send_button.set_child(Some(&send_icon));
    send_button.add_css_class("prompt-send-button");
    send_button.set_tooltip_text(Some("Send question"));
    composer.append(&entry_scroll);
    composer.append(&send_button);
    card.append(&composer);
    window.set_child(Some(&card));

    let image = Rc::new(RefCell::new(Some(image)));
    let on_submit = Rc::new(RefCell::new(Some(on_submit)));
    let submit_window = window.downgrade();
    let submit_entry = entry.clone();
    let submit_image = image.clone();
    let submit_callback = on_submit.clone();
    let submit = move || {
        let buffer = submit_entry.buffer();
        let (start, end) = buffer.bounds();
        let question = buffer.text(&start, &end, true).trim().to_string();
        if question.is_empty() {
            return;
        }

        let (Some(image), Some(callback)) = (
            submit_image.borrow_mut().take(),
            submit_callback.borrow_mut().take(),
        ) else {
            return;
        };
        if let Some(window) = submit_window.upgrade() {
            window.close();
        }
        callback(question, image);
    };

    let submit_button = submit.clone();
    send_button.connect_clicked(move |_| submit_button());
    let submit_key = submit.clone();
    let submit_controller = gtk4::EventControllerKey::new();
    submit_controller.connect_key_pressed(move |_, key, _, state| {
        if matches!(key, gdk::Key::Return | gdk::Key::KP_Enter)
            && state.contains(gdk::ModifierType::CONTROL_MASK)
        {
            submit_key();
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    entry.add_controller(submit_controller);

    let key_controller = gtk4::EventControllerKey::new();
    let key_window = window.downgrade();
    let dismiss_callback = Rc::new(RefCell::new(Some(on_dismiss)));
    let escape_callback = dismiss_callback.clone();
    key_controller.connect_key_pressed(move |_, key, _, _| {
        if key == gdk::Key::Escape {
            if let Some(window) = key_window.upgrade() {
                window.close();
            }
            if let Some(callback) = escape_callback.borrow_mut().take() {
                callback();
            }
            glib::Propagation::Stop
        } else {
            glib::Propagation::Proceed
        }
    });
    window.add_controller(key_controller);

    window.present();
    entry.grab_focus();
}

fn find_monitor(
    display: &gdk::Display,
    cursor: (i32, i32),
) -> Option<(gdk::Monitor, gdk::Rectangle)> {
    let monitors = display.monitors();
    let mut first_monitor = None;
    for index in 0..monitors.n_items() {
        let monitor = monitors.item(index)?.downcast::<gdk::Monitor>().ok()?;
        let geometry = monitor.geometry();
        if first_monitor.is_none() {
            first_monitor = Some((monitor.clone(), geometry));
        }

        if cursor.0 >= geometry.x()
            && cursor.1 >= geometry.y()
            && cursor.0 < geometry.x() + geometry.width()
            && cursor.1 < geometry.y() + geometry.height()
        {
            return Some((monitor, geometry));
        }
    }

    first_monitor
}

fn image_texture(image: &DynamicImage) -> gdk::MemoryTexture {
    let thumbnail = image.thumbnail(288, 192).to_rgba8();
    let width = thumbnail.width();
    let height = thumbnail.height();
    let bytes = glib::Bytes::from_owned(thumbnail.into_raw());
    gdk::MemoryTexture::new(
        width as i32,
        height as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &bytes,
        width as usize * 4,
    )
}

#[cfg(test)]
mod tests {
    use super::{PromptPosition, position_prompt};
    use gtk4::gdk;

    #[test]
    fn prompt_is_placed_beside_cursor_when_room_is_available() {
        let monitor = gdk::Rectangle::new(0, 0, 1920, 1080);
        assert_eq!(
            position_prompt((400, 300), monitor, (380, 220)),
            PromptPosition {
                left: 418,
                top: 318
            }
        );
    }

    #[test]
    fn prompt_flips_and_clamps_near_monitor_edges() {
        let monitor = gdk::Rectangle::new(0, 0, 1920, 1080);
        assert_eq!(
            position_prompt((1900, 1060), monitor, (380, 220)),
            PromptPosition {
                left: 1502,
                top: 822
            }
        );
    }

    #[test]
    fn prompt_uses_monitor_local_coordinates() {
        let monitor = gdk::Rectangle::new(-1920, 0, 1920, 1080);
        assert_eq!(
            position_prompt((-1800, 300), monitor, (380, 220)),
            PromptPosition {
                left: 138,
                top: 318
            }
        );
    }
}
