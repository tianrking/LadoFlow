use std::{
    collections::VecDeque,
    sync::{Arc, Mutex, MutexGuard},
};

use crate::{
    Channel, ConnectionState, Packet, PacketTransport, QueueDepth, QueueLimits, ReceiveError,
    SendError, SendReport,
};

const DEFAULT_CONTROL_LIMITS: QueueLimits = valid_limits(64, 4 * 1_024 * 1_024, 64 * 1_024);
const DEFAULT_MEDIA_LIMITS: QueueLimits = valid_limits(3, 32 * 1_024 * 1_024, 16 * 1_024 * 1_024);

const fn valid_limits(
    max_packets: usize,
    max_queued_bytes: usize,
    max_packet_bytes: usize,
) -> QueueLimits {
    match QueueLimits::new(max_packets, max_queued_bytes, max_packet_bytes) {
        Ok(limits) => limits,
        Err(_) => panic!("built-in loopback queue limits must be valid"),
    }
}

/// Symmetric per-direction queue configuration for a loopback pair.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LoopbackConfig {
    control: QueueLimits,
    media: QueueLimits,
}

impl LoopbackConfig {
    /// Configure the control and media queues in each direction.
    #[must_use]
    pub const fn new(control: QueueLimits, media: QueueLimits) -> Self {
        Self { control, media }
    }

    /// Limits applied to each direction's control queue.
    #[must_use]
    pub const fn control_limits(self) -> QueueLimits {
        self.control
    }

    /// Limits applied to each direction's media queue.
    #[must_use]
    pub const fn media_limits(self) -> QueueLimits {
        self.media
    }
}

impl Default for LoopbackConfig {
    fn default() -> Self {
        Self::new(DEFAULT_CONTROL_LIMITS, DEFAULT_MEDIA_LIMITS)
    }
}

/// Queue contents discarded across both directions by a disconnect.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
#[must_use = "disconnect reports identify packets invalidated with the connection"]
pub struct DisconnectReport {
    was_connected: bool,
    control: QueueDepth,
    media: QueueDepth,
}

impl DisconnectReport {
    /// Whether this call changed the link from connected to disconnected.
    #[must_use]
    pub const fn was_connected(self) -> bool {
        self.was_connected
    }

    /// Control packets invalidated across both directions.
    #[must_use]
    pub const fn discarded_control(self) -> QueueDepth {
        self.control
    }

    /// Media packets invalidated across both directions.
    #[must_use]
    pub const fn discarded_media(self) -> QueueDepth {
        self.media
    }
}

/// One side of a deterministic in-memory full-duplex link.
///
/// Both endpoints share connection state. Calling [`Self::disconnect`] from
/// either side atomically rejects new operations and clears every queued
/// packet in both directions. [`Self::reconnect`] starts a clean connection
/// using the same endpoint values.
#[derive(Debug)]
pub struct LoopbackEndpoint {
    side: Side,
    link: Arc<Mutex<Link>>,
}

impl LoopbackEndpoint {
    /// Disconnect both endpoints and invalidate all in-flight packets.
    pub fn disconnect(&self) -> DisconnectReport {
        let mut link = self.lock_link();
        let was_connected = link.connected;
        link.connected = false;
        let discarded = link.clear();
        DisconnectReport {
            was_connected,
            control: discarded.control,
            media: discarded.media,
        }
    }

    /// Reconnect both endpoints with empty queues.
    ///
    /// Returns `true` only when this call changed a disconnected link back to
    /// connected.
    #[must_use]
    pub fn reconnect(&self) -> bool {
        let mut link = self.lock_link();
        if link.connected {
            false
        } else {
            let _ = link.clear();
            link.connected = true;
            true
        }
    }

    fn lock_link(&self) -> MutexGuard<'_, Link> {
        self.link
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl PacketTransport for LoopbackEndpoint {
    fn connection_state(&self) -> ConnectionState {
        if self.lock_link().connected {
            ConnectionState::Connected
        } else {
            ConnectionState::Disconnected
        }
    }

    fn try_send(&mut self, packet: Packet) -> Result<SendReport, SendError> {
        let mut link = self.lock_link();
        if !link.connected {
            return Err(SendError::Disconnected(packet));
        }

        link.outgoing_mut(self.side).try_push(packet)
    }

    fn try_receive(&mut self, channel: Channel) -> Result<Option<Packet>, ReceiveError> {
        let mut link = self.lock_link();
        if !link.connected {
            return Err(ReceiveError::Disconnected);
        }

        Ok(link.incoming_mut(self.side).pop(channel))
    }
}

/// Create two connected endpoints backed only by bounded memory queues.
#[must_use]
pub fn loopback_pair(config: LoopbackConfig) -> (LoopbackEndpoint, LoopbackEndpoint) {
    let link = Arc::new(Mutex::new(Link::new(config)));
    let first = LoopbackEndpoint {
        side: Side::First,
        link: Arc::clone(&link),
    };
    let second = LoopbackEndpoint {
        side: Side::Second,
        link,
    };
    (first, second)
}

#[derive(Debug, Clone, Copy)]
enum Side {
    First,
    Second,
}

#[derive(Debug)]
struct Link {
    connected: bool,
    first_to_second: Direction,
    second_to_first: Direction,
}

impl Link {
    fn new(config: LoopbackConfig) -> Self {
        Self {
            connected: true,
            first_to_second: Direction::new(config),
            second_to_first: Direction::new(config),
        }
    }

