use std::{error::Error, fmt};

use crate::{Channel, Packet};

/// Current availability of a packet transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// Packets can be enqueued and received.
    Connected,
    /// Enqueue and receive operations fail until the transport reconnects.
    Disconnected,
}

/// Validated packet-count and byte limits for one channel queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QueueLimits {
    packet_capacity: usize,
    byte_capacity: usize,
    packet_size_limit: usize,
}

impl QueueLimits {
    /// Create queue limits with nonzero capacities.
    ///
    /// `max_packet_bytes` cannot exceed `max_queued_bytes`, because such a
    /// packet could never be enqueued even into an empty queue.
    ///
    /// # Errors
    ///
    /// Returns [`QueueLimitsError`] when any limit is zero or the per-packet
    /// byte limit is greater than the total byte capacity.
    pub const fn new(
        max_packets: usize,
        max_queued_bytes: usize,
        max_packet_bytes: usize,
    ) -> Result<Self, QueueLimitsError> {
        if max_packets == 0 {
            return Err(QueueLimitsError::ZeroPacketCapacity);
        }
        if max_queued_bytes == 0 {
            return Err(QueueLimitsError::ZeroByteCapacity);
        }
        if max_packet_bytes == 0 {
            return Err(QueueLimitsError::ZeroPacketSizeLimit);
        }
        if max_packet_bytes > max_queued_bytes {
            return Err(QueueLimitsError::PacketSizeLimitExceedsCapacity {
                max_packet_bytes,
                max_queued_bytes,
            });
        }

        Ok(Self {
            packet_capacity: max_packets,
            byte_capacity: max_queued_bytes,
            packet_size_limit: max_packet_bytes,
        })
    }

    /// Maximum number of packets retained by the queue.
    #[must_use]
    pub const fn max_packets(self) -> usize {
        self.packet_capacity
    }

    /// Maximum sum of retained payload sizes.
    #[must_use]
    pub const fn max_queued_bytes(self) -> usize {
        self.byte_capacity
    }

    /// Maximum payload size accepted for one packet.
    #[must_use]
    pub const fn max_packet_bytes(self) -> usize {
        self.packet_size_limit
    }
}

/// Invalid bounded-queue configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueueLimitsError {
    /// At least one packet slot is required.
    ZeroPacketCapacity,
    /// At least one queued byte is required.
    ZeroByteCapacity,
    /// The largest accepted packet must contain at least one byte.
    ZeroPacketSizeLimit,
    /// A single accepted packet would be larger than the entire queue.
    PacketSizeLimitExceedsCapacity {
        /// Configured single-packet limit.
        max_packet_bytes: usize,
        /// Configured queue byte capacity.
        max_queued_bytes: usize,
    },
}

impl fmt::Display for QueueLimitsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroPacketCapacity => formatter.write_str("packet capacity must be nonzero"),
            Self::ZeroByteCapacity => formatter.write_str("byte capacity must be nonzero"),
            Self::ZeroPacketSizeLimit => {
                formatter.write_str("per-packet byte limit must be nonzero")
            }
            Self::PacketSizeLimitExceedsCapacity {
                max_packet_bytes,
                max_queued_bytes,
            } => write!(
                formatter,
                "per-packet byte limit {max_packet_bytes} exceeds queue byte capacity {max_queued_bytes}"
            ),
        }
    }
}

impl Error for QueueLimitsError {}

/// Current occupancy of one or more queues.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct QueueDepth {
    packets: usize,
    bytes: usize,
}

impl QueueDepth {
    pub(crate) const fn new(packets: usize, bytes: usize) -> Self {
        Self { packets, bytes }
    }

    pub(crate) const fn saturating_add(self, other: Self) -> Self {
        Self {
            packets: self.packets.saturating_add(other.packets),
            bytes: self.bytes.saturating_add(other.bytes),
        }
    }

    /// Number of queued packets.
    #[must_use]
    pub const fn packets(self) -> usize {
        self.packets
    }

