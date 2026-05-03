use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::app::State;
use crate::worker::JobQueue;
use recto_core::error::Error;
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
        .default_width(400)
        .default_height(500)
        .resizable(false)
        .build();
    let header = adw::HeaderBar::new();
    let toolbar_view = adw::ToolbarView::new();
    toolbar_view.add_top_bar(&header);
    let content = build_export_content(state, job_queue);
    toolbar_view.set_content(Some(&content));
    dialog.set_content(Some(&toolbar_view));
    dialog.present();
}

fn build_export_content(state: State, job_queue: Rc<JobQueue>) -> gtk::Widget {
    let (tx, rx) = async_channel::unbounded::<ExportMsg>();
    let pdf_meta: Rc<RefCell<PdfMeta>> = Rc::new(RefCell::new(load_pdf_meta()));
    let p = state.project();

    // ---- Format ComboRow ----------------------------------------------------
    let format_list = gtk::StringList::new(&["JPEG", "PNG", "TIFF", "PDF"]);
    let format_combo = adw::ComboRow::builder()
        .title("Format")
        .model(&format_list)
        .build();
    {
        match p.export {
            ExportSettings::Jpeg { .. } => format_combo.set_selected(0),
            ExportSettings::Png { .. } => format_combo.set_selected(1),
            ExportSettings::Tiff => format_combo.set_selected(2),
            ExportSettings::Pdf { .. } => format_combo.set_selected(3),
            _ => {}
        }
    }

    // ---- PDF encoding ComboRow ---------------------------------------------
    let pdf_enc_list = gtk::StringList::new(&["JPEG", "Flate", "CCITT"]);
    let pdf_enc_combo = adw::ComboRow::builder()
        .title("PDF encoding")
        .model(&pdf_enc_list)
        .build();
    {
        if let ExportSettings::Pdf { compression, .. } = &p.export {
            match compression {
                PdfCompression::Jpeg => pdf_enc_combo.set_selected(0),
                PdfCompression::Flate => pdf_enc_combo.set_selected(1),
                PdfCompression::Ccit => pdf_enc_combo.set_selected(2),
            }
        }
    }

    // ---- Output directory ActionRow ----------------------------------------
    let dir_row = adw::ActionRow::builder()
        .title("Output directory")
        .subtitle(if p.output_dir.as_os_str().is_empty() {
            "Choose\u{2026}"
        } else {
            ""
        })
        .build();
    {
        if !p.output_dir.as_os_str().is_empty() {
            dir_row.set_subtitle(&p.output_dir.display().to_string());
        }
    }
    let dir_btn = gtk::Button::builder()
        .icon_name("folder-open-symbolic")
        .valign(gtk::Align::Center)
        .build();
    dir_row.add_suffix(&dir_btn);

    // ---- Filename prefix EntryRow (images) -----------------------------------
    let prefix_row = adw::EntryRow::builder()
        .title("Filename prefix")
        .text(p.prefix.as_str())
        .build();

    // ---- PDF filename EntryRow ----------------------------------------------
    let pdf_name_row = adw::EntryRow::builder()
        .title("PDF filename")
        .text(p.prefix.as_str())
        .build();

    // ---- PDF metadata ActionRow ----------------------------------------------
    let meta_btn = gtk::Button::builder()
        .label("PDF Metadata\u{2026}")
        .valign(gtk::Align::Center)
        .build();
    let meta_row = adw::ActionRow::builder().title("PDF metadata").build();
    meta_row.add_suffix(&meta_btn);

    // ---- Quality SpinRow (JPEG / PDF-JPEG) ----------------------------------
    let quality_adj = gtk::Adjustment::new(85.0, 1.0, 100.0, 1.0, 10.0, 0.0);
    {
        match &p.export {
            ExportSettings::Jpeg { quality } | ExportSettings::Pdf { quality, .. } => {
                quality_adj.set_value(quality.as_u8() as f64);
            }
            _ => {}
        }
    }
    let quality_row = adw::SpinRow::new(Some(&quality_adj), 1.0, 0);
    quality_row.set_title("JPEG quality");

    // ---- PNG compression SpinRow -------------------------------------------
    let png_comp_adj = gtk::Adjustment::new(3.0, 0.0, 9.0, 1.0, 2.0, 0.0);
    {
        if let ExportSettings::Png { compression } = &p.export {
            png_comp_adj.set_value(*compression as f64);
        }
    }
    let png_comp_row = adw::SpinRow::new(Some(&png_comp_adj), 1.0, 0);
    png_comp_row.set_title("PNG compression");

    // ---- Scale SpinRow -----------------------------------------------------
    let scale_adj = gtk::Adjustment::new(
        (p.export_scale.as_f64() * 100.0).round(),
        1.0,
        100.0,
        1.0,
        10.0,
        0.0,
    );
    let scale_row = adw::SpinRow::new(Some(&scale_adj), 1.0, 0);
    scale_row.set_title("Scale");
    scale_row.set_subtitle("Percentage of original size");

    // ---- PreferencesGroup containers ---------------------------------------
    let output_group = adw::PreferencesGroup::builder().title("Format").build();
    output_group.add(&format_combo);
    output_group.add(&pdf_enc_combo);
    output_group.add(&dir_row);
    output_group.add(&prefix_row);
    output_group.add(&pdf_name_row);
    output_group.add(&meta_row);

    let quality_group = adw::PreferencesGroup::builder().title("Quality").build();
    quality_group.add(&quality_row);
    quality_group.add(&png_comp_row);
    quality_group.add(&scale_row);

    // ---- Export button + progress + result ----------------------------------
    let export_btn = gtk::Button::builder()
        .label("Export")
        .css_classes(["suggested-action", "pill"])
        .halign(gtk::Align::Center)
        .build();

    let cancel_btn = gtk::Button::builder()
        .label("Cancel")
        .halign(gtk::Align::Center)
        .visible(false)
        .build();

    let btn_row = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::Center)
        .build();
    btn_row.append(&export_btn);
    btn_row.append(&cancel_btn);

    let current_cancel: Rc<RefCell<Option<Arc<AtomicBool>>>> = Rc::new(RefCell::new(None));

    cancel_btn.connect_clicked({
        let cancel_btn = cancel_btn.clone();
        let current_cancel = current_cancel.clone();
        move |btn| {
            if let Some(cancel) = current_cancel.borrow().as_ref() {
                cancel.store(true, Ordering::Relaxed);
            }
            btn.set_sensitive(false);
            btn.set_label("Cancelling\u{2026}");
            let _ = cancel_btn;
        }
    });

    let progress = gtk::ProgressBar::builder()
        .show_text(false)
        .visible(false)
        .margin_top(4)
        .build();

    let status_lbl = gtk::Label::builder()
        .css_classes(["dim-label", "caption"])
        .halign(gtk::Align::Center)
        .visible(false)
        .margin_top(4)
        .build();

    let result_lbl = gtk::Label::builder()
        .wrap(true)
        .xalign(0.0)
        .visible(false)
        .margin_top(6)
        .build();

    let action_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(0)
        .margin_top(18)
        .margin_bottom(12)
        .build();
    action_box.append(&btn_row);
    action_box.append(&status_lbl);
    action_box.append(&progress);
    action_box.append(&result_lbl);

    // ---- Root scrollable assembly ------------------------------------------
    let content_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(0)
        .margin_start(12)
        .margin_end(12)
        .margin_top(6)
        .build();
    content_box.append(&output_group);
    content_box.append(&quality_group);
    content_box.append(&action_box);

    // ---- Visibility logic --------------------------------------------------
    fn is_jpeg(format_combo: &adw::ComboRow) -> bool {
        format_combo.selected() == 0
    }
    fn is_png(format_combo: &adw::ComboRow) -> bool {
        format_combo.selected() == 1
    }
    fn is_pdf(format_combo: &adw::ComboRow) -> bool {
        format_combo.selected() == 3
    }
    fn pdf_enc_is_jpeg(pdf_enc_combo: &adw::ComboRow) -> bool {
        pdf_enc_combo.selected() == 0
    }

    let update_visibility = {
        let format_combo = format_combo.clone();
        let pdf_enc_combo = pdf_enc_combo.clone();
        let prefix_row = prefix_row.clone();
        let pdf_name_row = pdf_name_row.clone();
        let meta_row = meta_row.clone();
        let quality_row = quality_row.clone();
        let png_comp_row = png_comp_row.clone();
        move || {
            let pdf = is_pdf(&format_combo);
            pdf_enc_combo.set_visible(pdf);
            prefix_row.set_visible(!pdf);
            pdf_name_row.set_visible(pdf);
            meta_row.set_visible(pdf);
            quality_row
                .set_visible(is_jpeg(&format_combo) || (pdf && pdf_enc_is_jpeg(&pdf_enc_combo)));
            png_comp_row.set_visible(is_png(&format_combo));
        }
    };
    update_visibility();

    // ---- Signal handlers ---------------------------------------------------

    // Format combo changes → dispatch + update visibility.
    format_combo.connect_selected_notify({
        let state = state.clone();
        let update = update_visibility.clone();
        let quality_adj = quality_adj.clone();
        let png_comp_adj = png_comp_adj.clone();
        let pdf_enc_combo = pdf_enc_combo.clone();
        move |combo| {
            update();
            let idx = combo.selected();
            let settings = match idx {
                0 => ExportSettings::Jpeg {
                    quality: JpegQuality::new(quality_adj.value() as u8),
                },
                1 => ExportSettings::Png {
                    compression: png_comp_adj.value() as u8,
                },
                2 => ExportSettings::Tiff,
                3 => {
                    let enc = match pdf_enc_combo.selected() {
                        0 => PdfCompression::Jpeg,
                        1 => PdfCompression::Flate,
                        2 => PdfCompression::Ccit,
                        _ => PdfCompression::Jpeg,
                    };
                    ExportSettings::Pdf {
                        compression: enc,
                        quality: JpegQuality::new(quality_adj.value() as u8),
                    }
                }
                _ => return,
            };
            state.dispatch(Command::SetExportSettings(settings));
        }
    });

    // PDF encoding combo changes → dispatch + update visibility.
    pdf_enc_combo.connect_selected_notify({
        let state = state.clone();
        let update = update_visibility.clone();
        let quality_adj = quality_adj.clone();
        move |combo| {
            update();
            let enc = match combo.selected() {
                0 => PdfCompression::Jpeg,
                1 => PdfCompression::Flate,
                2 => PdfCompression::Ccit,
                _ => return,
            };
            state.dispatch(Command::SetExportSettings(ExportSettings::Pdf {
                compression: enc,
                quality: JpegQuality::new(quality_adj.value() as u8),
            }));
        }
    });

    // Quality adjustment.
    quality_adj.connect_value_changed({
        let state = state.clone();
        let format_combo = format_combo.clone();
        let pdf_enc_combo = pdf_enc_combo.clone();
        move |adj| {
            let q = JpegQuality::new(adj.value() as u8);
            if is_jpeg(&format_combo) {
                state.dispatch(Command::SetExportSettings(ExportSettings::Jpeg {
                    quality: q,
                }));
            } else if is_pdf(&format_combo) && pdf_enc_is_jpeg(&pdf_enc_combo) {
                state.dispatch(Command::SetExportSettings(ExportSettings::Pdf {
                    compression: PdfCompression::Jpeg,
                    quality: q,
                }));
            }
        }
    });

    // PNG compression adjustment.
    png_comp_adj.connect_value_changed({
        let state = state.clone();
        let format_combo = format_combo.clone();
        move |adj| {
            if is_png(&format_combo) {
                state.dispatch(Command::SetExportSettings(ExportSettings::Png {
                    compression: adj.value() as u8,
                }));
            }
        }
    });

    // Scale adjustment.
    scale_adj.connect_value_changed({
        let state = state.clone();
        move |adj| {
            state.dispatch(Command::SetExportScale(Scale::new(adj.value() / 100.0)));
        }
    });

    // Output directory button.
    dir_btn.connect_clicked({
        let state = state.clone();
        let dir_row = dir_row.clone();
        move |btn| {
            let dialog = gtk::FileDialog::builder()
                .title("Choose Output Directory")
                .modal(true)
                .build();
            let parent = btn.root().and_downcast::<gtk::Window>();
            let dir_row = dir_row.clone();
            let state = state.clone();
            dialog.select_folder(parent.as_ref(), gio::Cancellable::NONE, move |res| {
                let Ok(file) = res else { return };
                let Some(path) = file.path() else { return };
                dir_row.set_subtitle(&path.display().to_string());
                state.dispatch(Command::SetOutputDir(path));
            });
        }
    });

    // PDF metadata button.
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
        let prefix_row = prefix_row.clone();
        let pdf_name_row = pdf_name_row.clone();
        let quality_adj = quality_adj.clone();
        let export_btn = export_btn.clone();
        let cancel_btn = cancel_btn.clone();
        let status_lbl = status_lbl.clone();
        let progress = progress.clone();
        let result_lbl = result_lbl.clone();
        let tx = tx.clone();
        let pdf_meta = pdf_meta.clone();
        let job_queue = job_queue.clone();
        let format_combo = format_combo.clone();
        let current_cancel = current_cancel.clone();
        move |_| {
            let is_pdf = is_pdf(&format_combo);
            let text = if is_pdf {
                pdf_name_row.text().to_string()
            } else {
                prefix_row.text().to_string()
            };
            state.dispatch(Command::SetPrefix(text));

            let project = state.project().clone();

            if project.pages.is_empty() {
                result_lbl.set_visible(true);
                result_lbl.set_label("No pages to export.");
                return;
            }
            if project.output_dir.as_os_str().is_empty() {
                result_lbl.set_visible(true);
                result_lbl.set_label("Choose an output directory first.");
                return;
            }

            let total = project.pages.len();
            export_btn.set_visible(false);
            cancel_btn.set_visible(true);
            status_lbl.set_visible(true);
            status_lbl.set_label("Exporting...");
            progress.set_visible(true);
            progress.set_fraction(0.0);
            result_lbl.set_visible(false);

            let cancel = Arc::new(AtomicBool::new(false));
            current_cancel.replace(Some(cancel.clone()));

            cancel_btn.set_sensitive(true);
            cancel_btn.set_label("Cancel");

            let tx = tx.clone();

            if is_pdf {
                let meta = pdf_meta.borrow().clone();
                let quality = JpegQuality::new(quality_adj.value() as u8);
                let out_path = project.output_dir.join(format!("{}.pdf", project.prefix));
                let job_queue = job_queue.clone();
                job_queue.spawn(move || {
                    let result = recto_core::export::export_to_pdf(
                        &project,
                        &out_path,
                        &meta,
                        quality,
                        cancel,
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
                        Err(e) => {
                            let msg = if matches!(e, Error::Cancelled) {
                                "Cancelled".to_string()
                            } else {
                                e.to_string()
                            };
                            vec![(false, msg)]
                        }
                    };
                    let _ = tx.send_blocking(ExportMsg::Done(lines));
                });
            } else {
                let job_queue = job_queue.clone();
                job_queue.spawn(move || {
                    let results = recto_core::export::run_batch(&project, cancel, |done, _| {
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
                            Err(e) => {
                                let msg = if matches!(e, Error::Cancelled) {
                                    "Cancelled".to_string()
                                } else {
                                    e.to_string()
                                };
                                (false, msg)
                            }
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
        let cancel_btn = cancel_btn.clone();
        let status_lbl = status_lbl.clone();
        async move {
            while let Ok(msg) = rx.recv().await {
                match msg {
                    ExportMsg::Progress(done, total) => {
                        progress.set_fraction(done as f64 / total as f64);
                        status_lbl.set_label(&format!("Processed {}/{}", done, total));
                    }
                    ExportMsg::Done(lines) => {
                        let cancelled =
                            lines.len() == 1 && !lines[0].0 && lines[0].1 == "Cancelled";

                        export_btn.set_visible(true);
                        cancel_btn.set_visible(false);
                        cancel_btn.set_label("Cancel");
                        cancel_btn.set_sensitive(true);
                        progress.set_visible(false);

                        if cancelled {
                            status_lbl.set_label("Export cancelled");
                            result_lbl.set_visible(false);
                        } else {
                            status_lbl.set_visible(false);
                            let ok = lines.iter().filter(|(s, _)| *s).count();
                            let total = lines.len();
                            let mut text = String::new();
                            for (success, msg) in &lines {
                                let prefix = if *success { "\u{2713} " } else { "\u{2717} " };
                                text.push_str(&format!("{}{}\n", prefix, msg));
                            }
                            text.push_str(&format!("\n{} of {} files exported", ok, total));
                            result_lbl.set_visible(true);
                            result_lbl.set_label(&text);
                        }
                    }
                }
            }
        }
    });

    content_box.upcast()
}

fn show_pdf_meta_dialog(parent: Option<gtk::Window>, meta_rc: Rc<RefCell<PdfMeta>>) {
    let dialog = adw::Window::builder()
        .title("PDF Metadata")
        .modal(true)
        .resizable(false)
        .default_width(400)
        .build();
    if let Some(p) = &parent {
        dialog.set_transient_for(Some(p));
    }

    let meta = meta_rc.borrow().clone();

    let title_row = adw::EntryRow::builder()
        .title("Title")
        .text(&meta.title)
        .build();
    let author_row = adw::EntryRow::builder()
        .title("Author")
        .text(&meta.author)
        .build();
    let creator_row = adw::EntryRow::builder()
        .title("Creator")
        .text(&meta.creator)
        .build();
    let subject_row = adw::EntryRow::builder()
        .title("Subject")
        .text(&meta.subject)
        .build();
    let keywords_row = adw::EntryRow::builder()
        .title("Keywords")
        .text(&meta.keywords)
        .build();

    let dpi_adj = gtk::Adjustment::new(meta.dpi.as_f64(), 72.0, 1200.0, 1.0, 50.0, 0.0);
    let dpi_row = adw::SpinRow::new(Some(&dpi_adj), 1.0, 0);
    dpi_row.set_title("DPI");

    let group = adw::PreferencesGroup::new();
    group.add(&title_row);
    group.add(&author_row);
    group.add(&creator_row);
    group.add(&subject_row);
    group.add(&keywords_row);
    group.add(&dpi_row);

    let content = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .margin_start(12)
        .margin_end(12)
        .margin_top(12)
        .margin_bottom(12)
        .build();
    content.append(&group);

    let cancel_btn = gtk::Button::builder().label("Cancel").build();
    let save_btn = gtk::Button::builder()
        .label("Save")
        .css_classes(["suggested-action"])
        .build();

    let btn_box = gtk::Box::builder()
        .orientation(gtk::Orientation::Horizontal)
        .spacing(8)
        .halign(gtk::Align::End)
        .homogeneous(true)
        .margin_top(12)
        .build();
    btn_box.append(&cancel_btn);
    btn_box.append(&save_btn);
    content.append(&btn_box);

    dialog.set_content(Some(&content));

    cancel_btn.connect_clicked(glib::clone!(
        #[weak]
        dialog,
        move |_| dialog.close()
    ));

    save_btn.connect_clicked({
        let dialog = dialog.clone();
        let meta_rc = meta_rc.clone();
        let title_row = title_row.clone();
        let author_row = author_row.clone();
        let creator_row = creator_row.clone();
        let subject_row = subject_row.clone();
        let keywords_row = keywords_row.clone();
        let dpi_adj = dpi_adj.clone();
        move |_| {
            {
                let mut m = meta_rc.borrow_mut();
                m.title = title_row.text().to_string();
                m.author = author_row.text().to_string();
                m.creator = creator_row.text().to_string();
                m.subject = subject_row.text().to_string();
                m.keywords = keywords_row.text().to_string();
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
