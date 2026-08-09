//! Windows display discovery and capture capability boundary.
//!
//! Win32 monitor enumeration is synchronous: the callback and its context are
//! valid only for the duration of `EnumDisplayMonitors`. Unsafe code is kept in
//! this module so shared protocol/session crates remain unsafe-free.

#![allow(unsafe_code)]

use std::{
    collections::HashSet,
    fmt::Write as _,
    mem::size_of,
    sync::{
        Arc, Mutex, MutexGuard, OnceLock,
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use ::windows::{
    Foundation::TypedEventHandler,
    Graphics::{
        Capture::{Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession},
        DirectX::{Direct3D11::IDirect3DDevice, DirectXPixelFormat},
    },
    Win32::{
        Foundation::{HMODULE, LPARAM, RECT, RPC_E_CHANGED_MODE},
        Graphics::{
            Direct3D::{D3D_DRIVER_TYPE, D3D_DRIVER_TYPE_HARDWARE, D3D_DRIVER_TYPE_WARP},
            Direct3D11::{
                D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice,
                ID3D11Device,
            },
            Dxgi::{IDXGIAdapter, IDXGIDevice},
            Gdi::{
                EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW,
            },
        },
        System::WinRT::{
            Direct3D11::CreateDirect3D11DeviceFromDXGIDevice,
            Graphics::Capture::IGraphicsCaptureItemInterop, RO_INIT_MULTITHREADED, RoInitialize,
            RoUninitialize,
        },
        UI::WindowsAndMessaging::MONITORINFOF_PRIMARY,
    },
    core::{BOOL, IInspectable, Interface, factory},
};

use super::{
    CapturePermission, CaptureProbeReport, DisplaySource, H264AccessUnit, H264StreamBatch,
    H264StreamConfig, PlatformStatus, UsbLinkState,
};

mod capture_stream;
mod media_foundation;
mod usb_accessory;
mod video_processor;

pub use usb_accessory::UsbAccessoryManager;

use self::media_foundation::{HardwareEncodeProbe, HardwareEncoder, MediaFoundationRuntime};

const CAPTURE_PROBE_DURATION: Duration = Duration::from_millis(750);
const CAPTURE_COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
const CAPTURE_BUFFER_COUNT: i32 = 3;
const CAPTURE_PIXEL_FORMAT: DirectXPixelFormat = DirectXPixelFormat::B8G8R8A8UIntNormalized;

#[derive(Default)]
struct EnumerationContext {
    monitors: Vec<MonitorSource>,
    error: Option<String>,
}

struct MonitorSource {
    handle: HMONITOR,
    display: DisplaySource,
}

struct CaptureDevice {
    native: ID3D11Device,
    runtime: IDirect3DDevice,
    backend: &'static str,
}

#[derive(Clone, Default)]
struct ProbeCounters {
    callbacks: u64,
    frames_with_surface: u64,
    incomplete_frames: u64,
    dirty_rects: u64,
    first_surface_at: Option<Duration>,
    width: Option<u32>,
    height: Option<u32>,
    first_error: Option<String>,
}

struct FrameObservation {
    width: u32,
    height: u32,
    dirty_rects: u64,
}

struct WinRtApartment {
    uninitialize: bool,
}

enum CaptureCommand {
    IsSupported {
        response: SyncSender<Result<bool, String>>,
    },
    Probe {
        display_id: Option<String>,
        fps: u16,
        response: SyncSender<Result<CaptureProbeReport, String>>,
    },
    EncoderDiagnostics {
        response: SyncSender<EncoderDiagnostics>,
    },
}

#[derive(Clone)]
struct EncoderDiagnostics {
    capabilities: Result<Vec<HardwareEncoder>, String>,
    encode_probe: Result<HardwareEncodeProbe, String>,
}

struct CaptureWorker {
    commands: SyncSender<CaptureCommand>,
}

#[cfg(test)]
pub struct SyntheticH264Stream {
    cancel: Arc<AtomicBool>,
    receiver: Receiver<Result<H264StreamBatch, String>>,
    handle: Option<JoinHandle<()>>,
}

pub struct CapturedH264Stream {
    cancel: Arc<AtomicBool>,
    receiver: Receiver<Result<H264StreamBatch, String>>,
    handle: Option<JoinHandle<()>>,
}

impl CapturedH264Stream {
    pub fn start(config: H264StreamConfig, display_id: Option<String>) -> Result<Self, String> {
        let config =
            H264StreamConfig::new(config.width, config.height, config.fps, config.bitrate_kbps)?;
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let (sender, receiver) = sync_channel(2);
        let handle = thread::Builder::new()
            .name("ladoflow-windows-capture-h264".to_owned())
            .spawn(move || {
                if let Err(error) =
                    capture_stream::run(config, display_id.as_deref(), &worker_cancel, &sender)
                {
                    let _sent = send_encoder_result(&sender, Err(error), &worker_cancel);
                }
            })
            .map_err(|error| format!("failed to start Windows capture/H.264 worker: {error}"))?;
        Ok(Self {
            cancel,
            receiver,
            handle: Some(handle),
        })
    }

    pub fn try_next_batch(&self) -> Result<Option<H264StreamBatch>, String> {
        receive_h264_batch(&self.receiver, &self.cancel, "Windows capture/H.264 worker")
    }
}

impl Drop for CapturedH264Stream {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _result = handle.join();
        }
    }
}

