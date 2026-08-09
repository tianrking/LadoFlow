use std::fmt::Debug;

use ladoflow_protocol::{
    ButtonState, CodecProfile, DisplayConfig, ErrorCode, ErrorMessage, Frame, FrameFlags,
    InputEvent, InputEventKind, KeyModifiers, MAX_ENCODED_VIDEO_BYTES, MAX_ERROR_DIAGNOSTIC_BYTES,
    MAX_LOSS_PARTS_PER_MILLION, MAX_MEDIA_PAYLOAD, MAX_STAGE_DURATION_MICROS,
    MAX_TELEMETRY_QUEUE_DEPTH, MAX_TOUCH_CONTACTS, MessageType, Ping, PointerButton, Pong,
    ProtocolError, StageTimings, Telemetry, ThermalState, TouchPhase, VIDEO_FRAME_METADATA_LEN,
    VideoCodec, VideoFrame, VideoFrameMetadata, WirePayload,
};

fn assert_typed_round_trip<P>(expected: &P)
where
    P: WirePayload + Debug + PartialEq,
{
    let encoded = expected.encode().expect("payload encodes");
    let decoded = P::decode(&encoded).expect("payload decodes");
    assert_eq!(&decoded, expected);

    let frame = Frame::from_payload(FrameFlags::NONE, 73, expected).expect("typed frame");
    assert_eq!(frame.header().kind(), P::KIND);
    assert_eq!(
        frame.decode_payload::<P>().expect("typed payload"),
        *expected
    );
}

#[test]
fn display_config_round_trips_in_network_byte_order() {
    let config = DisplayConfig::new(
        0x1234,
        0x2345,
        0x0102_0304,
        0x1112_1314,
        VideoCodec::Hevc,
        CodecProfile::HevcMain10,
    )
    .expect("valid display config");
    let encoded = config.encode().expect("encode display config");

    assert_eq!(encoded.len(), 14);
    assert_eq!(&encoded[0..2], &0x1234_u16.to_be_bytes());
    assert_eq!(&encoded[2..4], &0x2345_u16.to_be_bytes());
    assert_eq!(&encoded[4..8], &0x0102_0304_u32.to_be_bytes());
    assert_eq!(&encoded[8..12], &0x1112_1314_u32.to_be_bytes());
    assert_eq!(encoded[12], VideoCodec::Hevc as u8);
    assert_eq!(encoded[13], CodecProfile::HevcMain10 as u8);
    assert_eq!(config.codec().capability().bits(), 1 << 1);
    assert_typed_round_trip(&config);
}

#[test]
fn display_config_rejects_wrong_lengths_unknown_values_and_invalid_combinations() {
    for length in [0, 13, 15] {
        assert!(matches!(
            DisplayConfig::decode(&vec![0; length]),
            Err(ProtocolError::InvalidPayload(_))
        ));
    }

    assert!(
        DisplayConfig::new(
            0,
            1080,
            60_000,
            20_000,
            VideoCodec::H264,
            CodecProfile::H264High,
        )
        .is_err()
    );
    assert!(
        DisplayConfig::new(
            1920,
            1080,
            60_000,
            0,
            VideoCodec::H264,
            CodecProfile::H264High,
        )
        .is_err()
    );
    assert!(
        DisplayConfig::new(
            1920,
            1080,
            60_000,
            20_000,
            VideoCodec::Av1,
            CodecProfile::H264High,
        )
        .is_err()
    );

    let valid = DisplayConfig::new(
        1920,
        1080,
        60_000,
        20_000,
        VideoCodec::H264,
        CodecProfile::H264High,
    )
    .expect("valid config")
    .encode()
    .expect("encode config");

    let mut zero_refresh = valid.clone();
    zero_refresh[4..8].fill(0);
    assert!(DisplayConfig::decode(&zero_refresh).is_err());

    let mut unknown_codec = valid.clone();
    unknown_codec[12] = 99;
    assert!(DisplayConfig::decode(&unknown_codec).is_err());

    let mut unknown_profile = valid.clone();
    unknown_profile[13] = 99;
    assert!(DisplayConfig::decode(&unknown_profile).is_err());

    let mut mismatched_profile = valid;
    mismatched_profile[13] = CodecProfile::Av1Main as u8;
    assert!(DisplayConfig::decode(&mismatched_profile).is_err());
}

