// Tauri command extractors are intentionally passed by value; the framework
// supplies lightweight handles rather than transferring managed state.
#![allow(clippy::needless_pass_by_value)]

use tauri::State;

use crate::{
    platform::request_capture_access,
    runtime::{DesktopRuntime, HostSnapshot, LoopbackConfig},
};

#[tauri::command]
pub fn get_host_snapshot(runtime: State<'_, DesktopRuntime>) -> HostSnapshot {
    runtime.snapshot()
}

#[tauri::command]
pub fn start_loopback(
    runtime: State<'_, DesktopRuntime>,
    config: LoopbackConfig,
) -> Result<HostSnapshot, String> {
    runtime.start(config)
}

#[tauri::command]
pub fn stop_loopback(runtime: State<'_, DesktopRuntime>) -> Result<HostSnapshot, String> {
    runtime.stop()
}

#[tauri::command]
pub fn request_screen_capture_access(runtime: State<'_, DesktopRuntime>) -> HostSnapshot {
    let _status = request_capture_access();
    runtime.snapshot()
}
