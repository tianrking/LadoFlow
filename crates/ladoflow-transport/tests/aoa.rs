use std::{collections::VecDeque, fmt, time::Duration};

use ladoflow_transport::{
    AOA_CONTROL_READ_TYPE, AOA_CONTROL_TIMEOUT, AOA_CONTROL_WRITE_TYPE, AOA_GET_PROTOCOL,
    AOA_SEND_IDENTIFICATION, AOA_START_ACCESSORY, AccessoryControlIo, AccessoryIdentity,
    AoaNegotiationError, is_aoa_app_accessory, negotiate_accessory_mode,
};

#[derive(Debug, Clone, PartialEq, Eq)]
struct FakeError(&'static str);

impl fmt::Display for FakeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for FakeError {}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Write {
    request_type: u8,
    request: u8,
    value: u16,
    index: u16,
    bytes: Vec<u8>,
    timeout: Duration,
}

struct FakeControl {
    protocol: VecDeque<Result<Vec<u8>, FakeError>>,
    writes: Vec<Write>,
    short_write: Option<usize>,
}

impl FakeControl {
    fn with_protocol(bytes: &[u8]) -> Self {
        Self {
            protocol: VecDeque::from([Ok(bytes.to_vec())]),
            writes: Vec::new(),
            short_write: None,
        }
    }
}

impl AccessoryControlIo for FakeControl {
    type Error = FakeError;

    fn read_control(
        &mut self,
        request_type: u8,
        request: u8,
        value: u16,
        index: u16,
        buffer: &mut [u8],
        timeout: Duration,
    ) -> Result<usize, Self::Error> {
        assert_eq!(request_type, AOA_CONTROL_READ_TYPE);
        assert_eq!(request, AOA_GET_PROTOCOL);
        assert_eq!((value, index), (0, 0));
        assert_eq!(timeout, AOA_CONTROL_TIMEOUT);
        let response = self.protocol.pop_front().expect("one protocol response")?;
        let count = response.len().min(buffer.len());
        buffer[..count].copy_from_slice(&response[..count]);
        Ok(count)
    }

    fn write_control(
        &mut self,
        request_type: u8,
        request: u8,
        value: u16,
        index: u16,
        buffer: &[u8],
        timeout: Duration,
    ) -> Result<usize, Self::Error> {
        self.writes.push(Write {
            request_type,
            request,
            value,
            index,
            bytes: buffer.to_vec(),
            timeout,
        });
        Ok(self.short_write.unwrap_or(buffer.len()))
    }
}

fn identity() -> AccessoryIdentity {
    AccessoryIdentity::ladoflow("Studio workstation", "0.1.0", "host-123").expect("valid identity")
}

#[test]
fn aoa2_negotiation_sends_all_identity_fields_then_starts() {
    let mut control = FakeControl::with_protocol(&2_u16.to_le_bytes());
    let protocol = negotiate_accessory_mode(&mut control, &identity()).expect("AOA negotiation");

    assert_eq!(protocol.get(), 2);
    assert!(protocol.supports_aoa2());
    assert_eq!(control.writes.len(), 7);
    for (expected_index, write) in control.writes[..6].iter().enumerate() {
        assert_eq!(write.request_type, AOA_CONTROL_WRITE_TYPE);
        assert_eq!(write.request, AOA_SEND_IDENTIFICATION);
        assert_eq!(write.value, 0);
        assert_eq!(usize::from(write.index), expected_index);
        assert_eq!(write.bytes.last(), Some(&0));
        assert_eq!(write.timeout, AOA_CONTROL_TIMEOUT);
    }
    assert_eq!(control.writes[0].bytes, b"LadoFlow\0");
    assert_eq!(control.writes[1].bytes, b"LadoFlow Host\0");
    assert_eq!(
        control.writes[4].bytes,
        b"https://github.com/tianrking/LadoFlow\0"
    );
    let start = control.writes.last().expect("start request");
    assert_eq!(start.request, AOA_START_ACCESSORY);
    assert!(start.bytes.is_empty());
}

#[test]
fn invalid_protocol_and_short_writes_fail_closed() {
    let mut truncated = FakeControl::with_protocol(&[2]);
    assert!(matches!(
        negotiate_accessory_mode(&mut truncated, &identity()),
        Err(AoaNegotiationError::InvalidProtocolResponseLength(1))
    ));

    let mut unsupported = FakeControl::with_protocol(&0_u16.to_le_bytes());
    assert!(matches!(
        negotiate_accessory_mode(&mut unsupported, &identity()),
        Err(AoaNegotiationError::UnsupportedProtocol)
    ));

    let mut short = FakeControl::with_protocol(&2_u16.to_le_bytes());
    short.short_write = Some(1);
    assert!(matches!(
        negotiate_accessory_mode(&mut short, &identity()),
        Err(AoaNegotiationError::ShortIdentificationWrite {
            field: "manufacturer",
            ..
        })
    ));
}

#[test]
fn identity_bounds_and_accessory_ids_match_android_contract() {
    assert!(AccessoryIdentity::ladoflow("", "", "").is_ok());
    assert!(AccessoryIdentity::new("", "model", "", "", "", "").is_err());
    assert!(AccessoryIdentity::new("maker", "bad\0model", "", "", "", "").is_err());
    assert!(AccessoryIdentity::new("maker", "x".repeat(256), "", "", "", "").is_err());

    for product in [0x2d00, 0x2d01, 0x2d04, 0x2d05] {
        assert!(is_aoa_app_accessory(0x18d1, product));
    }
    assert!(!is_aoa_app_accessory(0x18d1, 0x2d02));
    assert!(!is_aoa_app_accessory(0x1234, 0x2d00));
}