#[cfg(test)]
impl SyntheticH264Stream {
    pub fn start(config: H264StreamConfig) -> Result<Self, String> {
        let config =
            H264StreamConfig::new(config.width, config.height, config.fps, config.bitrate_kbps)?;
        let encoder_config = media_foundation::H264EncoderConfig::new(
            u32::from(config.width),
            u32::from(config.height),
            u32::from(config.fps),
            config
                .bitrate_kbps
                .checked_mul(1_000)
                .ok_or_else(|| "H.264 bitrate exceeds the Windows encoder range".to_owned())?,
        );
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let (sender, receiver) = sync_channel(2);
        let handle = thread::Builder::new()
            .name("ladoflow-windows-h264".to_owned())
            .spawn(move || {
                if let Err(error) =
                    run_synthetic_h264_stream(encoder_config, &worker_cancel, &sender)
                {
                    let _sent = send_encoder_result(&sender, Err(error), &worker_cancel);
                }
            })
            .map_err(|error| format!("failed to start Windows H.264 worker: {error}"))?;
        Ok(Self {
            cancel,
            receiver,
            handle: Some(handle),
        })
    }

    pub fn try_next_batch(&self) -> Result<Option<H264StreamBatch>, String> {
        receive_h264_batch(&self.receiver, &self.cancel, "Windows H.264 worker")
    }
}

fn receive_h264_batch(
    receiver: &Receiver<Result<H264StreamBatch, String>>,
    cancel: &AtomicBool,
    worker_name: &str,
) -> Result<Option<H264StreamBatch>, String> {
    match receiver.try_recv() {
        Ok(Ok(batch)) => Ok(Some(batch)),
        Ok(Err(error)) => Err(error),
        Err(TryRecvError::Empty) => Ok(None),
        Err(TryRecvError::Disconnected) if cancel.load(Ordering::Acquire) => Ok(None),
        Err(TryRecvError::Disconnected) => {
            Err(format!("{worker_name} stopped without a diagnostic"))
        }
    }
}

#[cfg(test)]
impl Drop for SyntheticH264Stream {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _result = handle.join();
        }
    }
}

#[cfg(test)]
fn run_synthetic_h264_stream(
    config: media_foundation::H264EncoderConfig,
    cancel: &AtomicBool,
    sender: &SyncSender<Result<H264StreamBatch, String>>,
) -> Result<(), String> {
    let _apartment = WinRtApartment::initialize()?;
    let _media_foundation = MediaFoundationRuntime::startup()?;
    let frame_count = config.fps;
    let mut start_frame_index = 0_u32;

    while !cancel.load(Ordering::Acquire) {
        let batch =
            MediaFoundationRuntime::encode_synthetic_h264(config, start_frame_index, frame_count)?;
        let submitted = batch.frames_submitted;
        let batch = convert_h264_batch(batch)?;
        if !send_encoder_result(sender, Ok(batch), cancel) {
            break;
        }
        start_frame_index = start_frame_index
            .checked_add(submitted)
            .ok_or_else(|| "synthetic H.264 stream frame index is exhausted".to_owned())?;
    }
    Ok(())
}

fn send_encoder_result(
    sender: &SyncSender<Result<H264StreamBatch, String>>,
    mut result: Result<H264StreamBatch, String>,
    cancel: &AtomicBool,
) -> bool {
    loop {
        if cancel.load(Ordering::Acquire) {
            return false;
        }
        match sender.try_send(result) {
            Ok(()) => return true,
            Err(TrySendError::Full(returned)) => {
                result = returned;
                thread::sleep(Duration::from_millis(1));
            }
            Err(TrySendError::Disconnected(_returned)) => return false,
        }
    }
}

