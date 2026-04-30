use gtk::prelude::*;
use gtk::{gio, glib};

use crate::app::State;
use pagecutter_core::project::ExportSettings;

enum ExportMsg {
    Progress(usize, usize),
    Done(Vec<(bool, String)>),
}

pub fn build(state: State) -> gtk::Widget {
    let (tx, rx) = async_channel::unbounded::<ExportMsg>();

    let dir_lbl = gtk::Label::builder()
        .label("No directory chosen")
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .xalign(0.0)
        .hexpand(true)
        .css_classes(["dim-label"])
        .build();
    {
        let p = state.borrow();
        if !p.output_dir.as_os_str().is_empty() {
            dir_lbl.set_label(&p.output_dir.display().to_string());
        }
    }
    let dir_btn = gtk::Button::builder()
        .icon_name("folder-open-symbolic")
        .tooltip_text("Choose output directory")
        .build();

    let dir_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    dir_row.append(&gtk::Label::new(Some("Output Directory")));
    dir_row.append(&dir_lbl);
    dir_row.append(&dir_btn);

    let prefix_entry = gtk::Entry::builder()
        .text(state.borrow().prefix.as_str())
        .placeholder_text("page")
        .hexpand(true)
        .build();

    let prefix_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    prefix_row.append(&gtk::Label::new(Some("Filename Prefix")));
    prefix_row.append(&prefix_entry);

    let btn_jpeg = gtk::ToggleButton::builder().label("JPEG").active(true).build();
    let btn_png = gtk::ToggleButton::builder().label("PNG").group(&btn_jpeg).build();
    let btn_tiff = gtk::ToggleButton::builder().label("TIFF").group(&btn_jpeg).build();

    {
        let p = state.borrow();
        match p.export {
            ExportSettings::Jpeg { .. } => btn_jpeg.set_active(true),
            ExportSettings::Png => btn_png.set_active(true),
            ExportSettings::Tiff => btn_tiff.set_active(true),
        }
    }

    let format_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(0)
        .css_classes(["linked"])
        .build();
    format_box.append(&btn_jpeg);
    format_box.append(&btn_png);
    format_box.append(&btn_tiff);

    let quality_adj = gtk::Adjustment::new(85.0, 1.0, 100.0, 1.0, 10.0, 0.0);
    {
        if let ExportSettings::Jpeg { quality } = state.borrow().export {
            quality_adj.set_value(quality as f64);
        }
    }
    let quality_scale = gtk::Scale::new(gtk::Orientation::Horizontal, Some(&quality_adj));
    quality_scale.set_width_request(180);
    quality_scale.set_draw_value(true);
    quality_scale.set_digits(0);

    let quality_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    quality_row.append(&gtk::Label::new(Some("JPEG Quality")));
    quality_row.append(&quality_scale);

    quality_row.set_visible(btn_jpeg.is_active());

    let format_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    format_row.append(&gtk::Label::new(Some("Format")));
    format_row.append(&format_box);

    let settings_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(12)
        .margin_top(24)
        .margin_bottom(24)
        .margin_start(48)
        .margin_end(48)
        .halign(gtk::Align::Fill)
        .build();
    settings_box.append(&dir_row);
    settings_box.append(&prefix_row);
    settings_box.append(&format_row);
    settings_box.append(&quality_row);

    let export_btn = gtk::Button::builder()
        .label("Export")
        .css_classes(["suggested-action", "pill"])
        .halign(gtk::Align::Center)
        .build();

    let progress = gtk::ProgressBar::builder()
        .show_text(true)
        .visible(false)
        .build();

    let result_buf = gtk::TextBuffer::new(None::<&gtk::TextTagTable>);
    let result_view = gtk::TextView::builder()
        .buffer(&result_buf)
        .editable(false)
        .monospace(true)
        .vexpand(true)
        .build();
    let result_scroll = gtk::ScrolledWindow::builder()
        .child(&result_view)
        .vexpand(true)
        .min_content_height(120)
        .build();

    let action_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_start(48)
        .margin_end(48)
        .build();
    action_box.append(&export_btn);
    action_box.append(&progress);
    action_box.append(&result_scroll);

    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(0)
        .build();
    root.append(&settings_box);
    root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    root.append(&action_box);

    dir_btn.connect_clicked({
        let state = state.clone();
        let dir_lbl = dir_lbl.clone();
        move |btn| {
            let dialog = gtk::FileDialog::builder()
                .title("Choose Output Directory")
                .modal(true)
                .build();
            let parent = btn.root().and_downcast::<gtk::Window>();
            let state = state.clone();
            let dir_lbl = dir_lbl.clone();
            dialog.select_folder(parent.as_ref(), gio::Cancellable::NONE, move |res| {
                let Ok(file) = res else { return };
                let Some(path) = file.path() else { return };
                dir_lbl.set_label(&path.display().to_string());
                state.borrow_mut().output_dir = path;
            });
        }
    });

    {
        let quality_row = quality_row.clone();
        btn_jpeg.connect_toggled(move |btn| {
            quality_row.set_visible(btn.is_active());
        });
    }

    export_btn.connect_clicked({
        let state = state.clone();
        let prefix_entry = prefix_entry.clone();
        let btn_jpeg = btn_jpeg.clone();
        let btn_png = btn_png.clone();
        let quality_adj = quality_adj.clone();
        let export_btn = export_btn.clone();
        let progress = progress.clone();
        let result_buf = result_buf.clone();
        let tx = tx.clone();
        move |_| {
            let format = if btn_jpeg.is_active() {
                ExportSettings::Jpeg { quality: quality_adj.value() as u8 }
            } else if btn_png.is_active() {
                ExportSettings::Png
            } else {
                ExportSettings::Tiff
            };
            {
                let mut p = state.borrow_mut();
                p.prefix = prefix_entry.text().to_string();
                p.export = format;
            }
            let project = state.borrow().clone();

            if project.pages.is_empty() {
                let mut end = result_buf.end_iter();
                result_buf.insert(&mut end, "No pages to export.\n");
                return;
            }
            if project.output_dir.as_os_str().is_empty() {
                let mut end = result_buf.end_iter();
                result_buf.insert(&mut end, "Choose an output directory first.\n");
                return;
            }

            export_btn.set_sensitive(false);
            progress.set_visible(true);
            progress.set_fraction(0.0);
            result_buf.set_text("");

            let tx = tx.clone();
            std::thread::spawn(move || {
                let total = project.pages.len();
                let results = pagecutter_core::pipeline::run_batch(&project, |done, _| {
                    let _ = tx.send_blocking(ExportMsg::Progress(done, total));
                });
                let lines: Vec<(bool, String)> = results
                    .into_iter()
                    .map(|r| match r {
                        Ok(p) => (true, p.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_else(|| p.display().to_string())),
                        Err(e) => (false, e.to_string()),
                    })
                    .collect();
                let _ = tx.send_blocking(ExportMsg::Done(lines));
            });
        }
    });

    glib::MainContext::default().spawn_local({
        let export_btn = export_btn.clone();
        async move {
            while let Ok(msg) = rx.recv().await {
                match msg {
                    ExportMsg::Progress(done, total) => {
                        progress.set_fraction(done as f64 / total as f64);
                        progress.set_text(Some(&format!("{}/{}", done, total)));
                    }
                    ExportMsg::Done(lines) => {
                        export_btn.set_sensitive(true);
                        progress.set_visible(false);
                        let ok = lines.iter().filter(|(s, _)| *s).count();
                        let total = lines.len();
                        let mut end = result_buf.end_iter();
                        for (success, msg) in &lines {
                            let prefix = if *success { "✓ " } else { "✗ " };
                            result_buf.insert(&mut end, &format!("{}{}\n", prefix, msg));
                        }
                        result_buf.insert(
                            &mut end,
                            &format!("\nDone: {}/{} succeeded.\n", ok, total),
                        );
                    }
                }
            }
        }
    });

    root.upcast()
}
