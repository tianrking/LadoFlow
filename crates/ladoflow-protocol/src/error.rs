use std::fmt;

use crate::MessageType;

/// Error returned when a frame or payload violates the wire contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProtocolError {
    /// The four-byte stream marker does not identify a `LadoFlow` frame.
    InvalidMagic([u8; 4]),
    /// The fixed header length differs from the supported layout.
    InvalidHeaderLength(u16),
    /// The peer is using a frame version this implementation cannot parse.
    UnsupportedVersion {
        /// Version found on the wire.
        found: u16,
        /// Version supported by this implementation.
        supported: u16,
    },
    /// The numeric message identifier has no defined meaning.
    UnknownMessageType(u16),
    /// One or more flag bits are not defined by this protocol version.
    UnknownFrameFlags(u16),
    /// The declared payload exceeds the limit for its message family.
    PayloadTooLarge {
        /// Message family used to select the limit.
        kind: MessageType,
        /// Declared number of bytes.
        length: usize,
        /// Maximum accepted number of bytes.
        limit: usize,
    },
    /// A constructed frame's declared and actual payload lengths differ.
    PayloadLengthMismatch {
        /// Length stored in the header.
        declared: usize,
        /// Bytes actually supplied.
        actual: usize,
    },
    /// An incremental decoder would exceed its configured memory ceiling.
    BufferLimitExceeded {
        /// Buffer size that would result from accepting the chunk.
        attempted: usize,
        /// Configured memory ceiling.
        limit: usize,
    },
    /// A typed payload was requested from the wrong frame family.
    UnexpectedMessageType {
        /// Message family required by the payload type.
        expected: MessageType,
        /// Message family present in the frame.
        actual: MessageType,
    },
    /// A typed control payload violates its schema.
    InvalidPayload(&'static str),
    /// A protocol string is not valid UTF-8.
    InvalidUtf8,
}

impl fmt::Display for ProtocolError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidMagic(found) => write!(formatter, "invalid frame magic: {found:02x?}"),
            Self::InvalidHeaderLength(found) => {
                write!(formatter, "invalid frame header length: {found}")
            }
            Self::UnsupportedVersion { found, supported } => write!(
                formatter,
                "unsupported protocol version {found}; supported version is {supported}"
            ),
            Self::UnknownMessageType(value) => {
                write!(formatter, "unknown message type: {value}")
            }
            Self::UnknownFrameFlags(bits) => write!(formatter, "unknown frame flags: 0x{bits:04x}"),
            Self::PayloadTooLarge {
                kind,
                length,
                limit,
            } => write!(
                formatter,
                "{kind:?} payload is {length} bytes; limit is {limit} bytes"
            ),
            Self::PayloadLengthMismatch { declared, actual } => write!(
                formatter,
                "payload length mismatch: header declares {declared} bytes, got {actual}"
            ),
            Self::BufferLimitExceeded { attempted, limit } => write!(
                formatter,
                "decoder buffer would grow to {attempted} bytes; limit is {limit} bytes"
            ),
            Self::UnexpectedMessageType { expected, actual } => write!(
                formatter,
                "expected {expected:?} payload, received {actual:?}"
            ),
            Self::InvalidPayload(reason) => write!(formatter, "invalid payload: {reason}"),
            Self::InvalidUtf8 => formatter.write_str("protocol string is not valid UTF-8"),
        }
    }
}

impl std::error::Error for ProtocolError {}