#[cfg(test)]
fn convert_h264_batch(batch: media_foundation::H264EncodeBatch) -> Result<H264StreamBatch, String> {
    if !batch.access_units.first().is_some_and(|unit| unit.keyframe) {
        return Err("Windows H.264 batch does not begin with a keyframe".to_owned());
    }
    convert_h264_access_units(batch.encoder_name, batch.access_units)
}

fn convert_h264_access_units(
    encoder_name: String,
    encoded: Vec<media_foundation::EncodedAccessUnit>,
) -> Result<H264StreamBatch, String> {
    if encoded.is_empty() {
        return Err("Windows H.264 encoder returned an empty access-unit batch".to_owned());
    }
    let mut previous_timestamp = None;
    let mut access_units = Vec::with_capacity(encoded.len());
    for unit in encoded {
        if unit.bytes.is_empty() {
            return Err("Windows H.264 encoder returned an empty access unit".to_owned());
        }
        let timestamp = media_time_to_duration(
            unit.timestamp_100ns
                .ok_or_else(|| "Windows H.264 access unit has no timestamp".to_owned())?,
            true,
            "timestamp",
        )?;
        let duration = media_time_to_duration(
            unit.duration_100ns
                .ok_or_else(|| "Windows H.264 access unit has no duration".to_owned())?,
            false,
            "duration",
        )?;
        if previous_timestamp.is_some_and(|previous| timestamp < previous) {
            return Err("Windows H.264 access-unit timestamps moved backwards".to_owned());
        }
        previous_timestamp = Some(timestamp);
        access_units.push(H264AccessUnit {
            bytes: unit.bytes,
            timestamp,
            duration,
            keyframe: unit.keyframe,
        });
    }
    Ok(H264StreamBatch {
        encoder_name,
        access_units,
    })
}

fn media_time_to_duration(
    value_100ns: i64,
    allow_zero: bool,
    field: &str,
) -> Result<Duration, String> {
    if value_100ns < 0 || (!allow_zero && value_100ns == 0) {
        return Err(format!(
            "Windows H.264 access-unit {field} must be {}",
            if allow_zero {
                "non-negative"
            } else {
                "positive"
            }
        ));
    }
    let ticks = u64::try_from(value_100ns)
        .map_err(|_| format!("Windows H.264 access-unit {field} is out of range"))?;
    let nanoseconds = ticks
        .checked_mul(100)
        .ok_or_else(|| format!("Windows H.264 access-unit {field} is out of range"))?;
    Ok(Duration::from_nanos(nanoseconds))
}

impl CaptureWorker {
    fn spawn() -> Result<Self, String> {
        let (commands, receiver) = sync_channel::<CaptureCommand>(8);
        let (ready_sender, ready_receiver) = sync_channel(1);
        thread::Builder::new()
            .name("ladoflow-windows-capture".to_owned())
            .spawn(move || {
                let apartment = WinRtApartment::initialize();
                let ready = apartment
                    .as_ref()
                    .map(|_apartment| ())
                    .map_err(Clone::clone);
                let _ = ready_sender.send(ready);
                let Ok(_apartment) = apartment else {
                    return;
                };
                let media_foundation = MediaFoundationRuntime::startup();
                let encoder_capabilities = match &media_foundation {
                    Ok(_runtime) => MediaFoundationRuntime::hardware_h264_encoders(),
                    Err(error) => Err(error.clone()),
                };
                let encoder_probe = match &media_foundation {
                    Ok(_runtime) => MediaFoundationRuntime::probe_hardware_h264_encode(),
                    Err(error) => Err(error.clone()),
                };
                let encoder_diagnostics = EncoderDiagnostics {
                    capabilities: encoder_capabilities,
                    encode_probe: encoder_probe,
                };
                let _media_foundation = media_foundation;

                while let Ok(command) = receiver.recv() {
                    match command {
                        CaptureCommand::IsSupported { response } => {
                            let result = GraphicsCaptureSession::IsSupported().map_err(|error| {
                                format!("Windows.Graphics.Capture probe failed: {error}")
                            });
                            let _ = response.send(result);
                        }
                        CaptureCommand::Probe {
                            display_id,
                            fps,
                            response,
                        } => {
                            let result = probe_screen_capture_on_worker(display_id.as_deref(), fps);
                            let _ = response.send(result);
                        }
                        CaptureCommand::EncoderDiagnostics { response } => {
                            let _ = response.send(encoder_diagnostics.clone());
                        }
                    }
                }
            })
            .map_err(|error| format!("failed to start Windows capture worker: {error}"))?;

        match ready_receiver.recv_timeout(CAPTURE_COMMAND_TIMEOUT) {
            Ok(Ok(())) => Ok(Self { commands }),
            Ok(Err(error)) => Err(error),
            Err(error) => Err(format!(
                "Windows capture worker did not initialize: {error}"
            )),
        }
    }

