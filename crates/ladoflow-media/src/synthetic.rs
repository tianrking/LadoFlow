use std::{error::Error, fmt, num::NonZeroU64};

use crate::{FrameKind, FrameMetadata, MediaFrame, VideoFormat};

/// Hard ceiling for a generated synthetic payload.
pub const MAX_SYNTHETIC_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;

/// Validated settings for a deterministic synthetic stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SyntheticConfig {
    format: VideoFormat,
    payload_bytes: usize,
    keyframe_interval: NonZeroU64,
    seed: u64,
}

impl SyntheticConfig {
    /// Construct a stream with seed zero.
    ///
    /// Frame zero is a key frame; subsequent key frames are separated by
    /// `keyframe_interval` frames.
    ///
    /// # Errors
    ///
    /// Returns [`SyntheticConfigError`] for a zero or oversized payload, or a
    /// zero key-frame interval.
    pub fn new(
        format: VideoFormat,
        payload_bytes: usize,
        keyframe_interval: u64,
    ) -> Result<Self, SyntheticConfigError> {
        if payload_bytes == 0 {
            return Err(SyntheticConfigError::EmptyPayload);
        }
        if payload_bytes > MAX_SYNTHETIC_PAYLOAD_BYTES {
            return Err(SyntheticConfigError::PayloadTooLarge {
                requested: payload_bytes,
                limit: MAX_SYNTHETIC_PAYLOAD_BYTES,
            });
        }
        let keyframe_interval =
            NonZeroU64::new(keyframe_interval).ok_or(SyntheticConfigError::ZeroKeyframeInterval)?;

        Ok(Self {
            format,
            payload_bytes,
            keyframe_interval,
            seed: 0,
        })
    }

    /// Select a deterministic stream variant.
    #[must_use]
    pub const fn with_seed(mut self, seed: u64) -> Self {
        self.seed = seed;
        self
    }

    /// Codec-neutral dimensions and frame rate.
    #[must_use]
    pub const fn format(self) -> VideoFormat {
        self.format
    }

    /// Exact number of bytes generated per frame.
    #[must_use]
    pub const fn payload_bytes(self) -> usize {
        self.payload_bytes
    }

    /// Number of frames from one key frame to the next.
    #[must_use]
    pub const fn keyframe_interval(self) -> u64 {
        self.keyframe_interval.get()
    }

    /// Seed used to distinguish deterministic stream variants.
    #[must_use]
    pub const fn seed(self) -> u64 {
        self.seed
    }
}

/// Invalid synthetic stream settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntheticConfigError {
    /// An empty payload cannot exercise a media path.
    EmptyPayload,
    /// Requested payload exceeds the generation ceiling.
    PayloadTooLarge {
        /// Requested byte count.
        requested: usize,
        /// Maximum accepted byte count.
        limit: usize,
    },
    /// Key-frame interval was zero.
    ZeroKeyframeInterval,
}

impl fmt::Display for SyntheticConfigError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::EmptyPayload => formatter.write_str("synthetic payload must be non-empty"),
            Self::PayloadTooLarge { requested, limit } => {
                write!(
                    formatter,
                    "synthetic payload of {requested} bytes exceeds the {limit}-byte limit"
                )
            }
            Self::ZeroKeyframeInterval => {
                formatter.write_str("key-frame interval must be non-zero")
            }
        }
    }
}

impl Error for SyntheticConfigError {}

/// Infinite-in-practice deterministic source of opaque synthetic frames.
///
/// The iterator ends only after producing every possible `u64` sequence
/// number. It allocates exactly one configured payload per yielded frame.
#[derive(Debug, Clone)]
pub struct SyntheticFrameProducer {
    config: SyntheticConfig,
    next_sequence: Option<u64>,
}

impl SyntheticFrameProducer {
    /// Begin a synthetic stream at sequence and timestamp zero.
    #[must_use]
    pub const fn new(config: SyntheticConfig) -> Self {
        Self {
            config,
            next_sequence: Some(0),
        }
    }

    /// Settings used for every generated frame.
    #[must_use]
    pub const fn config(&self) -> SyntheticConfig {
        self.config
    }

    /// Sequence number that will be generated next, or `None` after `u64`
    /// exhaustion.
    #[must_use]
    pub const fn next_sequence(&self) -> Option<u64> {
        self.next_sequence
    }

    /// Advance the next emitted frame to `sequence` without allocating payloads.
    ///
    /// This is useful when a real-time pacer skips obsolete slots after a
    /// delayed poll. Calls never rewind the producer. The return value is the
    /// number of frames skipped.
    #[must_use]
    pub fn advance_to_sequence(&mut self, sequence: u64) -> u64 {
        let Some(current) = self.next_sequence else {
            return 0;
        };
        if sequence <= current {
            return 0;
        }

        self.next_sequence = Some(sequence);
        sequence - current
    }

    /// Generate the next frame.
    pub fn next_frame(&mut self) -> Option<MediaFrame> {
        self.next()
    }
}

impl Iterator for SyntheticFrameProducer {
    type Item = MediaFrame;

    fn next(&mut self) -> Option<Self::Item> {
        let sequence = self.next_sequence?;
        self.next_sequence = sequence.checked_add(1);

        let rate = self.config.format.frame_rate();
        let presentation_time = rate.timestamp(sequence);
        let kind = if sequence % self.config.keyframe_interval.get() == 0 {
            FrameKind::Key
        } else {
            FrameKind::Delta
        };
        let metadata = FrameMetadata::new(
            sequence,
            presentation_time,
            presentation_time,
            rate.frame_duration(sequence),
            kind,
            self.config.format,
        );
        let payload = make_payload(sequence, self.config.seed, self.config.payload_bytes);

        Some(MediaFrame::new(metadata, payload))
    }
}

fn make_payload(sequence: u64, seed: u64, payload_bytes: usize) -> Vec<u8> {
    let mut payload = vec![0_u8; payload_bytes];
    for (block_index, chunk) in payload.chunks_mut(size_of::<u64>()).enumerate() {
        let block_index =
            u64::try_from(block_index).expect("bounded synthetic payload index fits in u64");
        let input =
            seed ^ sequence.rotate_left(17) ^ block_index.wrapping_mul(0xd6e8_feb8_6659_fd93);
        let bytes = mix64(input).to_le_bytes();
        chunk.copy_from_slice(&bytes[..chunk.len()]);
    }
    payload
}

fn mix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}
