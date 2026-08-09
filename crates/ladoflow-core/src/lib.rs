//! Shared, transport-independent session policy for `LadoFlow`.
//!
//! The crate negotiates protocol and capability limits, tracks connection and
//! reconnect state, summarizes a bounded latency window, and recommends a
//! conservative stream quality. It performs no I/O and has no clock or async
//! runtime dependency; platform code remains responsible for driving it.

#![forbid(unsafe_code)]

mod negotiation;
mod quality;
mod session;
mod telemetry;

pub use negotiation::{NegotiatedCapabilities, NegotiatedSession, NegotiationError, negotiate};
pub use quality::{QualityPolicy, QualityPolicyError, QualityRecommendation, QualityTier};
pub use session::{
    ReconnectDecision, ReconnectPolicy, ReconnectPolicyError, SequenceDisposition, Session,
    SessionError, SessionPhase, StreamContinuity,
};
pub use telemetry::{LatencyAggregator, LatencySnapshot, TelemetryError};