    fn is_supported(&self) -> Result<bool, String> {
        let (response, receiver) = sync_channel(1);
        self.commands
            .send(CaptureCommand::IsSupported { response })
            .map_err(|error| format!("Windows capture worker stopped: {error}"))?;
        receiver
            .recv_timeout(CAPTURE_COMMAND_TIMEOUT)
            .map_err(|error| format!("Windows capture support query timed out: {error}"))?
    }

    fn probe(&self, display_id: Option<&str>, fps: u16) -> Result<CaptureProbeReport, String> {
        let (response, receiver) = sync_channel(1);
        self.commands
            .send(CaptureCommand::Probe {
                display_id: display_id.map(ToOwned::to_owned),
                fps,
                response,
            })
            .map_err(|error| format!("Windows capture worker stopped: {error}"))?;
        receiver
            .recv_timeout(CAPTURE_COMMAND_TIMEOUT)
            .map_err(|error| format!("Windows capture probe timed out: {error}"))?
    }

    fn encoder_diagnostics(&self) -> Result<EncoderDiagnostics, String> {
        let (response, receiver) = sync_channel(1);
        self.commands
            .send(CaptureCommand::EncoderDiagnostics { response })
            .map_err(|error| format!("Windows media worker stopped: {error}"))?;
        receiver
            .recv_timeout(CAPTURE_COMMAND_TIMEOUT)
            .map_err(|error| format!("Windows encoder query timed out: {error}"))
    }
}

impl WinRtApartment {
    fn initialize() -> Result<Self, String> {
        // SAFETY: initialization is balanced by `Drop` for every successful
        // call, including S_FALSE. An existing STA returns RPC_E_CHANGED_MODE;
        // that thread is already initialized and must not be uninitialized here.
        match unsafe { RoInitialize(RO_INIT_MULTITHREADED) } {
            Ok(()) => Ok(Self { uninitialize: true }),
            Err(error) if error.code() == RPC_E_CHANGED_MODE => Ok(Self {
                uninitialize: false,
            }),
            Err(error) => Err(format!("failed to initialize Windows Runtime: {error}")),
        }
    }
}

impl Drop for WinRtApartment {
    fn drop(&mut self) {
        if self.uninitialize {
            // SAFETY: this object is dropped on the same thread that completed
            // the matching successful `RoInitialize` call.
            unsafe { RoUninitialize() };
        }
    }
}

