//! Android Open Accessory (AOA) control-plane negotiation.
//!
//! The byte-stream carried after re-enumeration is still the unmodified LDFL
//! stream. This module owns only endpoint-zero negotiation and deliberately has
//! no dependency on libusb so every host backend can share and test the exact
//! same request sequence.

use std::{error::Error, fmt, time::Duration};

pub const AOA_GOOGLE_VENDOR_ID: u16 = 0x18d1;
pub const AOA_ACCESSORY_PRODUCT_ID: u16 = 0x2d00;
pub const AOA_ACCESSORY_ADB_PRODUCT_ID: u16 = 0x2d01;
pub const AOA_ACCESSORY_AUDIO_PRODUCT_ID: u16 = 0x2d04;
pub const AOA_ACCESSORY_AUDIO_ADB_PRODUCT_ID: u16 = 0x2d05;
pub const AOA_CONTROL_READ_TYPE: u8 = 0xc0;
pub const AOA_CONTROL_WRITE_TYPE: u8 = 0x40;
pub const AOA_GET_PROTOCOL: u8 = 51;
pub const AOA_SEND_IDENTIFICATION: u8 = 52;
pub const AOA_START_ACCESSORY: u8 = 53;
pub const AOA_MAX_IDENTIFICATION_BYTES: usize = 256;
pub const AOA_CONTROL_TIMEOUT: Duration = Duration::from_secs(1);

const MANUFACTURER_INDEX: u16 = 0;
const MODEL_INDEX: u16 = 1;
const DESCRIPTION_INDEX: u16 = 2;
const VERSION_INDEX: u16 = 3;
const URI_INDEX: u16 = 4;
const SERIAL_INDEX: u16 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccessoryIdentity {
    manufacturer: String,
    model: String,
    description: String,
    version: String,
    uri: String,
    serial: String,
}

impl AccessoryIdentity {
    /// Builds and validates the six AOA identification strings.
    ///
    /// # Errors
    ///
    /// Returns [`AccessoryIdentityError`] when a required value is empty, a
    /// value contains an embedded null, or its terminated UTF-8 form exceeds
    /// the AOA 256-byte limit.
    pub fn new(
        manufacturer: impl Into<String>,
        model: impl Into<String>,
        description: impl Into<String>,
        version: impl Into<String>,
        uri: impl Into<String>,
        serial: impl Into<String>,
    ) -> Result<Self, AccessoryIdentityError> {
        let identity = Self {
            manufacturer: manufacturer.into(),
            model: model.into(),
            description: description.into(),
            version: version.into(),
            uri: uri.into(),
            serial: serial.into(),
        };
        for (field, value, required) in identity.fields() {
            validate_identification(field, value, required)?;
        }
        Ok(identity)
    }

    /// Builds the identity expected by the `LadoFlow` Android manifest filter.
    ///
    /// # Errors
    ///
    /// Returns [`AccessoryIdentityError`] when a supplied value cannot be
    /// represented as a valid AOA identification string.
    pub fn ladoflow(
        description: impl Into<String>,
        version: impl Into<String>,
        serial: impl Into<String>,
    ) -> Result<Self, AccessoryIdentityError> {
        Self::new(
            "LadoFlow",
            "LadoFlow Host",
            description,
            version,
            "https://github.com/tianrking/LadoFlow",
            serial,
        )
    }