    /// Sum of queued payload sizes.
    #[must_use]
    pub const fn bytes(self) -> usize {
        self.bytes
    }

    /// Whether no packets are queued.
    #[must_use]
    pub const fn is_empty(self) -> bool {
        self.packets == 0
    }
}

/// Successful enqueue details.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct SendReport {
    superseded: QueueDepth,
}

impl SendReport {
    pub(crate) const fn new(superseded: QueueDepth) -> Self {
        Self { superseded }
    }

    /// Obsolete media removed before this packet was enqueued.
    #[must_use]
    pub const fn superseded(self) -> QueueDepth {
        self.superseded
    }
}

/// Nonblocking enqueue failure that preserves ownership of the packet.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SendError {
    /// The link is disconnected.
    Disconnected(Packet),
    /// The selected channel has no remaining packet or byte capacity.
    Full {
        /// Packet that was not enqueued and can be retried.
        packet: Packet,
        /// Queue occupancy left unchanged by the failed operation.
        depth: QueueDepth,
        /// Capacity applied to the queue.
        limits: QueueLimits,
    },
    /// This payload exceeds the configured per-packet limit.
    PacketTooLarge {
        /// Packet that was not enqueued.
        packet: Packet,
        /// Actual payload size.
        size: usize,
        /// Configured per-packet maximum.
        limit: usize,
    },
}

impl SendError {
    /// Borrow the packet that was not enqueued.
    #[must_use]
    pub const fn packet(&self) -> &Packet {
        match self {
            Self::Disconnected(packet)
            | Self::Full { packet, .. }
            | Self::PacketTooLarge { packet, .. } => packet,
        }
    }

    /// Recover ownership of the packet for retry or inspection.
    #[must_use]
    pub fn into_packet(self) -> Packet {
        match self {
            Self::Disconnected(packet)
            | Self::Full { packet, .. }
            | Self::PacketTooLarge { packet, .. } => packet,
        }
    }
}

impl fmt::Display for SendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disconnected(packet) => {
                write!(formatter, "{:?} channel is disconnected", packet.channel())
            }
            Self::Full { packet, .. } => {
                write!(formatter, "{:?} channel queue is full", packet.channel())
            }
            Self::PacketTooLarge {
                packet,
                size,
                limit,
            } => write!(
                formatter,
                "{:?} packet has {size} bytes, exceeding the {limit}-byte limit",
                packet.channel()
            ),
        }
    }
}

impl Error for SendError {}

/// Nonblocking receive failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReceiveError {
    /// The link is disconnected.
    Disconnected,
}

impl fmt::Display for ReceiveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Disconnected => formatter.write_str("transport is disconnected"),
        }
    }
}

impl Error for ReceiveError {}

/// Transport-neutral, nonblocking packet interface.
///
/// Implementations maintain independent control and media backpressure. A
/// successful receive preserves FIFO order among the packets still retained in
/// the selected channel.
pub trait PacketTransport {
    /// Report whether sends and receives are currently available.
    fn connection_state(&self) -> ConnectionState;

    /// Attempt to enqueue one packet without blocking.
    ///
    /// # Errors
    ///
    /// Returns [`SendError::Disconnected`] when unavailable,
    /// [`SendError::Full`] when the selected queue cannot retain the packet, or
    /// [`SendError::PacketTooLarge`] when it exceeds that channel's packet
    /// limit. Every error returns ownership of the unchanged packet.
    fn try_send(&mut self, packet: Packet) -> Result<SendReport, SendError>;

    /// Attempt to receive the oldest retained packet from one channel.
    ///
    /// `Ok(None)` means the connected channel is currently empty.
    ///
    /// # Errors
    ///
    /// Returns [`ReceiveError::Disconnected`] while the transport is down.
    fn try_receive(&mut self, channel: Channel) -> Result<Option<Packet>, ReceiveError>;
}
