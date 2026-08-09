use std::{error::Error, fmt, num::NonZeroU32, time::Duration};

const NANOS_PER_SECOND: u128 = 1_000_000_000;

/// A positive rational number of frames per second.
///
/// Rational rates avoid cumulative rounding drift for rates such as 60 Hz or
/// 30,000/1,001 Hz. Frame timestamps are always calculated from the frame
/// index, never by repeatedly adding a rounded period.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameRate {
    numerator: NonZeroU32,
    denominator: NonZeroU32,
}

impl FrameRate {
    /// Construct `numerator / denominator` frames per second.
    ///
    /// # Errors
    ///
    /// Returns [`FrameRateError`] when either component is zero.
    pub fn new(numerator: u32, denominator: u32) -> Result<Self, FrameRateError> {
        let numerator = NonZeroU32::new(numerator).ok_or(FrameRateError::ZeroNumerator)?;
        let denominator = NonZeroU32::new(denominator).ok_or(FrameRateError::ZeroDenominator)?;
        Ok(Self {
            numerator,
            denominator,
        })
    }

    /// Construct an integer frame rate.
    ///
    /// # Errors
    ///
    /// Returns [`FrameRateError::ZeroNumerator`] when `frames_per_second` is
    /// zero.
    pub fn from_hz(frames_per_second: u32) -> Result<Self, FrameRateError> {
        Self::new(frames_per_second, 1)
    }

    /// Return the numerator of the frames-per-second ratio.
    #[must_use]
    pub const fn numerator(self) -> u32 {
        self.numerator.get()
    }

    /// Return the denominator of the frames-per-second ratio.
    #[must_use]
    pub const fn denominator(self) -> u32 {
        self.denominator.get()
    }

    /// Calculate the presentation timestamp for a zero-based frame index.
    ///
    /// The result is rounded down to the nearest nanosecond. It saturates only
    /// when the mathematical timestamp is larger than [`Duration::MAX`].
    #[must_use]
    pub fn timestamp(self, frame_index: u64) -> Duration {
        let scaled_seconds = u128::from(frame_index) * u128::from(self.denominator.get());
        let rate_numerator = u128::from(self.numerator.get());
        let whole_seconds = scaled_seconds / rate_numerator;
        let fractional_units = scaled_seconds % rate_numerator;

        let Ok(whole_seconds) = u64::try_from(whole_seconds) else {
            return Duration::MAX;
        };
        let nanos = fractional_units * NANOS_PER_SECOND / rate_numerator;
        let Ok(nanos) = u32::try_from(nanos) else {
            return Duration::MAX;
        };
        Duration::new(whole_seconds, nanos)
    }

    /// Calculate the duration of one indexed frame.
    ///
    /// Neighboring durations can differ by one nanosecond because each absolute
    /// deadline is rounded independently. Their accumulated timeline does not
    /// drift.
    #[must_use]
    pub fn frame_duration(self, frame_index: u64) -> Duration {
        let next_index = frame_index.saturating_add(1);
        self.timestamp(next_index)
            .saturating_sub(self.timestamp(frame_index))
    }

    /// Find the latest frame whose nanosecond-rounded timestamp is not after
    /// `elapsed`.
    #[must_use]
    pub fn frame_at_or_before(self, elapsed: Duration) -> u64 {
        // timestamp(i) = floor(i * denominator * 1e9 / numerator). Inverting
        // that floor requires treating the next nanosecond as an exclusive
        // upper bound, otherwise a 60 Hz deadline at 16,666,666 ns would be
        // incorrectly assigned to frame zero.
        let exclusive_nanos = elapsed.as_nanos() + 1;
        let scaled = exclusive_nanos * u128::from(self.numerator.get()) - 1;
        let divisor = u128::from(self.denominator.get()) * NANOS_PER_SECOND;
        u64::try_from(scaled / divisor).unwrap_or(u64::MAX)
    }
}

/// Invalid rational frame-rate components.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameRateError {
    /// Frames-per-second numerator was zero.
    ZeroNumerator,
    /// Frames-per-second denominator was zero.
    ZeroDenominator,
}

impl fmt::Display for FrameRateError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroNumerator => formatter.write_str("frame-rate numerator must be non-zero"),
            Self::ZeroDenominator => formatter.write_str("frame-rate denominator must be non-zero"),
        }
    }
}

impl Error for FrameRateError {}

/// Positive video dimensions in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameDimensions {
    width: NonZeroU32,
    height: NonZeroU32,
}

