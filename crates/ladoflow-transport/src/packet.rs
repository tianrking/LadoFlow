/// Independent delivery lane used by a transport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Channel {
    /// Reliable, ordered traffic that must be retried when its queue is full.
    Control,
    /// Latency-sensitive traffic that may explicitly opt into supersession.
    Media,
}

/// Application-defined identity for replaceable media packets.
///
/// Enqueuing a replaceable media packet removes queued media packets with the
/// same key, but only if the replacement itself fits. A caller should share a
/// key only between packets for which the newest queued value makes every older
/// queued value obsolete.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SupersessionKey(u64);

impl SupersessionKey {
    /// Create an application-defined supersession key.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Return the numeric key supplied by the application.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Owned bytes and delivery metadata passed through a [`PacketTransport`](crate::PacketTransport).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Packet {
    channel: Channel,
    supersession_key: Option<SupersessionKey>,
    payload: Box<[u8]>,
}

impl Packet {
    /// Create a reliable control packet.
    #[must_use]
    pub fn control(payload: impl Into<Vec<u8>>) -> Self {
        Self::new(Channel::Control, None, payload)
    }

    /// Create a media packet that must remain queued until received.
    ///
    /// This packet is still subject to media queue limits, but a later media
    /// packet will not supersede it.
    #[must_use]
    pub fn media(payload: impl Into<Vec<u8>>) -> Self {
        Self::new(Channel::Media, None, payload)
    }

    /// Create media whose older queued value with the same key is obsolete.
    #[must_use]
    pub fn replaceable_media(
        supersession_key: SupersessionKey,
        payload: impl Into<Vec<u8>>,
    ) -> Self {
        Self::new(Channel::Media, Some(supersession_key), payload)
    }

    /// Delivery lane selected for this packet.
    #[must_use]
    pub const fn channel(&self) -> Channel {
        self.channel
    }

    /// Key used to supersede obsolete media, when replacement is allowed.
    #[must_use]
    pub const fn supersession_key(&self) -> Option<SupersessionKey> {
        self.supersession_key
    }

    /// Packet payload bytes.
    #[must_use]
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }

    /// Number of payload bytes charged against the channel's byte capacity.
    #[must_use]
    pub fn len(&self) -> usize {
        self.payload.len()
    }

    /// Whether this packet carries no payload bytes.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.payload.is_empty()
    }

    /// Consume the packet and return its payload.
    #[must_use]
    pub fn into_payload(self) -> Box<[u8]> {
        self.payload
    }

    fn new(
        channel: Channel,
        supersession_key: Option<SupersessionKey>,
        payload: impl Into<Vec<u8>>,
    ) -> Self {
        Self {
            channel,
            supersession_key,
            payload: payload.into().into_boxed_slice(),
        }
    }
}
