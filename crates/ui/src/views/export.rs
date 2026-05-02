use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::app::State;
use crate::worker::JobQueue;
use recto_core::{load_pdf_meta, save_pdf_meta};
use recto_core::{Command, ExportSettings, JpegQuality, PdfMeta};

enum ExportMsg {
    Progress(usize, usize),
    Done(Vec<(bool, String)>),
}

pub fn show_export_dialog(state: State, parent: &gtk::Window, job_queue: Rc<JobQueue>) {
    let dialog = adw::Window::builder()
        .title("Export")
        .modal(true)
        .transient_for(parent)
        .default_width(540)
        .default_height(580)
        .resizable(false)
        .build();
    let header = adw::HeaderBar::new();
    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    toolbar_view.set_content(Some(&build_export_content(state, job_queue)));
    dialog.set_content(Some(&toolbar_view));
    dialog.present();
}

fn build_export_content(state: State, job_queue: Rc<JobQueue>) -> gtk::Widget {
    let (tx, rx) = async_channel::unbounded::<ExportMsg>();

    // PDF metadata — loaded from ~/.config/recto/pdf_meta.json once.
    let pdf_meta: Rc<RefCell<PdfMeta>> = Rc::new(RefCell::new(load_pdf_meta()));

    // --- Output directory row -----------------------------------------------
    let dir_lbl = gtk::Label::builder()
        .label("No directory chosen")
        .ellipsize(gtk::pango::EllipsizeMode::Middle)
        .xalign(0.0)
        .hexpand(true)
        .css_classes(["dim-label"])
        .build();
    {
        let p = state.project();
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

    // --- Prefix row ---------------------------------------------------------
    let prefix_entry = gtk::Entry::builder()
        .text(state.project().prefix.as_str())
        .placeholder_text("page")
        .hexpand(true)
        .build();

    let prefix_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    prefix_row.append(&gtk::Label::new(Some("Filename Prefix")));
    prefix_row.append(&prefix_entry);

    // --- Format toggle group ------------------------------------------------
    let btn_jpeg = gtk::ToggleButton::builder()
        .label("JPEG")
        .active(true)
        .build();
    let btn_png = gtk::ToggleButton::builder()
        .label("PNG")
        .group(&btn_jpeg)
        .build();
    let btn_tiff = gtk::ToggleButton::builder()
        .label("TIFF")
        .group(&btn_jpeg)
        .build();
    let btn_pdf = gtk::ToggleButton::builder()
        .label("PDF")
        .group(&btn_jpeg)
        .build();

    {
        let p = state.project();
        match p.export {
            ExportSettings::Jpeg { .. } => btn_jpeg.set_active(true),
            ExportSettings::Png => btn_png.set_active(true),
            ExportSettings::Tiff => btn_tiff.set_active(true),
            ExportSettings::Pdf { .. } => btn_pdf.set_active(true),
            _ => {}
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
    format_box.append(&btn_pdf);

    let format_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    format_row.append(&gtk::Label::new(Some("Format")));
    format_row.append(&format_box);

    // --- Quality slider (JPEG and PDF share it) -----------------------------
    let quality_adj = gtk::Adjustment::new(85.0, 1.0, 100.0, 1.0, 10.0, 0.0);
    {
        match state.project().export {
            ExportSettings::Jpeg { quality } | ExportSettings::Pdf { quality } => {
                quality_adj.set_value(quality.as_u8() as f64);
            }
            _ => {}
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

    // --- PDF metadata button (only visible when PDF is selected) ------------
    let meta_btn = gtk::Button::builder()
        .label("PDF Metadata…")
        .tooltip_text("Edit title, author, DPI, and other PDF properties")
        .build();
    let meta_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    meta_row.append(&meta_btn);

    let is_pdf = btn_pdf.is_active();
    quality_row.set_visible(btn_jpeg.is_active() || is_pdf);
    meta_row.set_visible(is_pdf);

    // --- Settings box -------------------------------------------------------
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
    settings_box.append(&meta_row);

    // --- Action area --------------------------------------------------------
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

    let btn_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(12)
        .halign(gtk::Align::Center)
        .build();
    btn_row.append(&export_btn);

    let action_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(8)
        .margin_start(48)
        .margin_end(48)
        .build();
    action_box.append(&btn_row);
    action_box.append(&progress);
    action_box.append(&result_scroll);

    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(0)
        .build();
    root.append(&settings_box);
    root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    root.append(&action_box);

    // --- Signal handlers ----------------------------------------------------

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
                state.dispatch(Command::SetOutputDir(path));
            });
        }
    });

    // Show/hide quality and metadata rows based on active format.
    let update_rows_visibility = {
        let quality_row = quality_row.clone();
        let meta_row = meta_row.clone();
        let btn_jpeg = btn_jpeg.clone();
        let btn_pdf = btn_pdf.clone();
        move || {
            quality_row.set_visible(btn_jpeg.is_active() || btn_pdf.is_active());
            meta_row.set_visible(btn_pdf.is_active());
        }
    };
    for btn in [&btn_jpeg, &btn_png, &btn_tiff, &btn_pdf] {
        btn.connect_toggled({
            let f = update_rows_visibility.clone();
            move |_| f()
        });
    }

    // PDF Metadata dialog.
    meta_btn.connect_clicked({
        let pdf_meta = pdf_meta.clone();
        move |btn| {
            let parent = btn.root().and_downcast::<gtk::Window>();
            show_pdf_meta_dialog(parent, pdf_meta.clone());
        }
    });

    // Export button.
    export_btn.connect_clicked({
        let state = state.clone();
        let prefix_entry = prefix_entry.clone();
        let btn_jpeg = btn_jpeg.clone();
        let btn_png = btn_png.clone();
        let btn_pdf = btn_pdf.clone();
        let quality_adj = quality_adj.clone();
        let export_btn = export_btn.clone();
        let progress = progress.clone();
        let result_buf = result_buf.clone();
        let tx = tx.clone();
        let pdf_meta = pdf_meta.clone();
        let job_queue = job_queue.clone();
        move |_| {
            let quality = JpegQuality::new(quality_adj.value() as u8);
            let format = if btn_jpeg.is_active() {
                ExportSettings::Jpeg { quality }
            } else if btn_png.is_active() {
                ExportSettings::Png
            } else if btn_pdf.is_active() {
                ExportSettings::Pdf { quality }
            } else {
                ExportSettings::Tiff
            };
            {
                state.dispatch(Command::SetPrefix(prefix_entry.text().to_string()));
                state.dispatch(Command::SetExportSettings(format.clone()));
            }
            let project = state.project().clone();

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

            if matches!(format, ExportSettings::Pdf { .. }) {
                let meta = pdf_meta.borrow().clone();
                let out_path = project.output_dir.join(format!("{}.pdf", project.prefix));
                let job_queue = job_queue.clone();
                job_queue.spawn(move || {
                    let total = project.pages.len();
                    let result = recto_core::export::export_to_pdf(
                        &project,
                        &out_path,
                        &meta,
                        quality,
                        |done, _| {
                            let _ = tx.send_blocking(ExportMsg::Progress(done, total));
                        },
                    );
                    let lines = match result {
                        Ok(()) => {
                            let name = out_path
                                .file_name()
                                .map(|n| n.to_string_lossy().into_owned())
                                .unwrap_or_default();
                            vec![(true, name)]
                        }
                        Err(e) => vec![(false, e.to_string())],
                    };
                    let _ = tx.send_blocking(ExportMsg::Done(lines));
                });
            } else {
                let job_queue = job_queue.clone();
                job_queue.spawn(move || {
                    let total = project.pages.len();
                    let results = recto_core::export::run_batch(&project, |done, _| {
                        let _ = tx.send_blocking(ExportMsg::Progress(done, total));
                    });
                    let lines: Vec<(bool, String)> = results
                        .into_iter()
                        .map(|r| match r {
                            Ok(p) => (
                                true,
                                p.file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_else(|| p.display().to_string()),
                            ),
                            Err(e) => (false, e.to_string()),
                        })
                        .collect();
                    let _ = tx.send_blocking(ExportMsg::Done(lines));
                });
            }
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
                        result_buf
                            .insert(&mut end, &format!("\nDone: {}/{} succeeded.\n", ok, total));
                    }
                }
            }
        }
    });

    root.upcast()
}

