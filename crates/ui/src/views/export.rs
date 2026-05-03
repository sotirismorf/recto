use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::app::State;
use crate::worker::JobQueue;
use recto_core::{load_pdf_meta, save_pdf_meta};
use recto_core::{Command, ExportSettings, JpegQuality, PdfCompression, PdfMeta, Scale};

enum ExportMsg {
    Progress(usize, usize),
    Done(Vec<(bool, String)>),
}

pub fn show_export_dialog(state: State, parent: &gtk::Window, job_queue: Rc<JobQueue>) {
    let dialog = adw::Window::builder()
        .title("Export")
        .modal(true)
        .transient_for(parent)
        .default_width(500)
        .default_height(540)
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

    let pdf_meta: Rc<RefCell<PdfMeta>> = Rc::new(RefCell::new(load_pdf_meta()));

    // --- Format toggle group --------------------------------------------------
    let btn_jpeg = gtk::ToggleButton::builder().label("JPEG").active(true).build();
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
            ExportSettings::Png { .. } => btn_png.set_active(true),
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
    format_row.append(&gtk::Label::builder().label("Format").xalign(1.0).width_request(80).build());
    format_row.append(&format_box);

    // --- PDF encoding sub-selector (visible when PDF is selected) -------------
    let btn_pdf_jpeg = gtk::ToggleButton::builder().label("JPEG").active(true).build();
    let btn_pdf_flate = gtk::ToggleButton::builder()
        .label("Flate")
        .group(&btn_pdf_jpeg)
        .build();
    let btn_pdf_ccit = gtk::ToggleButton::builder()
        .label("CCITT")
        .group(&btn_pdf_jpeg)
        .build();

    {
        if let ExportSettings::Pdf { compression, .. } = state.project().export {
            match compression {
                PdfCompression::Jpeg => btn_pdf_jpeg.set_active(true),
                PdfCompression::Flate => btn_pdf_flate.set_active(true),
                PdfCompression::Ccit => btn_pdf_ccit.set_active(true),
            }
        }
    }

    let pdf_enc_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(0)
        .css_classes(["linked"])
        .build();
    pdf_enc_box.append(&btn_pdf_jpeg);
    pdf_enc_box.append(&btn_pdf_flate);
    pdf_enc_box.append(&btn_pdf_ccit);

    let pdf_enc_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .margin_start(80)
        .build();
    pdf_enc_row.append(&pdf_enc_box);

    // --- Output directory row -------------------------------------------------
    let dir_lbl = gtk::Label::builder()
        .label("Choose directory\u{2026}")
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
    dir_row.append(&gtk::Label::builder().label("Output").xalign(1.0).width_request(80).build());
    dir_row.append(&dir_lbl);
    dir_row.append(&dir_btn);

    // --- Filename prefix (image modes) ----------------------------------------
    let prefix_entry = gtk::Entry::builder()
        .text(state.project().prefix.as_str())
        .placeholder_text("prefix")
        .hexpand(true)
        .width_request(140)
        .build();

    let suffix_lbl = gtk::Label::builder()
        .label("_01.jpg")
        .css_classes(["dim-label"])
        .build();

    let prefix_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    prefix_row.append(&gtk::Label::builder().label("Prefix").xalign(1.0).width_request(80).build());
    let prefix_input = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(0).build();
    prefix_input.append(&prefix_entry);
    prefix_input.append(&suffix_lbl);
    prefix_row.append(&prefix_input);

    // --- Filename (PDF mode) ---------------------------------------------------
    let pdf_name_entry = gtk::Entry::builder()
        .text(state.project().prefix.as_str())
        .placeholder_text("my_book")
        .hexpand(true)
        .width_request(140)
        .build();

    let pdf_name_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    pdf_name_row.append(&gtk::Label::builder().label("Filename").xalign(1.0).width_request(80).build());
    let pdf_name_input = gtk::Box::builder().orientation(gtk::Orientation::Horizontal).spacing(0).build();
    pdf_name_input.append(&pdf_name_entry);
    pdf_name_input.append(&gtk::Label::builder().label(".pdf").css_classes(["dim-label"]).build());
    pdf_name_row.append(&pdf_name_input);

    // --- PDF metadata button ---------------------------------------------------
    let meta_btn = gtk::Button::builder()
        .label("PDF Metadata\u{2026}")
        .tooltip_text("Title, author, DPI, and other PDF properties")
        .halign(gtk::Align::Start)
        .margin_start(80)
        .build();

    // --- Quality slider (JPEG format and PDF+JPEG encoding) --------------------
    let quality_adj = gtk::Adjustment::new(85.0, 1.0, 100.0, 1.0, 10.0, 0.0);
    {
        match state.project().export {
            ExportSettings::Jpeg { quality } | ExportSettings::Pdf { quality, .. } => {
                quality_adj.set_value(quality.as_u8() as f64);
            }
            _ => {}
        }
    }
    let quality_scale = gtk::Scale::new(gtk::Orientation::Horizontal, Some(&quality_adj));
    quality_scale.set_width_request(130);
    quality_scale.set_draw_value(false);
    let quality_spin = gtk::SpinButton::new(Some(&quality_adj), 1.0, 0);
    quality_spin.set_width_request(70);
    let quality_controls = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .build();
    quality_controls.append(&quality_scale);
    quality_controls.append(&quality_spin);

    let quality_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    quality_row.append(&gtk::Label::builder().label("Quality").xalign(1.0).width_request(80).build());
    quality_row.append(&quality_controls);

    // --- PNG compression slider -----------------------------------------------
    let png_comp_adj = gtk::Adjustment::new(3.0, 0.0, 9.0, 1.0, 2.0, 0.0);
    {
        if let ExportSettings::Png { compression } = state.project().export {
            png_comp_adj.set_value(compression as f64);
        }
    }
    let png_comp_scale = gtk::Scale::new(gtk::Orientation::Horizontal, Some(&png_comp_adj));
    png_comp_scale.set_width_request(130);
    png_comp_scale.set_draw_value(false);
    let png_comp_spin = gtk::SpinButton::new(Some(&png_comp_adj), 1.0, 0);
    png_comp_spin.set_width_request(70);
    let png_comp_controls = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .build();
    png_comp_controls.append(&png_comp_scale);
    png_comp_controls.append(&png_comp_spin);

    let png_comp_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    png_comp_row.append(&gtk::Label::builder().label("Compr.").xalign(1.0).width_request(80).build());
    png_comp_row.append(&png_comp_controls);

    // --- Scale slider ---------------------------------------------------------
    let scale_adj = gtk::Adjustment::new(
        (state.project().export_scale.as_f64() * 100.0).round(),
        1.0,
        100.0,
        1.0,
        10.0,
        0.0,
    );
    let scale_slider = gtk::Scale::new(gtk::Orientation::Horizontal, Some(&scale_adj));
    scale_slider.set_width_request(130);
    scale_slider.set_draw_value(false);
    let scale_spin = gtk::SpinButton::new(Some(&scale_adj), 1.0, 0);
    scale_spin.set_width_request(70);
    let scale_controls = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(6)
        .build();
    scale_controls.append(&scale_slider);
    scale_controls.append(&scale_spin);

    let scale_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .build();
    scale_row.append(&gtk::Label::builder().label("Scale").xalign(1.0).width_request(80).build());
    scale_row.append(&scale_controls);

    // --- Export button --------------------------------------------------------
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
        .min_content_height(100)
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
        .margin_start(24)
        .margin_end(24)
        .build();
    action_box.append(&btn_row);
    action_box.append(&progress);
    action_box.append(&result_scroll);

    // --- Root assembly --------------------------------------------------------
    let root = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(0)
        .margin_top(18)
        .margin_bottom(18)
        .build();
    root.append(&format_row);
    root.append(&pdf_enc_row);
    root.append(&dir_row);
    root.append(&prefix_row);
    root.append(&pdf_name_row);
    root.append(&meta_btn);
    root.append(&quality_row);
    root.append(&png_comp_row);
    root.append(&scale_row);
    root.append(&gtk::Separator::new(gtk::Orientation::Horizontal));
    root.append(&action_box);

    // --- Initial visibility ---------------------------------------------------
    let update_visibility = {
        let pdf_enc_row = pdf_enc_row.clone();
        let prefix_row = prefix_row.clone();
        let pdf_name_row = pdf_name_row.clone();
        let meta_btn = meta_btn.clone();
        let quality_row = quality_row.clone();
        let png_comp_row = png_comp_row.clone();
        let suffix_lbl = suffix_lbl.clone();
        let btn_jpeg = btn_jpeg.clone();
        let btn_png = btn_png.clone();
        let btn_pdf = btn_pdf.clone();
        let btn_pdf_jpeg = btn_pdf_jpeg.clone();
        move || {
            let is_jpeg = btn_jpeg.is_active();
            let is_png = btn_png.is_active();
            let is_pdf = btn_pdf.is_active();
            let is_tiff = !is_jpeg && !is_png && !is_pdf;

            pdf_enc_row.set_visible(is_pdf);
            prefix_row.set_visible(!is_pdf);
            pdf_name_row.set_visible(is_pdf);
            meta_btn.set_visible(is_pdf);
            quality_row.set_visible(is_jpeg || (is_pdf && btn_pdf_jpeg.is_active()));
            png_comp_row.set_visible(is_png);

            if is_jpeg {
                suffix_lbl.set_label("_01.jpg");
            } else if is_png {
                suffix_lbl.set_label("_01.png");
            } else if is_tiff {
                suffix_lbl.set_label("_01.tiff");
            }
        }
    };
    update_visibility();

    // --- Signal handlers ------------------------------------------------------

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

    // Format button handlers.
    for (btn, settings) in [
        (&btn_jpeg, ExportSettings::Jpeg { quality: JpegQuality::new(quality_adj.value() as u8) }),
        (&btn_png, ExportSettings::Png { compression: png_comp_adj.value() as u8 }),
        (&btn_tiff, ExportSettings::Tiff),
    ] {
        btn.connect_toggled({
            let state = state.clone();
            let update = update_visibility.clone();
            let settings = settings.clone();
            let quality_adj = quality_adj.clone();
            let png_comp_adj = png_comp_adj.clone();
            move |btn| {
                if !btn.is_active() { return; }
                update();
                let s = match &settings {
                    ExportSettings::Jpeg { quality: _ } => {
                        ExportSettings::Jpeg { quality: JpegQuality::new(quality_adj.value() as u8) }
                    }
                    ExportSettings::Png { compression: _ } => {
                        ExportSettings::Png { compression: png_comp_adj.value() as u8 }
                    }
                    ExportSettings::Tiff => ExportSettings::Tiff,
                    _ => return,
                };
                state.dispatch(Command::SetExportSettings(s));
            }
        });
    }

    // PDF format button handler.
    btn_pdf.connect_toggled({
        let state = state.clone();
        let update = update_visibility.clone();
        let quality_adj = quality_adj.clone();
        let btn_pdf_jpeg = btn_pdf_jpeg.clone();
        move |btn| {
            if !btn.is_active() { return; }
            update();
            let compression = if btn_pdf_jpeg.is_active() {
                PdfCompression::Jpeg
            } else {
                PdfCompression::Flate
            };
            state.dispatch(Command::SetExportSettings(ExportSettings::Pdf {
                compression,
                quality: JpegQuality::new(quality_adj.value() as u8),
            }));
        }
    });

    // PDF encoding sub-selector handlers.
    for (btn, comp) in [
        (&btn_pdf_jpeg, PdfCompression::Jpeg),
        (&btn_pdf_flate, PdfCompression::Flate),
        (&btn_pdf_ccit, PdfCompression::Ccit),
    ] {
        btn.connect_toggled({
            let state = state.clone();
            let update = update_visibility.clone();
            let quality_adj = quality_adj.clone();
            move |btn| {
                if !btn.is_active() { return; }
                update();
                state.dispatch(Command::SetExportSettings(ExportSettings::Pdf {
                    compression: comp,
                    quality: JpegQuality::new(quality_adj.value() as u8),
                }));
            }
        });
    }

    // Quality slider.
    quality_adj.connect_notify_local(Some("value"), {
        let state = state.clone();
        let btn_jpeg = btn_jpeg.clone();
        let btn_pdf = btn_pdf.clone();
        let btn_pdf_jpeg = btn_pdf_jpeg.clone();
        move |adj, _| {
            if btn_jpeg.is_active() {
                state.dispatch(Command::SetExportSettings(ExportSettings::Jpeg {
                    quality: JpegQuality::new(adj.value() as u8),
                }));
            } else if btn_pdf.is_active() && btn_pdf_jpeg.is_active() {
                state.dispatch(Command::SetExportSettings(ExportSettings::Pdf {
                    compression: PdfCompression::Jpeg,
                    quality: JpegQuality::new(adj.value() as u8),
                }));
            }
        }
    });

    // PNG compression slider.
    png_comp_adj.connect_notify_local(Some("value"), {
        let state = state.clone();
        let btn_png = btn_png.clone();
        move |adj, _| {
            if btn_png.is_active() {
                state.dispatch(Command::SetExportSettings(ExportSettings::Png {
                    compression: adj.value() as u8,
                }));
            }
        }
    });

    // Scale slider.
    scale_adj.connect_notify_local(Some("value"), {
        let state = state.clone();
        move |adj, _| {
            state.dispatch(Command::SetExportScale(Scale::new(adj.value() / 100.0)));
        }
    });

    // PDF metadata dialog.
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
        let pdf_name_entry = pdf_name_entry.clone();
        let btn_pdf = btn_pdf.clone();
        let quality_adj = quality_adj.clone();
        let export_btn = export_btn.clone();
        let progress = progress.clone();
        let result_buf = result_buf.clone();
        let tx = tx.clone();
        let pdf_meta = pdf_meta.clone();
        let job_queue = job_queue.clone();
        move |_| {
            let prefix = prefix_entry.text().to_string();
            let pdf_name = pdf_name_entry.text().to_string();
            let is_pdf = btn_pdf.is_active();

            state.dispatch(Command::SetPrefix(if is_pdf { pdf_name } else { prefix }));

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

            if is_pdf {
                let meta = pdf_meta.borrow().clone();
                let quality = JpegQuality::new(quality_adj.value() as u8);
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

    // Result receiver.
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
                            let prefix = if *success { "\u{2713} " } else { "\u{2717} " };
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