#[test]
fn video_frame_round_trips_metadata_and_encoded_bytes() {
    let metadata = VideoFrameMetadata::new(
        0x0102_0304_0506_0708,
        0x1112_1314_1516_1718,
        0x2122_2324_2526_2728,
        0x3132_3334,
    )
    .expect("valid metadata");
    let video = VideoFrame::new(metadata, [0xaa, 0xbb, 0xcc]).expect("valid video frame");
    let encoded = video.encode().expect("encode video frame");

    assert_eq!(encoded.len(), VIDEO_FRAME_METADATA_LEN + 3);
    assert_eq!(&encoded[0..8], &0x0102_0304_0506_0708_u64.to_be_bytes());
    assert_eq!(&encoded[8..16], &0x1112_1314_1516_1718_u64.to_be_bytes());
    assert_eq!(&encoded[16..24], &0x2122_2324_2526_2728_u64.to_be_bytes());
    assert_eq!(&encoded[24..28], &0x3132_3334_u32.to_be_bytes());
    assert_eq!(&encoded[28..], &[0xaa, 0xbb, 0xcc]);
    assert_eq!(video.metadata(), metadata);
    assert_typed_round_trip(&video);

    let frame = Frame::from_payload(FrameFlags::KEYFRAME, 1, &video).expect("keyframe");
    assert_eq!(frame.header().kind(), MessageType::VideoFrame);
    assert!(frame.header().flags().contains(FrameFlags::KEYFRAME));
}

#[test]
fn video_frame_rejects_missing_invalid_and_oversized_content() {
    assert!(VideoFrameMetadata::new(1, 2, 3, 0).is_err());

    let metadata = VideoFrameMetadata::new(1, 2, 3, 16_667).expect("valid metadata");
    assert!(VideoFrame::new(metadata, Vec::new()).is_err());

    for length in [0, VIDEO_FRAME_METADATA_LEN - 1, VIDEO_FRAME_METADATA_LEN] {
        assert!(matches!(
            VideoFrame::decode(&vec![0; length]),
            Err(ProtocolError::InvalidPayload(_))
        ));
    }

    let mut zero_duration = VideoFrame::new(metadata, [1])
        .expect("valid frame")
        .encode()
        .expect("encode frame");
    zero_duration[24..28].fill(0);
    assert!(VideoFrame::decode(&zero_duration).is_err());

    assert!(VideoFrame::new(metadata, vec![0; MAX_ENCODED_VIDEO_BYTES]).is_ok());
    assert!(matches!(
        VideoFrame::new(metadata, vec![0; MAX_ENCODED_VIDEO_BYTES + 1]),
        Err(ProtocolError::PayloadTooLarge {
            kind: MessageType::VideoFrame,
            ..
        })
    ));
    assert!(matches!(
        VideoFrame::decode(&vec![0; MAX_MEDIA_PAYLOAD + 1]),
        Err(ProtocolError::PayloadTooLarge {
            kind: MessageType::VideoFrame,
            ..
        })
    ));
}

#[test]
fn every_version_one_input_variant_round_trips() {
    let events = [
        InputEvent::new(100, InputEventKind::PointerMove { x: 1920, y: 1080 })
            .expect("pointer move"),
        InputEvent::new(
            101,
            InputEventKind::PointerButton {
                button: PointerButton::Secondary,
                state: ButtonState::Pressed,
            },
        )
        .expect("pointer button"),
        InputEvent::new(
            102,
            InputEventKind::Wheel {
                delta_x: -120,
                delta_y: 240,
            },
        )
        .expect("wheel"),
        InputEvent::new(
            103,
            InputEventKind::Key {
                usage: 0x04,
                state: ButtonState::Released,
                modifiers: KeyModifiers::SHIFT | KeyModifiers::CONTROL,
            },
        )
        .expect("key"),
        InputEvent::new(
            104,
            InputEventKind::Touch {
                contact_id: MAX_TOUCH_CONTACTS - 1,
                phase: TouchPhase::Move,
                x: 123,
                y: 456,
                pressure: 32_768,
            },
        )
        .expect("touch"),
        InputEvent::new(105, InputEventKind::Focus { focused: true }).expect("focus"),
    ];

    for event in events {
        assert_typed_round_trip(&event);
    }

    let encoded = events[3].encode().expect("encode key event");
    assert_eq!(&encoded[0..8], &103_u64.to_be_bytes());
    assert_eq!(encoded[8], 4);
    assert_eq!(&encoded[9..11], &0x04_u16.to_be_bytes());
    assert_eq!(encoded[11], ButtonState::Released as u8);
    assert_eq!(
        &encoded[12..14],
        &(KeyModifiers::SHIFT | KeyModifiers::CONTROL)
            .bits()
            .to_be_bytes()
    );
}