/// Collect Windows capture capability and active monitor geometry.
#[must_use]
pub fn collect_status() -> PlatformStatus {
    let capture_support = query_capture_support();
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
        encoder_status: query_encoder_status(),
        usb_link_state: UsbLinkState::Ready,
        usb_status: usb_accessory::collect_status(),
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

/// Capture real `Windows.Graphics.Capture` callbacks for a short probe.
///
/// Captured GPU surfaces stay inside the native callback and are not copied to
/// JavaScript. This proves that monitor selection, D3D11 device creation, the
/// free-threaded frame pool, and clean shutdown work before the encoder is
/// attached to the same zero-copy surface path.
pub fn probe_screen_capture(
    display_id: Option<&str>,
    fps: u16,
) -> Result<CaptureProbeReport, String> {
    validate_probe_fps(fps)?;
    capture_worker()?.probe(display_id, fps)
}

fn probe_screen_capture_on_worker(
    display_id: Option<&str>,
    fps: u16,
) -> Result<CaptureProbeReport, String> {
    if !GraphicsCaptureSession::IsSupported()
        .map_err(|error| format!("failed to query Windows.Graphics.Capture: {error}"))?
    {
        return Err("Windows.Graphics.Capture is not supported by this OS build".to_owned());
    }

    let monitors = enumerate_monitors()?;
    let selected = select_monitor(&monitors, display_id)?;
    let item = create_capture_item(selected.handle)?;
    let item_size = item
        .Size()
        .map_err(|error| format!("failed to query capture item size: {error}"))?;
    let configured_width = positive_dimension(item_size.Width, "width")?;
    let configured_height = positive_dimension(item_size.Height, "height")?;
    let capture_device = create_capture_device()?;
    let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
        &capture_device.runtime,
        CAPTURE_PIXEL_FORMAT,
        CAPTURE_BUFFER_COUNT,
        item_size,
    )
    .map_err(|error| format!("failed to create Windows capture frame pool: {error}"))?;

    let counters = Arc::new(Mutex::new(ProbeCounters::default()));
    let callback_counters = Arc::clone(&counters);
    let capture_started = Instant::now();
    let handler =
        TypedEventHandler::<Direct3D11CaptureFramePool, IInspectable>::new(move |sender, _args| {
            let observation = match sender.ok() {
                Ok(sender) => observe_frame(sender),
                Err(error) => Err(format!("capture callback had no frame pool: {error}")),
            };
            record_observation(&callback_counters, observation, capture_started);
            Ok(())
        });
    let handler_token = frame_pool
        .FrameArrived(&handler)
        .map_err(|error| format!("failed to subscribe to Windows capture frames: {error}"))?;
    let session = match frame_pool.CreateCaptureSession(&item) {
        Ok(session) => session,
        Err(error) => {
            let _ = frame_pool.RemoveFrameArrived(handler_token);
            let _ = frame_pool.Close();
            return Err(format!("failed to create Windows capture session: {error}"));
        }
    };
    let _ = session.SetIsCursorCaptureEnabled(false);
    if let Err(error) = session.StartCapture() {
        let _ = frame_pool.RemoveFrameArrived(handler_token);
        let _ = frame_pool.Close();
        let _ = session.Close();
        return Err(format!("failed to start Windows capture: {error}"));
    }

    thread::sleep(CAPTURE_PROBE_DURATION);
    let elapsed = capture_started.elapsed();
    let cleanup_errors = close_capture_session(&frame_pool, handler_token, &session);
    if !cleanup_errors.is_empty() {
        return Err(format!(
            "Windows capture probe could not shut down cleanly: {}",
            cleanup_errors.join("; ")
        ));
    }

    let snapshot = lock_counters(&counters).clone();
    if snapshot.frames_with_surface == 0
        && let Some(error) = snapshot.first_error
    {
        return Err(format!(
            "Windows capture callbacks did not expose a usable D3D11 surface: {error}"
        ));
    }

    Ok(build_probe_report(
        selected,
        &capture_device,
        &snapshot,
        configured_width,
        configured_height,
        fps,
        elapsed,
    ))
}

fn build_probe_report(
    selected: &MonitorSource,
    capture_device: &CaptureDevice,
    snapshot: &ProbeCounters,
    configured_width: u32,
    configured_height: u32,
    fps: u16,
    elapsed: Duration,
) -> CaptureProbeReport {
    let observed_fps = if elapsed.is_zero() {
        0.0
    } else {
        #[allow(clippy::cast_precision_loss)]
        let value = snapshot.frames_with_surface as f64 / elapsed.as_secs_f64();
        value
    };
    CaptureProbeReport {
        backend: format!(
            "Windows.Graphics.Capture / {} D3D11",
            capture_device.backend
        ),
        display_id: selected.display.id.clone(),
        display_name: selected.display.name.clone(),
        width: snapshot.width.unwrap_or(configured_width),
        height: snapshot.height.unwrap_or(configured_height),
        target_fps: fps,
        elapsed_ms: duration_millis_u64(elapsed),
        callbacks: snapshot.callbacks,
        content_frames: snapshot.frames_with_surface,
        idle_frames: 0,
        incomplete_frames: snapshot.incomplete_frames,
        frames_with_surface: snapshot.frames_with_surface,
        dirty_rects: snapshot.dirty_rects,
        observed_fps,
        startup_latency_ms: snapshot
            .first_surface_at
            .map(|duration| duration.as_secs_f64() * 1_000.0),
        pixel_format: Some("B8G8R8A8UIntNormalized".to_owned()),
        passed: snapshot.callbacks > 0 && snapshot.frames_with_surface > 0,
    }
}

fn query_capture_support() -> Result<bool, String> {
    capture_worker()?.is_supported()
}

