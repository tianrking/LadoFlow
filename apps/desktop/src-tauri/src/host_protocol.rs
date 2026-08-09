//! Transport-neutral host-side LDFL negotiation.
//!
//! USB and future LAN adapters expose the same [`PacketTransport`] boundary.
//! This module owns the ordered Hello/Capabilities/DisplayConfig exchange so
//! native link adapters never need to understand session policy.

use std::{
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

use ladoflow_core::{NegotiatedSession, Session, StreamContinuity, negotiate};
use ladoflow_protocol::{
    Capabilities, CodecProfile, CodecSet, DecodeOutcome, DisplayConfig, FeatureFlags,
    Frame as WireFrame, FrameFlags, Hello, InputCapabilities, MessageType, PROTOCOL_VERSION, Role,
    VideoCodec,
};
use ladoflow_transport::{Channel, Packet, PacketTransport, SendError};

const HELLO_SEQUENCE: u64 = 0;
const CAPABILITIES_SEQUENCE: u64 = 1;
const DISPLAY_CONFIG_SEQUENCE: u64 = 2;
pub const FIRST_ACTIVE_SEQUENCE: u64 = 3;
const CONTROL_RETRY_INTERVAL: Duration = Duration::from_millis(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HostProtocolConfig {
    width: u16,
    height: u16,
    refresh_hz: u16,
}

impl HostProtocolConfig {
    pub fn new(width: u16, height: u16, refresh_hz: u16) -> Result<Self, String> {
        if width == 0 || height == 0 {
            return Err("host protocol dimensions must be non-zero".to_owned());
        }
        if !matches!(refresh_hz, 30 | 60) {
            return Err("host protocol refresh rate must be 30 or 60 Hz".to_owned());
        }
        Ok(Self {
            width,
            height,
            refresh_hz,
        })
    }
}

#[derive(Debug)]
pub struct EstablishedHostSession {
    pub session: Session,
    pub display_config: DisplayConfig,
    pub peer_name: String,
    pub next_sequence: u64,
}

#[derive(Debug)]
pub struct HostHandshake {
    config: HostProtocolConfig,
    host_hello: Hello,
    host_capabilities: Capabilities,
    display_hello: Option<Hello>,
    display_capabilities: Option<Capabilities>,
    highest_display_sequence: Option<u64>,
}

impl HostHandshake {
    pub fn new(config: HostProtocolConfig) -> Result<Self, String> {
        let mut nonce = [0_u8; 16];
        getrandom::fill(&mut nonce)
            .map_err(|error| format!("failed to generate the host session nonce: {error}"))?;
        Self::with_nonce(config, nonce)
    }

    fn with_nonce(config: HostProtocolConfig, nonce: [u8; 16]) -> Result<Self, String> {
        let host_hello = Hello::new(
            PROTOCOL_VERSION,
            PROTOCOL_VERSION,
            Role::Host,
            nonce,
            "LadoFlow desktop",
        )
        .map_err(|error| error.to_string())?;
        let host_capabilities = Capabilities::new(
            config.width,
            config.height,
            u32::from(config.refresh_hz) * 1_000,
            40_000,
            CodecSet::H264,
            InputCapabilities::POINTER | InputCapabilities::TOUCH | InputCapabilities::KEYBOARD,
            FeatureFlags::DYNAMIC_ROTATION | FeatureFlags::REMOTE_CURSOR,
        )
        .map_err(|error| error.to_string())?;
        Ok(Self {
            config,
            host_hello,
            host_capabilities,
            display_hello: None,
            display_capabilities: None,
            highest_display_sequence: None,
        })
    }

    pub fn initial_packets(&self) -> Result<[Packet; 2], String> {
        Ok([
            control_packet(HELLO_SEQUENCE, &self.host_hello)?,
            control_packet(CAPABILITIES_SEQUENCE, &self.host_capabilities)?,
        ])
    }

    pub fn accept(&mut self, packet: Packet) -> Result<Option<EstablishedHostSession>, String> {
        if packet.channel() != Channel::Control {
            return Err("display sent media before LDFL negotiation completed".to_owned());
        }
        let packet_len = packet.len();
        let packet_bytes = packet.into_payload();
        let DecodeOutcome::Complete { frame, consumed } =
            WireFrame::decode_prefix(&packet_bytes).map_err(|error| error.to_string())?
        else {
            return Err("display sent a partial LDFL control frame".to_owned());
        };
        if consumed != packet_len {
            return Err("display control packet contains trailing LDFL bytes".to_owned());
        }
        let sequence = frame.header().sequence();
        if self
            .highest_display_sequence
            .is_some_and(|highest| sequence <= highest)
        {
            return Err(format!(
                "display LDFL sequence {sequence} is not newer than the prior sequence"
            ));
        }
        self.highest_display_sequence = Some(sequence);

        match frame.header().kind() {
            MessageType::Hello => {
                if self.display_hello.is_some() {
                    return Err("display sent Hello more than once".to_owned());
                }
                self.display_hello = Some(
                    frame
                        .decode_payload::<Hello>()
                        .map_err(|error| error.to_string())?,
                );
            }
            MessageType::Capabilities => {
                if self.display_capabilities.is_some() {
                    return Err("display sent Capabilities more than once".to_owned());
                }
                self.display_capabilities = Some(
                    frame
                        .decode_payload::<Capabilities>()
                        .map_err(|error| error.to_string())?,
                );
            }
            kind => {
                return Err(format!(
                    "display sent {kind:?} before Hello and Capabilities completed"
                ));
            }
        }

        if self.display_hello.is_some() && self.display_capabilities.is_some() {
            self.establish().map(Some)
        } else {
            Ok(None)
        }
    }

    fn establish(&self) -> Result<EstablishedHostSession, String> {
        let display_hello = self
            .display_hello
            .as_ref()
            .ok_or_else(|| "display Hello is missing".to_owned())?;
        let display_capabilities = self
            .display_capabilities
            .ok_or_else(|| "display Capabilities are missing".to_owned())?;
        let agreement = negotiate(
            &self.host_hello,
            self.host_capabilities,
            display_hello,
            display_capabilities,
        )
        .map_err(|error| error.to_string())?;
        let display_config = select_display_config(self.config, agreement)?;
        let mut session = Session::new();
        session.start().map_err(|error| error.to_string())?;
        session
            .establish(agreement, StreamContinuity::Restart)
            .map_err(|error| error.to_string())?;
        if let Some(sequence) = self.highest_display_sequence {
            session
                .observe_sequence(sequence)
                .map_err(|error| error.to_string())?;
        }

        Ok(EstablishedHostSession {
            session,
            display_config,
            peer_name: display_hello.implementation_name().to_owned(),
            next_sequence: FIRST_ACTIVE_SEQUENCE,
        })
    }
}

pub fn negotiate_host_transport(
    transport: &mut impl PacketTransport,
    config: HostProtocolConfig,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<EstablishedHostSession, String> {
    if timeout.is_zero() {
        return Err("LDFL negotiation timeout must be non-zero".to_owned());
    }
    let deadline = Instant::now() + timeout;
    let mut handshake = HostHandshake::new(config)?;
    for packet in handshake.initial_packets()? {
        send_reliable_control(transport, packet, cancel, deadline)?;
    }

    loop {
        check_wait(cancel, deadline)?;
        match transport.try_receive(Channel::Control) {
            Ok(Some(packet)) => {
                if let Some(established) = handshake.accept(packet)? {
                    let config_packet =
                        control_packet(DISPLAY_CONFIG_SEQUENCE, &established.display_config)?;
                    send_reliable_control(transport, config_packet, cancel, deadline)?;
                    return Ok(established);
                }
            }
            Ok(None) => thread::sleep(CONTROL_RETRY_INTERVAL),
            Err(error) => return Err(format!("LDFL negotiation transport failed: {error}")),
        }
    }
}

pub fn send_control_payload<P: ladoflow_protocol::WirePayload>(
    transport: &mut impl PacketTransport,
    sequence: u64,
    payload: &P,
    cancel: &AtomicBool,
    timeout: Duration,
) -> Result<(), String> {
    if timeout.is_zero() {
        return Err("LDFL control-send timeout must be non-zero".to_owned());
    }
    send_reliable_control(
        transport,
        control_packet(sequence, payload)?,
        cancel,
        Instant::now() + timeout,
    )
}

fn send_reliable_control(
    transport: &mut impl PacketTransport,
    mut packet: Packet,
    cancel: &AtomicBool,
    deadline: Instant,
) -> Result<(), String> {
    loop {
        check_wait(cancel, deadline)?;
        match transport.try_send(packet) {
            Ok(_report) => return Ok(()),
            Err(SendError::Full { packet: retry, .. }) => {
                packet = retry;
                thread::sleep(CONTROL_RETRY_INTERVAL);
            }
            Err(error) => return Err(format!("LDFL control send failed: {error}")),
        }
    }
}

fn check_wait(cancel: &AtomicBool, deadline: Instant) -> Result<(), String> {
    if cancel.load(Ordering::Acquire) {
        Err("LDFL negotiation was cancelled".to_owned())
    } else if Instant::now() >= deadline {
        Err("LDFL negotiation timed out waiting for the display".to_owned())
    } else {
        Ok(())
    }
}

fn control_packet<P: ladoflow_protocol::WirePayload>(
    sequence: u64,
    payload: &P,
) -> Result<Packet, String> {
    WireFrame::from_payload(FrameFlags::NONE, sequence, payload)
        .map(|frame| Packet::control(frame.encode()))
        .map_err(|error| error.to_string())
}

fn select_display_config(
    requested: HostProtocolConfig,
    agreement: NegotiatedSession,
) -> Result<DisplayConfig, String> {
    let capabilities = agreement.capabilities();
    if capabilities.codec_bits() & CodecSet::H264.bits() == 0 {
        return Err("the display did not negotiate H.264 support".to_owned());
    }
    let width = requested.width.min(capabilities.max_width());
    let height = requested.height.min(capabilities.max_height());
    let requested_refresh = u32::from(requested.refresh_hz) * 1_000;
    let refresh_millihz = if capabilities.max_refresh_millihz() >= requested_refresh {
        requested_refresh
    } else if requested.refresh_hz == 60 && capabilities.max_refresh_millihz() >= 30_000 {
        30_000
    } else {
        return Err(format!(
            "display maximum refresh {} mHz cannot satisfy a 30 Hz session",
            capabilities.max_refresh_millihz()
        ));
    };
    let bitrate_kbps =
        target_h264_bitrate(width, height, refresh_millihz).min(capabilities.max_bitrate_kbps());
    DisplayConfig::new(
        width,
        height,
        refresh_millihz,
        bitrate_kbps,
        VideoCodec::H264,
        CodecProfile::H264Main,
    )
    .map_err(|error| error.to_string())
}

fn target_h264_bitrate(width: u16, height: u16, refresh_millihz: u32) -> u32 {
    let pixels_per_second = u64::from(width)
        .saturating_mul(u64::from(height))
        .saturating_mul(u64::from(refresh_millihz))
        / 1_000;
    let estimate_kbps = pixels_per_second.saturating_mul(12) / 100_000;
    u32::try_from(estimate_kbps)
        .unwrap_or(u32::MAX)
        .clamp(4_000, 40_000)
}

#[cfg(test)]
mod tests {
    use std::{
        sync::{Arc, atomic::AtomicBool},
        thread,
        time::Duration,
    };

    use ladoflow_protocol::{
        Capabilities, CodecSet, DecodeOutcome, FeatureFlags, Frame as WireFrame, FrameFlags, Hello,
        InputCapabilities, MessageType, Role,
    };
    use ladoflow_transport::{Channel, LoopbackConfig, Packet, PacketTransport, loopback_pair};

    use super::{
        CAPABILITIES_SEQUENCE, DISPLAY_CONFIG_SEQUENCE, FIRST_ACTIVE_SEQUENCE, HELLO_SEQUENCE,
        HostHandshake, HostProtocolConfig, control_packet, negotiate_host_transport,
    };

    fn display_hello(role: Role) -> Hello {
        Hello::new(1, 1, role, [0x44; 16], "LadoFlow Android").expect("valid display Hello")
    }

    fn display_capabilities(width: u16, height: u16) -> Capabilities {
        display_capabilities_at(width, height, 60_000)
    }

    fn display_capabilities_at(width: u16, height: u16, refresh_millihz: u32) -> Capabilities {
        Capabilities::new(
            width,
            height,
            refresh_millihz,
            12_000,
            CodecSet::H264,
            InputCapabilities::POINTER | InputCapabilities::TOUCH,
            FeatureFlags::DYNAMIC_ROTATION,
        )
        .expect("valid display capabilities")
    }

    fn decode_packet(packet: &Packet) -> WireFrame {
        let DecodeOutcome::Complete { frame, consumed } =
            WireFrame::decode_prefix(packet.payload()).expect("valid frame")
        else {
            panic!("complete packet must contain a complete frame");
        };
        assert_eq!(consumed, packet.len());
        frame
    }

    #[test]
    fn handshake_accepts_either_peer_order_and_selects_bounded_h264_config() {
        let config = HostProtocolConfig::new(1_920, 1_080, 60).expect("valid config");
        let mut handshake = HostHandshake::with_nonce(config, [0x48; 16]).expect("handshake");
        let initial = handshake.initial_packets().expect("initial frames");
        let hello_frame = decode_packet(&initial[0]);
        let capabilities_frame = decode_packet(&initial[1]);
        assert_eq!(hello_frame.header().kind(), MessageType::Hello);
        assert_eq!(hello_frame.header().sequence(), HELLO_SEQUENCE);
        assert_eq!(
            capabilities_frame.header().kind(),
            MessageType::Capabilities
        );
        assert_eq!(
            capabilities_frame.header().sequence(),
            CAPABILITIES_SEQUENCE
        );

        assert!(
            handshake
                .accept(
                    control_packet(7, &display_capabilities(1_280, 800))
                        .expect("capabilities frame")
                )
                .expect("capabilities accepted")
                .is_none()
        );
        let established = handshake
            .accept(control_packet(8, &display_hello(Role::Display)).expect("Hello frame"))
            .expect("Hello accepted")
            .expect("both messages establish the session");

        assert_eq!(established.peer_name, "LadoFlow Android");
        assert_eq!(established.display_config.width(), 1_280);
        assert_eq!(established.display_config.height(), 800);
        assert_eq!(established.display_config.bitrate_kbps(), 7_372);
        assert_eq!(established.next_sequence, FIRST_ACTIVE_SEQUENCE);
    }

    #[test]
    fn wrong_peer_role_and_duplicate_messages_fail_closed() {
        let config = HostProtocolConfig::new(1_920, 1_080, 60).expect("valid config");
        let mut wrong_role = HostHandshake::with_nonce(config, [0x48; 16]).expect("handshake");
        wrong_role
            .accept(control_packet(1, &display_hello(Role::Host)).expect("Hello frame"))
            .expect("first message is structurally valid");
        let error = wrong_role
            .accept(control_packet(2, &display_capabilities(1_920, 1_080)).expect("capabilities"))
            .expect_err("matching host roles cannot negotiate");
        assert!(error.contains("same role"));

        let mut duplicate = HostHandshake::with_nonce(config, [0x48; 16]).expect("handshake");
        duplicate
            .accept(control_packet(1, &display_hello(Role::Display)).expect("Hello frame"))
            .expect("first Hello accepted");
        assert!(
            duplicate
                .accept(control_packet(2, &display_hello(Role::Display)).expect("Hello frame"))
                .expect_err("duplicate Hello rejected")
                .contains("more than once")
        );
    }

    #[test]
    fn sixty_hz_request_falls_back_to_thirty_but_rejects_lower_displays() {
        let config = HostProtocolConfig::new(1_920, 1_080, 60).expect("valid config");
        let mut fallback = HostHandshake::with_nonce(config, [0x48; 16]).expect("handshake");
        fallback
            .accept(control_packet(0, &display_hello(Role::Display)).expect("Hello"))
            .expect("Hello accepted");
        let established = fallback
            .accept(
                control_packet(1, &display_capabilities_at(1_920, 1_080, 45_000))
                    .expect("capabilities"),
            )
            .expect("30 Hz fallback is supported")
            .expect("session established");
        assert_eq!(established.display_config.refresh_millihz(), 30_000);

        let mut too_slow = HostHandshake::with_nonce(config, [0x48; 16]).expect("handshake");
        too_slow
            .accept(control_packet(0, &display_hello(Role::Display)).expect("Hello"))
            .expect("Hello accepted");
        assert!(
            too_slow
                .accept(
                    control_packet(1, &display_capabilities_at(1_920, 1_080, 29_999))
                        .expect("capabilities")
                )
                .expect_err("sub-30 Hz display is rejected")
                .contains("cannot satisfy")
        );
    }

    #[test]
    fn transport_driver_exchanges_all_three_host_control_frames() {
        let (mut host, mut display) = loopback_pair(LoopbackConfig::default());
        let display_worker = thread::spawn(move || {
            let mut host_kinds = Vec::new();
            while host_kinds.len() < 2 {
                if let Some(packet) = display
                    .try_receive(Channel::Control)
                    .expect("link remains connected")
                {
                    host_kinds.push(decode_packet(&packet).header().kind());
                } else {
                    thread::sleep(Duration::from_millis(1));
                }
            }
            display
                .try_send(control_packet(0, &display_hello(Role::Display)).expect("display Hello"))
                .expect("send display Hello");
            display
                .try_send(
                    control_packet(1, &display_capabilities(1_920, 1_080))
                        .expect("display capabilities"),
                )
                .expect("send display capabilities");

            loop {
                if let Some(packet) = display
                    .try_receive(Channel::Control)
                    .expect("link remains connected")
                {
                    let frame = decode_packet(&packet);
                    assert_eq!(frame.header().kind(), MessageType::DisplayConfig);
                    assert_eq!(frame.header().sequence(), DISPLAY_CONFIG_SEQUENCE);
                    return host_kinds;
                }
                thread::sleep(Duration::from_millis(1));
            }
        });

        let cancel = Arc::new(AtomicBool::new(false));
        let established = negotiate_host_transport(
            &mut host,
            HostProtocolConfig::new(1_920, 1_080, 60).expect("valid config"),
            &cancel,
            Duration::from_secs(1),
        )
        .expect("transport negotiation succeeds");
        assert_eq!(established.peer_name, "LadoFlow Android");
        assert_eq!(
            display_worker.join().expect("display worker"),
            [MessageType::Hello, MessageType::Capabilities]
        );
    }

    #[test]
    fn media_before_negotiation_is_rejected() {
        let config = HostProtocolConfig::new(1_920, 1_080, 60).expect("valid config");
        let mut handshake = HostHandshake::with_nonce(config, [0x48; 16]).expect("handshake");
        let media = WireFrame::new(MessageType::VideoFrame, FrameFlags::NONE, 1, vec![1])
            .expect("bounded media frame");
        assert!(
            handshake
                .accept(Packet::media(media.encode()))
                .expect_err("media must wait")
                .contains("before")
        );
    }
}
