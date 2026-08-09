use std::{error::Error, fmt, time::Duration};

use crate::{FrameMetadata, FrameRate, MediaFrame};

/// A pacing slot selected from an exact rational frame timeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PacingTick {
    index: u64,
    deadline: Duration,
    skipped: u64,
}

impl PacingTick {
    /// Zero-based slot index.
    #[must_use]
    pub const fn index(self) -> u64 {
        self.index
    }

    /// Absolute deadline in the caller's monotonic clock domain.
    #[must_use]
    pub const fn deadline(self) -> Duration {
        self.deadline
    }

    /// Older pacing slots skipped to avoid a burst after a delayed poll.
    #[must_use]
    pub const fn skipped(self) -> u64 {
        self.skipped
    }
}

/// Result of polling a [`FramePacer`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaceDecision {
    /// One pacing slot is ready now.
    Due(PacingTick),
    /// No slot is due; poll at or after this absolute time.
    WaitUntil(Duration),
    /// Every representable `u64` slot index has been consumed.
    Exhausted,
}

/// Deterministic absolute-deadline frame pacer.
///
/// The pacer never sleeps. A delayed poll selects only the newest due slot and
/// reports the skipped count, preventing catch-up bursts.
#[derive(Debug, Clone)]
pub struct FramePacer {
    frame_rate: FrameRate,
    origin: Duration,
    next_tick: Option<u64>,
}

impl FramePacer {
    /// Start a pacing timeline whose frame zero is due at `origin`.
    #[must_use]
    pub const fn new(frame_rate: FrameRate, origin: Duration) -> Self {
        Self {
            frame_rate,
            origin,
            next_tick: Some(0),
        }
    }

    /// Output frame rate.
    #[must_use]
    pub const fn frame_rate(&self) -> FrameRate {
        self.frame_rate
    }

    /// Absolute start of the pacing timeline.
    #[must_use]
    pub const fn origin(&self) -> Duration {
        self.origin
    }

    /// Next absolute deadline, or `None` after sequence exhaustion.
    #[must_use]
    pub fn next_deadline(&self) -> Option<Duration> {
        self.next_tick.map(|tick| self.deadline(tick))
    }

    /// Select a due pacing slot using the supplied monotonic time.
    pub fn poll(&mut self, now: Duration) -> PaceDecision {
        let Some(next_tick) = self.next_tick else {
            return PaceDecision::Exhausted;
        };
        let next_deadline = self.deadline(next_tick);
        if now < next_deadline {
            return PaceDecision::WaitUntil(next_deadline);
        }

        let elapsed = now.saturating_sub(self.origin);
        let due_tick = self.frame_rate.frame_at_or_before(elapsed).max(next_tick);
        let tick = PacingTick {
            index: due_tick,
            deadline: self.deadline(due_tick),
            skipped: due_tick - next_tick,
        };
        self.next_tick = due_tick.checked_add(1);
        PaceDecision::Due(tick)
    }

    fn deadline(&self, tick: u64) -> Duration {
        self.origin.saturating_add(self.frame_rate.timestamp(tick))
    }
}

/// Validated limits for a latest-frame scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SchedulerConfig {
    frame_rate: FrameRate,
    max_frame_age: Duration,
    max_frame_bytes: usize,
}

impl SchedulerConfig {
    /// Construct scheduler limits.
    ///
    /// `max_frame_age` may be zero for strict deadline behavior.
    ///
    /// # Errors
    ///
    /// Returns [`SchedulerConfigError::ZeroFrameByteLimit`] when
    /// `max_frame_bytes` is zero.
    pub fn new(
        frame_rate: FrameRate,
        max_frame_age: Duration,
        max_frame_bytes: usize,
    ) -> Result<Self, SchedulerConfigError> {
        if max_frame_bytes == 0 {
            return Err(SchedulerConfigError::ZeroFrameByteLimit);
        }
        Ok(Self {
            frame_rate,
            max_frame_age,
            max_frame_bytes,
        })
    }

