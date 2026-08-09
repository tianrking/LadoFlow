use std::ops::BitOr;

use crate::{MessageType, ProtocolError};

const HELLO_PREFIX_LEN: usize = 22;
const CAPABILITIES_LEN: usize = 20;

/// Maximum UTF-8 byte length of the implementation name in a hello payload.
pub const MAX_IMPLEMENTATION_NAME_BYTES: usize = 64;

/// Typed payload that can be placed inside a protocol frame.
pub trait WirePayload: Sized {
    /// Frame family required for this payload type.
    const KIND: MessageType;

    /// Encode a validated payload.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError`] when an instance no longer satisfies its wire
    /// invariants.
    fn encode(&self) -> Result<Vec<u8>, ProtocolError>;

    /// Decode and validate a complete payload.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError`] for a truncated, oversized, unknown, or
    /// otherwise invalid payload.
    fn decode(payload: &[u8]) -> Result<Self, ProtocolError>;
}

/// Endpoint role declared during session negotiation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Role {
    /// Computer that owns the source desktop.
    Host = 1,
    /// Mobile device that presents frames and returns input.
    Display = 2,
}

impl TryFrom<u8> for Role {
    type Error = ProtocolError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Host),
            2 => Ok(Self::Display),
            _ => Err(ProtocolError::InvalidPayload("unknown endpoint role")),
        }
    }
}

/// First reliable control payload exchanged by two endpoints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hello {
    min_protocol: u16,
    max_protocol: u16,
    role: Role,
    nonce: [u8; 16],
    implementation_name: String,
}

impl Hello {
    /// Construct a hello payload with a bounded UTF-8 implementation name.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidPayload`] for an empty or oversized
    /// implementation name, a null byte, or an invalid protocol range.
    pub fn new(
        min_protocol: u16,
        max_protocol: u16,
        role: Role,
        nonce: [u8; 16],
        implementation_name: impl Into<String>,
    ) -> Result<Self, ProtocolError> {
        let implementation_name = implementation_name.into();
        validate_protocol_range(min_protocol, max_protocol)?;
        validate_implementation_name(&implementation_name)?;
        Ok(Self {
            min_protocol,
            max_protocol,
            role,
            nonce,
            implementation_name,
        })
    }

    /// Lowest protocol generation this endpoint can negotiate.
    #[must_use]
    pub const fn min_protocol(&self) -> u16 {
        self.min_protocol
    }

    /// Highest protocol generation this endpoint can negotiate.
    #[must_use]
    pub const fn max_protocol(&self) -> u16 {
        self.max_protocol
    }

    /// Whether this endpoint is a host or display.
    #[must_use]
    pub const fn role(&self) -> Role {
        self.role
    }

    /// Per-session random value used by later pairing and replay protection.
    #[must_use]
    pub const fn nonce(&self) -> &[u8; 16] {
        &self.nonce
    }

    /// Human-readable implementation identity for diagnostics.
    #[must_use]
    pub fn implementation_name(&self) -> &str {
        &self.implementation_name
    }
}

impl WirePayload for Hello {
    const KIND: MessageType = MessageType::Hello;

    fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        validate_protocol_range(self.min_protocol, self.max_protocol)?;
        validate_implementation_name(&self.implementation_name)?;
        let name = self.implementation_name.as_bytes();
        let name_len = u8::try_from(name.len())
            .map_err(|_| ProtocolError::InvalidPayload("implementation name is too long"))?;

        let mut payload = Vec::with_capacity(HELLO_PREFIX_LEN + name.len());
        payload.extend_from_slice(&self.min_protocol.to_be_bytes());
        payload.extend_from_slice(&self.max_protocol.to_be_bytes());
        payload.push(self.role as u8);
        payload.push(name_len);
        payload.extend_from_slice(&self.nonce);
        payload.extend_from_slice(name);
        Ok(payload)
    }

    fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        if payload.len() < HELLO_PREFIX_LEN {
            return Err(ProtocolError::InvalidPayload("hello payload is truncated"));
        }

        let min_protocol = u16::from_be_bytes([payload[0], payload[1]]);
        let max_protocol = u16::from_be_bytes([payload[2], payload[3]]);
        let role = Role::try_from(payload[4])?;
        let name_len = usize::from(payload[5]);
        let expected_len = HELLO_PREFIX_LEN + name_len;
        if payload.len() != expected_len {
            return Err(ProtocolError::InvalidPayload(
                "hello implementation-name length does not match payload",
            ));
        }

        let mut nonce = [0_u8; 16];
        nonce.copy_from_slice(&payload[6..22]);
        let implementation_name =
            String::from_utf8(payload[22..].to_vec()).map_err(|_| ProtocolError::InvalidUtf8)?;
        Self::new(min_protocol, max_protocol, role, nonce, implementation_name)
    }
}

