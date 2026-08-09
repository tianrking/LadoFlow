use std::{
    sync::{Arc, Mutex, MutexGuard},
    thread,
    time::{Duration, Instant},
};

use core_graphics::{access::ScreenCaptureAccess, display::CGDisplay};
use screencapturekit::{
    cm::SCFrameStatus,
    prelude::{
        CMSampleBufferExt, CMSampleBufferSCExt, PixelFormat, SCContentFilter, SCShareableContent,
        SCStream, SCStreamConfiguration, SCStreamOutputType,
    },
};

use super::{CapturePermission, CaptureProbeReport, DisplaySource, PlatformStatus};

const CAPTURE_PROBE_DURATION: Duration = Duration::from_millis(750);
const CAPTURE_QUEUE_DEPTH: u32 = 3;

#[must_use]
pub fn collect_status() -> PlatformStatus {
    let access = ScreenCaptureAccess;
    let permission = if access.preflight() {
        CapturePermission::Granted
    } else {
        CapturePermission::Required
    };

    PlatformStatus {
        capture_backend: "ScreenCaptureKit native frame probe with CoreGraphics discovery"
            .to_owned(),
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

/// Capture real `ScreenCaptureKit` callbacks for a short, non-persistent probe.
///
/// Pixel contents never leave the native callback and are not copied into the
/// frontend. The report proves that `ScreenCaptureKit` delivered valid
/// `IOSurface`-backed buffers before the stream shut down cleanly.
pub fn probe_screen_capture(
    display_id: Option<&str>,
    fps: u16,
) -> Result<CaptureProbeReport, String> {
    validate_probe_fps(fps)?;
    if !ScreenCaptureAccess.preflight() {
        return Err(
            "screen recording access is required before running the native capture probe"
                .to_owned(),
        );
    }

    let requested_display_id = parse_display_id(display_id)?;
    let content = SCShareableContent::get()
        .map_err(|error| format!("failed to query ScreenCaptureKit content: {error}"))?;
    let displays = content.displays();
    let display = match requested_display_id {
        Some(requested) => displays
            .into_iter()
            .find(|display| display.display_id() == requested)
            .ok_or_else(|| format!("display {requested} is no longer available for capture"))?,
        None => displays
            .into_iter()
            .next()
            .ok_or_else(|| "ScreenCaptureKit reported no capturable displays".to_owned())?,
    };
    let selected_display_id = display.display_id();
    let configured_width = display.width();
    let configured_height = display.height();
    let display_name = display_name(selected_display_id);

    let filter = SCContentFilter::create()
        .with_display(&display)
        .with_excluding_windows(&[])
        .build();
    let configuration = SCStreamConfiguration::new()
        .with_width(configured_width)
        .with_height(configured_height)
        .with_pixel_format(PixelFormat::BGRA)
        .with_fps(u32::from(fps))
        .with_queue_depth(CAPTURE_QUEUE_DEPTH)
        .with_shows_cursor(false)
        .with_stream_name(Some("LadoFlow native capture probe"));

    let counters = Arc::new(Mutex::new(ProbeCounters::default()));
    let callback_counters = Arc::clone(&counters);
    let capture_started = Instant::now();
    let mut stream = SCStream::new(&filter, &configuration);
    let handler_id = stream.add_output_handler(
        move |sample, _output_type| {
            observe_sample(&callback_counters, &sample, capture_started);
        },
        SCStreamOutputType::Screen,
    );
    if handler_id.is_none() {
        return Err("ScreenCaptureKit rejected the screen output handler".to_owned());
    }

    stream
        .start_capture()
        .map_err(|error| format!("failed to start ScreenCaptureKit capture: {error}"))?;
    thread::sleep(CAPTURE_PROBE_DURATION);
    let stop_result = stream.stop_capture();
    let elapsed = capture_started.elapsed();
    stop_result.map_err(|error| format!("failed to stop ScreenCaptureKit capture: {error}"))?;

    let snapshot = lock_counters(&counters).clone();
    let width = snapshot.width.unwrap_or(configured_width);
    let height = snapshot.height.unwrap_or(configured_height);
    let observed_fps = if elapsed.is_zero() {
        0.0
    } else {
        #[allow(clippy::cast_precision_loss)]
        let value = snapshot.frames_with_surface as f64 / elapsed.as_secs_f64();
        value
    };

    Ok(CaptureProbeReport {
        backend: "ScreenCaptureKit".to_owned(),
        display_id: selected_display_id.to_string(),
        display_name,
        width,
        height,
        target_fps: fps,
        elapsed_ms: duration_millis_u64(elapsed),
        callbacks: snapshot.callbacks,
        content_frames: snapshot.content_frames,
        idle_frames: snapshot.idle_frames,
        incomplete_frames: snapshot.incomplete_frames,
        frames_with_surface: snapshot.frames_with_surface,
        dirty_rects: snapshot.dirty_rects,
        observed_fps,
        startup_latency_ms: snapshot
            .first_surface_at
            .map(|duration| duration.as_secs_f64() * 1_000.0),
        pixel_format: snapshot.pixel_format,
        passed: snapshot.callbacks > 0 && snapshot.frames_with_surface > 0,
    })
}

#[derive(Debug, Clone, Default)]
struct ProbeCounters {
    callbacks: u64,
    content_frames: u64,
    idle_frames: u64,
    incomplete_frames: u64,
    frames_with_surface: u64,
    dirty_rects: u64,
    first_surface_at: Option<Duration>,
    width: Option<u32>,
    height: Option<u32>,
    pixel_format: Option<String>,
}

fn observe_sample(
    counters: &Arc<Mutex<ProbeCounters>>,
    sample: &screencapturekit::cm::CMSampleBuffer,
    capture_started: Instant,
) {
    let mut counters = lock_counters(counters);
    counters.callbacks = counters.callbacks.saturating_add(1);

    match sample.frame_status() {
        Some(status) if status.has_content() => {
            counters.content_frames = counters.content_frames.saturating_add(1);
        }
        Some(SCFrameStatus::Idle) => {
            counters.idle_frames = counters.idle_frames.saturating_add(1);
        }
        _ => {
            counters.incomplete_frames = counters.incomplete_frames.saturating_add(1);
        }
    }

    if let Some(rects) = sample.dirty_rects() {
        counters.dirty_rects = counters
            .dirty_rects
            .saturating_add(u64::try_from(rects.len()).unwrap_or(u64::MAX));
    }

    if let Some(surface) = sample.image_buffer() {
        if surface.is_backed_by_io_surface() {
            counters.frames_with_surface = counters.frames_with_surface.saturating_add(1);
            counters
                .first_surface_at
                .get_or_insert(capture_started.elapsed());
        }
        counters.width = u32::try_from(surface.width()).ok();
        counters.height = u32::try_from(surface.height()).ok();
        counters
            .pixel_format
            .get_or_insert_with(|| PixelFormat::from(surface.pixel_format()).to_string());
    }
}

fn validate_probe_fps(fps: u16) -> Result<(), String> {
    if matches!(fps, 30 | 60) {
        Ok(())
    } else {
        Err("native capture probe refresh rate must be 30 or 60 Hz".to_owned())
    }
}

fn parse_display_id(display_id: Option<&str>) -> Result<Option<u32>, String> {
    display_id
        .map(|value| {
            value
                .parse::<u32>()
                .map_err(|_error| format!("invalid macOS display identifier: {value}"))
        })
        .transpose()
}

fn display_name(display_id: u32) -> String {
    active_displays()
        .into_iter()
        .find(|display| display.id == display_id.to_string())
        .map_or_else(|| format!("Display {display_id}"), |display| display.name)
}

fn lock_counters(counters: &Arc<Mutex<ProbeCounters>>) -> MutexGuard<'_, ProbeCounters> {
    counters
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn duration_millis_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
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
    use super::{active_displays, parse_display_id, validate_probe_fps};

    #[test]
    fn active_display_metadata_is_well_formed() {
        for display in active_displays() {
            assert!(!display.id.is_empty());
            assert!(!display.name.is_empty());
            assert!(display.width > 0);
            assert!(display.height > 0);
        }
    }

    #[test]
    fn probe_configuration_rejects_invalid_input_without_capture() {
        assert!(validate_probe_fps(30).is_ok());
        assert!(validate_probe_fps(60).is_ok());
        assert!(validate_probe_fps(59).is_err());
        assert_eq!(parse_display_id(Some("42")).expect("valid ID"), Some(42));
        assert!(parse_display_id(Some("display-42")).is_err());
    }
}