fn query_encoder_status() -> String {
    let diagnostics = match capture_worker().and_then(CaptureWorker::encoder_diagnostics) {
        Ok(diagnostics) => diagnostics,
        Err(error) => return format!("Media Foundation hardware H.264 query failed: {error}"),
    };
    let encoders = match diagnostics.capabilities {
        Ok(encoders) if encoders.is_empty() => {
            return "Media Foundation reported no hardware H.264 encoder for NV12 input".to_owned();
        }
        Ok(encoders) => encoders,
        Err(error) => return format!("Media Foundation hardware H.264 query failed: {error}"),
    };
    let names = encoders
        .into_iter()
        .map(|encoder| encoder.name)
        .collect::<Vec<_>>()
        .join(", ");
    match diagnostics.encode_probe {
        Ok(probe) => format!(
            "Media Foundation hardware H.264 Main encode verified with {}: {}x{}, {} input frame(s), {} timestamped / {} access unit(s), {} keyframe(s), {} bytes / {} Annex B NAL unit(s), {} ms; available: {names}",
            probe.encoder_name,
            probe.width,
            probe.height,
            probe.frames_submitted,
            probe.timestamped_access_units,
            probe.access_units,
            probe.keyframes,
            probe.encoded_bytes,
            probe.nal_units,
            probe.elapsed_ms
        ),
        Err(error) => format!(
            "Media Foundation hardware H.264 encoders available ({names}), but the encode probe failed: {error}"
        ),
    }
}

fn capture_worker() -> Result<&'static CaptureWorker, String> {
    static WORKER: OnceLock<Result<CaptureWorker, String>> = OnceLock::new();
    match WORKER.get_or_init(CaptureWorker::spawn) {
        Ok(worker) => Ok(worker),
        Err(error) => Err(error.clone()),
    }
}

fn validate_probe_fps(fps: u16) -> Result<(), String> {
    if matches!(fps, 30 | 60) {
        Ok(())
    } else {
        Err("native capture probe refresh rate must be 30 or 60 Hz".to_owned())
    }
}

fn select_monitor<'a>(
    monitors: &'a [MonitorSource],
    display_id: Option<&str>,
) -> Result<&'a MonitorSource, String> {
    match display_id {
        Some(id) => monitors
            .iter()
            .find(|monitor| monitor.display.id == id)
            .ok_or_else(|| format!("selected Windows display `{id}` is no longer available")),
        None => monitors
            .iter()
            .find(|monitor| monitor.display.primary)
            .or_else(|| monitors.first())
            .ok_or_else(|| "Windows reported no active display sources".to_owned()),
    }
}

fn create_capture_item(monitor: HMONITOR) -> Result<GraphicsCaptureItem, String> {
    let interop = factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>()
        .map_err(|error| format!("failed to load GraphicsCaptureItem interop: {error}"))?;
    // SAFETY: `monitor` came from the current synchronous monitor enumeration
    // and remains a valid opaque handle while this probe is running.
    unsafe { interop.CreateForMonitor(monitor) }
        .map_err(|error| format!("failed to create capture item for monitor: {error}"))
}

fn create_capture_device() -> Result<CaptureDevice, String> {
    match create_native_d3d11_device(D3D_DRIVER_TYPE_HARDWARE) {
        Ok(device) => wrap_capture_device(&device, "hardware"),
        Err(hardware_error) => match create_native_d3d11_device(D3D_DRIVER_TYPE_WARP) {
            Ok(device) => wrap_capture_device(&device, "WARP fallback"),
            Err(warp_error) => Err(format!(
                "failed to create D3D11 device (hardware: {hardware_error}; WARP: {warp_error})"
            )),
        },
    }
}

fn create_native_d3d11_device(driver_type: D3D_DRIVER_TYPE) -> Result<ID3D11Device, String> {
    let mut device = None;
    // SAFETY: all optional output pointers either reference initialized local
    // storage or are omitted. The returned COM object owns its native lifetime.
    unsafe {
        D3D11CreateDevice(
            None::<&IDXGIAdapter>,
            driver_type,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&raw mut device),
            None,
            None,
        )
    }
    .map_err(|error| error.to_string())?;
    device.ok_or_else(|| "D3D11CreateDevice returned no device".to_owned())
}

fn wrap_capture_device(
    native_device: &ID3D11Device,
    backend: &'static str,
) -> Result<CaptureDevice, String> {
    let dxgi_device: IDXGIDevice = native_device
        .cast()
        .map_err(|error| format!("failed to query IDXGIDevice: {error}"))?;
    // SAFETY: `dxgi_device` is a live D3D11 device queried from the COM object
    // created above; the WinRT wrapper retains its own reference.
    let inspectable = unsafe { CreateDirect3D11DeviceFromDXGIDevice(&dxgi_device) }
        .map_err(|error| format!("failed to wrap D3D11 device for WinRT: {error}"))?;
    let runtime = inspectable
        .cast::<IDirect3DDevice>()
        .map_err(|error| format!("failed to query WinRT IDirect3DDevice: {error}"))?;
    Ok(CaptureDevice {
        native: native_device.clone(),
        runtime,
        backend,
    })
}