fn show_pdf_meta_dialog(parent: Option<gtk::Window>, meta_rc: Rc<RefCell<PdfMeta>>) {
    let dialog = gtk::Window::builder()
        .title("PDF Metadata")
        .modal(true)
        .resizable(false)
        .default_width(400)
        .build();
    if let Some(p) = &parent {
        dialog.set_transient_for(Some(p));
    }

    let meta = meta_rc.borrow().clone();

    let grid = gtk::Grid::builder()
        .row_spacing(12)
        .column_spacing(12)
        .margin_top(24)
        .margin_bottom(12)
        .margin_start(24)
        .margin_end(24)
        .build();

    let make_row = |grid: &gtk::Grid, row: i32, label: &str, value: &str| -> gtk::Entry {
        let lbl = gtk::Label::builder().label(label).xalign(1.0).build();
        let entry = gtk::Entry::builder().text(value).hexpand(true).build();
        grid.attach(&lbl, 0, row, 1, 1);
        grid.attach(&entry, 1, row, 1, 1);
        entry
    };

    let title_entry = make_row(&grid, 0, "Title:", &meta.title);
    let author_entry = make_row(&grid, 1, "Author:", &meta.author);
    let creator_entry = make_row(&grid, 2, "Creator:", &meta.creator);
    let subject_entry = make_row(&grid, 3, "Subject:", &meta.subject);
    let keywords_entry = make_row(&grid, 4, "Keywords:", &meta.keywords);

    let dpi_lbl = gtk::Label::builder().label("DPI:").xalign(1.0).build();
    let dpi_adj = gtk::Adjustment::new(meta.dpi.as_f64(), 72.0, 1200.0, 1.0, 50.0, 0.0);
    let dpi_spin = gtk::SpinButton::new(Some(&dpi_adj), 1.0, 0);
    dpi_spin.set_hexpand(true);
    grid.attach(&dpi_lbl, 0, 5, 1, 1);
    grid.attach(&dpi_spin, 1, 5, 1, 1);

    let btn_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .margin_top(8)
        .margin_bottom(24)
        .margin_start(24)
        .margin_end(24)
        .build();
    let cancel_btn = gtk::Button::builder().label("Cancel").build();
    let save_btn = gtk::Button::builder()
        .label("Save")
        .css_classes(["suggested-action"])
        .build();
    btn_box.append(&cancel_btn);
    btn_box.append(&save_btn);

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .build();
    content.append(&grid);
    content.append(&btn_box);
    dialog.set_child(Some(&content));

    cancel_btn.connect_clicked({
        let dialog = dialog.clone();
        move |_| dialog.close()
    });

    save_btn.connect_clicked({
        let dialog = dialog.clone();
        let meta_rc = meta_rc.clone();
        let title_entry = title_entry.clone();
        let author_entry = author_entry.clone();
        let creator_entry = creator_entry.clone();
        let subject_entry = subject_entry.clone();
        let keywords_entry = keywords_entry.clone();
        let dpi_adj = dpi_adj.clone();
        move |_| {
            {
                let mut m = meta_rc.borrow_mut();
                m.title = title_entry.text().to_string();
                m.author = author_entry.text().to_string();
                m.creator = creator_entry.text().to_string();
                m.subject = subject_entry.text().to_string();
                m.keywords = keywords_entry.text().to_string();
                m.dpi = recto_core::Dpi::new(dpi_adj.value());
            }
            if let Err(e) = save_pdf_meta(&meta_rc.borrow()) {
                tracing::error!("save pdf meta: {e}");
            }
            dialog.close();
        }
    });

    dialog.present();
}