    /// Output pacing rate.
    #[must_use]
    pub const fn frame_rate(self) -> FrameRate {
        self.frame_rate
    }

    /// Time after a frame's presentation target before it is stale.
    #[must_use]
    pub const fn max_frame_age(self) -> Duration {
        self.max_frame_age
    }

    /// Maximum payload retained by the one-frame queue.
    #[must_use]
    pub const fn max_frame_bytes(self) -> usize {
        self.max_frame_bytes
    }
}

/// Invalid scheduler limits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerConfigError {
    /// A zero-byte queue cannot accept any useful media frame.
    ZeroFrameByteLimit,
}

impl fmt::Display for SchedulerConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroFrameByteLimit => {
                formatter.write_str("scheduler frame-byte limit must be non-zero")
            }
        }
    }
}

impl Error for SchedulerConfigError {}

/// Result of placing a frame into the one-frame queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubmitOutcome {
    /// Frame occupied an empty queue.
    Queued,
    /// Newer frame replaced the queued frame.
    Replaced {
        /// Sequence number removed from the queue.
        dropped_sequence: u64,
    },
    /// Frame exceeded its presentation-age budget before enqueue.
    DroppedStale {
        /// Sequence number that was discarded.
        sequence: u64,
    },
    /// Frame was no newer than the frame already queued.
    DroppedSuperseded {
        /// Sequence number that was discarded.
        sequence: u64,
        /// Sequence number retained in the queue.
        kept_sequence: u64,
    },
    /// Frame payload exceeded the configured byte ceiling.
    DroppedOversized {
        /// Sequence number that was discarded.
        sequence: u64,
        /// Actual payload byte count.
        payload_bytes: usize,
        /// Configured payload byte ceiling.
        limit: usize,
    },
}

/// Why a due pacing slot did not produce a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdleReason {
    /// No frame was queued.
    QueueEmpty,
    /// The latest queued frame has a future presentation timestamp.
    FrameNotDue,
}

/// Frame selected for a pacing slot with per-frame diagnostic timings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScheduledFrame {
    frame: MediaFrame,
    pacing_tick: PacingTick,
    queue_time: Duration,
    pacing_lateness: Duration,
    frame_latency: Duration,
}

impl ScheduledFrame {
    /// Selected media frame.
    #[must_use]
    pub const fn frame(&self) -> &MediaFrame {
        &self.frame
    }

    /// Pacing slot used for dispatch.
    #[must_use]
    pub const fn pacing_tick(&self) -> PacingTick {
        self.pacing_tick
    }

    /// Time from scheduler submission to dispatch.
    #[must_use]
    pub const fn queue_time(&self) -> Duration {
        self.queue_time
    }

    /// Dispatch delay after the selected pacing deadline.
    #[must_use]
    pub const fn pacing_lateness(&self) -> Duration {
        self.pacing_lateness
    }

    /// Time from the frame's capture timestamp to dispatch.
    #[must_use]
    pub const fn frame_latency(&self) -> Duration {
        self.frame_latency
    }

    /// Consume the diagnostics wrapper and return the media frame.
    #[must_use]
    pub fn into_frame(self) -> MediaFrame {
        self.frame
    }
}

/// Result of polling a [`LatestFrameScheduler`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollOutcome {
    /// A frame is ready for dispatch.
    Ready(ScheduledFrame),
    /// No pacing slot is due yet.
    WaitingUntil(Duration),
    /// A pacing slot was consumed without a ready frame.
    Idle {
        /// Consumed pacing slot.
        tick: PacingTick,
        /// Reason no frame was dispatched.
        reason: IdleReason,
    },
    /// A queued frame became stale at dispatch time.
    DroppedStale {
        /// Sequence number that was discarded.
        sequence: u64,
        /// Consumed pacing slot.
        tick: PacingTick,
    },
    /// Every representable pacing slot has been consumed.
    Exhausted,
}