    fn fields(&self) -> [(&'static str, &str, bool); 6] {
        [
            ("manufacturer", &self.manufacturer, true),
            ("model", &self.model, true),
            ("description", &self.description, false),
            ("version", &self.version, false),
            ("uri", &self.uri, false),
            ("serial", &self.serial, false),
        ]
    }

    fn indexed_fields(&self) -> [(u16, &'static str, &str); 6] {
        [
            (MANUFACTURER_INDEX, "manufacturer", &self.manufacturer),
            (MODEL_INDEX, "model", &self.model),
            (DESCRIPTION_INDEX, "description", &self.description),
            (VERSION_INDEX, "version", &self.version),
            (URI_INDEX, "uri", &self.uri),
            (SERIAL_INDEX, "serial", &self.serial),
        ]
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AccessoryIdentityError {
    MissingRequiredField(&'static str),
    EmbeddedNull(&'static str),
    FieldTooLong {
        field: &'static str,
        encoded_bytes: usize,
    },
}

impl fmt::Display for AccessoryIdentityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingRequiredField(field) => {
                write!(formatter, "AOA {field} must not be empty")
            }
            Self::EmbeddedNull(field) => {
                write!(formatter, "AOA {field} must not contain a null byte")
            }
            Self::FieldTooLong {
                field,
                encoded_bytes,
            } => write!(
                formatter,
                "AOA {field} uses {encoded_bytes} bytes including its terminator; maximum is {AOA_MAX_IDENTIFICATION_BYTES}"
            ),
        }
    }
}

impl Error for AccessoryIdentityError {}

fn validate_identification(
    field: &'static str,
    value: &str,
    required: bool,
) -> Result<(), AccessoryIdentityError> {
    if required && value.is_empty() {
        return Err(AccessoryIdentityError::MissingRequiredField(field));
    }
    if value.as_bytes().contains(&0) {
        return Err(AccessoryIdentityError::EmbeddedNull(field));
    }
    let encoded_bytes = value.len().saturating_add(1);
    if encoded_bytes > AOA_MAX_IDENTIFICATION_BYTES {
        return Err(AccessoryIdentityError::FieldTooLong {
            field,
            encoded_bytes,
        });
    }
    Ok(())
}

pub trait AccessoryControlIo {
    type Error: Error + Send + Sync + 'static;

    /// Executes one endpoint-zero device-to-host vendor request.
    ///
    /// # Errors
    ///
    /// Returns the backend error when the USB control transfer fails.
    fn read_control(
        &mut self,
        request_type: u8,
        request: u8,
        value: u16,
        index: u16,
        buffer: &mut [u8],
        timeout: Duration,
    ) -> Result<usize, Self::Error>;

    /// Executes one endpoint-zero host-to-device vendor request.
    ///
    /// # Errors
    ///
    /// Returns the backend error when the USB control transfer fails.
    fn write_control(
        &mut self,
        request_type: u8,
        request: u8,
        value: u16,
        index: u16,
        buffer: &[u8],
        timeout: Duration,
    ) -> Result<usize, Self::Error>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AoaProtocolVersion(u16);

impl AoaProtocolVersion {
    #[must_use]
    pub const fn get(self) -> u16 {
        self.0
    }

    #[must_use]
    pub const fn supports_aoa2(self) -> bool {
        self.0 >= 2
    }
}

#[derive(Debug)]
pub enum AoaNegotiationError<E> {
    Control {
        stage: &'static str,
        source: E,
    },
    InvalidProtocolResponseLength(usize),
    UnsupportedProtocol,
    ShortIdentificationWrite {
        field: &'static str,
        expected: usize,
        actual: usize,
    },
    UnexpectedStartResponse(usize),
}

impl<E: fmt::Display> fmt::Display for AoaNegotiationError<E> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Control { stage, source } => write!(formatter, "AOA {stage} failed: {source}"),
            Self::InvalidProtocolResponseLength(actual) => write!(
                formatter,
                "AOA protocol response was {actual} bytes; expected exactly 2"
            ),
            Self::UnsupportedProtocol => {
                write!(formatter, "device returned unsupported AOA version 0")
            }
            Self::ShortIdentificationWrite {
                field,
                expected,
                actual,
            } => write!(
                formatter,
                "AOA {field} write transferred {actual} of {expected} bytes"
            ),
            Self::UnexpectedStartResponse(actual) => write!(
                formatter,
                "AOA start request transferred {actual} bytes; expected zero"
            ),
        }
    }
}

impl<E: Error + 'static> Error for AoaNegotiationError<E> {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Control { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// Queries AOA support, sends all six identity strings, and requests mode switch.
///
/// # Errors
///
/// Returns [`AoaNegotiationError`] for transfer failures, malformed protocol
/// replies, unsupported version zero, or any short control write.
pub fn negotiate_accessory_mode<T: AccessoryControlIo>(
    io: &mut T,
    identity: &AccessoryIdentity,
) -> Result<AoaProtocolVersion, AoaNegotiationError<T::Error>> {
    let mut version_bytes = [0_u8; 2];
    let count = io
        .read_control(
            AOA_CONTROL_READ_TYPE,
            AOA_GET_PROTOCOL,
            0,
            0,
            &mut version_bytes,
            AOA_CONTROL_TIMEOUT,
        )
        .map_err(|source| AoaNegotiationError::Control {
            stage: "protocol query",
            source,
        })?;
    if count != version_bytes.len() {
        return Err(AoaNegotiationError::InvalidProtocolResponseLength(count));
    }
    let protocol = u16::from_le_bytes(version_bytes);
    if protocol == 0 {
        return Err(AoaNegotiationError::UnsupportedProtocol);
    }

    for (index, field, value) in identity.indexed_fields() {
        let mut encoded = Vec::with_capacity(value.len() + 1);
        encoded.extend_from_slice(value.as_bytes());
        encoded.push(0);
        let count = io
            .write_control(
                AOA_CONTROL_WRITE_TYPE,
                AOA_SEND_IDENTIFICATION,
                0,
                index,
                &encoded,
                AOA_CONTROL_TIMEOUT,
            )
            .map_err(|source| AoaNegotiationError::Control {
                stage: field,
                source,
            })?;
        if count != encoded.len() {
            return Err(AoaNegotiationError::ShortIdentificationWrite {
                field,
                expected: encoded.len(),
                actual: count,
            });
        }
    }

    let count = io
        .write_control(
            AOA_CONTROL_WRITE_TYPE,
            AOA_START_ACCESSORY,
            0,
            0,
            &[],
            AOA_CONTROL_TIMEOUT,
        )
        .map_err(|source| AoaNegotiationError::Control {
            stage: "accessory start",
            source,
        })?;
    if count != 0 {
        return Err(AoaNegotiationError::UnexpectedStartResponse(count));
    }
    Ok(AoaProtocolVersion(protocol))
}

#[must_use]
pub const fn is_aoa_app_accessory(vendor_id: u16, product_id: u16) -> bool {
    vendor_id == AOA_GOOGLE_VENDOR_ID
        && matches!(
            product_id,
            AOA_ACCESSORY_PRODUCT_ID
                | AOA_ACCESSORY_ADB_PRODUCT_ID
                | AOA_ACCESSORY_AUDIO_PRODUCT_ID
                | AOA_ACCESSORY_AUDIO_ADB_PRODUCT_ID
        )
}
