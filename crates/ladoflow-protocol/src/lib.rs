//! Transport-independent wire protocol primitives for `LadoFlow`.
//!
//! This crate owns the stable binary frame boundary shared by every host and
//! display implementation. It deliberately contains no socket, USB, codec, or
//! operating-system code.

#![forbid(unsafe_code)]

mod error;
mod frame;
mod message;

pub use error::ProtocolError;
pub use frame::{
    DecodeOutcome, FRAME_HEADER_LEN, FRAME_MAGIC, Frame, FrameDecoder, FrameFlags, FrameHeader,
    MAX_BUFFERED_BYTES, MAX_CONTROL_PAYLOAD, MAX_MEDIA_PAYLOAD, MessageType,
};
pub use message::{
    Capabilities, CodecSet, FeatureFlags, Hello, InputCapabilities, Role, WirePayload,
};

/// First protocol generation used during pre-alpha development.
pub const PROTOCOL_VERSION: u16 = 1;
