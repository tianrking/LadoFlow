use std::{
    collections::HashMap,
    num::NonZeroUsize,
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use ladoflow_core::{LatencyAggregator, Session, SessionPhase, StreamContinuity, negotiate};
use ladoflow_media::{
    FrameDimensions, FrameKind, FramePacer, FrameRate, PaceDecision, SyntheticConfig,
    SyntheticFrameProducer, VideoFormat,
};
use ladoflow_protocol::{
    Capabilities, CodecSet, DecodeOutcome, FeatureFlags, Frame as WireFrame, FrameFlags, Hello,
    InputCapabilities, PROTOCOL_VERSION, Role, VideoFrame, VideoFrameMetadata,
};
use ladoflow_transport::LoopbackConfig as TransportLoopbackConfig;
use ladoflow_transport::{Channel, Packet, PacketTransport, SupersessionKey, loopback_pair};
use serde::{Deserialize, Serialize};

use crate::platform::{PlatformStatus, collect_status};

const LATENCY_WINDOW: NonZeroUsize = NonZeroUsize::new(240).expect("240 is non-zero");
const MEDIA_STREAM_KEY: SupersessionKey = SupersessionKey::new(1);

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
    Streaming,
    Stopped,
    Failed,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    phase: SessionPhaseView,
    transport: &'static str,
    peer_name: Option<&'static str>,
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
    started_at: Option<Instant>,
    latency: LatencyAggregator,
    frames_produced: u64,
    frames_presented: u64,
    frames_dropped: u64,
    frames_superseded: u64,
    queue_depth: usize,
}

impl SharedState {
    fn new() -> Self {
        Self {
            phase: SessionPhaseView::Idle,
            session: None,
            config: None,
            started_at: None,
            latency: LatencyAggregator::new(LATENCY_WINDOW),
            frames_produced: 0,
            frames_presented: 0,
            frames_dropped: 0,
            frames_superseded: 0,
            queue_depth: 0,
        }
    }

    fn reset_for_start(&mut self, config: LoopbackConfig, session: Session) {
        self.phase = SessionPhaseView::Streaming;
        self.session = Some(session);
        self.config = Some(config);
        self.started_at = Some(Instant::now());
        self.latency = LatencyAggregator::new(LATENCY_WINDOW);
        self.frames_produced = 0;
        self.frames_presented = 0;
        self.frames_dropped = 0;
        self.frames_superseded = 0;
        self.queue_depth = 0;
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
                transport: "In-memory duplex",
                peer_name: matches!(self.phase, SessionPhaseView::Streaming)
                    .then_some("LadoFlow synthetic display"),
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
}

impl DesktopRuntime {
    pub fn snapshot(&self) -> HostSnapshot {
        self.lock_shared().snapshot(collect_status())
    }

    pub fn start(&self, config: LoopbackConfig) -> Result<HostSnapshot, String> {
        let config = config.validate()?;
        let mut worker_slot = self.lock_worker();
        if let Some(worker) = worker_slot.as_ref() {
            if !worker.handle.is_finished() {
                return Err("a loopback session is already running".to_owned());
            }
        }
        if let Some(finished) = worker_slot.take() {
            finished
                .handle
                .join()
                .map_err(|_| "the previous loopback worker panicked".to_owned())?;
        }

        {
            let mut shared = self.lock_shared();
            shared.phase = SessionPhaseView::Negotiating;
        }

        let session = negotiated_session(config).inspect_err(|_error| {
            self.lock_shared().phase = SessionPhaseView::Failed;
        })?;
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let worker_shared = Arc::clone(&self.shared);

        self.lock_shared().reset_for_start(config, session);
        let handle = thread::Builder::new()
            .name("ladoflow-loopback".to_owned())
            .spawn(move || run_loopback(&worker_shared, &worker_cancel, config))
            .map_err(|error| {
                self.lock_shared().phase = SessionPhaseView::Failed;
                format!("failed to start loopback worker: {error}")
            })?;
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
        let snapshot = shared.snapshot(collect_status());
        drop(shared);
        Ok(snapshot)
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
    use std::{thread, time::Duration};

    use super::{DesktopRuntime, LoopbackConfig, SessionPhaseView};

    #[test]
    fn loopback_starts_records_frames_and_stops() {
        let runtime = DesktopRuntime::default();
        let started = runtime
            .start(LoopbackConfig {
                width: 1_920,
                height: 1_080,
                fps: 60,
            })
            .expect("start loopback");
        assert!(matches!(started.session.phase, SessionPhaseView::Streaming));

        thread::sleep(Duration::from_millis(80));
        let running = runtime.snapshot();
        assert!(running.telemetry.frames_presented >= 2);

        let stopped = runtime.stop().expect("stop loopback");
        assert!(matches!(stopped.session.phase, SessionPhaseView::Stopped));
    }

    #[test]
    fn invalid_refresh_rate_is_rejected() {
        let runtime = DesktopRuntime::default();
        let error = runtime
            .start(LoopbackConfig {
                width: 1_920,
                height: 1_080,
                fps: 59,
            })
            .expect_err("reject invalid refresh rate");
        assert!(error.contains("30 or 60"));
    }
}
