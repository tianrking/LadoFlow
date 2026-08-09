use std::ops::BitOr;

use crate::{PROTOCOL_VERSION, ProtocolError, WirePayload};

/// Four-byte marker at the beginning of every frame.
pub const FRAME_MAGIC: [u8; 4] = *b"LDFL";

/// Size of the version-one frame header.
pub const FRAME_HEADER_LEN: usize = 24;

const FRAME_HEADER_LEN_U16: u16 = 24;

/// Maximum payload accepted for control, input, and telemetry frames.
pub const MAX_CONTROL_PAYLOAD: usize = 64 * 1024;

/// Maximum encoded video payload accepted in one frame.
pub const MAX_MEDIA_PAYLOAD: usize = 16 * 1024 * 1024;

/// Default memory ceiling for an incremental decoder.
pub const MAX_BUFFERED_BYTES: usize = 2 * (FRAME_HEADER_LEN + MAX_MEDIA_PAYLOAD);

/// Stable message identifiers used in the binary frame header.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum MessageType {
    /// Protocol range, endpoint role, nonce, and implementation name.
    Hello = 1,
    /// Display, codec, input, and optional-feature support.
    Capabilities = 2,
    /// Negotiated display and encoder configuration.
    DisplayConfig = 3,
    /// Encoded video access unit and presentation metadata.
    VideoFrame = 4,
    /// Touch, pointer, keyboard, or focus event.
    Input = 5,
    /// Queue, latency, drop, and device-health measurements.
    Telemetry = 6,
    /// Liveness and clock-offset request.
    Ping = 7,
    /// Liveness and clock-offset response.
    Pong = 8,
    /// Stable error code and bounded diagnostic information.
    Error = 9,
}

impl MessageType {
    #[must_use]
    const fn payload_limit(self) -> usize {
        match self {
            Self::VideoFrame => MAX_MEDIA_PAYLOAD,
            Self::Hello
            | Self::Capabilities
            | Self::DisplayConfig
            | Self::Input
            | Self::Telemetry
            | Self::Ping
            | Self::Pong
            | Self::Error => MAX_CONTROL_PAYLOAD,
        }
    }
}

impl TryFrom<u16> for MessageType {
    type Error = ProtocolError;

    fn try_from(value: u16) -> Result<Self, ProtocolError> {
        match value {
            1 => Ok(Self::Hello),
            2 => Ok(Self::Capabilities),
            3 => Ok(Self::DisplayConfig),
            4 => Ok(Self::VideoFrame),
            5 => Ok(Self::Input),
            6 => Ok(Self::Telemetry),
            7 => Ok(Self::Ping),
            8 => Ok(Self::Pong),
            9 => Ok(Self::Error),
            unknown => Err(ProtocolError::UnknownMessageType(unknown)),
        }
    }
}

/// Validated bit flags carried by a frame.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct FrameFlags(u16);

impl FrameFlags {
    /// Frame has no special semantics.
    pub const NONE: Self = Self(0);
    /// A video payload can initialize or reset a decoder.
    pub const KEYFRAME: Self = Self(1 << 0);
    /// Sender will not emit another payload in this stream.
    pub const END_OF_STREAM: Self = Self(1 << 1);
    /// Sender expects an explicit acknowledgement.
    pub const ACK_REQUIRED: Self = Self(1 << 2);

    const KNOWN_MASK: u16 = Self::KEYFRAME.0 | Self::END_OF_STREAM.0 | Self::ACK_REQUIRED.0;

    /// Construct flags after rejecting bits unknown to this protocol version.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::UnknownFrameFlags`] when `bits` contains a flag
    /// not defined by the active protocol version.
    pub fn from_bits(bits: u16) -> Result<Self, ProtocolError> {
        if bits & !Self::KNOWN_MASK == 0 {
            Ok(Self(bits))
        } else {
            Err(ProtocolError::UnknownFrameFlags(bits))
        }
    }

    /// Return the endian-independent numeric representation.
    #[must_use]
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Test whether every bit in `other` is present.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for FrameFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

/// Validated fixed-size metadata preceding every payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    version: u16,
    kind: MessageType,
    flags: FrameFlags,
    sequence: u64,
    payload_len: u32,
}

impl FrameHeader {
    /// Construct a version-one header while enforcing the message-size limit.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::PayloadTooLarge`] when `payload_len` exceeds
    /// the limit for `kind` or cannot be represented by the wire field.
    pub fn new(
        kind: MessageType,
        flags: FrameFlags,
        sequence: u64,
        payload_len: usize,
    ) -> Result<Self, ProtocolError> {
        validate_payload_len(kind, payload_len)?;
        let payload_len =
            u32::try_from(payload_len).map_err(|_| ProtocolError::PayloadTooLarge {
                kind,
                length: payload_len,
                limit: kind.payload_limit(),
            })?;

        Ok(Self {
            version: PROTOCOL_VERSION,
            kind,
            flags,
            sequence,
            payload_len,
        })
    }

