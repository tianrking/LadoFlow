use crate::{MessageType, ProtocolError, WirePayload};

const TELEMETRY_LEN: usize = 51;

/// Largest individual pipeline-stage duration accepted by version one.
pub const MAX_STAGE_DURATION_MICROS: u32 = 60_000_000;

/// Largest reported queue depth accepted by version one.
pub const MAX_TELEMETRY_QUEUE_DEPTH: u16 = 4_096;

/// One million parts per million, representing complete loss.
pub const MAX_LOSS_PARTS_PER_MILLION: u32 = 1_000_000;

/// Coarse device thermal pressure suitable for cross-platform policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum ThermalState {
    /// The platform does not expose a thermal state.
    Unknown = 0,
    /// No meaningful thermal pressure.
    Nominal = 1,
    /// Mild thermal pressure; quality reduction may help.
    Fair = 2,
    /// Sustained operation is likely to throttle.
    Serious = 3,
    /// Immediate load reduction is required.
    Critical = 4,
}

impl TryFrom<u8> for ThermalState {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0 => Ok(Self::Unknown),
            1 => Ok(Self::Nominal),
            2 => Ok(Self::Fair),
            3 => Ok(Self::Serious),
            4 => Ok(Self::Critical),
            _ => Err(ProtocolError::InvalidPayload("unknown thermal state")),
        }
    }
}

/// Per-frame pipeline durations in microseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StageTimings {
    capture: u32,
    encode: u32,
    transport: u32,
    decode: u32,
    presentation: u32,
}

impl StageTimings {
    /// Construct bounded pipeline timings.
    ///
    /// A zero duration means that the stage was not measured. This lets an
    /// endpoint report the stages visible to it without inventing data.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidPayload`] when any duration exceeds
    /// [`MAX_STAGE_DURATION_MICROS`].
    pub fn new(
        capture_micros: u32,
        encode_micros: u32,
        transport_micros: u32,
        decode_micros: u32,
        presentation_micros: u32,
    ) -> Result<Self, ProtocolError> {
        let timings = Self {
            capture: capture_micros,
            encode: encode_micros,
            transport: transport_micros,
            decode: decode_micros,
            presentation: presentation_micros,
        };
        timings.validate()?;
        Ok(timings)
    }

    /// Capture-stage duration.
    #[must_use]
    pub const fn capture_micros(self) -> u32 {
        self.capture
    }

    /// Encode-stage duration.
    #[must_use]
    pub const fn encode_micros(self) -> u32 {
        self.encode
    }

    /// Time from transport enqueue through transport dequeue.
    #[must_use]
    pub const fn transport_micros(self) -> u32 {
        self.transport
    }

    /// Decode-stage duration.
    #[must_use]
    pub const fn decode_micros(self) -> u32 {
        self.decode
    }

    /// Time from decoded output becoming available through presentation.
    #[must_use]
    pub const fn presentation_micros(self) -> u32 {
        self.presentation
    }

    fn validate(self) -> Result<(), ProtocolError> {
        let durations = [
            self.capture,
            self.encode,
            self.transport,
            self.decode,
            self.presentation,
        ];
        if durations
            .iter()
            .any(|duration| *duration > MAX_STAGE_DURATION_MICROS)
        {
            Err(ProtocolError::InvalidPayload(
                "telemetry stage duration exceeds version-one limit",
            ))
        } else {
            Ok(())
        }
    }

    fn encode_into(self, payload: &mut Vec<u8>) {
        payload.extend_from_slice(&self.capture.to_be_bytes());
        payload.extend_from_slice(&self.encode.to_be_bytes());
        payload.extend_from_slice(&self.transport.to_be_bytes());
        payload.extend_from_slice(&self.decode.to_be_bytes());
        payload.extend_from_slice(&self.presentation.to_be_bytes());
    }
}

/// Bounded latency, pacing, loss, and device-health sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Telemetry {
    sample_timestamp_micros: u64,
    frame_id: u64,
    timings: StageTimings,
    queue_depth: u16,
    loss_parts_per_million: u32,
    dropped_frames: u32,
    late_frames: u32,
    thermal_state: ThermalState,
}