/// Cumulative scheduler diagnostics.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SchedulerMetrics {
    submitted_frames: u64,
    presented_frames: u64,
    dropped_stale_frames: u64,
    dropped_superseded_frames: u64,
    dropped_oversized_frames: u64,
    pacing_ticks: u64,
    skipped_pacing_ticks: u64,
    idle_pacing_ticks: u64,
    total_queue_time: Duration,
    max_queue_time: Duration,
    total_pacing_lateness: Duration,
    max_pacing_lateness: Duration,
    total_frame_latency: Duration,
    max_frame_latency: Duration,
}

impl SchedulerMetrics {
    /// Frames passed to `submit`, including rejected frames.
    #[must_use]
    pub const fn submitted_frames(self) -> u64 {
        self.submitted_frames
    }

    /// Frames dispatched to the caller.
    #[must_use]
    pub const fn presented_frames(self) -> u64 {
        self.presented_frames
    }

    /// Frames discarded after exceeding their age budget.
    #[must_use]
    pub const fn dropped_stale_frames(self) -> u64 {
        self.dropped_stale_frames
    }

    /// Frames discarded because a newer frame occupied the queue.
    #[must_use]
    pub const fn dropped_superseded_frames(self) -> u64 {
        self.dropped_superseded_frames
    }

    /// Frames rejected for exceeding the payload byte ceiling.
    #[must_use]
    pub const fn dropped_oversized_frames(self) -> u64 {
        self.dropped_oversized_frames
    }

    /// Total drops across every scheduler drop category.
    #[must_use]
    pub const fn dropped_frames(self) -> u64 {
        self.dropped_stale_frames
            .saturating_add(self.dropped_superseded_frames)
            .saturating_add(self.dropped_oversized_frames)
    }

    /// Pacing polls that consumed a due slot.
    #[must_use]
    pub const fn pacing_ticks(self) -> u64 {
        self.pacing_ticks
    }

    /// Obsolete pacing slots skipped after delayed polls.
    #[must_use]
    pub const fn skipped_pacing_ticks(self) -> u64 {
        self.skipped_pacing_ticks
    }

    /// Consumed pacing slots that dispatched no frame.
    #[must_use]
    pub const fn idle_pacing_ticks(self) -> u64 {
        self.idle_pacing_ticks
    }

    /// Largest measured scheduler queue time.
    #[must_use]
    pub const fn max_queue_time(self) -> Duration {
        self.max_queue_time
    }

    /// Mean scheduler queue time across dispatched frames.
    #[must_use]
    pub fn average_queue_time(self) -> Option<Duration> {
        duration_average(self.total_queue_time, self.presented_frames)
    }

    /// Largest dispatch delay after a pacing deadline.
    #[must_use]
    pub const fn max_pacing_lateness(self) -> Duration {
        self.max_pacing_lateness
    }

    /// Mean dispatch delay after pacing deadlines.
    #[must_use]
    pub fn average_pacing_lateness(self) -> Option<Duration> {
        duration_average(self.total_pacing_lateness, self.presented_frames)
    }

    /// Largest capture-to-dispatch latency.
    #[must_use]
    pub const fn max_frame_latency(self) -> Duration {
        self.max_frame_latency
    }

    /// Mean capture-to-dispatch latency across dispatched frames.
    #[must_use]
    pub fn average_frame_latency(self) -> Option<Duration> {
        duration_average(self.total_frame_latency, self.presented_frames)
    }

    fn record_pacing_tick(&mut self, tick: PacingTick) {
        self.pacing_ticks = self.pacing_ticks.saturating_add(1);
        self.skipped_pacing_ticks = self.skipped_pacing_ticks.saturating_add(tick.skipped);
    }

    fn record_idle_tick(&mut self) {
        self.idle_pacing_ticks = self.idle_pacing_ticks.saturating_add(1);
    }

