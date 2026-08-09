use std::{
    collections::{HashMap, VecDeque},
    num::NonZeroUsize,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use ladoflow_core::{
    LatencyAggregator, SequenceDisposition, Session, SessionPhase, StreamContinuity, negotiate,
};
use ladoflow_media::{
    FrameDimensions, FrameKind, FramePacer, FrameRate, PaceDecision, SyntheticConfig,
    SyntheticFrameProducer, VideoFormat,
};
use ladoflow_protocol::{
    Capabilities, CodecSet, DecodeOutcome, ErrorMessage, FeatureFlags, Frame as WireFrame,
    FrameFlags, Hello, InputCapabilities, InputEvent, MessageType, PROTOCOL_VERSION, Ping, Pong,
    Role, Telemetry, VideoFrame, VideoFrameMetadata,
};
use ladoflow_transport::LoopbackConfig as TransportLoopbackConfig;
use ladoflow_transport::{
    Channel, ConnectionState, Packet, PacketTransport, SupersessionKey, loopback_pair,
};
use serde::{Deserialize, Serialize};

use crate::host_protocol::{HostProtocolConfig, negotiate_host_transport, send_control_payload};
use crate::platform::{
    CapturedH264Stream, H264AccessUnit, H264StreamConfig, PlatformStatus, UsbAccessoryManager,
    UsbAccessoryProbeReport, collect_status,
};

const LATENCY_WINDOW: NonZeroUsize = NonZeroUsize::new(240).expect("240 is non-zero");
const MEDIA_STREAM_KEY: SupersessionKey = SupersessionKey::new(1);
const USB_NEGOTIATION_TIMEOUT: Duration = Duration::from_secs(5);
const USB_CONTROL_SEND_TIMEOUT: Duration = Duration::from_secs(1);
const USB_CONTROL_POLL_INTERVAL: Duration = Duration::from_millis(2);
const SENT_VIDEO_HISTORY: usize = 1_024;
const LOOPBACK_TRANSPORT_NAME: &str = "In-memory duplex";
const USB_TRANSPORT_NAME: &str = "Android Open Accessory USB";

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoopbackConfig {
    width: u16,
    height: u16,
    fps: u16,
}

impl LoopbackConfig {
    fn validate(self) -> Result<Self, String> {
        if self.width == 0 || self.height == 0 {
            return Err("loopback dimensions must be non-zero".to_owned());
        }
        if !matches!(self.fps, 30 | 60) {
            return Err("loopback refresh rate must be 30 or 60 Hz".to_owned());
        }
        Ok(self)
    }
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
enum SessionPhaseView {
    Idle,
    Negotiating,
    Connected,
    Streaming,
    Stopped,
    Failed,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    phase: SessionPhaseView,
    transport: String,
    peer_name: Option<String>,
    last_error: Option<String>,
    configured_width: Option<u16>,
    configured_height: Option<u16>,
    configured_fps: Option<u16>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TelemetrySnapshot {
    frames_produced: u64,
    frames_presented: u64,
    frames_dropped: u64,
    frames_superseded: u64,
    actual_fps: f64,
    p50_latency_ms: Option<f64>,
    p95_latency_ms: Option<f64>,
    queue_depth: usize,
    uptime_ms: u64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostSnapshot {
    app_version: &'static str,
    os: &'static str,
    architecture: &'static str,
    protocol_version: u16,
    session: SessionSnapshot,
    telemetry: TelemetrySnapshot,
    platform: PlatformStatus,
}

#[derive(Debug)]
struct SharedState {
    phase: SessionPhaseView,
    session: Option<Session>,
    config: Option<LoopbackConfig>,
    transport: &'static str,
    peer_name: Option<String>,
    last_error: Option<String>,
    started_at: Option<Instant>,
    latency: LatencyAggregator,
    frames_produced: u64,
    frames_presented: u64,
    frames_dropped: u64,
    frames_superseded: u64,
    queue_depth: usize,
    sent_video_ordinals: VecDeque<(u64, u64)>,
}

impl SharedState {
    fn new() -> Self {
        Self {
            phase: SessionPhaseView::Idle,
            session: None,
            config: None,
            transport: "No active transport",
            peer_name: None,
            last_error: None,
            started_at: None,
            latency: LatencyAggregator::new(LATENCY_WINDOW),
            frames_produced: 0,
            frames_presented: 0,
            frames_dropped: 0,
            frames_superseded: 0,
            queue_depth: 0,
            sent_video_ordinals: VecDeque::with_capacity(SENT_VIDEO_HISTORY),
        }
    }

    fn reset_for_start(&mut self, config: LoopbackConfig, session: Session) {
        self.phase = SessionPhaseView::Streaming;
        self.session = Some(session);
        self.config = Some(config);
        self.transport = LOOPBACK_TRANSPORT_NAME;
        self.peer_name = Some("LadoFlow synthetic display".to_owned());
        self.last_error = None;
        self.started_at = Some(Instant::now());
        self.latency = LatencyAggregator::new(LATENCY_WINDOW);
        self.frames_produced = 0;
        self.frames_presented = 0;
        self.frames_dropped = 0;
        self.frames_superseded = 0;
        self.queue_depth = 0;
        self.sent_video_ordinals.clear();
    }

    fn reset_for_usb_negotiation(&mut self, config: LoopbackConfig) {
        self.phase = SessionPhaseView::Negotiating;
        self.session = None;
        self.config = Some(config);
        self.transport = USB_TRANSPORT_NAME;
        self.peer_name = None;
        self.last_error = None;
        self.started_at = Some(Instant::now());
        self.latency = LatencyAggregator::new(LATENCY_WINDOW);
        self.frames_produced = 0;
        self.frames_presented = 0;
        self.frames_dropped = 0;
        self.frames_superseded = 0;
        self.queue_depth = 0;
        self.sent_video_ordinals.clear();
    }

    fn establish_usb_control(
        &mut self,
        config: LoopbackConfig,
        session: Session,
        peer_name: String,
    ) {
        self.phase = SessionPhaseView::Connected;
        self.session = Some(session);
        self.config = Some(config);
        self.peer_name = Some(peer_name);
        self.last_error = None;
    }

    fn snapshot(&self, platform: PlatformStatus) -> HostSnapshot {
        let uptime = self
            .started_at
            .map_or(Duration::ZERO, |start| start.elapsed());
        let latency = self.latency.snapshot();
        let config = self.config;

        HostSnapshot {
            app_version: env!("CARGO_PKG_VERSION"),
            os: std::env::consts::OS,
            architecture: std::env::consts::ARCH,
            protocol_version: PROTOCOL_VERSION,
            session: SessionSnapshot {
                phase: self.phase,
                transport: self.transport.to_owned(),
                peer_name: self.peer_name.clone(),
                last_error: self.last_error.clone(),
                configured_width: config.map(|value| value.width),
                configured_height: config.map(|value| value.height),
                configured_fps: config.map(|value| value.fps),
            },
            telemetry: TelemetrySnapshot {
                frames_produced: self.frames_produced,
                frames_presented: self.frames_presented,
                frames_dropped: self.frames_dropped,
                frames_superseded: self.frames_superseded,
                actual_fps: measured_fps(self.frames_presented, uptime),
                p50_latency_ms: latency.map(|value| duration_millis(value.p50())),
                p95_latency_ms: latency.map(|value| duration_millis(value.p95())),
                queue_depth: self.queue_depth,
                uptime_ms: u64::try_from(uptime.as_millis()).unwrap_or(u64::MAX),
            },
            platform,
        }
    }
}

impl Default for SharedState {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug)]
struct Worker {
    cancel: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

#[derive(Debug, Default)]
pub struct DesktopRuntime {
    shared: Arc<Mutex<SharedState>>,
    worker: Mutex<Option<Worker>>,
    usb_accessory: UsbAccessoryManager,
}

impl DesktopRuntime {
    pub fn snapshot(&self) -> HostSnapshot {
        self.lock_shared().snapshot(self.platform_status())
    }

    pub fn prepare_android_usb(&self) -> UsbAccessoryProbeReport {
        self.usb_accessory.prepare()
    }

    pub fn disconnect_android_usb(&self) -> Result<HostSnapshot, String> {
        let usb_session_active = {
            let shared = self.lock_shared();
            shared.transport == USB_TRANSPORT_NAME
                && matches!(
                    shared.phase,
                    SessionPhaseView::Negotiating
                        | SessionPhaseView::Connected
                        | SessionPhaseView::Streaming
                )
        };
        if usb_session_active {
            let _stopped = self.stop()?;
        }
        self.usb_accessory.disconnect()?;
        Ok(self.snapshot())
    }

    pub fn start(
        &self,
        config: LoopbackConfig,
        display_id: Option<String>,
    ) -> Result<HostSnapshot, String> {
        let config = config.validate()?;
        let display_id = display_id
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty());
        let mut worker_slot = self.lock_worker();
        if let Some(worker) = worker_slot.as_ref() {
            if !worker.handle.is_finished() {
                return Err("a display session is already running".to_owned());
            }
        }
        if let Some(finished) = worker_slot.take() {
            finished
                .handle
                .join()
                .map_err(|_| "the previous loopback worker panicked".to_owned())?;
        }

        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let worker_shared = Arc::clone(&self.shared);
        let use_usb = self.usb_accessory.connection_state() == ConnectionState::Connected;
        let handle = if use_usb {
            self.lock_shared().reset_for_usb_negotiation(config);
            let usb_transport = self.usb_accessory.clone();
            thread::Builder::new()
                .name("ladoflow-usb-session".to_owned())
                .spawn(move || {
                    run_usb_control_session(
                        &worker_shared,
                        &worker_cancel,
                        config,
                        display_id.as_deref(),
                        usb_transport,
                    );
                })
                .map_err(|error| {
                    self.lock_shared().phase = SessionPhaseView::Failed;
                    format!("failed to start USB session worker: {error}")
                })?
        } else {
            self.lock_shared().phase = SessionPhaseView::Negotiating;
            let session = negotiated_session(config).inspect_err(|_error| {
                self.lock_shared().phase = SessionPhaseView::Failed;
            })?;
            self.lock_shared().reset_for_start(config, session);
            thread::Builder::new()
                .name("ladoflow-loopback".to_owned())
                .spawn(move || run_loopback(&worker_shared, &worker_cancel, config))
                .map_err(|error| {
                    self.lock_shared().phase = SessionPhaseView::Failed;
                    format!("failed to start loopback worker: {error}")
                })?
        };
        *worker_slot = Some(Worker { cancel, handle });
        drop(worker_slot);

        Ok(self.snapshot())
    }

    pub fn stop(&self) -> Result<HostSnapshot, String> {
        let worker = self.lock_worker().take();
        if let Some(worker) = worker {
            worker.cancel.store(true, Ordering::Release);
            worker
                .handle
                .join()
                .map_err(|_| "the loopback worker panicked while stopping".to_owned())?;
        }

        let mut shared = self.lock_shared();
        if let Some(session) = shared.session.as_mut() {
            session.close();
        }
        shared.phase = SessionPhaseView::Stopped;
        let snapshot = shared.snapshot(self.platform_status());
        drop(shared);
        Ok(snapshot)
    }

    fn platform_status(&self) -> PlatformStatus {
        let mut platform = collect_status();
        if let Some((state, detail)) = self.usb_accessory.runtime_status() {
            platform.usb_link_state = state;
            platform.usb_status = detail;
        }
        platform
    }

    fn lock_shared(&self) -> MutexGuard<'_, SharedState> {
        self.shared
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn lock_worker(&self) -> MutexGuard<'_, Option<Worker>> {
        self.worker
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Drop for DesktopRuntime {
    fn drop(&mut self) {
        let worker_slot = self
            .worker
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(worker) = worker_slot.take() {
            worker.cancel.store(true, Ordering::Release);
            let _result = worker.handle.join();
        }
    }
}

fn run_usb_control_session(
    shared: &Arc<Mutex<SharedState>>,
    cancel: &AtomicBool,
    config: LoopbackConfig,
    display_id: Option<&str>,
    mut transport: UsbAccessoryManager,
) {
    let result = run_usb_control_session_inner(shared, cancel, config, display_id, &mut transport);
    if let Err(error) = result {
        if cancel.load(Ordering::Acquire) {
            return;
        }
        let mut state = lock_arc(shared);
        if let Some(session) = state.session.as_mut() {
            let _result = session.transport_lost();
        }
        state.phase = SessionPhaseView::Failed;
        state.last_error = Some(error);
    }
}

fn run_usb_control_session_inner(
    shared: &Arc<Mutex<SharedState>>,
    cancel: &AtomicBool,
    config: LoopbackConfig,
    display_id: Option<&str>,
    transport: &mut impl PacketTransport,
) -> Result<(), String> {
    let protocol_config = HostProtocolConfig::new(config.width, config.height, config.fps)?;
    let established =
        negotiate_host_transport(transport, protocol_config, cancel, USB_NEGOTIATION_TIMEOUT)?;
    let refresh_millihz = established.display_config.refresh_millihz();
    if refresh_millihz % 1_000 != 0 {
        return Err(format!(
            "negotiated refresh rate {refresh_millihz} mHz cannot drive the integer-Hz runtime"
        ));
    }
    let negotiated_config = LoopbackConfig {
        width: established.display_config.width(),
        height: established.display_config.height(),
        fps: u16::try_from(refresh_millihz / 1_000)
            .map_err(|_| "negotiated refresh rate exceeds the desktop range".to_owned())?,
    };
    let mut next_sequence = established.next_sequence;
    {
        let mut state = lock_arc(shared);
        state.establish_usb_control(
            negotiated_config,
            established.session,
            established.peer_name,
        );
    }

    let clock_origin = Instant::now();
    let stream_config = H264StreamConfig::new(
        negotiated_config.width,
        negotiated_config.height,
        negotiated_config.fps,
        established.display_config.bitrate_kbps(),
    )?;
    let video_stream = CapturedH264Stream::start(stream_config, display_id.map(ToOwned::to_owned))?;
    let mut access_units = VecDeque::<H264AccessUnit>::new();
    let mut media_clock_offset = None::<Duration>;
    let mut encoder_name = None::<String>;

    while !cancel.load(Ordering::Acquire) {
        let mut made_progress = false;
        if transport.connection_state() != ConnectionState::Connected {
            return Err("Android USB disconnected during the LDFL session".to_owned());
        }
        if transport
            .try_receive(Channel::Media)
            .map_err(|error| format!("Android USB media receive failed: {error}"))?
            .is_some()
        {
            return Err("Android display sent an unexpected media frame to the host".to_owned());
        }
        if let Some(packet) = transport
            .try_receive(Channel::Control)
            .map_err(|error| format!("Android USB control receive failed: {error}"))?
        {
            handle_usb_control(
                shared,
                cancel,
                transport,
                packet,
                clock_origin,
                &mut next_sequence,
            )?;
            made_progress = true;
        }

        if access_units.is_empty()
            && let Some(batch) = video_stream.try_next_batch()?
        {
            if encoder_name
                .as_ref()
                .is_some_and(|name| name != &batch.encoder_name)
            {
                return Err(format!(
                    "Windows H.264 encoder changed from {} to {} during one session",
                    encoder_name.as_deref().unwrap_or("unknown"),
                    batch.encoder_name
                ));
            }
            encoder_name.get_or_insert(batch.encoder_name);
            access_units.extend(batch.access_units);
            align_media_clock(
                &access_units,
                clock_origin.elapsed(),
                &mut media_clock_offset,
            )?;
            made_progress = true;
        }

        if let (Some(unit), Some(offset)) = (access_units.front(), media_clock_offset) {
            let due = offset
                .checked_add(unit.timestamp)
                .ok_or_else(|| "H.264 media clock overflowed".to_owned())?;
            if clock_origin.elapsed() >= due {
                let unit = access_units
                    .pop_front()
                    .ok_or_else(|| "H.264 access-unit queue changed unexpectedly".to_owned())?;
                send_usb_h264_access_unit(shared, transport, unit, offset, &mut next_sequence)?;
                made_progress = true;
            }
        }

        if !made_progress {
            thread::sleep(USB_CONTROL_POLL_INTERVAL);
        }
    }
    Ok(())
}

fn align_media_clock(
    access_units: &VecDeque<H264AccessUnit>,
    now: Duration,
    offset: &mut Option<Duration>,
) -> Result<(), String> {
    let first = access_units
        .front()
        .ok_or_else(|| "Windows H.264 batch is unexpectedly empty".to_owned())?;
    let current_offset = offset.get_or_insert_with(|| now.saturating_sub(first.timestamp));
    let scheduled = current_offset
        .checked_add(first.timestamp)
        .ok_or_else(|| "H.264 media clock overflowed".to_owned())?;
    let late_threshold = first
        .duration
        .checked_mul(2)
        .ok_or_else(|| "H.264 frame duration overflowed".to_owned())?;
    if first.keyframe && now.saturating_sub(scheduled) > late_threshold {
        *current_offset = now.saturating_sub(first.timestamp);
    }
    Ok(())
}

fn send_usb_h264_access_unit(
    shared: &Arc<Mutex<SharedState>>,
    transport: &mut impl PacketTransport,
    unit: H264AccessUnit,
    media_clock_offset: Duration,
    next_sequence: &mut u64,
) -> Result<(), String> {
    let sequence = *next_sequence;
    let capture_time = media_clock_offset
        .checked_add(unit.timestamp)
        .ok_or_else(|| "H.264 capture timestamp overflowed".to_owned())?;
    let presentation_time = capture_time
        .checked_add(unit.duration)
        .ok_or_else(|| "H.264 presentation timestamp overflowed".to_owned())?;
    let metadata = VideoFrameMetadata::new(
        sequence,
        duration_micros_u64(capture_time),
        duration_micros_u64(presentation_time),
        duration_micros_u32(unit.duration)?,
    )
    .map_err(|error| error.to_string())?;
    let payload = VideoFrame::new(metadata, unit.bytes).map_err(|error| error.to_string())?;
    let flags = if unit.keyframe {
        FrameFlags::KEYFRAME
    } else {
        FrameFlags::NONE
    };
    let frame =
        WireFrame::from_payload(flags, sequence, &payload).map_err(|error| error.to_string())?;
    transport
        .try_send(Packet::media(frame.encode()))
        .map_err(|error| format!("Android USB media queue rejected H.264: {error}"))?;
    *next_sequence = next_sequence
        .checked_add(1)
        .ok_or_else(|| "host LDFL sequence is exhausted".to_owned())?;

    let mut state = lock_arc(shared);
    state.phase = SessionPhaseView::Streaming;
    state.frames_produced = state.frames_produced.saturating_add(1);
    let ordinal = state.frames_produced;
    state.sent_video_ordinals.push_back((sequence, ordinal));
    while state.sent_video_ordinals.len() > SENT_VIDEO_HISTORY {
        state.sent_video_ordinals.pop_front();
    }
    Ok(())
}

fn handle_usb_control(
    shared: &Arc<Mutex<SharedState>>,
    cancel: &AtomicBool,
    transport: &mut impl PacketTransport,
    packet: Packet,
    clock_origin: Instant,
    next_sequence: &mut u64,
) -> Result<(), String> {
    let packet_len = packet.len();
    let packet_bytes = packet.into_payload();
    let DecodeOutcome::Complete { frame, consumed } =
        WireFrame::decode_prefix(&packet_bytes).map_err(|error| error.to_string())?
    else {
        return Err("Android sent a partial LDFL control frame".to_owned());
    };
    if consumed != packet_len {
        return Err("Android control packet contains trailing LDFL bytes".to_owned());
    }
    let sequence = frame.header().sequence();
    let disposition = {
        let mut state = lock_arc(shared);
        state
            .session
            .as_mut()
            .ok_or_else(|| "active USB control arrived without a session".to_owned())?
            .observe_sequence(sequence)
            .map_err(|error| error.to_string())?
    };
    if !matches!(disposition, SequenceDisposition::Accepted { .. }) {
        return Err(format!(
            "Android LDFL sequence {sequence} is duplicate or stale"
        ));
    }

    match frame.header().kind() {
        MessageType::Ping => {
            let request = frame
                .decode_payload::<Ping>()
                .map_err(|error| error.to_string())?;
            let received_at = duration_micros_u64(clock_origin.elapsed());
            let response = Pong::new(
                request.token(),
                request.client_send_timestamp_micros(),
                received_at,
                duration_micros_u64(clock_origin.elapsed()),
            )
            .map_err(|error| error.to_string())?;
            send_control_payload(
                transport,
                *next_sequence,
                &response,
                cancel,
                USB_CONTROL_SEND_TIMEOUT,
            )?;
            *next_sequence = next_sequence
                .checked_add(1)
                .ok_or_else(|| "host LDFL sequence is exhausted".to_owned())?;
        }
        MessageType::Pong => {
            frame
                .decode_payload::<Pong>()
                .map_err(|error| error.to_string())?;
        }
        MessageType::Input => {
            frame
                .decode_payload::<InputEvent>()
                .map_err(|error| error.to_string())?;
        }
        MessageType::Telemetry => {
            let telemetry = frame
                .decode_payload::<Telemetry>()
                .map_err(|error| error.to_string())?;
            let mut state = lock_arc(shared);
            apply_usb_telemetry(&mut state, telemetry)?;
        }
        MessageType::Error => {
            let error = frame
                .decode_payload::<ErrorMessage>()
                .map_err(|decode_error| decode_error.to_string())?;
            return Err(format!(
                "Android display reported {:?}: {}",
                error.code(),
                error.diagnostic()
            ));
        }
        kind => {
            return Err(format!(
                "Android sent unexpected {kind:?} after LDFL negotiation"
            ));
        }
    }
    Ok(())
}

fn apply_usb_telemetry(state: &mut SharedState, telemetry: Telemetry) -> Result<(), String> {
    state.frames_dropped = u64::from(telemetry.dropped_frames());
    state.queue_depth = usize::from(telemetry.queue_depth());
    let presented_ordinal = (telemetry.frame_id() != 0)
        .then(|| {
            state
                .sent_video_ordinals
                .iter()
                .rev()
                .find(|(sequence, _ordinal)| *sequence == telemetry.frame_id())
                .map(|(_sequence, ordinal)| *ordinal)
        })
        .flatten();
    if let Some(ordinal) = presented_ordinal {
        state.frames_presented = ordinal.saturating_sub(state.frames_dropped);
    }
    let timings = telemetry.timings();
    let total_latency_micros = u64::from(timings.capture_micros())
        .saturating_add(u64::from(timings.encode_micros()))
        .saturating_add(u64::from(timings.transport_micros()))
        .saturating_add(u64::from(timings.decode_micros()))
        .saturating_add(u64::from(timings.presentation_micros()));
    if total_latency_micros > 0 {
        state
            .latency
            .record(Duration::from_micros(total_latency_micros))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn negotiated_session(config: LoopbackConfig) -> Result<Session, String> {
    let host_hello = Hello::new(
        PROTOCOL_VERSION,
        PROTOCOL_VERSION,
        Role::Host,
        [0x48; 16],
        "LadoFlow desktop",
    )
    .map_err(|error| error.to_string())?;
    let display_hello = Hello::new(
        PROTOCOL_VERSION,
        PROTOCOL_VERSION,
        Role::Display,
        [0x44; 16],
        "LadoFlow synthetic display",
    )
    .map_err(|error| error.to_string())?;
    let host_capabilities = Capabilities::new(
        config.width,
        config.height,
        u32::from(config.fps) * 1_000,
        40_000,
        CodecSet::H264 | CodecSet::HEVC,
        InputCapabilities::POINTER | InputCapabilities::TOUCH | InputCapabilities::KEYBOARD,
        FeatureFlags::DYNAMIC_ROTATION | FeatureFlags::REMOTE_CURSOR,
    )
    .map_err(|error| error.to_string())?;
    let display_capabilities = Capabilities::new(
        config.width,
        config.height,
        u32::from(config.fps) * 1_000,
        20_000,
        CodecSet::H264,
        InputCapabilities::POINTER | InputCapabilities::TOUCH,
        FeatureFlags::DYNAMIC_ROTATION | FeatureFlags::REMOTE_CURSOR,
    )
    .map_err(|error| error.to_string())?;
    let agreement = negotiate(
        &host_hello,
        host_capabilities,
        &display_hello,
        display_capabilities,
    )
    .map_err(|error| error.to_string())?;

    let mut session = Session::new();
    session.start().map_err(|error| error.to_string())?;
    session
        .establish(agreement, StreamContinuity::Restart)
        .map_err(|error| error.to_string())?;
    Ok(session)
}

fn run_loopback(
    shared: &Arc<Mutex<SharedState>>,
    cancel: &Arc<AtomicBool>,
    config: LoopbackConfig,
) {
    let (mut host, mut display) = loopback_pair(TransportLoopbackConfig::default());
    let frame_rate = FrameRate::from_hz(u32::from(config.fps)).expect("validated frame rate");
    let dimensions = FrameDimensions::new(u32::from(config.width), u32::from(config.height))
        .expect("validated frame dimensions");
    let format = VideoFormat::new(dimensions, frame_rate);
    let synthetic_config = SyntheticConfig::new(format, 4 * 1_024, u64::from(config.fps) * 2)
        .expect("built-in synthetic configuration is valid")
        .with_seed(0x4c_44_46_4c);
    let mut producer = SyntheticFrameProducer::new(synthetic_config);
    let stream_origin = Instant::now();
    let mut pacer = FramePacer::new(frame_rate, Duration::ZERO);
    let mut sent_at = HashMap::<u64, Instant>::new();
    let mut last_key_sequence = None;

    while !cancel.load(Ordering::Acquire) {
        let elapsed = stream_origin.elapsed();
        let tick = match pacer.poll(elapsed) {
            PaceDecision::Due(tick) => tick,
            PaceDecision::WaitUntil(deadline) => {
                thread::sleep(deadline.saturating_sub(elapsed));
                continue;
            }
            PaceDecision::Exhausted => break,
        };
        if tick.skipped() > 0 {
            let producer_skipped = producer.advance_to_sequence(tick.index());
            debug_assert_eq!(producer_skipped, tick.skipped());
            let mut state = lock_arc(shared);
            state.frames_dropped = state.frames_dropped.saturating_add(tick.skipped());
        }

        let Some(media_frame) = producer.next_frame() else {
            break;
        };
        let sequence = media_frame.metadata().sequence();
        let is_key = media_frame.metadata().kind() == FrameKind::Key;
        let Ok(packet) = protocol_packet(media_frame) else {
            let mut state = lock_arc(shared);
            state.frames_dropped = state.frames_dropped.saturating_add(1);
            continue;
        };
        match host.try_send(packet) {
            Ok(report) => {
                let superseded = report.superseded().packets();
                if superseded > 0 {
                    sent_at.retain(|queued_sequence, _time| {
                        Some(*queued_sequence) == last_key_sequence
                    });
                }
                if is_key {
                    last_key_sequence = Some(sequence);
                }
                sent_at.insert(sequence, Instant::now());
                let mut state = lock_arc(shared);
                state.frames_produced = state.frames_produced.saturating_add(1);
                state.frames_superseded = state
                    .frames_superseded
                    .saturating_add(u64::try_from(superseded).unwrap_or(u64::MAX));
                state.queue_depth = state
                    .queue_depth
                    .saturating_sub(superseded)
                    .saturating_add(1);
            }
            Err(_error) => {
                let mut state = lock_arc(shared);
                state.frames_dropped = state.frames_dropped.saturating_add(1);
            }
        }

        // Skip one presentation interval periodically. The next replaceable
        // media packet exercises the same latest-frame behavior used under load.
        if sequence % 120 != 0 {
            present_latest(&mut display, shared, &mut sent_at);
        }
    }

    lock_arc(shared).queue_depth = 0;
}

fn protocol_packet(frame: ladoflow_media::MediaFrame) -> Result<Packet, String> {
    let media_metadata = frame.metadata();
    let wire_metadata = VideoFrameMetadata::new(
        media_metadata.sequence(),
        duration_micros_u64(media_metadata.capture_time()),
        duration_micros_u64(media_metadata.presentation_time()),
        duration_micros_u32(media_metadata.duration())?,
    )
    .map_err(|error| error.to_string())?;
    let kind = media_metadata.kind();
    let (_metadata, payload) = frame.into_parts();
    let video_frame = VideoFrame::new(wire_metadata, payload).map_err(|error| error.to_string())?;
    let flags = if kind == FrameKind::Key {
        FrameFlags::KEYFRAME
    } else {
        FrameFlags::NONE
    };
    let wire_frame = WireFrame::from_payload(flags, media_metadata.sequence(), &video_frame)
        .map_err(|error| error.to_string())?;

    Ok(if kind == FrameKind::Key {
        Packet::media(wire_frame.encode())
    } else {
        Packet::replaceable_media(MEDIA_STREAM_KEY, wire_frame.encode())
    })
}

fn present_latest(
    display: &mut impl PacketTransport,
    shared: &Arc<Mutex<SharedState>>,
    sent_at: &mut HashMap<u64, Instant>,
) {
    let Ok(Some(packet)) = display.try_receive(Channel::Media) else {
        return;
    };
    let Ok(DecodeOutcome::Complete {
        frame: wire_frame,
        consumed,
    }) = WireFrame::decode_prefix(packet.payload())
    else {
        let mut state = lock_arc(shared);
        state.frames_dropped = state.frames_dropped.saturating_add(1);
        return;
    };
    if consumed != packet.len() {
        let mut state = lock_arc(shared);
        state.frames_dropped = state.frames_dropped.saturating_add(1);
        return;
    }
    let Ok(video_frame) = wire_frame.decode_payload::<VideoFrame>() else {
        let mut state = lock_arc(shared);
        state.frames_dropped = state.frames_dropped.saturating_add(1);
        return;
    };
    let sequence = video_frame.metadata().frame_id();
    let Some(started_at) = sent_at.remove(&sequence) else {
        return;
    };

    let mut state = lock_arc(shared);
    if let Some(session) = state.session.as_mut() {
        if session.phase() == SessionPhase::Active && session.observe_sequence(sequence).is_err() {
            state.frames_dropped = state.frames_dropped.saturating_add(1);
            return;
        }
    }
    state.frames_presented = state.frames_presented.saturating_add(1);
    state.queue_depth = state.queue_depth.saturating_sub(1);
    if state.latency.record(started_at.elapsed()).is_err() {
        state.frames_dropped = state.frames_dropped.saturating_add(1);
    }
}

fn lock_arc(shared: &Arc<Mutex<SharedState>>) -> MutexGuard<'_, SharedState> {
    shared
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[allow(clippy::cast_precision_loss)]
fn measured_fps(frames: u64, uptime: Duration) -> f64 {
    if uptime.is_zero() {
        0.0
    } else {
        frames as f64 / uptime.as_secs_f64()
    }
}

fn duration_millis(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1_000.0
}

fn duration_micros_u64(duration: Duration) -> u64 {
    u64::try_from(duration.as_micros()).unwrap_or(u64::MAX)
}

fn duration_micros_u32(duration: Duration) -> Result<u32, String> {
    u32::try_from(duration.as_micros())
        .map_err(|_| "synthetic frame duration exceeds protocol range".to_owned())
}

#[cfg(test)]
mod tests {
    use std::{
        collections::VecDeque,
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        },
        thread,
        time::{Duration, Instant},
    };

    use ladoflow_protocol::{
        Capabilities, CodecSet, DecodeOutcome, FeatureFlags, Frame as WireFrame, FrameFlags, Hello,
        InputCapabilities, Ping, Pong, Role, StageTimings, Telemetry, ThermalState, VideoFrame,
    };
    use ladoflow_transport::{
        Channel, LoopbackConfig as TransportConfig, Packet, PacketTransport, loopback_pair,
    };

    use super::{
        DesktopRuntime, H264AccessUnit, LoopbackConfig, SessionPhaseView, SharedState,
        align_media_clock, apply_usb_telemetry, handle_usb_control, negotiated_session,
        run_usb_control_session_inner, send_usb_h264_access_unit,
    };

    #[test]
    fn loopback_starts_records_frames_and_stops() {
        let runtime = DesktopRuntime::default();
        let started = runtime
            .start(
                LoopbackConfig {
                    width: 1_920,
                    height: 1_080,
                    fps: 60,
                },
                None,
            )
            .expect("start loopback");
        assert!(matches!(started.session.phase, SessionPhaseView::Streaming));

        thread::sleep(Duration::from_millis(80));
        let running = runtime.snapshot();
        assert!(running.telemetry.frames_presented >= 2);

        let stopped = runtime.stop().expect("stop loopback");
        assert!(matches!(stopped.session.phase, SessionPhaseView::Stopped));
    }

    #[test]
    fn h264_media_clock_realigns_only_on_a_late_keyframe() {
        let mut units = VecDeque::from([H264AccessUnit {
            bytes: vec![0, 0, 0, 1, 0x65],
            timestamp: Duration::ZERO,
            duration: Duration::from_millis(16),
            keyframe: true,
        }]);
        let mut offset = None;
        align_media_clock(&units, Duration::from_millis(500), &mut offset)
            .expect("initial media clock");
        assert_eq!(offset, Some(Duration::from_millis(500)));

        units.front_mut().expect("one unit").timestamp = Duration::from_secs(1);
        align_media_clock(&units, Duration::from_secs(2), &mut offset)
            .expect("late keyframe realigns");
        assert_eq!(offset, Some(Duration::from_secs(1)));

        units.front_mut().expect("one unit").timestamp = Duration::from_secs(3);
        units.front_mut().expect("one unit").keyframe = false;
        align_media_clock(&units, Duration::from_secs(5), &mut offset)
            .expect("late inter frame does not move the clock");
        assert_eq!(offset, Some(Duration::from_secs(1)));
    }

    #[test]
    fn usb_h264_access_unit_is_framed_without_supersession() {
        let (mut host, mut display) = loopback_pair(TransportConfig::default());
        let shared = Arc::new(Mutex::new(SharedState::new()));
        let mut next_sequence = 3;
        send_usb_h264_access_unit(
            &shared,
            &mut host,
            H264AccessUnit {
                bytes: vec![0, 0, 0, 1, 0x65, 1, 2, 3],
                timestamp: Duration::from_millis(2),
                duration: Duration::from_millis(16),
                keyframe: true,
            },
            Duration::from_millis(100),
            &mut next_sequence,
        )
        .expect("H.264 frame is queued");

        let packet = display
            .try_receive(Channel::Media)
            .expect("media queue remains connected")
            .expect("one H.264 packet");
        assert_eq!(packet.supersession_key(), None);
        let DecodeOutcome::Complete { frame, consumed } =
            WireFrame::decode_prefix(packet.payload()).expect("valid LDFL frame")
        else {
            panic!("complete H.264 frame expected");
        };
        assert_eq!(consumed, packet.len());
        assert_eq!(frame.header().sequence(), 3);
        assert!(frame.header().flags().contains(FrameFlags::KEYFRAME));
        let video = frame
            .decode_payload::<VideoFrame>()
            .expect("valid VideoFrame payload");
        assert_eq!(video.metadata().frame_id(), 3);
        assert_eq!(video.metadata().capture_timestamp_micros(), 102_000);
        assert_eq!(video.metadata().presentation_timestamp_micros(), 118_000);
        assert_eq!(next_sequence, 4);
        let mut state = shared.lock().expect("shared state");
        assert!(matches!(state.phase, SessionPhaseView::Streaming));
        assert_eq!(state.frames_produced, 1);
        assert_eq!(state.sent_video_ordinals.back(), Some(&(3, 1)));
        let telemetry = Telemetry::new(
            200_000,
            3,
            StageTimings::new(100, 200, 300, 400, 500).expect("valid timings"),
            0,
            0,
            0,
            0,
            ThermalState::Nominal,
        )
        .expect("valid telemetry");
        apply_usb_telemetry(&mut state, telemetry).expect("telemetry is applied");
        assert_eq!(state.frames_presented, 1);
        assert!(state.latency.snapshot().is_some());
    }

    #[test]
    fn invalid_refresh_rate_is_rejected() {
        let runtime = DesktopRuntime::default();
        let error = runtime
            .start(
                LoopbackConfig {
                    width: 1_920,
                    height: 1_080,
                    fps: 59,
                },
                None,
            )
            .expect_err("reject invalid refresh rate");
        assert!(error.contains("30 or 60"));
    }

    #[test]
    fn active_usb_ping_gets_pong_and_replayed_sequence_is_rejected() {
        let config = LoopbackConfig {
            width: 1_920,
            height: 1_080,
            fps: 60,
        };
        let mut state = SharedState::new();
        state.establish_usb_control(
            config,
            negotiated_session(config).expect("test session"),
            "LadoFlow Android".to_owned(),
        );
        let shared = Arc::new(Mutex::new(state));
        let (mut host, mut display) = loopback_pair(TransportConfig::default());
        let request = Ping::new(42, 100);
        let wire_ping = WireFrame::from_payload(FrameFlags::NONE, 2, &request).expect("Ping frame");
        let packet = Packet::control(wire_ping.encode());
        let cancel = AtomicBool::new(false);
        let mut next_sequence = 3;

        handle_usb_control(
            &shared,
            &cancel,
            &mut host,
            packet.clone(),
            Instant::now(),
            &mut next_sequence,
        )
        .expect("Ping is accepted");
        assert_eq!(next_sequence, 4);
        let response = display
            .try_receive(Channel::Control)
            .expect("link remains connected")
            .expect("Pong is queued");
        let DecodeOutcome::Complete { frame, consumed } =
            WireFrame::decode_prefix(response.payload()).expect("valid Pong frame")
        else {
            panic!("Pong packet must be complete");
        };
        assert_eq!(consumed, response.len());
        assert_eq!(frame.header().sequence(), 3);
        let response = frame.decode_payload::<Pong>().expect("typed Pong");
        assert_eq!(response.token(), 42);
        assert_eq!(response.client_send_timestamp_micros(), 100);

        assert!(
            handle_usb_control(
                &shared,
                &cancel,
                &mut host,
                packet,
                Instant::now(),
                &mut next_sequence,
            )
            .expect_err("replayed sequence is rejected")
            .contains("duplicate or stale")
        );
    }

    #[test]
    #[ignore = "requires an interactive Windows desktop and a physical H.264 encoder"]
    #[allow(clippy::too_many_lines)]
    fn native_capture_reaches_a_protocol_display_end_to_end() {
        let platform = crate::platform::collect_status();
        let selected_display_id = platform
            .displays
            .iter()
            .find(|display| display.primary)
            .or_else(|| platform.displays.first())
            .expect("an active Windows display")
            .id
            .clone();
        let (mut host, mut display) = loopback_pair(TransportConfig::default());
        let cancel = Arc::new(AtomicBool::new(false));
        let display_cancel = Arc::clone(&cancel);
        let display_worker = thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut initial_control = 0;
            while initial_control < 2 {
                if display
                    .try_receive(Channel::Control)
                    .expect("display control queue remains connected")
                    .is_some()
                {
                    initial_control += 1;
                } else {
                    assert!(Instant::now() < deadline, "host handshake timed out");
                    thread::sleep(Duration::from_millis(1));
                }
            }
            let hello = Hello::new(1, 1, Role::Display, [0x44; 16], "LadoFlow test display")
                .expect("valid display Hello");
            let capabilities = Capabilities::new(
                1_280,
                720,
                30_000,
                20_000,
                CodecSet::H264,
                InputCapabilities::POINTER | InputCapabilities::TOUCH,
                FeatureFlags::DYNAMIC_ROTATION,
            )
            .expect("valid display capabilities");
            for (sequence, payload) in [
                (
                    0,
                    WireFrame::from_payload(FrameFlags::NONE, 0, &hello)
                        .expect("display Hello frame"),
                ),
                (
                    1,
                    WireFrame::from_payload(FrameFlags::NONE, 1, &capabilities)
                        .expect("display capabilities frame"),
                ),
            ] {
                assert_eq!(payload.header().sequence(), sequence);
                display
                    .try_send(Packet::control(payload.encode()))
                    .expect("send display handshake frame");
            }

            let mut configured = false;
            let mut media_frames = 0_usize;
            let mut keyframes = 0_usize;
            while media_frames < 10 {
                if let Some(packet) = display
                    .try_receive(Channel::Control)
                    .expect("display control queue remains connected")
                {
                    let DecodeOutcome::Complete { frame, consumed } =
                        WireFrame::decode_prefix(packet.payload()).expect("valid host control")
                    else {
                        panic!("complete host control expected");
                    };
                    assert_eq!(consumed, packet.len());
                    if frame.header().kind() == ladoflow_protocol::MessageType::DisplayConfig {
                        configured = true;
                    }
                }
                if let Some(packet) = display
                    .try_receive(Channel::Media)
                    .expect("display media queue remains connected")
                {
                    assert!(configured, "media arrived before DisplayConfig");
                    let DecodeOutcome::Complete { frame, consumed } =
                        WireFrame::decode_prefix(packet.payload()).expect("valid host media")
                    else {
                        panic!("complete host media expected");
                    };
                    assert_eq!(consumed, packet.len());
                    let video = frame
                        .decode_payload::<VideoFrame>()
                        .expect("valid H.264 VideoFrame");
                    assert_eq!(video.metadata().frame_id(), frame.header().sequence());
                    assert!(
                        video
                            .encoded_bytes()
                            .windows(3)
                            .any(|bytes| bytes == [0, 0, 1])
                    );
                    keyframes += usize::from(frame.header().flags().contains(FrameFlags::KEYFRAME));
                    media_frames += 1;
                }
                assert!(Instant::now() < deadline, "media stream timed out");
                thread::sleep(Duration::from_millis(1));
            }
            display_cancel.store(true, Ordering::Release);
            (media_frames, keyframes)
        });

        let shared = Arc::new(Mutex::new(SharedState::new()));
        shared
            .lock()
            .expect("shared state")
            .reset_for_usb_negotiation(LoopbackConfig {
                width: 1_280,
                height: 720,
                fps: 30,
            });
        run_usb_control_session_inner(
            &shared,
            &cancel,
            LoopbackConfig {
                width: 1_280,
                height: 720,
                fps: 30,
            },
            Some(&selected_display_id),
            &mut host,
        )
        .expect("native capture reaches protocol display");
        let (media_frames, keyframes) = display_worker.join().expect("display worker");
        assert_eq!(media_frames, 10);
        assert!(keyframes > 0);
        assert!(shared.lock().expect("shared state").frames_produced >= 10);
    }
}
