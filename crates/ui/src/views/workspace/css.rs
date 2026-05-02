use std::sync::OnceLock;

#[allow(unused_imports)]
use gtk::prelude::*;
use gtk::gdk;

pub(crate) fn load_sidebar_css() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let Some(display) = gdk::Display::default() else {
            return;
        };
        let css = gtk::CssProvider::new();
        css.load_from_string(
            "\
            .sidebar-pane { background: @sidebar_bg_color; }\
            .mode-button { padding: 6px 10px; border-radius: 6px; }\
            .mode-button:checked { background: alpha(@accent_bg_color, 0.15); color: @accent_color; font-weight: 600; }\
            .mode-button:checked:hover { background: alpha(@accent_bg_color, 0.22); }\
            ",
        );
        gtk::style_context_add_provider_for_display(
            &display,
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    });
}

pub(crate) fn load_preset_css() {
    static ONCE: OnceLock<()> = OnceLock::new();
    ONCE.get_or_init(|| {
        let css = gtk::CssProvider::new();
        css.load_from_string(
            "\
            .preset-chip {\
              border-radius: 8px;\
              padding: 2px;\
            }\
            .preset-chip.preset-active {\
              outline: 2px solid @theme_selected_bg_color;\
              outline-offset: 1px;\
            }\
            .preset-label {\
              background: transparent;\
              color: white;\
              font-weight: bold;\
              padding: 4px 10px;\
              border: none;\
              border-radius: 6px;\
              min-height: 24px;\
            }\
            .preset-label:hover {\
              opacity: 0.85;\
            }\
            .preset-lock, .preset-close {\
              background: transparent;\
              color: rgba(255,255,255,0.7);\
              border: none;\
              border-radius: 6px;\
              padding: 2px 4px;\
              min-height: 20px;\
              min-width: 20px;\
            }\
            .preset-lock:hover, .preset-close:hover {\
              background: rgba(255,255,255,0.15);\
              color: white;\
            }\
            .preset-c0 { background: #3584e4; }\
            .preset-c1 { background: #33d17a; }\
            .preset-c2 { background: #ff7800; }\
            .preset-c3 { background: #9141ac; }\
            .preset-c4 { background: #ed333b; }\
            .preset-c5 { background: #1c71d8; }\
            .preset-c6 { background: #c061cb; }\
            .preset-c7 { background: #986a44; }\
        ",
        );
        gtk::style_context_add_provider_for_display(
            &gdk::Display::default().expect("display must be available"),
            &css,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    });
}