fn observe_frame(frame_pool: &Direct3D11CaptureFramePool) -> Result<FrameObservation, String> {
    let frame = frame_pool
        .TryGetNextFrame()
        .map_err(|error| format!("failed to dequeue captured frame: {error}"))?;
    let observation = (|| {
        let size = frame
            .ContentSize()
            .map_err(|error| format!("failed to query captured frame size: {error}"))?;
        let width = positive_dimension(size.Width, "frame width")?;
        let height = positive_dimension(size.Height, "frame height")?;
        let _surface = frame
            .Surface()
            .map_err(|error| format!("captured frame has no D3D11 surface: {error}"))?;
        let dirty_rects = frame
            .DirtyRegions()
            .ok()
            .and_then(|regions| regions.Size().ok())
            .map_or(0, u64::from);
        Ok(FrameObservation {
            width,
            height,
            dirty_rects,
        })
    })();
    let _ = frame.Close();
    observation
}

fn record_observation(
    counters: &Arc<Mutex<ProbeCounters>>,
    observation: Result<FrameObservation, String>,
    capture_started: Instant,
) {
    let mut counters = lock_counters(counters);
    counters.callbacks = counters.callbacks.saturating_add(1);
    match observation {
        Ok(observation) => {
            counters.frames_with_surface = counters.frames_with_surface.saturating_add(1);
            counters.dirty_rects = counters.dirty_rects.saturating_add(observation.dirty_rects);
            counters
                .first_surface_at
                .get_or_insert(capture_started.elapsed());
            counters.width = Some(observation.width);
            counters.height = Some(observation.height);
        }
        Err(error) => {
            counters.incomplete_frames = counters.incomplete_frames.saturating_add(1);
            counters.first_error.get_or_insert(error);
        }
    }
}

fn close_capture_session(
    frame_pool: &Direct3D11CaptureFramePool,
    handler_token: i64,
    session: &GraphicsCaptureSession,
) -> Vec<String> {
    let mut errors = Vec::new();
    if let Err(error) = frame_pool.RemoveFrameArrived(handler_token) {
        errors.push(format!("remove frame handler: {error}"));
    }
    if let Err(error) = frame_pool.Close() {
        errors.push(format!("close frame pool: {error}"));
    }
    if let Err(error) = session.Close() {
        errors.push(format!("close capture session: {error}"));
    }
    errors
}

fn positive_dimension(value: i32, label: &str) -> Result<u32, String> {
    u32::try_from(value)
        .ok()
        .filter(|value| *value > 0)
        .ok_or_else(|| format!("Windows capture returned invalid {label}: {value}"))
}

fn duration_millis_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

fn lock_counters(counters: &Arc<Mutex<ProbeCounters>>) -> MutexGuard<'_, ProbeCounters> {
    counters
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn enumerate_display_sources() -> Result<Vec<DisplaySource>, String> {
    enumerate_monitors().map(|monitors| {
        monitors
            .into_iter()
            .map(|monitor| monitor.display)
            .collect()
    })
}

