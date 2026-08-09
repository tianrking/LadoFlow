use ladoflow_protocol::{
    DecodeOutcome, FRAME_HEADER_LEN, Frame, FrameDecoder, FrameFlags, MAX_CONTROL_PAYLOAD,
    MessageType, PROTOCOL_VERSION, ProtocolError,
};

#[test]
fn frame_layout_is_stable_and_big_endian() {
    let frame = Frame::new(
        MessageType::Ping,
        FrameFlags::ACK_REQUIRED,
        0x0102_0304_0506_0708,
        [0xaa, 0xbb],
    )
    .expect("valid frame");

    let encoded = frame.encode();
    assert_eq!(&encoded[0..4], b"LDFL");
    assert_eq!(&encoded[4..6], &PROTOCOL_VERSION.to_be_bytes());
    assert_eq!(
        &encoded[6..8],
        &u16::try_from(FRAME_HEADER_LEN)
            .expect("test header length fits u16")
            .to_be_bytes()
    );
    assert_eq!(&encoded[8..10], &(MessageType::Ping as u16).to_be_bytes());
    assert_eq!(
        &encoded[10..12],
        &FrameFlags::ACK_REQUIRED.bits().to_be_bytes()
    );
    assert_eq!(&encoded[12..20], &0x0102_0304_0506_0708_u64.to_be_bytes());
    assert_eq!(&encoded[20..24], &2_u32.to_be_bytes());
    assert_eq!(&encoded[24..], &[0xaa, 0xbb]);
}

#[test]
fn complete_frame_round_trips_with_trailing_bytes() {
    let expected = Frame::new(
        MessageType::VideoFrame,
        FrameFlags::KEYFRAME,
        42,
        b"encoded-access-unit".to_vec(),
    )
    .expect("valid frame");
    let mut stream = expected.encode();
    stream.extend_from_slice(b"next-frame");

    let outcome = Frame::decode_prefix(&stream).expect("valid prefix");
    match outcome {
        DecodeOutcome::Complete { frame, consumed } => {
            assert_eq!(frame, expected);
            assert_eq!(consumed, expected.encoded_len());
            assert_eq!(&stream[consumed..], b"next-frame");
        }
        DecodeOutcome::NeedMoreData { .. } => panic!("frame should be complete"),
    }
}

#[test]
fn partial_input_reports_the_exact_next_minimum() {
    let frame =
        Frame::new(MessageType::Pong, FrameFlags::NONE, 7, [1, 2, 3, 4]).expect("valid frame");
    let encoded = frame.encode();

    assert_eq!(
        Frame::decode_prefix(&encoded[..7]).expect("partial header is not invalid"),
        DecodeOutcome::NeedMoreData {
            minimum: FRAME_HEADER_LEN
        }
    );
    assert_eq!(
        Frame::decode_prefix(&encoded[..FRAME_HEADER_LEN + 2])
            .expect("partial payload is not invalid"),
        DecodeOutcome::NeedMoreData {
            minimum: encoded.len()
        }
    );
}

#[test]
fn incremental_decoder_accepts_one_byte_chunks() {
    let expected = Frame::new(
        MessageType::Input,
        FrameFlags::ACK_REQUIRED,
        99,
        b"pointer".to_vec(),
    )
    .expect("valid frame");
    let mut decoder = FrameDecoder::new();
    let mut decoded_frames = Vec::new();

    for byte in expected.encode() {
        decoded_frames.extend(decoder.push(&[byte]).expect("valid chunk"));
    }

    assert_eq!(decoded_frames, vec![expected]);
    assert_eq!(decoder.buffered_len(), 0);
}

#[test]
fn incremental_decoder_emits_multiple_frames_from_one_chunk() {
    let first = Frame::new(MessageType::Ping, FrameFlags::NONE, 10, []).expect("valid frame");
    let second = Frame::new(MessageType::Pong, FrameFlags::NONE, 11, []).expect("valid frame");
    let mut bytes = first.encode();
    bytes.extend_from_slice(&second.encode());

    let frames = FrameDecoder::new().push(&bytes).expect("valid stream");
    assert_eq!(frames, vec![first, second]);
}

#[test]
fn malformed_header_fields_are_rejected() {
    let frame = Frame::new(MessageType::Ping, FrameFlags::NONE, 1, []).expect("valid frame");

    let mut bad_magic = frame.encode();
    bad_magic[0] = b'X';
    assert!(matches!(
        Frame::decode_prefix(&bad_magic),
        Err(ProtocolError::InvalidMagic(_))
    ));

    let mut bad_version = frame.encode();
    bad_version[4..6].copy_from_slice(&(PROTOCOL_VERSION + 1).to_be_bytes());
    assert!(matches!(
        Frame::decode_prefix(&bad_version),
        Err(ProtocolError::UnsupportedVersion { .. })
    ));

    let mut bad_kind = frame.encode();
    bad_kind[8..10].copy_from_slice(&99_u16.to_be_bytes());
    assert_eq!(
        Frame::decode_prefix(&bad_kind),
        Err(ProtocolError::UnknownMessageType(99))
    );

    let mut bad_flags = frame.encode();
    bad_flags[10..12].copy_from_slice(&0x8000_u16.to_be_bytes());
    assert_eq!(
        Frame::decode_prefix(&bad_flags),
        Err(ProtocolError::UnknownFrameFlags(0x8000))
    );
}

#[test]
fn declared_oversized_control_payload_is_rejected_before_allocation() {
    let frame = Frame::new(MessageType::Ping, FrameFlags::NONE, 1, []).expect("valid frame");
    let mut encoded = frame.encode();
    let oversized = u32::try_from(MAX_CONTROL_PAYLOAD + 1).expect("test size fits u32");
    encoded[20..24].copy_from_slice(&oversized.to_be_bytes());

    assert_eq!(
        Frame::decode_prefix(&encoded),
        Err(ProtocolError::PayloadTooLarge {
            kind: MessageType::Ping,
            length: MAX_CONTROL_PAYLOAD + 1,
            limit: MAX_CONTROL_PAYLOAD,
        })
    );
}

#[test]
fn decoder_enforces_configured_memory_limit_without_mutating_buffer() {
    let mut decoder = FrameDecoder::with_buffer_limit(8);
    assert_eq!(
        decoder.push(&[0; 9]),
        Err(ProtocolError::BufferLimitExceeded {
            attempted: 9,
            limit: 8,
        })
    );
    assert_eq!(decoder.buffered_len(), 0);
}