impl Telemetry {
    /// Construct a validated telemetry sample.
    ///
    /// Counters are cumulative within a session. `frame_id` may be zero when a
    /// sample does not correspond to one frame.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidPayload`] when a stage duration, queue
    /// depth, or loss ratio exceeds its version-one bound.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        sample_timestamp_micros: u64,
        frame_id: u64,
        timings: StageTimings,
        queue_depth: u16,
        loss_parts_per_million: u32,
        dropped_frames: u32,
        late_frames: u32,
        thermal_state: ThermalState,
    ) -> Result<Self, ProtocolError> {
        timings.validate()?;
        if queue_depth > MAX_TELEMETRY_QUEUE_DEPTH {
            return Err(ProtocolError::InvalidPayload(
                "telemetry queue depth exceeds version-one limit",
            ));
        }
        if loss_parts_per_million > MAX_LOSS_PARTS_PER_MILLION {
            return Err(ProtocolError::InvalidPayload(
                "telemetry loss ratio exceeds one million parts per million",
            ));
        }

        Ok(Self {
            sample_timestamp_micros,
            frame_id,
            timings,
            queue_depth,
            loss_parts_per_million,
            dropped_frames,
            late_frames,
            thermal_state,
        })
    }

    /// Time at which this sample was emitted, in the sender's clock domain.
    #[must_use]
    pub const fn sample_timestamp_micros(self) -> u64 {
        self.sample_timestamp_micros
    }

    /// Frame associated with the pipeline timings, or zero if none.
    #[must_use]
    pub const fn frame_id(self) -> u64 {
        self.frame_id
    }

    /// Per-stage frame-processing durations.
    #[must_use]
    pub const fn timings(self) -> StageTimings {
        self.timings
    }

    /// Number of frames waiting at the measured queue.
    #[must_use]
    pub const fn queue_depth(self) -> u16 {
        self.queue_depth
    }

    /// Transport loss ratio in parts per million.
    #[must_use]
    pub const fn loss_parts_per_million(self) -> u32 {
        self.loss_parts_per_million
    }

    /// Session-cumulative frames dropped before presentation.
    #[must_use]
    pub const fn dropped_frames(self) -> u32 {
        self.dropped_frames
    }

    /// Session-cumulative frames presented after their deadline.
    #[must_use]
    pub const fn late_frames(self) -> u32 {
        self.late_frames
    }

    /// Coarse thermal pressure reported by the endpoint.
    #[must_use]
    pub const fn thermal_state(self) -> ThermalState {
        self.thermal_state
    }
}

impl WirePayload for Telemetry {
    const KIND: MessageType = MessageType::Telemetry;

    fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        Self::new(
            self.sample_timestamp_micros,
            self.frame_id,
            self.timings,
            self.queue_depth,
            self.loss_parts_per_million,
            self.dropped_frames,
            self.late_frames,
            self.thermal_state,
        )?;

        let mut payload = Vec::with_capacity(TELEMETRY_LEN);
        payload.extend_from_slice(&self.sample_timestamp_micros.to_be_bytes());
        payload.extend_from_slice(&self.frame_id.to_be_bytes());
        self.timings.encode_into(&mut payload);
        payload.extend_from_slice(&self.queue_depth.to_be_bytes());
        payload.extend_from_slice(&self.loss_parts_per_million.to_be_bytes());
        payload.extend_from_slice(&self.dropped_frames.to_be_bytes());
        payload.extend_from_slice(&self.late_frames.to_be_bytes());
        payload.push(self.thermal_state as u8);
        Ok(payload)
    }

    fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        if payload.len() != TELEMETRY_LEN {
            return Err(ProtocolError::InvalidPayload(
                "telemetry payload must be exactly 51 bytes",
            ));
        }

        Self::new(
            read_u64(payload, 0),
            read_u64(payload, 8),
            StageTimings::new(
                read_u32(payload, 16),
                read_u32(payload, 20),
                read_u32(payload, 24),
                read_u32(payload, 28),
                read_u32(payload, 32),
            )?,
            read_u16(payload, 36),
            read_u32(payload, 38),
            read_u32(payload, 42),
            read_u32(payload, 46),
            ThermalState::try_from(payload[50])?,
        )
    }
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