fn enumerate_monitors() -> Result<Vec<MonitorSource>, String> {
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

    context.monitors.sort_by(|left, right| {
        right
            .display
            .primary
            .cmp(&left.display.primary)
            .then(left.display.id.cmp(&right.display.id))
    });
    let mut ids = HashSet::new();
    context
        .monitors
        .retain(|monitor| ids.insert(monitor.display.id.clone()));
    Ok(context.monitors)
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

    context.monitors.push(MonitorSource {
        handle: monitor,
        display: DisplaySource {
            id,
            name,
            width,
            height,
            primary: info.monitorInfo.dwFlags & MONITORINFOF_PRIMARY != 0,
        },
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
    use std::{
        collections::HashSet,
        thread,
        time::{Duration, Instant},
    };

    use super::{
        CapturedH264Stream, H264StreamConfig, SyntheticH264Stream, collect_status,
        enumerate_display_sources, null_terminated_utf16, positive_dimension, probe_screen_capture,
        validate_probe_fps,
    };

    #[test]
    fn utf16_device_names_stop_at_the_first_null() {
        assert_eq!(
            null_terminated_utf16(&[u16::from(b'D'), u16::from(b'1'), 0, u16::from(b'X')]),
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
        eprintln!("{status:#?}");
        assert!(status.capture_backend.contains("Windows.Graphics.Capture"));
        assert!(status.encoder_status.contains("Media Foundation"));
        assert!(status.virtual_display_status.contains("IddCx"));
    }

    #[test]
    fn probe_configuration_rejects_invalid_values() {
        assert!(validate_probe_fps(30).is_ok());
        assert!(validate_probe_fps(60).is_ok());
        assert!(validate_probe_fps(59).is_err());
        assert_eq!(positive_dimension(1, "width").expect("positive"), 1);
        assert!(positive_dimension(0, "width").is_err());
        assert!(positive_dimension(-1, "width").is_err());
    }

    #[test]
    #[ignore = "requires an interactive Windows desktop and a working graphics adapter"]
    fn native_capture_probe_receives_a_gpu_surface() {
        for attempt in 1..=2 {
            let report =
                probe_screen_capture(None, 60).expect("native Windows capture probe succeeds");
            eprintln!("attempt {attempt}: {report:#?}");
            assert!(report.passed);
            assert!(report.callbacks > 0);
            assert!(report.frames_with_surface > 0);
        }
    }

    #[test]
    #[ignore = "requires a physical Windows hardware H.264 encoder"]
    fn hardware_h264_encoder_outputs_annex_b_stream() {
        let status = collect_status();
        eprintln!("{}", status.encoder_status);
        assert!(status.encoder_status.contains("encode verified"));
        assert!(status.encoder_status.contains("Annex B NAL unit"));
        assert!(status.encoder_status.contains("timestamped"));
        assert!(status.encoder_status.contains("keyframe"));
    }

    #[test]
    #[ignore = "requires a physical Windows hardware H.264 encoder"]
    fn synthetic_h264_worker_produces_a_timestamped_main_batch() {
        for (width, height, bitrate_kbps) in [
            (1_280, 800, 7_373),
            (1_920, 1_080, 14_929),
            (2_560, 1_440, 26_542),
            (2_732, 2_048, 40_000),
        ] {
            let started = Instant::now();
            let stream = SyntheticH264Stream::start(
                H264StreamConfig::new(width, height, 60, bitrate_kbps)
                    .expect("valid stream config"),
            )
            .expect("start H.264 worker");
            let deadline = Instant::now() + Duration::from_secs(10);
            let batch = loop {
                if let Some(batch) = stream
                    .try_next_batch()
                    .expect("H.264 worker remains healthy")
                {
                    break batch;
                }
                assert!(Instant::now() < deadline, "H.264 worker timed out");
                thread::sleep(Duration::from_millis(2));
            };
            eprintln!(
                "{width}x{height}: {} produced {} access units in {} ms",
                batch.encoder_name,
                batch.access_units.len(),
                started.elapsed().as_millis()
            );
            assert_eq!(batch.access_units.len(), 60);
            assert!(batch.access_units[0].keyframe);
            assert_eq!(batch.access_units[0].timestamp, Duration::ZERO);
            assert!(batch.access_units.iter().all(|unit| !unit.bytes.is_empty()));
            assert!(
                batch
                    .access_units
                    .iter()
                    .all(|unit| !unit.duration.is_zero())
            );
            drop(stream);
        }
    }

    #[test]
    #[ignore = "requires an interactive Windows desktop and a physical H.264 encoder"]
    fn native_capture_stream_produces_gpu_encoded_h264() {
        let stream = CapturedH264Stream::start(
            H264StreamConfig::new(1_280, 720, 30, 8_000).expect("valid stream config"),
            None,
        )
        .expect("start native capture/H.264 worker");
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut access_units = 0_usize;
        let mut keyframes = 0_usize;
        while access_units < 10 {
            if let Some(batch) = stream
                .try_next_batch()
                .expect("native capture/H.264 worker remains healthy")
            {
                access_units += batch.access_units.len();
                keyframes += batch
                    .access_units
                    .iter()
                    .filter(|unit| unit.keyframe)
                    .count();
            }
            assert!(
                Instant::now() < deadline,
                "native capture/H.264 worker timed out after {access_units} units"
            );
            thread::sleep(Duration::from_millis(1));
        }
        eprintln!("captured {access_units} H.264 access units with {keyframes} keyframe(s)");
        assert!(keyframes > 0);
    }
}
