use adw::prelude::*;
use gtk::{gdk, gdk_pixbuf, gio, glib};
use std::path::PathBuf;
use std::rc::Rc;

pub fn build(
    on_files: Rc<dyn Fn(Vec<PathBuf>)>,
    on_project: Rc<dyn Fn(PathBuf)>,
) -> gtk::Widget {
    // Render the SVG much larger than its display size so the GPU
    // downscales it cleanly instead of showing pixelated edges.
    let logo_bytes = include_bytes!("../logo.svg");
    let stream = gio::MemoryInputStream::from_bytes(&glib::Bytes::from(&logo_bytes[..]));
    let pixbuf = gdk_pixbuf::Pixbuf::from_stream_at_scale(
        &stream,
        1024,
        1024,
        true,
        gio::Cancellable::NONE,
    )
    .expect("Failed to load embedded logo");
    let logo_texture = gdk::Texture::for_pixbuf(&pixbuf);

    // ---------- Left side: brand ----------
    let logo = gtk::Image::builder()
        .paintable(&logo_texture)
        .pixel_size(320)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .hexpand(false)
        .vexpand(false)
        .build();

    let title = gtk::Label::builder()
        .label("Recto")
        .css_classes(["title-1", "recto-title"])
        .halign(gtk::Align::Center)
        .build();

    let subtitle = gtk::Label::builder()
        .label("The powerful book scanning post-processor")
        .css_classes(["dim-label"])
        .halign(gtk::Align::Center)
        .wrap(true)
        .justify(gtk::Justification::Center)
        .max_width_chars(32)
        .build();

    let brand = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    brand.append(&logo);
    brand.append(&title);
    brand.append(&subtitle);

    // ---------- Right side: actions ----------
    let drop_icon = gtk::Image::builder()
        .icon_name("emblem-photos-symbolic")
        .pixel_size(48)
        .build();
    let drop_title = gtk::Label::builder()
        .label("Drop images here to start")
        .css_classes(["title-3"])
        .build();
    let drop_hint = gtk::Label::builder()
        .label("or click to choose files")
        .css_classes(["dim-label"])
        .build();

    let drop_content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .build();
    drop_content.append(&drop_icon);
    drop_content.append(&drop_title);
    drop_content.append(&drop_hint);

    let drop_btn = gtk::Button::builder()
        .child(&drop_content)
        .css_classes(["flat", "drop-area"])
        .width_request(380)
        .height_request(220)
        .build();

    drop_btn.connect_clicked(glib::clone!(
        #[strong] on_files,
        move |btn| {
            let dialog = gtk::FileDialog::builder()
                .title("Add page images")
                .modal(true)
                .build();
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Images"));
            for mime in [
                "image/jpeg",
                "image/png",
                "image/tiff",
                "image/webp",
                "image/bmp",
            ] {
                filter.add_mime_type(mime);
            }
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            dialog.set_filters(Some(&filters));
            let parent = btn.root().and_downcast::<gtk::Window>();
            dialog.open_multiple(
                parent.as_ref(),
                gio::Cancellable::NONE,
                glib::clone!(
                    #[strong] on_files,
                    move |result| {
                        let Ok(files) = result else { return };
                        let mut paths = Vec::new();
                        for i in 0..files.n_items() {
                            let Some(file) = files.item(i).and_downcast::<gio::File>() else {
                                continue;
                            };
                            if let Some(path) = file.path() {
                                paths.push(path);
                            }
                        }
                        if !paths.is_empty() {
                            on_files(paths);
                        }
                    }
                ),
            );
        }
    ));

    let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    drop.connect_drop({
        let on_files = Rc::clone(&on_files);
        move |_, value, _, _| {
            let Ok(file_list) = value.get::<gdk::FileList>() else {
                return false;
            };
            let mut paths = Vec::new();
            for file in file_list.files() {
                if let Some(path) = file.path() {
                    paths.push(path);
                }
            }
            if !paths.is_empty() {
                on_files(paths);
            }
            true
        }
    });
    drop_btn.add_controller(drop);

    let project_btn = gtk::Button::builder()
        .label("Open existing project\u{2026}")
        .css_classes(["flat"])
        .halign(gtk::Align::Center)
        .build();

    project_btn.connect_clicked(glib::clone!(
        #[strong] on_project,
        move |btn| {
            let dialog = gtk::FileDialog::builder()
                .title("Open Project")
                .modal(true)
                .build();
            let filter = gtk::FileFilter::new();
            filter.set_name(Some("Recto project (*.recto)"));
            filter.add_pattern("*.recto");
            let filters = gio::ListStore::new::<gtk::FileFilter>();
            filters.append(&filter);
            dialog.set_filters(Some(&filters));
            let parent = btn.root().and_downcast::<gtk::Window>();
            dialog.open(
                parent.as_ref(),
                gio::Cancellable::NONE,
                glib::clone!(
                    #[strong] on_project,
                    move |result| {
                        let Ok(file) = result else { return };
                        let Some(path) = file.path() else { return };
                        on_project(path);
                    }
                ),
            );
        }
    ));

    let actions = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .build();
    actions.append(&drop_btn);
    actions.append(&project_btn);

    // ---------- Outer layout: brand left, actions right ----------
    let row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(48)
        .halign(gtk::Align::Center)
        .valign(gtk::Align::Center)
        .hexpand(true)
        .vexpand(true)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .build();
    row.append(&brand);
    row.append(&actions);

    let provider = gtk::CssProvider::new();
    provider.load_from_string(
        ".drop-area { \
             background: transparent; \
             border: 2px dashed alpha(currentColor, 0.25); \
             border-radius: 12px; \
             padding: 8px; \
         } \
         .drop-area:hover { \
             border-color: alpha(@accent_color, 0.6); \
         } \
         .recto-title { \
             font-size: 36pt; \
             font-weight: 800; \
         }",
    );
    gtk::style_context_add_provider_for_display(
        &gdk::Display::default().expect("display must be available"),
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );

    row.upcast()
}