    fn record_presented(
        &mut self,
        queue_time: Duration,
        pacing_lateness: Duration,
        frame_latency: Duration,
    ) {
        self.presented_frames = self.presented_frames.saturating_add(1);
        self.total_queue_time = self.total_queue_time.saturating_add(queue_time);
        self.max_queue_time = self.max_queue_time.max(queue_time);
        self.total_pacing_lateness = self.total_pacing_lateness.saturating_add(pacing_lateness);
        self.max_pacing_lateness = self.max_pacing_lateness.max(pacing_lateness);
        self.total_frame_latency = self.total_frame_latency.saturating_add(frame_latency);
        self.max_frame_latency = self.max_frame_latency.max(frame_latency);
    }
}

#[derive(Debug)]
struct PendingFrame {
    frame: MediaFrame,
    enqueued_at: Duration,
}

/// One-frame, byte-capped queue combined with an absolute-deadline pacer.
///
/// Newer presentation timestamps replace older queued frames. No method sleeps
/// or starts a thread; integrations retain control of their event loop.
#[derive(Debug)]
pub struct LatestFrameScheduler {
    config: SchedulerConfig,
    stream_origin: Duration,
    pacer: FramePacer,
    pending: Option<PendingFrame>,
    metrics: SchedulerMetrics,
}

impl LatestFrameScheduler {
    /// Construct an empty scheduler whose media timestamps are relative to
    /// `stream_origin`.
    #[must_use]
    pub const fn new(config: SchedulerConfig, stream_origin: Duration) -> Self {
        Self {
            config,
            stream_origin,
            pacer: FramePacer::new(config.frame_rate, stream_origin),
            pending: None,
            metrics: SchedulerMetrics {
                submitted_frames: 0,
                presented_frames: 0,
                dropped_stale_frames: 0,
                dropped_superseded_frames: 0,
                dropped_oversized_frames: 0,
                pacing_ticks: 0,
                skipped_pacing_ticks: 0,
                idle_pacing_ticks: 0,
                total_queue_time: Duration::ZERO,
                max_queue_time: Duration::ZERO,
                total_pacing_lateness: Duration::ZERO,
                max_pacing_lateness: Duration::ZERO,
                total_frame_latency: Duration::ZERO,
                max_frame_latency: Duration::ZERO,
            },
        }
    }

    /// Validated scheduler limits.
    #[must_use]
    pub const fn config(&self) -> SchedulerConfig {
        self.config
    }

    /// Whether the single queue slot is occupied.
    #[must_use]
    pub const fn has_pending_frame(&self) -> bool {
        self.pending.is_some()
    }

    /// Sequence currently occupying the queue, if any.
    #[must_use]
    pub fn pending_sequence(&self) -> Option<u64> {
        self.pending
            .as_ref()
            .map(|pending| pending.frame.metadata().sequence())
    }

    /// Current cumulative diagnostic counters and durations.
    #[must_use]
    pub const fn metrics(&self) -> SchedulerMetrics {
        self.metrics
    }

    /// Reset the diagnostic window and return its previous values.
    pub fn take_metrics(&mut self) -> SchedulerMetrics {
        std::mem::take(&mut self.metrics)
    }

    /// Submit a frame at an absolute monotonic time.
    ///
    /// Oversized and already-stale frames are rejected without displacing the
    /// current queue occupant. A frame is newer when its presentation timestamp
    /// is later, with sequence number used as a tie breaker.
    pub fn submit(&mut self, frame: MediaFrame, now: Duration) -> SubmitOutcome {
        self.metrics.submitted_frames = self.metrics.submitted_frames.saturating_add(1);
        let metadata = frame.metadata();
        let sequence = metadata.sequence();
        let payload_bytes = frame.payload_len();

        if payload_bytes > self.config.max_frame_bytes {
            self.metrics.dropped_oversized_frames =
                self.metrics.dropped_oversized_frames.saturating_add(1);
            return SubmitOutcome::DroppedOversized {
                sequence,
                payload_bytes,
                limit: self.config.max_frame_bytes,
            };
        }
        if self.is_stale(metadata, now) {
            self.metrics.dropped_stale_frames = self.metrics.dropped_stale_frames.saturating_add(1);
            return SubmitOutcome::DroppedStale { sequence };
        }

        if let Some(pending) = &self.pending {
            let kept_metadata = pending.frame.metadata();
            if frame_order(metadata) <= frame_order(kept_metadata) {
                self.metrics.dropped_superseded_frames =
                    self.metrics.dropped_superseded_frames.saturating_add(1);
                return SubmitOutcome::DroppedSuperseded {
                    sequence,
                    kept_sequence: kept_metadata.sequence(),
                };
            }
        }

        let replaced = self.pending.replace(PendingFrame {
            frame,
            enqueued_at: now,
        });
        if let Some(replaced) = replaced {
            self.metrics.dropped_superseded_frames =
                self.metrics.dropped_superseded_frames.saturating_add(1);
            SubmitOutcome::Replaced {
                dropped_sequence: replaced.frame.metadata().sequence(),
            }
        } else {
            SubmitOutcome::Queued
        }
    }