/// Supported encoded video formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CodecSet(u16);

impl CodecSet {
    /// H.264/AVC support.
    pub const H264: Self = Self(1 << 0);
    /// H.265/HEVC support.
    pub const HEVC: Self = Self(1 << 1);
    /// AV1 support.
    pub const AV1: Self = Self(1 << 2);

    const KNOWN_MASK: u16 = Self::H264.0 | Self::HEVC.0 | Self::AV1.0;

    /// Decode a codec mask after rejecting future bits.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidPayload`] when an unknown bit is set.
    pub fn from_bits(bits: u16) -> Result<Self, ProtocolError> {
        if bits & !Self::KNOWN_MASK == 0 {
            Ok(Self(bits))
        } else {
            Err(ProtocolError::InvalidPayload("unknown codec capability"))
        }
    }

    /// Numeric mask used on the wire.
    #[must_use]
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Whether the set contains every codec in `other`.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    #[must_use]
    const fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl BitOr for CodecSet {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

/// Input event families a display can return to its host.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct InputCapabilities(u16);

impl InputCapabilities {
    /// Absolute or relative pointer input.
    pub const POINTER: Self = Self(1 << 0);
    /// One or more direct touch contacts.
    pub const TOUCH: Self = Self(1 << 1);
    /// Keyboard input.
    pub const KEYBOARD: Self = Self(1 << 2);

    const KNOWN_MASK: u16 = Self::POINTER.0 | Self::TOUCH.0 | Self::KEYBOARD.0;

    /// Decode an input mask after rejecting future bits.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidPayload`] when an unknown bit is set.
    pub fn from_bits(bits: u16) -> Result<Self, ProtocolError> {
        if bits & !Self::KNOWN_MASK == 0 {
            Ok(Self(bits))
        } else {
            Err(ProtocolError::InvalidPayload("unknown input capability"))
        }
    }

    /// Numeric mask used on the wire.
    #[must_use]
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Whether the set contains every input family in `other`.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for InputCapabilities {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

/// Optional session behavior independent of codecs and input.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct FeatureFlags(u32);

impl FeatureFlags {
    /// Display can rotate without ending the session.
    pub const DYNAMIC_ROTATION: Self = Self(1 << 0);
    /// Display can render a separately transmitted cursor.
    pub const REMOTE_CURSOR: Self = Self(1 << 1);
    /// Endpoint supports audio alongside the display stream.
    pub const AUDIO: Self = Self(1 << 2);

    const KNOWN_MASK: u32 = Self::DYNAMIC_ROTATION.0 | Self::REMOTE_CURSOR.0 | Self::AUDIO.0;

    /// Decode a feature mask after rejecting future bits.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidPayload`] when an unknown bit is set.
    pub fn from_bits(bits: u32) -> Result<Self, ProtocolError> {
        if bits & !Self::KNOWN_MASK == 0 {
            Ok(Self(bits))
        } else {
            Err(ProtocolError::InvalidPayload("unknown feature capability"))
        }
    }

    /// Numeric mask used on the wire.
    #[must_use]
    pub const fn bits(self) -> u32 {
        self.0
    }

    /// Whether the set contains every feature in `other`.
    #[must_use]
    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }
}

impl BitOr for FeatureFlags {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        Self(self.0 | rhs.0)
    }
}

/// Bounded display and interaction capabilities advertised by an endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capabilities {
    max_width: u16,
    max_height: u16,
    max_refresh_millihz: u32,
    max_bitrate_kbps: u32,
    codecs: CodecSet,
    input: InputCapabilities,
    features: FeatureFlags,
}

impl Capabilities {
    /// Construct a validated capability record.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidPayload`] when dimensions, refresh
    /// rate, or bitrate are zero, or when no codec is supported.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        max_width: u16,
        max_height: u16,
        max_refresh_millihz: u32,
        max_bitrate_kbps: u32,
        codecs: CodecSet,
        input: InputCapabilities,
        features: FeatureFlags,
    ) -> Result<Self, ProtocolError> {
        if max_width == 0 || max_height == 0 {
            return Err(ProtocolError::InvalidPayload(
                "maximum display dimensions must be non-zero",
            ));
        }
        if max_refresh_millihz == 0 {
            return Err(ProtocolError::InvalidPayload(
                "maximum refresh rate must be non-zero",
            ));
        }
        if max_bitrate_kbps == 0 {
            return Err(ProtocolError::InvalidPayload(
                "maximum bitrate must be non-zero",
            ));
        }
        if codecs.is_empty() {
            return Err(ProtocolError::InvalidPayload(
                "at least one codec must be supported",
            ));
        }

        Ok(Self {
            max_width,
            max_height,
            max_refresh_millihz,
            max_bitrate_kbps,
            codecs,
            input,
            features,
        })
    }

