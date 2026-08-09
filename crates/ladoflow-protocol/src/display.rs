use crate::{CodecSet, MAX_MEDIA_PAYLOAD, MessageType, ProtocolError, WirePayload};

const DISPLAY_CONFIG_LEN: usize = 14;

/// Number of metadata bytes at the beginning of a [`VideoFrame`] payload.
pub const VIDEO_FRAME_METADATA_LEN: usize = 28;

/// Largest encoded access unit that fits in one version-one media frame.
pub const MAX_ENCODED_VIDEO_BYTES: usize = MAX_MEDIA_PAYLOAD - VIDEO_FRAME_METADATA_LEN;

/// Encoded video format selected for a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum VideoCodec {
    /// H.264/AVC.
    H264 = 1,
    /// H.265/HEVC.
    Hevc = 2,
    /// AV1.
    Av1 = 3,
}

impl VideoCodec {
    /// Capability bit corresponding to this codec.
    #[must_use]
    pub const fn capability(self) -> CodecSet {
        match self {
            Self::H264 => CodecSet::H264,
            Self::Hevc => CodecSet::HEVC,
            Self::Av1 => CodecSet::AV1,
        }
    }
}

impl TryFrom<u8> for VideoCodec {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::H264),
            2 => Ok(Self::Hevc),
            3 => Ok(Self::Av1),
            _ => Err(ProtocolError::InvalidPayload("unknown video codec")),
        }
    }
}

/// Version-one codec profiles with portable hardware-decoder mappings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum CodecProfile {
    /// H.264 constrained baseline profile.
    H264Baseline = 1,
    /// H.264 main profile.
    H264Main = 2,
    /// H.264 high profile.
    H264High = 3,
    /// HEVC main profile with 8-bit samples.
    HevcMain = 16,
    /// HEVC main 10 profile.
    HevcMain10 = 17,
    /// AV1 main profile.
    Av1Main = 32,
}

impl CodecProfile {
    /// Codec family to which the profile belongs.
    #[must_use]
    pub const fn codec(self) -> VideoCodec {
        match self {
            Self::H264Baseline | Self::H264Main | Self::H264High => VideoCodec::H264,
            Self::HevcMain | Self::HevcMain10 => VideoCodec::Hevc,
            Self::Av1Main => VideoCodec::Av1,
        }
    }
}

impl TryFrom<u8> for CodecProfile {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::H264Baseline),
            2 => Ok(Self::H264Main),
            3 => Ok(Self::H264High),
            16 => Ok(Self::HevcMain),
            17 => Ok(Self::HevcMain10),
            32 => Ok(Self::Av1Main),
            _ => Err(ProtocolError::InvalidPayload("unknown codec profile")),
        }
    }
}

/// Negotiated display and encoder configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DisplayConfig {
    width: u16,
    height: u16,
    refresh_millihz: u32,
    bitrate_kbps: u32,
    codec: VideoCodec,
    profile: CodecProfile,
}

impl DisplayConfig {
    /// Construct a validated display configuration.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidPayload`] when a dimension, refresh
    /// rate, or bitrate is zero, or when `profile` does not belong to `codec`.
    pub fn new(
        width: u16,
        height: u16,
        refresh_millihz: u32,
        bitrate_kbps: u32,
        codec: VideoCodec,
        profile: CodecProfile,
    ) -> Result<Self, ProtocolError> {
        if width == 0 || height == 0 {
            return Err(ProtocolError::InvalidPayload(
                "display dimensions must be non-zero",
            ));
        }
        if refresh_millihz == 0 {
            return Err(ProtocolError::InvalidPayload(
                "display refresh rate must be non-zero",
            ));
        }
        if bitrate_kbps == 0 {
            return Err(ProtocolError::InvalidPayload(
                "display bitrate must be non-zero",
            ));
        }
        if profile.codec() != codec {
            return Err(ProtocolError::InvalidPayload(
                "codec profile does not belong to selected codec",
            ));
        }

        Ok(Self {
            width,
            height,
            refresh_millihz,
            bitrate_kbps,
            codec,
            profile,
        })
    }

    /// Negotiated coded width in pixels.
    #[must_use]
    pub const fn width(self) -> u16 {
        self.width
    }

    /// Negotiated coded height in pixels.
    #[must_use]
    pub const fn height(self) -> u16 {
        self.height
    }

    /// Negotiated refresh rate in thousandths of a hertz.
    #[must_use]
    pub const fn refresh_millihz(self) -> u32 {
        self.refresh_millihz
    }

    /// Target encoder bitrate in kilobits per second.
    #[must_use]
    pub const fn bitrate_kbps(self) -> u32 {
        self.bitrate_kbps
    }

    /// Selected encoded video format.
    #[must_use]
    pub const fn codec(self) -> VideoCodec {
        self.codec
    }

    /// Selected codec profile.
    #[must_use]
    pub const fn profile(self) -> CodecProfile {
        self.profile
    }
}

impl WirePayload for DisplayConfig {
    const KIND: MessageType = MessageType::DisplayConfig;

    fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        Self::new(
            self.width,
            self.height,
            self.refresh_millihz,
            self.bitrate_kbps,
            self.codec,
            self.profile,
        )?;

        let mut payload = Vec::with_capacity(DISPLAY_CONFIG_LEN);
        payload.extend_from_slice(&self.width.to_be_bytes());
        payload.extend_from_slice(&self.height.to_be_bytes());
        payload.extend_from_slice(&self.refresh_millihz.to_be_bytes());
        payload.extend_from_slice(&self.bitrate_kbps.to_be_bytes());
        payload.push(self.codec as u8);
        payload.push(self.profile as u8);
        Ok(payload)
    }

    fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        if payload.len() != DISPLAY_CONFIG_LEN {
            return Err(ProtocolError::InvalidPayload(
                "display-config payload must be exactly 14 bytes",
            ));
        }

        Self::new(
            read_u16(payload, 0),
            read_u16(payload, 2),
            read_u32(payload, 4),
            read_u32(payload, 8),
            VideoCodec::try_from(payload[12])?,
            CodecProfile::try_from(payload[13])?,
        )
    }
}

