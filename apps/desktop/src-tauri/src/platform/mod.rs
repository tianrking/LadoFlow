use serde::Serialize;

#[cfg(target_os = "macos")]
mod macos;

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

#[cfg(target_os = "macos")]
pub use macos::{collect_status, request_capture_access};

#[cfg(not(target_os = "macos"))]
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

#[cfg(not(target_os = "macos"))]
#[must_use]
pub fn request_capture_access() -> PlatformStatus {
    collect_status()
}
