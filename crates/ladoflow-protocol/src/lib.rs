//! Transport-independent wire protocol primitives for `LadoFlow`.
//!
//! This crate owns the stable binary frame boundary shared by every host and
//! display implementation. It deliberately contains no socket, USB, codec, or
//! operating-system code.

#![forbid(unsafe_code)]

mod control;
mod display;
mod error;
mod frame;
mod input;
mod message;
mod telemetry;

pub use control::{ErrorCode, ErrorMessage, MAX_ERROR_DIAGNOSTIC_BYTES, Ping, Pong};
pub use display::{
    CodecProfile, DisplayConfig, MAX_ENCODED_VIDEO_BYTES, VIDEO_FRAME_METADATA_LEN, VideoCodec,
    VideoFrame, VideoFrameMetadata,
};
pub use error::ProtocolError;
pub use frame::{
    DecodeOutcome, FRAME_HEADER_LEN, FRAME_MAGIC, Frame, FrameDecoder, FrameFlags, FrameHeader,
    MAX_BUFFERED_BYTES, MAX_CONTROL_PAYLOAD, MAX_MEDIA_PAYLOAD, MessageType,
};
pub use input::{
    ButtonState, InputEvent, InputEventKind, KeyModifiers, MAX_TOUCH_CONTACTS, PointerButton,
    TouchPhase,
};
pub use message::{
    Capabilities, CodecSet, FeatureFlags, Hello, InputCapabilities, Role, WirePayload,
};
pub use telemetry::{
    MAX_LOSS_PARTS_PER_MILLION, MAX_STAGE_DURATION_MICROS, MAX_TELEMETRY_QUEUE_DEPTH, StageTimings,
    Telemetry, ThermalState,
};

/// First protocol generation used during pre-alpha development.
pub const PROTOCOL_VERSION: u16 = 1;