#[test]
fn input_rejects_unknown_kinds_values_and_noncanonical_lengths() {
    assert!(InputEvent::decode(&[0; 8]).is_err());

    let mut unknown_kind = vec![0; 9];
    unknown_kind[8] = 99;
    assert!(InputEvent::decode(&unknown_kind).is_err());

    let valid_events = [
        InputEvent::new(1, InputEventKind::PointerMove { x: 1, y: 2 }).expect("move"),
        InputEvent::new(
            1,
            InputEventKind::PointerButton {
                button: PointerButton::Primary,
                state: ButtonState::Pressed,
            },
        )
        .expect("button"),
        InputEvent::new(
            1,
            InputEventKind::Wheel {
                delta_x: 1,
                delta_y: -1,
            },
        )
        .expect("wheel"),
        InputEvent::new(
            1,
            InputEventKind::Key {
                usage: 4,
                state: ButtonState::Pressed,
                modifiers: KeyModifiers::default(),
            },
        )
        .expect("key"),
        InputEvent::new(
            1,
            InputEventKind::Touch {
                contact_id: 0,
                phase: TouchPhase::Begin,
                x: 1,
                y: 2,
                pressure: 3,
            },
        )
        .expect("touch"),
        InputEvent::new(1, InputEventKind::Focus { focused: false }).expect("focus"),
    ];

    for event in valid_events {
        let encoded = event.encode().expect("encode event");
        assert!(InputEvent::decode(&encoded[..encoded.len() - 1]).is_err());
        let mut extended = encoded;
        extended.push(0);
        assert!(InputEvent::decode(&extended).is_err());
    }

    let mut pointer_button = valid_events[1].encode().expect("button bytes");
    pointer_button[9] = 0;
    assert!(InputEvent::decode(&pointer_button).is_err());
    pointer_button[9] = PointerButton::Primary as u8;
    pointer_button[10] = 2;
    assert!(InputEvent::decode(&pointer_button).is_err());

    let mut key = valid_events[3].encode().expect("key bytes");
    key[9..11].fill(0);
    assert!(InputEvent::decode(&key).is_err());
    key[9..11].copy_from_slice(&4_u16.to_be_bytes());
    key[12..14].copy_from_slice(&0x8000_u16.to_be_bytes());
    assert!(InputEvent::decode(&key).is_err());

    let mut touch = valid_events[4].encode().expect("touch bytes");
    touch[9] = MAX_TOUCH_CONTACTS;
    assert!(InputEvent::decode(&touch).is_err());
    touch[9] = 0;
    touch[10] = 99;
    assert!(InputEvent::decode(&touch).is_err());

    let mut focus = valid_events[5].encode().expect("focus bytes");
    focus[9] = 2;
    assert!(InputEvent::decode(&focus).is_err());
}

#[test]
fn input_constructor_enforces_key_and_touch_bounds() {
    assert!(
        InputEvent::new(
            0,
            InputEventKind::Key {
                usage: 0,
                state: ButtonState::Pressed,
                modifiers: KeyModifiers::default(),
            }
        )
        .is_err()
    );
    assert!(
        InputEvent::new(
            0,
            InputEventKind::Touch {
                contact_id: MAX_TOUCH_CONTACTS,
                phase: TouchPhase::Begin,
                x: 0,
                y: 0,
                pressure: 0,
            }
        )
        .is_err()
    );
    assert!(KeyModifiers::from_bits(0x8000).is_err());
}

