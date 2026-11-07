use std::cell::RefCell;
use std::rc::Rc;

use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use image::DynamicImage;

use crate::hypr_ipc;
use crate::prompt_overlay;
use crate::prompt_overlay::{apply_annotations_to_image, show_annotation_modal};

pub fn refresh_attachment_previews(container: &gtk4::Box, images: &[DynamicImage]) {
    const PREVIEW_MIN_WIDTH: i32 = 96;
    const PREVIEW_MIN_HEIGHT: i32 = 72;

    while let Some(child) = container.first_child() {
        container.remove(&child);
    }

    for image in images {
        let preview = picture_for_image(image, PREVIEW_MIN_WIDTH, PREVIEW_MIN_HEIGHT);
        preview.add_css_class("attachment-preview");
        container.append(&preview);
    }
}

pub fn refresh_pending_attachment_previews(
    app: &gtk4::Application,
    container: &gtk4::Box,
    images: Rc<RefCell<Vec<DynamicImage>>>,
    attachment_row: gtk4::Box,
) {
    while let Some(child) = container.first_child() {
        container.remove(&child);
    }

    let image_count = images.borrow().len();
    for index in 0..image_count {
        let preview = gtk4::Overlay::new();
        preview.add_css_class("pending-attachment-preview");
        preview.set_child(Some(&picture_for_image(&images.borrow()[index], 96, 72)));

        let annotate_button = gtk4::Button::from_icon_name("document-edit-symbolic");
        annotate_button.add_css_class("annotate-attachment-button");
        annotate_button.set_tooltip_text(Some("Annotate capture"));
        annotate_button.set_halign(gtk4::Align::Start);
        annotate_button.set_valign(gtk4::Align::Start);
        preview.add_overlay(&annotate_button);

        let remove_button = gtk4::Button::from_icon_name("window-close-symbolic");
        remove_button.add_css_class("remove-attachment-button");
        remove_button.set_tooltip_text(Some("Remove capture"));
        remove_button.set_halign(gtk4::Align::End);
        remove_button.set_valign(gtk4::Align::Start);
        preview.add_overlay(&remove_button);

        let annotate_app = app.clone();
        let annotate_images = images.clone();
        let annotate_previews = container.clone();
        let annotate_row = attachment_row.clone();
        annotate_button.connect_clicked(move |_| {
            let Some(image) = annotate_images.borrow().get(index).cloned() else {
                return;
            };
            let Some(display) = gdk::Display::default() else {
                tracing::error!("Cannot annotate capture: no GDK display is available");
                return;
            };
            let cursor = hypr_ipc::get_hyprland_state()
                .map(|state| state.cursor)
                .unwrap_or((0, 0));
            let Some((monitor, geometry)) = prompt_overlay::find_monitor(&display, cursor) else {
                tracing::error!("Cannot annotate capture: no display monitor is available");
                return;
            };

            let updated_images = annotate_images.clone();
            let updated_previews = annotate_previews.clone();
            let updated_row = annotate_row.clone();
            let updated_app = annotate_app.clone();
            let annotated_image = image.clone();
            show_annotation_modal(
                &annotate_app,
                &monitor,
                geometry,
                &image,
                Vec::new(),
                move |annotations| {
                    let annotated = apply_annotations_to_image(&annotated_image, &annotations);
                    let mut images = updated_images.borrow_mut();
                    if let Some(pending_image) = images.get_mut(index) {
                        *pending_image = annotated;
                    }
                    drop(images);
                    refresh_pending_attachment_previews(
                        &updated_app,
                        &updated_previews,
                        updated_images,
                        updated_row,
                    );
                },
            );
        });

        let pending_images = images.clone();
        let previews = container.clone();
        let attachment_row = attachment_row.clone();
        let app = app.clone();
        remove_button.connect_clicked(move |_| {
            let mut images = pending_images.borrow_mut();
            if index < images.len() {
                images.remove(index);
            }
            let is_empty = images.is_empty();
            drop(images);
            attachment_row.set_visible(!is_empty);
            refresh_pending_attachment_previews(
                &app,
                &previews,
                pending_images.clone(),
                attachment_row.clone(),
            );
        });

        container.append(&preview);
    }
}

pub fn picture_for_image(image: &DynamicImage, max_width: i32, max_height: i32) -> gtk4::Picture {
    let thumbnail = image
        .thumbnail(max_width as u32, max_height as u32)
        .to_rgba8();
    let width = thumbnail.width();
    let height = thumbnail.height();
    let bytes = glib::Bytes::from_owned(thumbnail.into_raw());
    let texture = gdk::MemoryTexture::new(
        width as i32,
        height as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &bytes,
        width as usize * 4,
    );
    let picture = gtk4::Picture::for_paintable(&texture);
    picture.set_can_shrink(true);
    picture.set_keep_aspect_ratio(true);
    picture.set_size_request(max_width, max_height);
    picture.set_overflow(gtk4::Overflow::Hidden);
    picture
}
