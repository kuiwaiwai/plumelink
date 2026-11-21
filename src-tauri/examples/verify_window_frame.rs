//! Isolated native-frame preview: no single-instance plugin, service or tray.
//! Reuses the production configuration and shadow implementation.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[path = "../src/window_frame.rs"]
mod window_frame;

use tauri::Manager;

fn main() {
    tauri::Builder::default()
        .setup(|app| {
            let window = app.get_webview_window("main").unwrap();
            window_frame::apply_shadow(&window)?;
            window.set_title("PlumeLink - Shadow Preview")?;
            window_frame::fit_main_window(&window, true)?;
            window.show()?;
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("native frame preview failed");
}