    /// Protocol generation used to interpret this frame.
    #[must_use]
    pub const fn version(self) -> u16 {
        self.version
    }

    /// Logical family of the payload.
    #[must_use]
    pub const fn kind(self) -> MessageType {
        self.kind
    }

    /// Validated flags attached to the payload.
    #[must_use]
    pub const fn flags(self) -> FrameFlags {
        self.flags
    }

    /// Monotonic sequence number assigned by the sender.
    #[must_use]
    pub const fn sequence(self) -> u64 {
        self.sequence
    }

    /// Number of payload bytes following the fixed header.
    #[must_use]
    pub const fn payload_len(self) -> u32 {
        self.payload_len
    }

    /// Encode the fixed header in network byte order.
    #[must_use]
    pub fn encode(self) -> [u8; FRAME_HEADER_LEN] {
        let mut bytes = [0_u8; FRAME_HEADER_LEN];
        bytes[0..4].copy_from_slice(&FRAME_MAGIC);
        bytes[4..6].copy_from_slice(&self.version.to_be_bytes());
        bytes[6..8].copy_from_slice(&FRAME_HEADER_LEN_U16.to_be_bytes());
        bytes[8..10].copy_from_slice(&(self.kind as u16).to_be_bytes());
        bytes[10..12].copy_from_slice(&self.flags.bits().to_be_bytes());
        bytes[12..20].copy_from_slice(&self.sequence.to_be_bytes());
        bytes[20..24].copy_from_slice(&self.payload_len.to_be_bytes());
        bytes
    }

    fn decode(bytes: &[u8]) -> Result<Self, ProtocolError> {
        let found_magic = [bytes[0], bytes[1], bytes[2], bytes[3]];
        if found_magic != FRAME_MAGIC {
            return Err(ProtocolError::InvalidMagic(found_magic));
        }

        let version = read_u16(bytes, 4);
        if version != PROTOCOL_VERSION {
            return Err(ProtocolError::UnsupportedVersion {
                found: version,
                supported: PROTOCOL_VERSION,
            });
        }

        let header_len = read_u16(bytes, 6);
        if usize::from(header_len) != FRAME_HEADER_LEN {
            return Err(ProtocolError::InvalidHeaderLength(header_len));
        }

        let kind = MessageType::try_from(read_u16(bytes, 8))?;
        let flags = FrameFlags::from_bits(read_u16(bytes, 10))?;
        let sequence = read_u64(bytes, 12);
        let payload_len = read_u32(bytes, 20);
        let payload_len_usize = payload_len_to_usize(kind, payload_len)?;
        validate_payload_len(kind, payload_len_usize)?;

        Ok(Self {
            version,
            kind,
            flags,
            sequence,
            payload_len,
        })
    }
}

/// Owned protocol frame with a validated header and bounded payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    header: FrameHeader,
    payload: Vec<u8>,
}

impl Frame {
    /// Construct a frame and derive its declared length from the payload.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::PayloadTooLarge`] when the payload exceeds the
    /// limit for `kind`.
    pub fn new(
        kind: MessageType,
        flags: FrameFlags,
        sequence: u64,
        payload: impl Into<Vec<u8>>,
    ) -> Result<Self, ProtocolError> {
        let payload = payload.into();
        let header = FrameHeader::new(kind, flags, sequence, payload.len())?;
        Ok(Self { header, payload })
    }

    /// Encode a typed payload in a frame with its required kind.
    ///
    /// # Errors
    ///
    /// Returns a payload validation error from [`WirePayload::encode`] or
    /// [`ProtocolError::PayloadTooLarge`] if its encoded form exceeds the frame
    /// family limit.
    pub fn from_payload<P: WirePayload>(
        flags: FrameFlags,
        sequence: u64,
        payload: &P,
    ) -> Result<Self, ProtocolError> {
        Self::new(P::KIND, flags, sequence, payload.encode()?)
    }

    /// Decode the frame as a typed payload after checking its kind.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::UnexpectedMessageType`] if `P` does not match
    /// the frame family, or the validation error returned by `P::decode`.
    pub fn decode_payload<P: WirePayload>(&self) -> Result<P, ProtocolError> {
        if self.header.kind != P::KIND {
            return Err(ProtocolError::UnexpectedMessageType {
                expected: P::KIND,
                actual: self.header.kind,
            });
        }
        P::decode(&self.payload)
    }

    /// Validated frame metadata.
    #[must_use]
    pub const fn header(&self) -> FrameHeader {
        self.header
    }

    /// Payload bytes without the fixed header.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// Total number of encoded bytes.
    #[must_use]
    pub fn encoded_len(&self) -> usize {
        FRAME_HEADER_LEN + self.payload.len()
    }