    /// Poll pacing and dispatch the latest eligible frame when a slot is due.
    pub fn poll(&mut self, now: Duration) -> PollOutcome {
        let tick = match self.pacer.poll(now) {
            PaceDecision::Due(tick) => tick,
            PaceDecision::WaitUntil(deadline) => return PollOutcome::WaitingUntil(deadline),
            PaceDecision::Exhausted => return PollOutcome::Exhausted,
        };
        self.metrics.record_pacing_tick(tick);

        let Some(pending) = self.pending.take() else {
            self.metrics.record_idle_tick();
            return PollOutcome::Idle {
                tick,
                reason: IdleReason::QueueEmpty,
            };
        };
        let metadata = pending.frame.metadata();

        if self.is_stale(metadata, now) {
            self.metrics.dropped_stale_frames = self.metrics.dropped_stale_frames.saturating_add(1);
            self.metrics.record_idle_tick();
            return PollOutcome::DroppedStale {
                sequence: metadata.sequence(),
                tick,
            };
        }

        let presentation_target = self.absolute_time(metadata.presentation_time());
        if now < presentation_target {
            self.pending = Some(pending);
            self.metrics.record_idle_tick();
            return PollOutcome::Idle {
                tick,
                reason: IdleReason::FrameNotDue,
            };
        }

        let queue_time = now.saturating_sub(pending.enqueued_at);
        let pacing_lateness = now.saturating_sub(tick.deadline);
        let capture_time = self.absolute_time(metadata.capture_time());
        let frame_latency = now.saturating_sub(capture_time);
        self.metrics
            .record_presented(queue_time, pacing_lateness, frame_latency);

        PollOutcome::Ready(ScheduledFrame {
            frame: pending.frame,
            pacing_tick: tick,
            queue_time,
            pacing_lateness,
            frame_latency,
        })
    }

    fn is_stale(&self, metadata: FrameMetadata, now: Duration) -> bool {
        let stale_after = self
            .absolute_time(metadata.presentation_time())
            .saturating_add(self.config.max_frame_age);
        now > stale_after
    }

    fn absolute_time(&self, stream_time: Duration) -> Duration {
        self.stream_origin.saturating_add(stream_time)
    }
}

fn frame_order(metadata: FrameMetadata) -> (Duration, u64) {
    (metadata.presentation_time(), metadata.sequence())
}

fn duration_average(total: Duration, samples: u64) -> Option<Duration> {
    if samples == 0 {
        return None;
    }
    Some(duration_from_nanos(total.as_nanos() / u128::from(samples)))
}

fn duration_from_nanos(nanos: u128) -> Duration {
    const NANOS_PER_SECOND: u128 = 1_000_000_000;

    let seconds = nanos / NANOS_PER_SECOND;
    let Ok(seconds) = u64::try_from(seconds) else {
        return Duration::MAX;
    };
    let subsecond_nanos = nanos % NANOS_PER_SECOND;
    let subsecond_nanos = u32::try_from(subsecond_nanos).expect("subsecond nanoseconds fit in u32");
    Duration::new(seconds, subsecond_nanos)
}
