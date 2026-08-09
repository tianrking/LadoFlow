use std::{error::Error, fmt};

use ladoflow_protocol::{DecodeOutcome, Frame as WireFrame, FrameDecoder, MessageType};

use crate::{Channel, Packet, PacketTransport};

/// Invalid LDFL framing or queue state at a byte-stream transport boundary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LdflStreamError {
    /// The incremental LDFL decoder rejected inbound bytes.
    Decode(String),
    /// An outbound packet did not contain one complete LDFL frame yet.
    PartialPacket,
    /// An outbound packet contained bytes after its one LDFL frame.
    TrailingBytes,
    /// An outbound frame was placed in the wrong delivery lane.
    WrongChannel {
        /// Message family decoded from the LDFL header.
        kind: MessageType,
        /// Queue in which the packet was found.
        actual: Channel,
        /// Queue required by the message family.
        expected: Channel,
    },
    /// Control and media queues contained different frames with one sequence.
    DuplicateSequence(u64),
    /// The underlying bounded queue disconnected while it was being drained.
    QueueDisconnected(Channel),
}

impl fmt::Display for LdflStreamError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Decode(detail) => write!(formatter, "invalid LDFL byte stream: {detail}"),
            Self::PartialPacket => {
                formatter.write_str("outbound packet contains a partial LDFL frame")
            }
            Self::TrailingBytes => {
                formatter.write_str("outbound LDFL packet contains trailing bytes")
            }
            Self::WrongChannel {
                kind,
                actual,
                expected,
            } => write!(
                formatter,
                "outbound {kind:?} frame is queued on {actual:?}, expected {expected:?}"
            ),
            Self::DuplicateSequence(sequence) => write!(
                formatter,
                "control and media queues contain duplicate LDFL sequence {sequence}"
            ),
            Self::QueueDisconnected(channel) => {
                write!(formatter, "outbound {channel:?} queue disconnected")
            }
        }
    }
}

impl Error for LdflStreamError {}

/// Incrementally converts a raw LDFL byte stream into channel-classified packets.
#[derive(Debug)]
pub struct LdflPacketDecoder {
    decoder: FrameDecoder,
}

impl LdflPacketDecoder {
    /// Create a decoder with the protocol's bounded control/media limits.
    #[must_use]
    pub fn new() -> Self {
        Self {
            decoder: FrameDecoder::new(),
        }
    }

    /// Add arbitrary stream bytes and return every complete LDFL packet.
    ///
    /// Video frames enter the media lane; every other message family enters
    /// the reliable control lane. Partial frames remain buffered internally.
    ///
    /// # Errors
    ///
    /// Returns [`LdflStreamError::Decode`] when an LDFL header or payload is
    /// malformed or exceeds the protocol's memory bounds.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Packet>, LdflStreamError> {
        self.decoder
            .push(bytes)
            .map(|frames| frames.iter().map(packet_from_frame).collect::<Vec<_>>())
            .map_err(|error| LdflStreamError::Decode(error.to_string()))
    }
}

impl Default for LdflPacketDecoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Merge independent control and media queues back into global LDFL order.
#[derive(Debug, Default)]
pub struct LdflPacketMux {
    control: Option<Packet>,
    media: Option<Packet>,
}

impl LdflPacketMux {
    /// Return the globally earliest queued LDFL frame without blocking.
    ///
    /// # Errors
    ///
    /// Rejects disconnected queues, malformed frames, wrong-channel packets,
    /// and duplicate global sequence numbers.
    pub fn next(
        &mut self,
        endpoint: &mut impl PacketTransport,
    ) -> Result<Option<Packet>, LdflStreamError> {
        if self.control.is_none() {
            self.control = endpoint
                .try_receive(Channel::Control)
                .map_err(|_error| LdflStreamError::QueueDisconnected(Channel::Control))?;
        }
        if self.media.is_none() {
            self.media = endpoint
                .try_receive(Channel::Media)
                .map_err(|_error| LdflStreamError::QueueDisconnected(Channel::Media))?;
        }

        match (&self.control, &self.media) {
            (Some(control), Some(media)) => {
                let control_sequence = ldfl_packet_sequence(control)?;
                let media_sequence = ldfl_packet_sequence(media)?;
                if control_sequence == media_sequence {
                    return Err(LdflStreamError::DuplicateSequence(control_sequence));
                }
                if control_sequence < media_sequence {
                    Ok(self.control.take())
                } else {
                    Ok(self.media.take())
                }
            }
            (Some(control), None) => {
                ldfl_packet_sequence(control)?;
                Ok(self.control.take())
            }
            (None, Some(media)) => {
                ldfl_packet_sequence(media)?;
                Ok(self.media.take())
            }
            (None, None) => Ok(None),
        }
    }
}

