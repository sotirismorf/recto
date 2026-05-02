use adw::prelude::*;

use crate::app::{new_state, new_store, State};
use crate::steps;

pub fn build(app: &adw::Application) {
    let state: State = new_state();
    let page_store = new_store();
    let paned_sync = steps::PanedSync::new();

    let stack = adw::ViewStack::new();
    stack.add_titled_with_icon(
        &steps::import::build(state.clone(), page_store.clone(), paned_sync.clone()),
        Some("import"),
        "Import",
        "document-open-symbolic",
    );
    stack.add_titled_with_icon(
        &steps::crop::build(state.clone(), page_store.clone(), paned_sync.clone()),
        Some("crop"),
        "Crop",
        "edit-cut-symbolic",
    );
    stack.add_titled_with_icon(
        &steps::colors::build(state.clone(), page_store.clone(), paned_sync.clone()),
        Some("colors"),
        "Colors",
        "preferences-color-symbolic",
    );
    stack.add_titled_with_icon(
        &steps::export::build(state.clone()),
        Some("export"),
        "Export",
        "document-save-symbolic",
    );

    let switcher = adw::ViewSwitcher::builder()
        .stack(&stack)
        .policy(adw::ViewSwitcherPolicy::Wide)
        .build();

    let header = adw::HeaderBar::builder().title_widget(&switcher).build();

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&stack));

    let window = adw::ApplicationWindow::builder()
        .application(app)
        .default_width(1100)
        .default_height(720)
        .content(&toolbar)
        .build();
    window.present();
}