    /// Serialize the frame into one contiguous byte vector.
    #[must_use]
    pub fn encode(&self) -> Vec<u8> {
        debug_assert_eq!(
            u32::try_from(self.payload.len()).ok(),
            Some(self.header.payload_len)
        );
        let mut bytes = Vec::with_capacity(self.encoded_len());
        bytes.extend_from_slice(&self.header.encode());
        bytes.extend_from_slice(&self.payload);
        bytes
    }

    /// Decode the first frame in `bytes` without treating partial input as an error.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError`] when a complete header contains an invalid
    /// marker, version, type, flag, header length, or payload length.
    pub fn decode_prefix(bytes: &[u8]) -> Result<DecodeOutcome, ProtocolError> {
        if bytes.len() < FRAME_HEADER_LEN {
            return Ok(DecodeOutcome::NeedMoreData {
                minimum: FRAME_HEADER_LEN,
            });
        }

        let header = FrameHeader::decode(&bytes[..FRAME_HEADER_LEN])?;
        let payload_len = payload_len_to_usize(header.kind, header.payload_len)?;
        let total_len = FRAME_HEADER_LEN + payload_len;
        if bytes.len() < total_len {
            return Ok(DecodeOutcome::NeedMoreData { minimum: total_len });
        }

        Ok(DecodeOutcome::Complete {
            frame: Self {
                header,
                payload: bytes[FRAME_HEADER_LEN..total_len].to_vec(),
            },
            consumed: total_len,
        })
    }
}

/// Result of parsing a possibly incomplete byte slice.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DecodeOutcome {
    /// A complete frame was decoded; remaining bytes may contain another frame.
    Complete {
        /// Validated, owned frame.
        frame: Frame,
        /// Number of bytes consumed from the input prefix.
        consumed: usize,
    },
    /// The prefix is valid so far but cannot form a complete frame yet.
    NeedMoreData {
        /// Minimum total prefix length needed for the next parsing step.
        minimum: usize,
    },
}

/// Incremental decoder for chunked USB or socket reads.
#[derive(Debug, Clone)]
pub struct FrameDecoder {
    buffer: Vec<u8>,
    buffer_limit: usize,
}

impl Default for FrameDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameDecoder {
    /// Create a decoder using the protocol's conservative default memory ceiling.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            buffer: Vec::new(),
            buffer_limit: MAX_BUFFERED_BYTES,
        }
    }

    /// Create a decoder with an explicit memory ceiling, useful for constrained links.
    #[must_use]
    pub const fn with_buffer_limit(buffer_limit: usize) -> Self {
        Self {
            buffer: Vec::new(),
            buffer_limit,
        }
    }

    /// Append one transport chunk and return every complete frame now available.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::BufferLimitExceeded`] before mutating the
    /// buffer when the new chunk would exceed its memory ceiling. It also
    /// returns any frame-validation error found in a complete header.
    pub fn push(&mut self, chunk: &[u8]) -> Result<Vec<Frame>, ProtocolError> {
        let attempted = self.buffer.len().checked_add(chunk.len()).ok_or(
            ProtocolError::BufferLimitExceeded {
                attempted: usize::MAX,
                limit: self.buffer_limit,
            },
        )?;
        if attempted > self.buffer_limit {
            return Err(ProtocolError::BufferLimitExceeded {
                attempted,
                limit: self.buffer_limit,
            });
        }

        self.buffer.extend_from_slice(chunk);
        let mut frames = Vec::new();
        let mut consumed_total = 0;

        while let DecodeOutcome::Complete { frame, consumed } =
            Frame::decode_prefix(&self.buffer[consumed_total..])?
        {
            frames.push(frame);
            consumed_total += consumed;
        }

        if consumed_total > 0 {
            self.buffer.drain(..consumed_total);
        }
        Ok(frames)
    }

    /// Bytes retained because they do not yet form a complete frame.
    #[must_use]
    pub fn buffered_len(&self) -> usize {
        self.buffer.len()
    }

    /// Discard a partial or invalid stream after the caller resets the transport.
    pub fn clear(&mut self) {
        self.buffer.clear();
    }
}

fn validate_payload_len(kind: MessageType, payload_len: usize) -> Result<(), ProtocolError> {
    let limit = kind.payload_limit();
    if payload_len > limit {
        Err(ProtocolError::PayloadTooLarge {
            kind,
            length: payload_len,
            limit,
        })
    } else {
        Ok(())
    }
}

fn payload_len_to_usize(kind: MessageType, payload_len: u32) -> Result<usize, ProtocolError> {
    usize::try_from(payload_len).map_err(|_| ProtocolError::PayloadTooLarge {
        kind,
        length: usize::MAX,
        limit: kind.payload_limit(),
    })
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_be_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_be_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}
