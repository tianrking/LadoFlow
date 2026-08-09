use serde::Serialize;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "windows")]
mod windows;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DisplaySource {
    pub id: String,
    pub name: String,
    pub width: u64,
    pub height: u64,
    pub primary: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum CapturePermission {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Granted,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Required,
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    Unsupported,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformStatus {
    pub capture_backend: String,
    pub capture_permission: CapturePermission,
    pub virtual_display_status: String,
    pub displays: Vec<DisplaySource>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureProbeReport {
    pub backend: String,
    pub display_id: String,
    pub display_name: String,
    pub width: u32,
    pub height: u32,
    pub target_fps: u16,
    pub elapsed_ms: u64,
    pub callbacks: u64,
    pub content_frames: u64,
    pub idle_frames: u64,
    pub incomplete_frames: u64,
    pub frames_with_surface: u64,
    pub dirty_rects: u64,
    pub observed_fps: f64,
    pub startup_latency_ms: Option<f64>,
    pub pixel_format: Option<String>,
    pub passed: bool,
}

#[cfg(target_os = "macos")]
pub use macos::{collect_status, probe_screen_capture, request_capture_access};

#[cfg(target_os = "windows")]
pub use windows::{collect_status, probe_screen_capture, request_capture_access};

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
#[must_use]
pub fn collect_status() -> PlatformStatus {
    let backend = match std::env::consts::OS {
        "windows" => "Windows Graphics Capture adapter boundary",
        "linux" => "Wayland/X11 capture adapter boundary",
        _ => "Native capture adapter boundary",
    };

    PlatformStatus {
        capture_backend: backend.to_owned(),
        capture_permission: CapturePermission::Unsupported,
        virtual_display_status: "Native virtual-display adapter is not installed yet.".to_owned(),
        displays: Vec::new(),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
#[must_use]
pub fn request_capture_access() -> PlatformStatus {
    collect_status()
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn probe_screen_capture(
    _display_id: Option<&str>,
    _fps: u16,
) -> Result<CaptureProbeReport, String> {
    Err(format!(
        "native capture probe is not implemented for {}",
        std::env::consts::OS
    ))
}
