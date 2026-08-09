mod commands;
mod platform;
mod runtime;

use runtime::DesktopRuntime;

/// Launch the native `LadoFlow` desktop host.
///
/// # Panics
///
/// Panics when Tauri cannot initialize or the application event loop exits
/// with a fatal runtime error.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(DesktopRuntime::default())
        .invoke_handler(tauri::generate_handler![
            commands::get_host_snapshot,
            commands::start_loopback,
            commands::stop_loopback,
            commands::request_screen_capture_access,
        ])
        .run(tauri::generate_context!())
        .expect("error while running LadoFlow");
}
