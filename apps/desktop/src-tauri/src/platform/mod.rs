use serde::Serialize;
use std::time::Duration;

#[cfg(not(target_os = "windows"))]
use std::sync::atomic::AtomicBool;

#[cfg(not(target_os = "windows"))]
use ladoflow_protocol::InputEvent;

#[cfg(not(target_os = "windows"))]
use ladoflow_transport::{
    Channel, ConnectionState, Packet, PacketTransport, ReceiveError, SendError, SendReport,
};

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
    pub virtual_display: bool,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum VirtualDisplayState {
    Unsupported,
    ClientMissing,
    NotInstalled,
    ServiceStopped,
    Ready,
    Enabling,
    Enabled,
    Disabling,
    Failed,
    Stopping,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualDisplayStatus {
    pub state: VirtualDisplayState,
    pub detail: String,
    pub service_installed: bool,
    pub service_state: String,
    pub enabled: bool,
    pub device_instance_id: Option<String>,
    pub last_error: Option<String>,
    pub generation: u64,
}

#[cfg(not(target_os = "windows"))]
impl VirtualDisplayStatus {
    pub(crate) fn unsupported(detail: String) -> Self {
        Self {
            state: VirtualDisplayState::Unsupported,
            detail,
            service_installed: false,
            service_state: "unsupported".to_owned(),
            enabled: false,
            device_instance_id: None,
            last_error: None,
            generation: 0,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VirtualDisplayActionReport {
    pub passed: bool,
    pub status: VirtualDisplayStatus,
    pub selected_display_id: Option<String>,
    pub elapsed_ms: u64,
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

// Each target constructs only the states supported by its native adapter.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum UsbLinkState {
    Unsupported,
    Ready,
    Connecting,
    Connected,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlatformStatus {
    pub capture_backend: String,
    pub encoder_status: String,
    pub usb_link_state: UsbLinkState,
    pub usb_status: String,
    pub capture_permission: CapturePermission,
    pub virtual_display: VirtualDisplayStatus,
    pub displays: Vec<DisplaySource>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsbAccessoryProbeReport {
    pub passed: bool,
    pub state: String,
    pub detail: String,
    pub protocol_version: Option<u16>,
    pub bus_number: Option<u8>,
    pub device_address: Option<u8>,
    pub vendor_id: Option<u16>,
    pub product_id: Option<u16>,
    pub interface_number: Option<u8>,
    pub input_endpoint: Option<u8>,
    pub output_endpoint: Option<u8>,
    pub max_packet_size: Option<u16>,
}

impl UsbAccessoryProbeReport {
    fn failed(detail: String) -> Self {
        Self {
            passed: false,
            state: "unavailable".to_owned(),
            detail,
            protocol_version: None,
            bus_number: None,
            device_address: None,
            vendor_id: None,
            product_id: None,
            interface_number: None,
            input_endpoint: None,
            output_endpoint: None,
            max_packet_size: None,
        }
    }
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct H264StreamConfig {
    pub width: u16,
    pub height: u16,
    pub fps: u16,
    pub bitrate_kbps: u32,
}

impl H264StreamConfig {
    pub fn new(width: u16, height: u16, fps: u16, bitrate_kbps: u32) -> Result<Self, String> {
        if width == 0 || height == 0 || width % 2 != 0 || height % 2 != 0 {
            return Err("H.264 dimensions must be non-zero and even".to_owned());
        }
        if !matches!(fps, 30 | 60) {
            return Err("H.264 stream frame rate must be 30 or 60 Hz".to_owned());
        }
        if bitrate_kbps == 0 {
            return Err("H.264 stream bitrate must be non-zero".to_owned());
        }
        Ok(Self {
            width,
            height,
            fps,
            bitrate_kbps,
        })
    }
}

#[derive(Debug)]
pub struct H264AccessUnit {
    pub bytes: Vec<u8>,
    pub timestamp: Duration,
    pub duration: Duration,
    pub keyframe: bool,
}

#[derive(Debug)]
pub struct H264StreamBatch {
    pub encoder_name: String,
    pub access_units: Vec<H264AccessUnit>,
}

#[cfg(target_os = "macos")]
pub use macos::{collect_status, probe_screen_capture, request_capture_access};

#[cfg(target_os = "windows")]
pub use windows::{
    CapturedH264Stream, NativeInputController, UsbAccessoryManager, collect_status,
    disable_virtual_display, enable_virtual_display, probe_screen_capture, request_capture_access,
};

#[cfg(not(target_os = "windows"))]
pub struct CapturedH264Stream;

#[cfg(not(target_os = "windows"))]
impl CapturedH264Stream {
    pub fn start(_config: H264StreamConfig, _display_id: Option<String>) -> Result<Self, String> {
        Err(format!(
            "native capture/H.264 streaming is not implemented for {}",
            std::env::consts::OS
        ))
    }

    // Keep the same instance method contract as the Windows worker so runtime
    // composition remains target-agnostic.
    #[allow(clippy::unused_self)]
    pub fn try_next_batch(&self) -> Result<Option<H264StreamBatch>, String> {
        Err(format!(
            "native capture/H.264 streaming is not implemented for {}",
            std::env::consts::OS
        ))
    }
}

#[cfg(not(target_os = "windows"))]
pub struct NativeInputController;

#[cfg(not(target_os = "windows"))]
impl NativeInputController {
    pub fn new(
        _display_id: Option<&str>,
        _stream_width: u16,
        _stream_height: u16,
    ) -> Result<Self, String> {
        Err(format!(
            "native input injection is not implemented for {}",
            std::env::consts::OS
        ))
    }

    // The non-Windows adapter deliberately preserves the native sink shape.
    #[allow(clippy::unused_self)]
    pub fn inject(&mut self, _event: InputEvent) -> Result<(), String> {
        Err(format!(
            "native input injection is not implemented for {}",
            std::env::consts::OS
        ))
    }
}

#[cfg(not(target_os = "windows"))]
#[derive(Debug, Clone, Default)]
pub struct UsbAccessoryManager;

#[cfg(not(target_os = "windows"))]
impl UsbAccessoryManager {
    #[must_use]
    #[allow(clippy::unused_self)]
    pub fn prepare(&self) -> UsbAccessoryProbeReport {
        UsbAccessoryProbeReport::failed(format!(
            "Android Open Accessory preparation is not implemented for {} yet",
            std::env::consts::OS
        ))
    }

    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    pub fn reconnect(&self, _cancel: &AtomicBool) -> Result<(), String> {
        Err(format!(
            "Android Open Accessory reconnection is not implemented for {} yet",
            std::env::consts::OS
        ))
    }

    // Callers share one fallible disconnect path with the Windows owner even
    // though this target has no resource to release yet.
    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    pub fn disconnect(&self) -> Result<(), String> {
        Ok(())
    }

    #[allow(clippy::unused_self, clippy::unnecessary_wraps)]
    pub fn close_runtime_session(&self) -> Result<(), String> {
        Ok(())
    }

    #[must_use]
    #[allow(clippy::unused_self)]
    pub const fn runtime_status(&self) -> Option<(UsbLinkState, String)> {
        None
    }
}

#[cfg(not(target_os = "windows"))]
impl PacketTransport for UsbAccessoryManager {
    fn connection_state(&self) -> ConnectionState {
        ConnectionState::Disconnected
    }

    fn try_send(&mut self, packet: Packet) -> Result<SendReport, SendError> {
        Err(SendError::Disconnected(packet))
    }

    fn try_receive(&mut self, _channel: Channel) -> Result<Option<Packet>, ReceiveError> {
        Err(ReceiveError::Disconnected)
    }
}

#[cfg(not(target_os = "windows"))]
pub fn enable_virtual_display() -> Result<VirtualDisplayActionReport, String> {
    Err(format!(
        "virtual-display lifecycle control is not implemented for {}",
        std::env::consts::OS
    ))
}

#[cfg(not(target_os = "windows"))]
pub fn disable_virtual_display() -> Result<VirtualDisplayActionReport, String> {
    Err(format!(
        "virtual-display lifecycle control is not implemented for {}",
        std::env::consts::OS
    ))
}

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
        encoder_status: format!(
            "Native encoder capability probe is not implemented for {}",
            std::env::consts::OS
        ),
        usb_link_state: UsbLinkState::Unsupported,
        usb_status: "Android Open Accessory host is not implemented on this platform yet."
            .to_owned(),
        capture_permission: CapturePermission::Unsupported,
        virtual_display: VirtualDisplayStatus::unsupported(
            "Native virtual-display lifecycle control is not implemented on this platform yet."
                .to_owned(),
        ),
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
