// Tauri command extractors are intentionally passed by value; the framework
// supplies lightweight handles rather than transferring managed state.
#![allow(clippy::needless_pass_by_value)]

use std::sync::Arc;

use tauri::State;

use crate::{
    platform::{
        CaptureProbeReport, UsbAccessoryProbeReport, VirtualDisplayActionReport,
        disable_virtual_display as disable_platform_virtual_display,
        enable_virtual_display as enable_platform_virtual_display, probe_screen_capture,
        request_capture_access,
    },
    runtime::{DesktopRuntime, HostSnapshot, LoopbackConfig},
    tether::{
        TetherDiscoveryReport, TetherPairingReport, TetherPairingRequest, discover_tether_endpoints,
    },
};

#[tauri::command]
pub fn get_host_snapshot(runtime: State<'_, Arc<DesktopRuntime>>) -> HostSnapshot {
    runtime.snapshot()
}

#[tauri::command]
pub fn start_loopback(
    runtime: State<'_, Arc<DesktopRuntime>>,
    config: LoopbackConfig,
    display_id: Option<String>,
) -> Result<HostSnapshot, String> {
    runtime.start(config, display_id)
}

#[tauri::command]
pub fn stop_loopback(runtime: State<'_, Arc<DesktopRuntime>>) -> Result<HostSnapshot, String> {
    runtime.stop()
}

#[tauri::command]
pub fn request_screen_capture_access(runtime: State<'_, Arc<DesktopRuntime>>) -> HostSnapshot {
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
pub async fn prepare_android_usb(
    runtime: State<'_, Arc<DesktopRuntime>>,
) -> Result<UsbAccessoryProbeReport, String> {
    let runtime = Arc::clone(runtime.inner());
    tauri::async_runtime::spawn_blocking(move || runtime.prepare_android_usb())
        .await
        .map_err(|error| format!("Android USB preparation worker failed: {error}"))
}

#[tauri::command]
pub async fn pair_android_tether(
    runtime: State<'_, Arc<DesktopRuntime>>,
    request: TetherPairingRequest,
) -> Result<TetherPairingReport, String> {
    let runtime = Arc::clone(runtime.inner());
    tauri::async_runtime::spawn_blocking(move || runtime.pair_android_tether(request))
        .await
        .map_err(|error| format!("Android USB-tether pairing worker failed: {error}"))?
}

#[tauri::command]
pub async fn discover_android_tether() -> Result<TetherDiscoveryReport, String> {
    tauri::async_runtime::spawn_blocking(discover_tether_endpoints)
        .await
        .map_err(|error| format!("Android USB-tether discovery worker failed: {error}"))?
}

#[tauri::command]
pub fn disconnect_android_usb(
    runtime: State<'_, Arc<DesktopRuntime>>,
) -> Result<HostSnapshot, String> {
    runtime.disconnect_android_usb()
}

#[tauri::command]
pub async fn enable_virtual_display() -> Result<VirtualDisplayActionReport, String> {
    tauri::async_runtime::spawn_blocking(enable_platform_virtual_display)
        .await
        .map_err(|error| format!("virtual-display enable worker failed: {error}"))?
}

#[tauri::command]
pub async fn disable_virtual_display(
    runtime: State<'_, Arc<DesktopRuntime>>,
) -> Result<VirtualDisplayActionReport, String> {
    let runtime = Arc::clone(runtime.inner());
    tauri::async_runtime::spawn_blocking(move || {
        let _snapshot = runtime.stop()?;
        disable_platform_virtual_display()
    })
    .await
    .map_err(|error| format!("virtual-display disable worker failed: {error}"))?
}
