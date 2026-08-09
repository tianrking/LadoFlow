use ladoflow_transport::{
    Channel, ConnectionState, LoopbackConfig, Packet, PacketTransport, QueueLimits, ReceiveError,
    SendError, SupersessionKey, loopback_pair,
};

fn limits(max_packets: usize, max_bytes: usize, max_packet_bytes: usize) -> QueueLimits {
    QueueLimits::new(max_packets, max_bytes, max_packet_bytes).expect("valid test limits")
}

fn test_config() -> LoopbackConfig {
    LoopbackConfig::new(limits(4, 128, 64), limits(4, 128, 64))
}

fn received_payload(endpoint: &mut impl PacketTransport, channel: Channel) -> Option<Box<[u8]>> {
    endpoint
        .try_receive(channel)
        .expect("link remains connected")
        .map(Packet::into_payload)
}

#[test]
fn duplex_control_queues_preserve_order_independently() {
    let (mut host, mut display) = loopback_pair(test_config());

    for payload in [b"host-1".as_slice(), b"host-2", b"host-3"] {
        host.try_send(Packet::control(payload))
            .expect("host queue has capacity");
    }
    for payload in [b"display-1".as_slice(), b"display-2"] {
        display
            .try_send(Packet::control(payload))
            .expect("display queue has capacity");
    }

    assert_eq!(
        received_payload(&mut display, Channel::Control).as_deref(),
        Some(b"host-1".as_slice())
    );
    assert_eq!(
        received_payload(&mut display, Channel::Control).as_deref(),
        Some(b"host-2".as_slice())
    );
    assert_eq!(
        received_payload(&mut display, Channel::Control).as_deref(),
        Some(b"host-3".as_slice())
    );
    assert_eq!(received_payload(&mut display, Channel::Control), None);

    assert_eq!(
        received_payload(&mut host, Channel::Control).as_deref(),
        Some(b"display-1".as_slice())
    );
    assert_eq!(
        received_payload(&mut host, Channel::Control).as_deref(),
        Some(b"display-2".as_slice())
    );
    assert_eq!(received_payload(&mut host, Channel::Control), None);
}

#[test]
fn full_control_queue_returns_packet_for_fifo_retry() {
    let config = LoopbackConfig::new(limits(2, 4, 2), limits(1, 4, 4));
    let (mut host, mut display) = loopback_pair(config);

    host.try_send(Packet::control(b"aa"))
        .expect("first packet fits");
    host.try_send(Packet::control(b"bb"))
        .expect("second packet fits");

    let error = host
        .try_send(Packet::control(b"cc"))
        .expect_err("packet-count and byte capacities are full");
    match &error {
        SendError::Full { depth, limits, .. } => {
            assert_eq!(depth.packets(), 2);
            assert_eq!(depth.bytes(), 4);
            assert_eq!(limits.max_packets(), 2);
            assert_eq!(limits.max_queued_bytes(), 4);
        }
        unexpected => panic!("expected full queue, got {unexpected:?}"),
    }
    let retry = error.into_packet();

    assert_eq!(
        received_payload(&mut display, Channel::Control).as_deref(),
        Some(b"aa".as_slice())
    );
    host.try_send(retry).expect("retry fits after one receive");
    assert_eq!(
        received_payload(&mut display, Channel::Control).as_deref(),
        Some(b"bb".as_slice())
    );
    assert_eq!(
        received_payload(&mut display, Channel::Control).as_deref(),
        Some(b"cc".as_slice())
    );
}

#[test]
fn replaceable_media_supersedes_only_matching_obsolete_frames() {
    let config = LoopbackConfig::new(limits(2, 8, 4), limits(2, 8, 4));
    let (mut host, mut display) = loopback_pair(config);
    let primary = SupersessionKey::new(7);
    let overlay = SupersessionKey::new(8);

    host.try_send(Packet::replaceable_media(primary, b"old"))
        .expect("first frame fits");
    host.try_send(Packet::replaceable_media(overlay, b"hud"))
        .expect("second stream fits");
    let report = host
        .try_send(Packet::replaceable_media(primary, b"new!"))
        .expect("replacement fits by superseding the obsolete frame");

    assert_eq!(report.superseded().packets(), 1);
    assert_eq!(report.superseded().bytes(), 3);
    assert_eq!(
        received_payload(&mut display, Channel::Media).as_deref(),
        Some(b"hud".as_slice())
    );
    assert_eq!(
        received_payload(&mut display, Channel::Media).as_deref(),
        Some(b"new!".as_slice())
    );
    assert_eq!(received_payload(&mut display, Channel::Media), None);
}

#[test]
fn disconnect_flushes_both_directions_and_reconnect_starts_clean() {
    let (mut host, mut display) = loopback_pair(test_config());

    host.try_send(Packet::control(b"stale-control"))
        .expect("control packet fits");
    host.try_send(Packet::media(b"stale-media"))
        .expect("media packet fits");
    display
        .try_send(Packet::control(b"stale-reply"))
        .expect("reverse control packet fits");

    let report = host.disconnect();
    assert!(report.was_connected());
    assert_eq!(report.discarded_control().packets(), 2);
    assert_eq!(report.discarded_control().bytes(), 24);
    assert_eq!(report.discarded_media().packets(), 1);
    assert_eq!(report.discarded_media().bytes(), 11);
    assert_eq!(host.connection_state(), ConnectionState::Disconnected);
    assert_eq!(display.connection_state(), ConnectionState::Disconnected);
    assert_eq!(
        display.try_receive(Channel::Control),
        Err(ReceiveError::Disconnected)
    );

    let unsent = Packet::control(b"retry-after-reconnect");
    let error = host
        .try_send(unsent.clone())
        .expect_err("send fails while disconnected");
    assert_eq!(error.packet(), &unsent);

    assert!(display.reconnect());
    assert!(!host.reconnect());
    assert_eq!(host.connection_state(), ConnectionState::Connected);
    assert_eq!(display.connection_state(), ConnectionState::Connected);
    assert_eq!(received_payload(&mut display, Channel::Control), None);
    assert_eq!(received_payload(&mut display, Channel::Media), None);
    assert_eq!(received_payload(&mut host, Channel::Control), None);

    host.try_send(error.into_packet())
        .expect("preserved packet sends after reconnect");
    display
        .try_send(Packet::control(b"fresh-reply"))
        .expect("reverse direction also reconnects");
    assert_eq!(
        received_payload(&mut display, Channel::Control).as_deref(),
        Some(b"retry-after-reconnect".as_slice())
    );
    assert_eq!(
        received_payload(&mut host, Channel::Control).as_deref(),
        Some(b"fresh-reply".as_slice())
    );
}
