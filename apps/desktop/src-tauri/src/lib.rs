mod commands;
mod host_protocol;
mod platform;
mod runtime;
mod tether;

use std::sync::Arc;

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
        .manage(Arc::new(DesktopRuntime::default()))
        .invoke_handler(tauri::generate_handler![
            commands::get_host_snapshot,
            commands::start_loopback,
            commands::stop_loopback,
            commands::request_screen_capture_access,
            commands::run_screen_capture_probe,
            commands::prepare_android_usb,
            commands::pair_android_tether,
            commands::discover_android_tether,
            commands::disconnect_android_usb,
            commands::enable_virtual_display,
            commands::disable_virtual_display,
        ])
        .run(tauri::generate_context!())
        .expect("error while running LadoFlow");
}
