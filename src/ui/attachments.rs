use std::cell::RefCell;
use std::rc::Rc;

use gtk4::gdk;
use gtk4::glib;
use gtk4::prelude::*;
use image::DynamicImage;

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

        let remove_button = gtk4::Button::from_icon_name("window-close-symbolic");
        remove_button.add_css_class("remove-attachment-button");
        remove_button.set_tooltip_text(Some("Remove capture"));
        remove_button.set_halign(gtk4::Align::End);
        remove_button.set_valign(gtk4::Align::Start);
        preview.add_overlay(&remove_button);

        let pending_images = images.clone();
        let previews = container.clone();
        let attachment_row = attachment_row.clone();
        remove_button.connect_clicked(move |_| {
            let mut images = pending_images.borrow_mut();
            if index < images.len() {
                images.remove(index);
            }
            let is_empty = images.is_empty();
            drop(images);
            attachment_row.set_visible(!is_empty);
            refresh_pending_attachment_previews(
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
