//! Codec-neutral media primitives for `LadoFlow` loopback and diagnostics.
//!
//! The crate deliberately performs no capture, encoding, decoding, transport,
//! sleeping, or platform I/O. Callers provide a monotonic clock represented by
//! [`std::time::Duration`], making the producer and scheduler deterministic in
//! tests and straightforward to drive from a desktop event loop.

#![forbid(unsafe_code)]

mod metadata;
mod scheduler;
mod synthetic;

pub use metadata::{
    FrameDimensions, FrameDimensionsError, FrameKind, FrameMetadata, FrameRate, FrameRateError,
    MediaFrame, VideoFormat,
};
pub use scheduler::{
    FramePacer, IdleReason, LatestFrameScheduler, PaceDecision, PacingTick, PollOutcome,
    ScheduledFrame, SchedulerConfig, SchedulerConfigError, SchedulerMetrics, SubmitOutcome,
};
pub use synthetic::{
    MAX_SYNTHETIC_PAYLOAD_BYTES, SyntheticConfig, SyntheticConfigError, SyntheticFrameProducer,
};