impl FrameDimensions {
    /// Construct validated pixel dimensions.
    ///
    /// # Errors
    ///
    /// Returns [`FrameDimensionsError`] when either dimension is zero.
    pub fn new(width: u32, height: u32) -> Result<Self, FrameDimensionsError> {
        let width = NonZeroU32::new(width).ok_or(FrameDimensionsError::ZeroWidth)?;
        let height = NonZeroU32::new(height).ok_or(FrameDimensionsError::ZeroHeight)?;
        Ok(Self { width, height })
    }

    /// Width in pixels.
    #[must_use]
    pub const fn width(self) -> u32 {
        self.width.get()
    }

    /// Height in pixels.
    #[must_use]
    pub const fn height(self) -> u32 {
        self.height.get()
    }

    /// Total number of pixels without platform-sized integer overflow.
    #[must_use]
    pub fn pixel_count(self) -> u64 {
        u64::from(self.width.get()) * u64::from(self.height.get())
    }
}

/// Invalid video dimensions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameDimensionsError {
    /// Width was zero.
    ZeroWidth,
    /// Height was zero.
    ZeroHeight,
}

impl fmt::Display for FrameDimensionsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroWidth => formatter.write_str("frame width must be non-zero"),
            Self::ZeroHeight => formatter.write_str("frame height must be non-zero"),
        }
    }
}

impl Error for FrameDimensionsError {}

/// Stable stream properties that do not name a codec or platform API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct VideoFormat {
    dimensions: FrameDimensions,
    frame_rate: FrameRate,
}

impl VideoFormat {
    /// Construct a codec-neutral video format.
    #[must_use]
    pub const fn new(dimensions: FrameDimensions, frame_rate: FrameRate) -> Self {
        Self {
            dimensions,
            frame_rate,
        }
    }

    /// Pixel dimensions of each frame.
    #[must_use]
    pub const fn dimensions(self) -> FrameDimensions {
        self.dimensions
    }

    /// Intended presentation rate.
    #[must_use]
    pub const fn frame_rate(self) -> FrameRate {
        self.frame_rate
    }
}

/// Decoder-independent frame dependency marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameKind {
    /// Independently decodable synchronization frame.
    Key,
    /// Frame that can depend on earlier stream state.
    Delta,
}

/// Immutable identity and timing data attached to a media payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FrameMetadata {
    sequence: u64,
    capture_time: Duration,
    presentation_time: Duration,
    duration: Duration,
    kind: FrameKind,
    format: VideoFormat,
}

impl FrameMetadata {
    /// Construct metadata whose timestamps are relative to a stream origin.
    #[must_use]
    pub const fn new(
        sequence: u64,
        capture_time: Duration,
        presentation_time: Duration,
        duration: Duration,
        kind: FrameKind,
        format: VideoFormat,
    ) -> Self {
        Self {
            sequence,
            capture_time,
            presentation_time,
            duration,
            kind,
            format,
        }
    }

    /// Monotonic identity assigned by the producer.
    #[must_use]
    pub const fn sequence(self) -> u64 {
        self.sequence
    }

    /// Capture timestamp relative to the stream origin.
    #[must_use]
    pub const fn capture_time(self) -> Duration {
        self.capture_time
    }

    /// Intended presentation timestamp relative to the stream origin.
    #[must_use]
    pub const fn presentation_time(self) -> Duration {
        self.presentation_time
    }

    /// Intended duration on the media timeline.
    #[must_use]
    pub const fn duration(self) -> Duration {
        self.duration
    }

    /// Whether this is a key frame or a dependent delta frame.
    #[must_use]
    pub const fn kind(self) -> FrameKind {
        self.kind
    }

    /// Stream format associated with this frame.
    #[must_use]
    pub const fn format(self) -> VideoFormat {
        self.format
    }
}

/// Owned opaque media bytes with codec-neutral metadata.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MediaFrame {
    metadata: FrameMetadata,
    payload: Vec<u8>,
}

impl MediaFrame {
    /// Attach opaque bytes to validated metadata.
    #[must_use]
    pub fn new(metadata: FrameMetadata, payload: impl Into<Vec<u8>>) -> Self {
        Self {
            metadata,
            payload: payload.into(),
        }
    }

    /// Frame identity, format, and timeline values.
    #[must_use]
    pub const fn metadata(&self) -> FrameMetadata {
        self.metadata
    }

    /// Opaque media bytes; their interpretation belongs to the negotiated
    /// capture/codec adapter.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// Number of opaque payload bytes.
    #[must_use]
    pub fn payload_len(&self) -> usize {
        self.payload.len()
    }

    /// Whether the opaque payload is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.payload.is_empty()
    }

    /// Split the frame into metadata and owned payload bytes.
    #[must_use]
    pub fn into_parts(self) -> (FrameMetadata, Vec<u8>) {
        (self.metadata, self.payload)
    }
}
