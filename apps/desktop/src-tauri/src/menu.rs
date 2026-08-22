//! The application menu.
//!
//! Tauri's default menu has no Settings item and no supported way to insert one
//! into the predefined app submenu, so the whole menu is built here. Everything
//! except "Settings…" is a predefined item, which keeps the system's own
//! behaviour and localization rather than reimplementing them.
//!
//! The **Edit** submenu is not decoration. Installing a custom menu replaces the
//! macOS default, and without Edit the standard Cut/Copy/Paste/Undo
//! accelerators stop reaching the editor — which is a strange way for a text
//! editor to behave.

use tauri::menu::{AboutMetadata, Menu, MenuItem, SubmenuBuilder};
use tauri::{AppHandle, Runtime};

/// The id the Settings item reports.
pub const SETTINGS_ITEM: &str = "settings";

/// The event the window listens for. Kept beside the id so the two cannot be
/// renamed independently.
pub const SETTINGS_EVENT: &str = "menu:settings";

pub fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    let package = app.package_info();
    let about = AboutMetadata {
        name: Some(package.name.clone()),
        version: Some(package.version.to_string()),
        ..Default::default()
    };

    // ⌘, is the platform convention, and the accelerator is what makes it work
    // without a global-shortcut plugin.
    let settings = MenuItem::with_id(
        app,
        SETTINGS_ITEM,
        "Settings…",
        true,
        Some("CmdOrCtrl+,"),
    )?;

    let app_menu = SubmenuBuilder::new(app, package.name.clone())
        .about(Some(about))
        .separator()
        .item(&settings)
        .separator()
        .services()
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .quit()
        .build()?;

    let file_menu = SubmenuBuilder::new(app, "File").close_window().build()?;

    let edit_menu = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;

    let view_menu = SubmenuBuilder::new(app, "View").fullscreen().build()?;

    let window_menu = SubmenuBuilder::new(app, "Window")
        .minimize()
        .maximize()
        .separator()
        .close_window()
        .build()?;

    Menu::with_items(
        app,
        &[&app_menu, &file_menu, &edit_menu, &view_menu, &window_menu],
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_settings_event_is_the_one_the_window_listens_for() {
        // The frontend subscribes to this exact string. Renaming one side
        // without the other would leave a menu item that silently does nothing,
        // which no type checks across the bridge.
        assert_eq!(SETTINGS_EVENT, "menu:settings");
        assert_eq!(SETTINGS_ITEM, "settings");
    }
}