#[test]
fn telemetry_round_trips_all_metrics_in_network_byte_order() {
    let timings = StageTimings::new(10, 20, 30, 40, 50).expect("valid timings");
    let telemetry = Telemetry::new(
        0x0102_0304_0506_0708,
        0x1112_1314_1516_1718,
        timings,
        7,
        125_000,
        8,
        9,
        ThermalState::Fair,
    )
    .expect("valid telemetry");
    let encoded = telemetry.encode().expect("encode telemetry");

    assert_eq!(encoded.len(), 51);
    assert_eq!(&encoded[0..8], &0x0102_0304_0506_0708_u64.to_be_bytes());
    assert_eq!(&encoded[8..16], &0x1112_1314_1516_1718_u64.to_be_bytes());
    assert_eq!(&encoded[16..20], &10_u32.to_be_bytes());
    assert_eq!(&encoded[32..36], &50_u32.to_be_bytes());
    assert_eq!(&encoded[36..38], &7_u16.to_be_bytes());
    assert_eq!(&encoded[38..42], &125_000_u32.to_be_bytes());
    assert_eq!(encoded[50], ThermalState::Fair as u8);
    assert_typed_round_trip(&telemetry);
}

#[test]
fn telemetry_rejects_wrong_lengths_and_out_of_range_metrics() {
    assert!(StageTimings::new(MAX_STAGE_DURATION_MICROS + 1, 0, 0, 0, 0).is_err());
    let timings = StageTimings::new(1, 2, 3, 4, 5).expect("valid timings");
    assert!(
        Telemetry::new(
            1,
            2,
            timings,
            MAX_TELEMETRY_QUEUE_DEPTH + 1,
            0,
            0,
            0,
            ThermalState::Nominal,
        )
        .is_err()
    );
    assert!(
        Telemetry::new(
            1,
            2,
            timings,
            0,
            MAX_LOSS_PARTS_PER_MILLION + 1,
            0,
            0,
            ThermalState::Nominal,
        )
        .is_err()
    );

    let valid = Telemetry::new(1, 2, timings, 3, 4, 5, 6, ThermalState::Nominal)
        .expect("valid telemetry")
        .encode()
        .expect("telemetry bytes");
    assert!(Telemetry::decode(&valid[..50]).is_err());
    let mut extended = valid.clone();
    extended.push(0);
    assert!(Telemetry::decode(&extended).is_err());

    let mut long_stage = valid.clone();
    long_stage[16..20].copy_from_slice(&(MAX_STAGE_DURATION_MICROS + 1).to_be_bytes());
    assert!(Telemetry::decode(&long_stage).is_err());

    let mut deep_queue = valid.clone();
    deep_queue[36..38].copy_from_slice(&(MAX_TELEMETRY_QUEUE_DEPTH + 1).to_be_bytes());
    assert!(Telemetry::decode(&deep_queue).is_err());

    let mut excess_loss = valid.clone();
    excess_loss[38..42].copy_from_slice(&(MAX_LOSS_PARTS_PER_MILLION + 1).to_be_bytes());
    assert!(Telemetry::decode(&excess_loss).is_err());

    let mut unknown_thermal = valid;
    unknown_thermal[50] = 99;
    assert!(Telemetry::decode(&unknown_thermal).is_err());
}

#[test]
fn ping_and_pong_round_trip_with_ntp_style_timestamps() {
    let request = Ping::new(0x0102_0304_0506_0708, 0x1112_1314_1516_1718);
    let request_bytes = request.encode().expect("encode ping");
    assert_eq!(
        &request_bytes[0..8],
        &0x0102_0304_0506_0708_u64.to_be_bytes()
    );
    assert_eq!(
        &request_bytes[8..16],
        &0x1112_1314_1516_1718_u64.to_be_bytes()
    );
    assert_typed_round_trip(&request);

    let response = Pong::new(
        request.token(),
        request.client_send_timestamp_micros(),
        0x2122_2324_2526_2728,
        0x3132_3334_3536_3738,
    )
    .expect("valid pong");
    let response_bytes = response.encode().expect("encode pong");
    assert_eq!(response_bytes.len(), 32);
    assert_eq!(
        &response_bytes[16..24],
        &0x2122_2324_2526_2728_u64.to_be_bytes()
    );
    assert_eq!(
        &response_bytes[24..32],
        &0x3132_3334_3536_3738_u64.to_be_bytes()
    );
    assert_typed_round_trip(&response);
}