    fn outgoing_mut(&mut self, side: Side) -> &mut Direction {
        match side {
            Side::First => &mut self.first_to_second,
            Side::Second => &mut self.second_to_first,
        }
    }

    fn incoming_mut(&mut self, side: Side) -> &mut Direction {
        match side {
            Side::First => &mut self.second_to_first,
            Side::Second => &mut self.first_to_second,
        }
    }

    fn clear(&mut self) -> ClearedQueues {
        self.first_to_second
            .clear()
            .saturating_add(self.second_to_first.clear())
    }
}

#[derive(Debug)]
struct Direction {
    control: BoundedQueue,
    media: BoundedQueue,
}

impl Direction {
    fn new(config: LoopbackConfig) -> Self {
        Self {
            control: BoundedQueue::new(config.control),
            media: BoundedQueue::new(config.media),
        }
    }

    fn try_push(&mut self, packet: Packet) -> Result<SendReport, SendError> {
        match packet.channel() {
            Channel::Control => self.control.try_push(packet),
            Channel::Media => self.media.try_push(packet),
        }
    }

    fn pop(&mut self, channel: Channel) -> Option<Packet> {
        match channel {
            Channel::Control => self.control.pop(),
            Channel::Media => self.media.pop(),
        }
    }

    fn clear(&mut self) -> ClearedQueues {
        ClearedQueues {
            control: self.control.clear(),
            media: self.media.clear(),
        }
    }
}

#[derive(Debug)]
struct BoundedQueue {
    limits: QueueLimits,
    packets: VecDeque<Packet>,
    queued_bytes: usize,
}

impl BoundedQueue {
    fn new(limits: QueueLimits) -> Self {
        Self {
            limits,
            packets: VecDeque::new(),
            queued_bytes: 0,
        }
    }

    fn try_push(&mut self, packet: Packet) -> Result<SendReport, SendError> {
        let packet_size = packet.len();
        if packet_size > self.limits.max_packet_bytes() {
            return Err(SendError::PacketTooLarge {
                packet,
                size: packet_size,
                limit: self.limits.max_packet_bytes(),
            });
        }

        let superseded = self.superseded_by(&packet);
        let retained_packets = self.packets.len() - superseded.packets();
        let retained_bytes = self.queued_bytes - superseded.bytes();
        let packet_slots_full = retained_packets >= self.limits.max_packets();
        let byte_capacity_full = packet_size > self.limits.max_queued_bytes() - retained_bytes;

        if packet_slots_full || byte_capacity_full {
            return Err(SendError::Full {
                packet,
                depth: self.depth(),
                limits: self.limits,
            });
        }

        if let Some(key) = packet.supersession_key() {
            self.packets
                .retain(|queued| queued.supersession_key() != Some(key));
        }
        self.queued_bytes = retained_bytes + packet_size;
        self.packets.push_back(packet);
        Ok(SendReport::new(superseded))
    }

    fn pop(&mut self) -> Option<Packet> {
        let packet = self.packets.pop_front()?;
        self.queued_bytes -= packet.len();
        Some(packet)
    }

    fn superseded_by(&self, packet: &Packet) -> QueueDepth {
        let Some(key) = packet.supersession_key() else {
            return QueueDepth::default();
        };

        let mut packets = 0_usize;
        let mut bytes = 0_usize;
        for queued in &self.packets {
            if queued.supersession_key() == Some(key) {
                packets = packets.saturating_add(1);
                bytes = bytes.saturating_add(queued.len());
            }
        }
        QueueDepth::new(packets, bytes)
    }

    fn depth(&self) -> QueueDepth {
        QueueDepth::new(self.packets.len(), self.queued_bytes)
    }

    fn clear(&mut self) -> QueueDepth {
        let depth = self.depth();
        self.packets.clear();
        self.queued_bytes = 0;
        depth
    }
}

#[derive(Debug, Default, Clone, Copy)]
struct ClearedQueues {
    control: QueueDepth,
    media: QueueDepth,
}

impl ClearedQueues {
    fn saturating_add(self, other: Self) -> Self {
        Self {
            control: self.control.saturating_add(other.control),
            media: self.media.saturating_add(other.media),
        }
    }
}
