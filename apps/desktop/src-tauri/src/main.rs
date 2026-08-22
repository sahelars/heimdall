// Windows release builds should open a window, not a console.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    heimdall_desktop_lib::run()
}
