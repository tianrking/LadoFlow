use core_graphics::{access::ScreenCaptureAccess, display::CGDisplay};

use super::{CapturePermission, DisplaySource, PlatformStatus};

#[must_use]
pub fn collect_status() -> PlatformStatus {
    let access = ScreenCaptureAccess;
    let permission = if access.preflight() {
        CapturePermission::Granted
    } else {
        CapturePermission::Required
    };

    PlatformStatus {
        capture_backend: "ScreenCaptureKit capture boundary with CoreGraphics discovery".to_owned(),
        capture_permission: permission,
        virtual_display_status:
            "Virtual-display creation remains isolated behind the native macOS adapter.".to_owned(),
        displays: active_displays(),
    }
}

#[must_use]
pub fn request_capture_access() -> PlatformStatus {
    let access = ScreenCaptureAccess;
    let _granted = access.request();
    collect_status()
}

fn active_displays() -> Vec<DisplaySource> {
    let Ok(ids) = CGDisplay::active_displays() else {
        return Vec::new();
    };

    ids.into_iter()
        .enumerate()
        .map(|(index, id)| {
            let display = CGDisplay::new(id);
            DisplaySource {
                id: id.to_string(),
                name: if display.is_main() {
                    "Main display".to_owned()
                } else {
                    format!("Display {}", index + 1)
                },
                width: display.pixels_wide(),
                height: display.pixels_high(),
                primary: display.is_main(),
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::active_displays;

    #[test]
    fn active_display_metadata_is_well_formed() {
        for display in active_displays() {
            assert!(!display.id.is_empty());
            assert!(!display.name.is_empty());
            assert!(display.width > 0);
            assert!(display.height > 0);
        }
    }
}