/// Fixed-size presentation metadata preceding encoded video bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VideoFrameMetadata {
    frame_id: u64,
    capture_timestamp_micros: u64,
    presentation_timestamp_micros: u64,
    duration_micros: u32,
}

impl VideoFrameMetadata {
    /// Construct frame metadata.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidPayload`] when `duration_micros` is
    /// zero. Timestamps are opaque monotonic-clock values and may start at
    /// zero.
    pub fn new(
        frame_id: u64,
        capture_timestamp_micros: u64,
        presentation_timestamp_micros: u64,
        duration_micros: u32,
    ) -> Result<Self, ProtocolError> {
        if duration_micros == 0 {
            return Err(ProtocolError::InvalidPayload(
                "video frame duration must be non-zero",
            ));
        }
        Ok(Self {
            frame_id,
            capture_timestamp_micros,
            presentation_timestamp_micros,
            duration_micros,
        })
    }

    /// Sender-assigned frame identity.
    #[must_use]
    pub const fn frame_id(self) -> u64 {
        self.frame_id
    }

    /// Monotonic timestamp at which capture completed, in microseconds.
    #[must_use]
    pub const fn capture_timestamp_micros(self) -> u64 {
        self.capture_timestamp_micros
    }

    /// Target presentation timestamp, in the sender's monotonic clock domain.
    #[must_use]
    pub const fn presentation_timestamp_micros(self) -> u64 {
        self.presentation_timestamp_micros
    }

    /// Intended on-screen duration in microseconds.
    #[must_use]
    pub const fn duration_micros(self) -> u32 {
        self.duration_micros
    }

    fn encode_into(self, payload: &mut Vec<u8>) {
        payload.extend_from_slice(&self.frame_id.to_be_bytes());
        payload.extend_from_slice(&self.capture_timestamp_micros.to_be_bytes());
        payload.extend_from_slice(&self.presentation_timestamp_micros.to_be_bytes());
        payload.extend_from_slice(&self.duration_micros.to_be_bytes());
    }

    fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        Self::new(
            read_u64(payload, 0),
            read_u64(payload, 8),
            read_u64(payload, 16),
            read_u32(payload, 24),
        )
    }
}

/// One encoded video access unit and its presentation metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFrame {
    metadata: VideoFrameMetadata,
    encoded_bytes: Vec<u8>,
}

impl VideoFrame {
    /// Construct a bounded encoded video frame.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidPayload`] for an empty access unit and
    /// [`ProtocolError::PayloadTooLarge`] when the metadata and encoded bytes
    /// exceed the media-frame limit.
    pub fn new(
        metadata: VideoFrameMetadata,
        encoded_bytes: impl Into<Vec<u8>>,
    ) -> Result<Self, ProtocolError> {
        let encoded_bytes = encoded_bytes.into();
        validate_encoded_bytes(&encoded_bytes)?;
        Ok(Self {
            metadata,
            encoded_bytes,
        })
    }

    /// Presentation metadata for this access unit.
    #[must_use]
    pub const fn metadata(&self) -> VideoFrameMetadata {
        self.metadata
    }

    /// Codec-produced bytes, without a transport-specific length prefix.
    #[must_use]
    pub fn encoded_bytes(&self) -> &[u8] {
        &self.encoded_bytes
    }

    /// Consume the payload and return its codec-produced bytes.
    #[must_use]
    pub fn into_encoded_bytes(self) -> Vec<u8> {
        self.encoded_bytes
    }
}

impl WirePayload for VideoFrame {
    const KIND: MessageType = MessageType::VideoFrame;

    fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        validate_encoded_bytes(&self.encoded_bytes)?;
        let mut payload = Vec::with_capacity(VIDEO_FRAME_METADATA_LEN + self.encoded_bytes.len());
        self.metadata.encode_into(&mut payload);
        payload.extend_from_slice(&self.encoded_bytes);
        Ok(payload)
    }

    fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        if payload.len() > MAX_MEDIA_PAYLOAD {
            return Err(ProtocolError::PayloadTooLarge {
                kind: MessageType::VideoFrame,
                length: payload.len(),
                limit: MAX_MEDIA_PAYLOAD,
            });
        }
        if payload.len() <= VIDEO_FRAME_METADATA_LEN {
            return Err(ProtocolError::InvalidPayload(
                "video-frame payload is missing metadata or encoded bytes",
            ));
        }

        Self::new(
            VideoFrameMetadata::decode(&payload[..VIDEO_FRAME_METADATA_LEN])?,
            payload[VIDEO_FRAME_METADATA_LEN..].to_vec(),
        )
    }
}

fn validate_encoded_bytes(encoded_bytes: &[u8]) -> Result<(), ProtocolError> {
    if encoded_bytes.is_empty() {
        return Err(ProtocolError::InvalidPayload(
            "encoded video access unit must not be empty",
        ));
    }
    if encoded_bytes.len() > MAX_ENCODED_VIDEO_BYTES {
        return Err(ProtocolError::PayloadTooLarge {
            kind: MessageType::VideoFrame,
            length: VIDEO_FRAME_METADATA_LEN.saturating_add(encoded_bytes.len()),
            limit: MAX_MEDIA_PAYLOAD,
        });
    }
    Ok(())
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
