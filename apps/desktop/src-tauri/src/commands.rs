// Tauri command extractors are intentionally passed by value; the framework
// supplies lightweight handles rather than transferring managed state.
#![allow(clippy::needless_pass_by_value)]

use tauri::State;

use crate::{
    platform::{
        CaptureProbeReport, UsbAccessoryProbeReport, prepare_android_accessory,
        probe_screen_capture, request_capture_access,
    },
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

#[tauri::command]
pub async fn run_screen_capture_probe(
    display_id: Option<String>,
    fps: u16,
) -> Result<CaptureProbeReport, String> {
    tauri::async_runtime::spawn_blocking(move || probe_screen_capture(display_id.as_deref(), fps))
        .await
        .map_err(|error| format!("native capture probe worker failed: {error}"))?
}

#[tauri::command]
pub async fn prepare_android_usb() -> Result<UsbAccessoryProbeReport, String> {
    tauri::async_runtime::spawn_blocking(prepare_android_accessory)
        .await
        .map_err(|error| format!("Android USB preparation worker failed: {error}"))
}
