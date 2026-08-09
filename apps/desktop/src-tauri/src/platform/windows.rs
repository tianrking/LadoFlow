//! Windows display discovery and capture capability boundary.
//!
//! Win32 monitor enumeration is synchronous: the callback and its context are
//! valid only for the duration of `EnumDisplayMonitors`. Unsafe code is kept in
//! this module so shared protocol/session crates remain unsafe-free.

#![allow(unsafe_code)]

use std::{collections::HashSet, fmt::Write as _, mem::size_of};

use ::windows::{
    Graphics::Capture::GraphicsCaptureSession,
    Win32::{
        Foundation::{LPARAM, RECT},
        Graphics::Gdi::{
            EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
        },
        UI::WindowsAndMessaging::MONITORINFOF_PRIMARY,
    },
    core::BOOL,
};

use super::{CapturePermission, CaptureProbeReport, DisplaySource, PlatformStatus};

#[derive(Default)]
struct EnumerationContext {
    displays: Vec<DisplaySource>,
    error: Option<String>,
}

/// Collect Windows capture capability and active monitor geometry.
#[must_use]
pub fn collect_status() -> PlatformStatus {
    let capture_support = GraphicsCaptureSession::IsSupported()
        .map_err(|error| format!("Windows.Graphics.Capture probe failed: {error}"));
    let (capture_supported, capture_detail) = match capture_support {
        Ok(true) => (true, "supported".to_owned()),
        Ok(false) => (false, "not supported by this Windows build".to_owned()),
        Err(error) => (false, error),
    };

    let (displays, display_detail) = match enumerate_display_sources() {
        Ok(displays) => (displays, None),
        Err(error) => (Vec::new(), Some(error)),
    };

    let mut backend = format!("Windows.Graphics.Capture ({capture_detail})");
    if let Some(error) = display_detail {
        let _ = write!(backend, "; monitor enumeration failed: {error}");
    }

    PlatformStatus {
        capture_backend: backend,
        capture_permission: if capture_supported {
            CapturePermission::Granted
        } else {
            CapturePermission::Unsupported
        },
        virtual_display_status: "IddCx virtual-display driver is not installed by LadoFlow yet."
            .to_owned(),
        displays,
    }
}

/// Windows.Graphics.Capture does not use the macOS-style global permission prompt.
#[must_use]
pub fn request_capture_access() -> PlatformStatus {
    collect_status()
}

/// Validate source selection before the native D3D11 frame-pool slice is attached.
///
/// # Errors
///
/// Returns a precise implementation boundary because this commit discovers
/// real displays but does not yet start a capture frame pool.
pub fn probe_screen_capture(
    display_id: Option<&str>,
    fps: u16,
) -> Result<CaptureProbeReport, String> {
    if !matches!(fps, 30 | 60) {
        return Err("Windows capture probe supports 30 or 60 Hz".to_owned());
    }

    if !GraphicsCaptureSession::IsSupported()
        .map_err(|error| format!("failed to query Windows.Graphics.Capture: {error}"))?
    {
        return Err("Windows.Graphics.Capture is not supported by this OS build".to_owned());
    }

    let displays = enumerate_display_sources()?;
    let selected = match display_id {
        Some(id) => displays
            .iter()
            .find(|display| display.id == id)
            .ok_or_else(|| format!("selected Windows display `{id}` is no longer available"))?,
        None => displays
            .iter()
            .find(|display| display.primary)
            .or_else(|| displays.first())
            .ok_or_else(|| "Windows reported no active display sources".to_owned())?,
    };

    Err(format!(
        "Windows display `{}` ({} x {}) is valid and Windows.Graphics.Capture is available, but the D3D11 frame-pool probe is not implemented in this slice",
        selected.name, selected.width, selected.height
    ))
}