#[test]
fn ping_and_pong_reject_wrong_lengths_and_reversed_server_timestamps() {
    assert!(Ping::decode(&[0; 15]).is_err());
    assert!(Ping::decode(&[0; 17]).is_err());
    assert!(Pong::decode(&[0; 31]).is_err());
    assert!(Pong::decode(&[0; 33]).is_err());
    assert!(Pong::new(1, 2, 4, 3).is_err());

    let mut reversed = Pong::new(1, 2, 3, 4)
        .expect("valid pong")
        .encode()
        .expect("pong bytes");
    reversed[16..24].copy_from_slice(&5_u64.to_be_bytes());
    assert!(Pong::decode(&reversed).is_err());
}

#[test]
fn error_message_round_trips_bounded_utf8_and_empty_diagnostics() {
    let error = ErrorMessage::new(ErrorCode::DecoderFailure, true, "解码器 needs a keyframe")
        .expect("valid error");
    let encoded = error.encode().expect("encode error");

    assert_eq!(
        &encoded[0..2],
        &(ErrorCode::DecoderFailure as u16).to_be_bytes()
    );
    assert_eq!(encoded[2], 1);
    assert_eq!(
        &encoded[3..5],
        &u16::try_from(error.diagnostic().len())
            .expect("bounded diagnostic")
            .to_be_bytes()
    );
    assert_eq!(&encoded[5..], error.diagnostic().as_bytes());
    assert_typed_round_trip(&error);

    let empty = ErrorMessage::new(ErrorCode::Busy, false, "").expect("empty is valid");
    assert_eq!(empty.encode().expect("encode empty").len(), 5);
    assert_typed_round_trip(&empty);

    assert!(
        ErrorMessage::new(
            ErrorCode::Internal,
            false,
            "x".repeat(MAX_ERROR_DIAGNOSTIC_BYTES)
        )
        .is_ok()
    );
}

#[test]
fn error_message_rejects_malformed_or_oversized_diagnostics() {
    assert!(
        ErrorMessage::new(
            ErrorCode::Internal,
            false,
            "x".repeat(MAX_ERROR_DIAGNOSTIC_BYTES + 1)
        )
        .is_err()
    );
    assert!(ErrorMessage::new(ErrorCode::Internal, false, "bad\0text").is_err());
    assert!(ErrorMessage::decode(&[0; 4]).is_err());

    let valid = ErrorMessage::new(ErrorCode::Internal, false, "failure")
        .expect("valid error")
        .encode()
        .expect("error bytes");

    let mut unknown_code = valid.clone();
    unknown_code[0..2].copy_from_slice(&99_u16.to_be_bytes());
    assert!(ErrorMessage::decode(&unknown_code).is_err());

    let mut invalid_bool = valid.clone();
    invalid_bool[2] = 2;
    assert!(ErrorMessage::decode(&invalid_bool).is_err());

    let mut wrong_length = valid.clone();
    wrong_length[3..5].copy_from_slice(&100_u16.to_be_bytes());
    assert!(ErrorMessage::decode(&wrong_length).is_err());

    let mut trailing = valid.clone();
    trailing.push(0);
    assert!(ErrorMessage::decode(&trailing).is_err());

    let mut invalid_utf8 = ErrorMessage::new(ErrorCode::Internal, false, "x")
        .expect("valid error")
        .encode()
        .expect("error bytes");
    invalid_utf8[5] = 0xff;
    assert_eq!(
        ErrorMessage::decode(&invalid_utf8),
        Err(ProtocolError::InvalidUtf8)
    );

    let mut null_diagnostic = invalid_utf8;
    null_diagnostic[5] = 0;
    assert!(ErrorMessage::decode(&null_diagnostic).is_err());

    let oversized_len = u16::try_from(MAX_ERROR_DIAGNOSTIC_BYTES + 1).expect("bound fits u16");
    let mut oversized = vec![0; 5 + usize::from(oversized_len)];
    oversized[0..2].copy_from_slice(&(ErrorCode::Internal as u16).to_be_bytes());
    oversized[3..5].copy_from_slice(&oversized_len.to_be_bytes());
    assert!(ErrorMessage::decode(&oversized).is_err());
}
