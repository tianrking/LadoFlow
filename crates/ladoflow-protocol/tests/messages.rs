use ladoflow_protocol::{
    Capabilities, CodecSet, FeatureFlags, Frame, FrameFlags, Hello, InputCapabilities, MessageType,
    PROTOCOL_VERSION, ProtocolError, Role, WirePayload,
};

#[test]
fn hello_round_trips_through_a_typed_frame() {
    let hello = Hello::new(
        PROTOCOL_VERSION,
        PROTOCOL_VERSION,
        Role::Display,
        [0x5a; 16],
        "LadoFlow Android",
    )
    .expect("valid hello");
    let frame = Frame::from_payload(FrameFlags::ACK_REQUIRED, 1, &hello).expect("valid frame");
    let decoded: Hello = frame.decode_payload().expect("typed hello");

    assert_eq!(frame.header().kind(), MessageType::Hello);
    assert_eq!(decoded, hello);
    assert_eq!(decoded.implementation_name(), "LadoFlow Android");
    assert_eq!(decoded.nonce(), &[0x5a; 16]);
}

#[test]
fn hello_rejects_invalid_ranges_and_names() {
    assert!(matches!(
        Hello::new(2, 1, Role::Host, [0; 16], "host"),
        Err(ProtocolError::InvalidPayload(_))
    ));
    assert!(matches!(
        Hello::new(1, 1, Role::Host, [0; 16], ""),
        Err(ProtocolError::InvalidPayload(_))
    ));
    assert!(matches!(
        Hello::new(1, 1, Role::Host, [0; 16], "x".repeat(65)),
        Err(ProtocolError::InvalidPayload(_))
    ));
}

#[test]
fn hello_rejects_truncation_length_mismatch_and_invalid_utf8() {
    assert!(matches!(
        Hello::decode(&[0; 4]),
        Err(ProtocolError::InvalidPayload(_))
    ));

    let hello = Hello::new(1, 1, Role::Host, [1; 16], "host").expect("valid hello");
    let mut mismatched = hello.encode().expect("encode hello");
    mismatched[5] = 20;
    assert!(matches!(
        Hello::decode(&mismatched),
        Err(ProtocolError::InvalidPayload(_))
    ));

    let mut invalid_utf8 = hello.encode().expect("encode hello");
    invalid_utf8[22] = 0xff;
    assert_eq!(
        Hello::decode(&invalid_utf8),
        Err(ProtocolError::InvalidUtf8)
    );
}

#[test]
fn capabilities_round_trip_preserves_masks_and_limits() {
    let expected = Capabilities::new(
        2732,
        2048,
        120_000,
        80_000,
        CodecSet::H264 | CodecSet::HEVC,
        InputCapabilities::POINTER | InputCapabilities::TOUCH | InputCapabilities::KEYBOARD,
        FeatureFlags::DYNAMIC_ROTATION | FeatureFlags::REMOTE_CURSOR,
    )
    .expect("valid capabilities");
    let payload = expected.encode().expect("encode capabilities");
    let actual = Capabilities::decode(&payload).expect("decode capabilities");

    assert_eq!(payload.len(), 20);
    assert_eq!(actual, expected);
    assert!(actual.codecs().contains(CodecSet::H264));
    assert!(actual.input().contains(InputCapabilities::TOUCH));
    assert!(actual.features().contains(FeatureFlags::REMOTE_CURSOR));
    assert_eq!(actual.max_refresh_millihz(), 120_000);
}

#[test]
fn capabilities_reject_unknown_bits_and_impossible_limits() {
    assert!(matches!(
        Capabilities::new(
            0,
            1080,
            60_000,
            20_000,
            CodecSet::H264,
            InputCapabilities::default(),
            FeatureFlags::default(),
        ),
        Err(ProtocolError::InvalidPayload(_))
    ));

    let capabilities = Capabilities::new(
        1920,
        1080,
        60_000,
        20_000,
        CodecSet::H264,
        InputCapabilities::default(),
        FeatureFlags::default(),
    )
    .expect("valid capabilities");
    let mut payload = capabilities.encode().expect("encode capabilities");
    payload[12..14].copy_from_slice(&0x8000_u16.to_be_bytes());
    assert!(matches!(
        Capabilities::decode(&payload),
        Err(ProtocolError::InvalidPayload(_))
    ));
}

#[test]
fn typed_decode_rejects_the_wrong_frame_family() {
    let frame = Frame::new(MessageType::Ping, FrameFlags::NONE, 1, []).expect("valid frame");
    assert_eq!(
        frame.decode_payload::<Hello>(),
        Err(ProtocolError::UnexpectedMessageType {
            expected: MessageType::Hello,
            actual: MessageType::Ping,
        })
    );
}