fn enumerate_display_sources() -> Result<Vec<DisplaySource>, String> {
    let mut context = EnumerationContext::default();
    let context_pointer = std::ptr::from_mut(&mut context).cast::<core::ffi::c_void>();

    // SAFETY: `EnumDisplayMonitors` is synchronous. `context_pointer` points to
    // a live stack value for the full call, and the callback never retains it.
    let completed = unsafe {
        EnumDisplayMonitors(
            None,
            None,
            Some(enumerate_monitor),
            LPARAM(context_pointer as isize),
        )
    };

    if let Some(error) = context.error {
        return Err(error);
    }
    if !completed.as_bool() {
        return Err(format!(
            "EnumDisplayMonitors returned false: {}",
            ::windows::core::Error::from_win32()
        ));
    }

    context.displays.sort_by(|left, right| {
        right
            .primary
            .cmp(&left.primary)
            .then(left.id.cmp(&right.id))
    });
    let mut ids = HashSet::new();
    context
        .displays
        .retain(|display| ids.insert(display.id.clone()));
    Ok(context.displays)
}

unsafe extern "system" fn enumerate_monitor(
    monitor: HMONITOR,
    _device_context: HDC,
    _monitor_rect: *mut RECT,
    context: LPARAM,
) -> BOOL {
    let context_pointer = context.0 as *mut EnumerationContext;
    if context_pointer.is_null() {
        return BOOL(0);
    }

    // SAFETY: the pointer was created from a live `EnumerationContext` by the
    // synchronous caller and is used by one callback at a time.
    let context = unsafe { &mut *context_pointer };
    let mut info = MONITORINFOEXW::default();
    info.monitorInfo.cbSize = u32::try_from(size_of::<MONITORINFOEXW>())
        .expect("MONITORINFOEXW size fits in a Win32 DWORD");

    // SAFETY: `MONITORINFOEXW` starts with `MONITORINFO`, and `cbSize` tells
    // Win32 that the full extended structure is writable.
    let succeeded = unsafe {
        GetMonitorInfoW(
            monitor,
            std::ptr::from_mut(&mut info.monitorInfo).cast::<MONITORINFO>(),
        )
    };
    if !succeeded.as_bool() {
        context.error = Some(format!(
            "GetMonitorInfoW failed: {}",
            ::windows::core::Error::from_win32()
        ));
        return BOOL(0);
    }

    let width =
        i64::from(info.monitorInfo.rcMonitor.right) - i64::from(info.monitorInfo.rcMonitor.left);
    let height =
        i64::from(info.monitorInfo.rcMonitor.bottom) - i64::from(info.monitorInfo.rcMonitor.top);
    let (Ok(width), Ok(height)) = (u64::try_from(width), u64::try_from(height)) else {
        context.error = Some("GetMonitorInfoW returned invalid monitor geometry".to_owned());
        return BOOL(0);
    };
    if width == 0 || height == 0 {
        context.error = Some("GetMonitorInfoW returned an empty monitor rectangle".to_owned());
        return BOOL(0);
    }

    let device_path = null_terminated_utf16(&info.szDevice);
    let id = if device_path.is_empty() {
        format!("hmonitor:{:p}", monitor.0)
    } else {
        device_path.clone()
    };
    let name = device_path
        .strip_prefix(r"\\.\")
        .filter(|name| !name.is_empty())
        .unwrap_or("Windows display")
        .to_owned();

    context.displays.push(DisplaySource {
        id,
        name,
        width,
        height,
        primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
    });
    BOOL(1)
}

fn null_terminated_utf16(buffer: &[u16]) -> String {
    let end = buffer
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..end])
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::{collect_status, enumerate_display_sources, null_terminated_utf16};

    #[test]
    fn utf16_device_names_stop_at_the_first_null() {
        assert_eq!(
            null_terminated_utf16(&[u16::from(b'D'), u16::from(b'1'), 0, u16::from(b'X'),]),
            "D1"
        );
        assert_eq!(null_terminated_utf16(&[]), "");
    }

    #[test]
    fn monitor_enumeration_returns_unique_valid_geometry() {
        let displays = enumerate_display_sources().expect("Windows monitor enumeration succeeds");
        let mut ids = HashSet::new();
        for display in displays {
            assert!(!display.id.is_empty());
            assert!(ids.insert(display.id));
            assert!(display.width > 0);
            assert!(display.height > 0);
        }
    }

    #[test]
    fn platform_status_uses_the_windows_backend() {
        let status = collect_status();
        assert!(status.capture_backend.contains("Windows.Graphics.Capture"));
        assert!(status.virtual_display_status.contains("IddCx"));
    }
}