    /// Largest coded width in pixels.
    #[must_use]
    pub const fn max_width(self) -> u16 {
        self.max_width
    }

    /// Largest coded height in pixels.
    #[must_use]
    pub const fn max_height(self) -> u16 {
        self.max_height
    }

    /// Largest refresh rate in thousandths of a hertz.
    #[must_use]
    pub const fn max_refresh_millihz(self) -> u32 {
        self.max_refresh_millihz
    }

    /// Largest sustained decoder bitrate in kilobits per second.
    #[must_use]
    pub const fn max_bitrate_kbps(self) -> u32 {
        self.max_bitrate_kbps
    }

    /// Supported encoded video formats.
    #[must_use]
    pub const fn codecs(self) -> CodecSet {
        self.codecs
    }

    /// Supported reverse-input event families.
    #[must_use]
    pub const fn input(self) -> InputCapabilities {
        self.input
    }

    /// Supported optional session behavior.
    #[must_use]
    pub const fn features(self) -> FeatureFlags {
        self.features
    }
}

impl WirePayload for Capabilities {
    const KIND: MessageType = MessageType::Capabilities;

    fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        Self::new(
            self.max_width,
            self.max_height,
            self.max_refresh_millihz,
            self.max_bitrate_kbps,
            self.codecs,
            self.input,
            self.features,
        )?;

        let mut payload = Vec::with_capacity(CAPABILITIES_LEN);
        payload.extend_from_slice(&self.max_width.to_be_bytes());
        payload.extend_from_slice(&self.max_height.to_be_bytes());
        payload.extend_from_slice(&self.max_refresh_millihz.to_be_bytes());
        payload.extend_from_slice(&self.max_bitrate_kbps.to_be_bytes());
        payload.extend_from_slice(&self.codecs.bits().to_be_bytes());
        payload.extend_from_slice(&self.input.bits().to_be_bytes());
        payload.extend_from_slice(&self.features.bits().to_be_bytes());
        Ok(payload)
    }

    fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        if payload.len() != CAPABILITIES_LEN {
            return Err(ProtocolError::InvalidPayload(
                "capabilities payload must be exactly 20 bytes",
            ));
        }

        let max_width = u16::from_be_bytes([payload[0], payload[1]]);
        let max_height = u16::from_be_bytes([payload[2], payload[3]]);
        let max_refresh_millihz =
            u32::from_be_bytes([payload[4], payload[5], payload[6], payload[7]]);
        let max_bitrate_kbps =
            u32::from_be_bytes([payload[8], payload[9], payload[10], payload[11]]);
        let codecs = CodecSet::from_bits(u16::from_be_bytes([payload[12], payload[13]]))?;
        let input = InputCapabilities::from_bits(u16::from_be_bytes([payload[14], payload[15]]))?;
        let features = FeatureFlags::from_bits(u32::from_be_bytes([
            payload[16],
            payload[17],
            payload[18],
            payload[19],
        ]))?;

        Self::new(
            max_width,
            max_height,
            max_refresh_millihz,
            max_bitrate_kbps,
            codecs,
            input,
            features,
        )
    }
}

fn validate_protocol_range(min_protocol: u16, max_protocol: u16) -> Result<(), ProtocolError> {
    if min_protocol == 0 {
        Err(ProtocolError::InvalidPayload(
            "minimum protocol version must be non-zero",
        ))
    } else if min_protocol > max_protocol {
        Err(ProtocolError::InvalidPayload(
            "minimum protocol version exceeds maximum",
        ))
    } else {
        Ok(())
    }
}

fn validate_implementation_name(name: &str) -> Result<(), ProtocolError> {
    if name.is_empty() {
        Err(ProtocolError::InvalidPayload(
            "implementation name must not be empty",
        ))
    } else if name.len() > MAX_IMPLEMENTATION_NAME_BYTES {
        Err(ProtocolError::InvalidPayload(
            "implementation name is too long",
        ))
    } else if name.contains('\0') {
        Err(ProtocolError::InvalidPayload(
            "implementation name contains a null byte",
        ))
    } else {
        Ok(())
    }
}