/// Validate one packet and return its global LDFL sequence number.
///
/// # Errors
///
/// Rejects partial frames, trailing bytes, and a frame queued on the wrong
/// control/media lane.
pub fn ldfl_packet_sequence(packet: &Packet) -> Result<u64, LdflStreamError> {
    let DecodeOutcome::Complete { frame, consumed } = WireFrame::decode_prefix(packet.payload())
        .map_err(|error| LdflStreamError::Decode(error.to_string()))?
    else {
        return Err(LdflStreamError::PartialPacket);
    };
    if consumed != packet.len() {
        return Err(LdflStreamError::TrailingBytes);
    }
    let expected = channel_for_message(frame.header().kind());
    if packet.channel() != expected {
        return Err(LdflStreamError::WrongChannel {
            kind: frame.header().kind(),
            actual: packet.channel(),
            expected,
        });
    }
    Ok(frame.header().sequence())
}

const fn channel_for_message(kind: MessageType) -> Channel {
    if matches!(kind, MessageType::VideoFrame) {
        Channel::Media
    } else {
        Channel::Control
    }
}

fn packet_from_frame(frame: &WireFrame) -> Packet {
    if channel_for_message(frame.header().kind()) == Channel::Media {
        Packet::media(frame.encode())
    } else {
        Packet::control(frame.encode())
    }
}

#[cfg(test)]
mod tests {
    use ladoflow_protocol::{Frame as WireFrame, FrameFlags, MessageType};

    use crate::{
        Channel, LdflPacketDecoder, LdflPacketMux, LdflStreamError, LoopbackConfig, Packet,
        PacketTransport, loopback_pair,
    };

    fn frame(kind: MessageType, sequence: u64, payload: &[u8]) -> Vec<u8> {
        WireFrame::new(kind, FrameFlags::NONE, sequence, payload)
            .expect("valid frame")
            .encode()
    }

    #[test]
    fn decoder_handles_split_frames_and_classifies_channels() {
        let control = frame(MessageType::Ping, 1, b"control");
        let media = frame(MessageType::VideoFrame, 2, b"media");
        let split = control.len() / 2;
        let mut decoder = LdflPacketDecoder::new();
        assert!(
            decoder
                .push(&control[..split])
                .expect("partial input")
                .is_empty()
        );

        let mut remainder = control[split..].to_vec();
        remainder.extend_from_slice(&media);
        let packets = decoder.push(&remainder).expect("complete frames");
        assert_eq!(packets.len(), 2);
        assert_eq!(packets[0].channel(), Channel::Control);
        assert_eq!(packets[0].payload(), control);
        assert_eq!(packets[1].channel(), Channel::Media);
        assert_eq!(packets[1].payload(), media);
    }

    #[test]
    fn mux_restores_global_sequence_across_independent_queues() {
        let (mut sender, mut receiver) = loopback_pair(LoopbackConfig::default());
        sender
            .try_send(Packet::control(frame(MessageType::Ping, 4, b"control")))
            .expect("control queued");
        sender
            .try_send(Packet::media(frame(MessageType::VideoFrame, 3, b"media")))
            .expect("media queued");

        let mut mux = LdflPacketMux::default();
        assert_eq!(
            super::ldfl_packet_sequence(
                &mux.next(&mut receiver)
                    .expect("queue valid")
                    .expect("first packet")
            ),
            Ok(3)
        );
        assert_eq!(
            super::ldfl_packet_sequence(
                &mux.next(&mut receiver)
                    .expect("queue valid")
                    .expect("second packet")
            ),
            Ok(4)
        );
        assert!(mux.next(&mut receiver).expect("queue valid").is_none());
    }

    #[test]
    fn mux_rejects_duplicate_sequences_and_wrong_channels() {
        let (mut sender, mut receiver) = loopback_pair(LoopbackConfig::default());
        sender
            .try_send(Packet::control(frame(MessageType::Ping, 9, b"control")))
            .expect("control queued");
        sender
            .try_send(Packet::media(frame(MessageType::VideoFrame, 9, b"media")))
            .expect("media queued");
        assert_eq!(
            LdflPacketMux::default()
                .next(&mut receiver)
                .expect_err("duplicate rejected"),
            LdflStreamError::DuplicateSequence(9)
        );

        let wrong = Packet::media(frame(MessageType::Ping, 10, b"wrong"));
        assert!(matches!(
            super::ldfl_packet_sequence(&wrong),
            Err(LdflStreamError::WrongChannel { .. })
        ));
    }
}
