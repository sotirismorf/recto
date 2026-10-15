use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use adw::prelude::*;
use gtk::{gio, glib};

use crate::app::State;
use crate::background::job_queue::JobQueue;
use recto_core::error::Error;
use recto_core::{load_config, save_config};
use recto_core::{
    AppConfig, Command, ExportSettings, JpegQuality, PdfCompression, PdfMeta, PdfOptions, Scale,
};

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
    // Export settings are app preferences, kept in the config file rather
    // than the project: changing them is neither undoable nor a project edit.
    let config = Rc::new(RefCell::new(load_config()));
    let content = build_export_content(state, job_queue, config);
    toolbar_view.set_content(Some(&content));
    dialog.set_content(Some(&toolbar_view));
    dialog.present();
}

fn build_export_content(
    state: State,
    job_queue: Rc<JobQueue>,
    config: Rc<RefCell<AppConfig>>,
) -> gtk::Widget {
    let (tx, rx) = async_channel::unbounded::<ExportMsg>();
    let pdf_meta = Rc::new(RefCell::new(config.borrow().pdf_meta.clone()));
    let p = state.project();

    let settings = config.borrow().export.clone();
    let pdf_opts = match &settings {
        ExportSettings::Pdf(opts) => opts.clone(),
        _ => PdfOptions::default(),
    };

    // ---- Format ComboRow ----------------------------------------------------
    let format_list = gtk::StringList::new(&["JPEG", "PNG", "TIFF", "PDF"]);
    let format_combo = adw::ComboRow::builder()
        .title("Format")
        .model(&format_list)
        .build();
    format_combo.set_selected(match settings {
        ExportSettings::Jpeg { .. } => 0,
        ExportSettings::Png { .. } => 1,
        ExportSettings::Tiff => 2,
        _ => 3,
    });

    // ---- PDF encoding ComboRow ---------------------------------------------
    let pdf_enc_list = gtk::StringList::new(&["JPEG (smaller)", "Lossless"]);
    let pdf_enc_combo = adw::ComboRow::builder()
        .title("PDF encoding")
        .model(&pdf_enc_list)
        .build();
    pdf_enc_combo.set_selected(match pdf_opts.compression {
        PdfCompression::Jpeg => 0,
        PdfCompression::Flate => 1,
    });

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
    let quality = match &settings {
        ExportSettings::Jpeg { quality } => *quality,
        _ => pdf_opts.quality,
    };
    let quality_adj = gtk::Adjustment::new(quality.as_u8() as f64, 1.0, 100.0, 1.0, 10.0, 0.0);
    let quality_row = adw::SpinRow::new(Some(&quality_adj), 1.0, 0);
    quality_row.set_title("JPEG quality");

    // ---- PNG compression SpinRow -------------------------------------------
    let png_comp_adj = gtk::Adjustment::new(3.0, 0.0, 9.0, 1.0, 2.0, 0.0);
    {
        if let ExportSettings::Png { compression } = &settings {
            png_comp_adj.set_value(*compression as f64);
        }
    }
    let png_comp_row = adw::SpinRow::new(Some(&png_comp_adj), 1.0, 0);
    png_comp_row.set_title("PNG compression");

    // ---- Scale SpinRow -----------------------------------------------------
    let scale_adj = gtk::Adjustment::new(
        (config.borrow().export_scale.as_f64() * 100.0).round(),
        1.0,
        100.0,
        1.0,
        10.0,
        0.0,
    );
    let scale_row = adw::SpinRow::new(Some(&scale_adj), 1.0, 0);
    scale_row.set_title("Scale");
    scale_row.set_subtitle("Percentage of original size");

    // ---- Page height SpinRow (PDF only) ------------------------------------
    let page_height_adj =
        gtk::Adjustment::new(pdf_opts.page_height_mm as f64, 0.0, 5000.0, 1.0, 10.0, 0.0);
    let page_height_row = adw::SpinRow::new(Some(&page_height_adj), 1.0, 0);
    page_height_row.set_title("Page height (mm)");
    page_height_row.set_subtitle("0 for original size");

    // ---- Estimate page height SwitchRow (PDF only) -------------------------
    let estimate = p
        .pages
        .first()
        .and_then(recto_core::estimate_page_height_mm);
    let estimate_row = adw::SwitchRow::builder()
        .title("Estimate height from camera")
        .subtitle(match estimate {
            Some(mm) => format!("\u{2248} {mm:.0} mm, from the first page's EXIF focus data"),
            None => "No usable camera data on the first page; page height is used".into(),
        })
        .active(pdf_opts.estimate_page_height)
        .build();

    // While the estimate is in use the spin row shows it and is locked; the
    // user's own value is kept here so unchecking restores it and it stays
    // the saved fallback.
    let manual_height = Rc::new(Cell::new(page_height_adj.value() as u32));
    let apply_estimate_lock = {
        let page_height_adj = page_height_adj.clone();
        let page_height_row = page_height_row.clone();
        let manual_height = manual_height.clone();
        move |active: bool| {
            let shown = match estimate {
                Some(mm) if active => mm.round(),
                _ => manual_height.get() as f64,
            };
            page_height_row.set_sensitive(!(active && estimate.is_some()));
            page_height_adj.set_value(shown);
        }
    };
    apply_estimate_lock(estimate_row.is_active());

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
    quality_group.add(&page_height_row);
    quality_group.add(&estimate_row);

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
        let page_height_row = page_height_row.clone();
        let estimate_row = estimate_row.clone();
        move || {
            let pdf = is_pdf(&format_combo);
            pdf_enc_combo.set_visible(pdf);
            prefix_row.set_visible(!pdf);
            pdf_name_row.set_visible(pdf);
            meta_row.set_visible(pdf);
            quality_row
                .set_visible(is_jpeg(&format_combo) || (pdf && pdf_enc_is_jpeg(&pdf_enc_combo)));
            png_comp_row.set_visible(is_png(&format_combo));
            page_height_row.set_visible(pdf);
            estimate_row.set_visible(pdf);
        }
    };
    update_visibility();

    // ---- Signal handlers ---------------------------------------------------

    // Every control saves the whole settings value, read back from the widgets.
    let save: Rc<dyn Fn()> = Rc::new({
        let config = config.clone();
        let format_combo = format_combo.clone();
        let pdf_enc_combo = pdf_enc_combo.clone();
        let quality_adj = quality_adj.clone();
        let png_comp_adj = png_comp_adj.clone();
        let scale_adj = scale_adj.clone();
        let manual_height = manual_height.clone();
        let estimate_row = estimate_row.clone();
        move || {
            let quality = JpegQuality::new(quality_adj.value() as u8);
            let export = match format_combo.selected() {
                0 => ExportSettings::Jpeg { quality },
                1 => ExportSettings::Png {
                    compression: png_comp_adj.value() as u8,
                },
                2 => ExportSettings::Tiff,
                _ => ExportSettings::Pdf(PdfOptions {
                    compression: if pdf_enc_combo.selected() == 1 {
                        PdfCompression::Flate
                    } else {
                        PdfCompression::Jpeg
                    },
                    quality,
                    page_height_mm: manual_height.get(),
                    estimate_page_height: estimate_row.is_active(),
                }),
            };
            let mut cfg = config.borrow_mut();
            cfg.export = export;
            cfg.export_scale = Scale::new(scale_adj.value() / 100.0);
            if let Err(e) = save_config(&cfg) {
                tracing::error!("Failed to save export settings: {e}");
            }
        }
    });

    format_combo.connect_selected_notify({
        let update = update_visibility.clone();
        let save = save.clone();
        move |_| {
            update();
            save();
        }
    });
    pdf_enc_combo.connect_selected_notify({
        let update = update_visibility.clone();
        let save = save.clone();
        move |_| {
            update();
            save();
        }
    });
    for adj in [&quality_adj, &png_comp_adj, &scale_adj] {
        let save = save.clone();
        adj.connect_value_changed(move |_| save());
    }

    page_height_adj.connect_value_changed({
        let estimate_row = estimate_row.clone();
        let manual_height = manual_height.clone();
        let save = save.clone();
        move |adj| {
            // Ignore the value written by the estimate lock.
            if estimate_row.is_active() && estimate.is_some() {
                return;
            }
            manual_height.set(adj.value() as u32);
            save();
        }
    });

    estimate_row.connect_active_notify({
        let save = save.clone();
        move |row| {
            apply_estimate_lock(row.is_active());
            save();
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
        let config = config.clone();
        let pdf_meta = pdf_meta.clone();
        move |btn| {
            let parent = btn.root().and_downcast::<gtk::Window>();
            show_pdf_meta_dialog(parent, config.clone(), pdf_meta.clone());
        }
    });

    // Export button.
    export_btn.connect_clicked({
        let state = state.clone();
        let prefix_row = prefix_row.clone();
        let pdf_name_row = pdf_name_row.clone();
        let config = config.clone();
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
            // The name is part of the project; only record a real change.
            if state.project().prefix != text {
                state.dispatch(Command::SetPrefix(text));
            }
            let (settings, scale) = {
                let cfg = config.borrow();
                (cfg.export.clone(), cfg.export_scale)
            };

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

            if let ExportSettings::Pdf(opts) = settings {
                let meta = pdf_meta.borrow().clone();
                let out_path = project.output_dir.join(format!("{}.pdf", project.prefix));
                let job_queue = job_queue.clone();
                job_queue.spawn(move || {
                    let result = recto_core::export::export_to_pdf(
                        &project,
                        &out_path,
                        &meta,
                        &opts,
                        scale,
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
            } else if let Some(format) = settings.image_format() {
                let job_queue = job_queue.clone();
                job_queue.spawn(move || {
                    let results = recto_core::export::run_batch(
                        &project,
                        format,
                        scale,
                        cancel,
                        |done, _| {
                            let _ = tx.send_blocking(ExportMsg::Progress(done, total));
                        },
                    );
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

fn show_pdf_meta_dialog(
    parent: Option<gtk::Window>,
    config: Rc<RefCell<AppConfig>>,
    meta_rc: Rc<RefCell<PdfMeta>>,
) {
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
        let config = config.clone();
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
            {
                let mut cfg = config.borrow_mut();
                cfg.pdf_meta = meta_rc.borrow().clone();
                if let Err(e) = save_config(&cfg) {
                    tracing::error!("save config: {e}");
                }
            }
            dialog.close();
        }
    });

    dialog.present();
}
