use std::fmt;

use ladoflow_protocol::{Capabilities, Hello};

/// Capability limits and masks supported by both endpoints.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NegotiatedCapabilities {
    max_width: u16,
    max_height: u16,
    max_refresh_millihz: u32,
    max_bitrate_kbps: u32,
    codec_bits: u16,
    input_bits: u16,
    feature_bits: u32,
}

impl NegotiatedCapabilities {
    /// Largest coded width accepted by both endpoints.
    #[must_use]
    pub const fn max_width(self) -> u16 {
        self.max_width
    }

    /// Largest coded height accepted by both endpoints.
    #[must_use]
    pub const fn max_height(self) -> u16 {
        self.max_height
    }

    /// Largest refresh rate accepted by both endpoints, in millihertz.
    #[must_use]
    pub const fn max_refresh_millihz(self) -> u32 {
        self.max_refresh_millihz
    }

    /// Largest bitrate accepted by both endpoints, in kilobits per second.
    #[must_use]
    pub const fn max_bitrate_kbps(self) -> u32 {
        self.max_bitrate_kbps
    }

    /// Intersection of the protocol codec masks.
    #[must_use]
    pub const fn codec_bits(self) -> u16 {
        self.codec_bits
    }

    /// Intersection of the protocol reverse-input masks.
    #[must_use]
    pub const fn input_bits(self) -> u16 {
        self.input_bits
    }

    /// Intersection of the protocol optional-feature masks.
    #[must_use]
    pub const fn feature_bits(self) -> u32 {
        self.feature_bits
    }
}

/// Result of a successful hello and capability exchange.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NegotiatedSession {
    protocol_version: u16,
    capabilities: NegotiatedCapabilities,
}

impl NegotiatedSession {
    /// Highest protocol version shared by both hello ranges.
    #[must_use]
    pub const fn protocol_version(self) -> u16 {
        self.protocol_version
    }

    /// Limits and masks shared by both capability advertisements.
    #[must_use]
    pub const fn capabilities(self) -> NegotiatedCapabilities {
        self.capabilities
    }
}

/// Reason two endpoint advertisements cannot form a session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NegotiationError {
    /// Both hellos advertise the same endpoint role.
    MatchingRoles,
    /// The advertised protocol ranges do not overlap.
    NoProtocolOverlap,
    /// The capability advertisements share no video codec.
    NoCommonCodec,
}

impl fmt::Display for NegotiationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MatchingRoles => formatter.write_str("both endpoints advertise the same role"),
            Self::NoProtocolOverlap => {
                formatter.write_str("endpoint protocol ranges do not overlap")
            }
            Self::NoCommonCodec => formatter.write_str("endpoints share no video codec"),
        }
    }
}

impl std::error::Error for NegotiationError {}

/// Select the highest shared protocol version and intersect endpoint limits.
///
/// Scalar capability limits use the lower endpoint maximum. Codec, input, and
/// feature masks use bitwise intersection so the result never promises support
/// absent from either advertisement.
///
/// # Errors
///
/// Returns [`NegotiationError`] when roles match, protocol ranges do not
/// overlap, or no video codec is shared.
pub fn negotiate(
    local_hello: &Hello,
    local_capabilities: Capabilities,
    remote_hello: &Hello,
    remote_capabilities: Capabilities,
) -> Result<NegotiatedSession, NegotiationError> {
    if local_hello.role() == remote_hello.role() {
        return Err(NegotiationError::MatchingRoles);
    }

    let lowest_shared_version = local_hello.min_protocol().max(remote_hello.min_protocol());
    let highest_shared_version = local_hello.max_protocol().min(remote_hello.max_protocol());
    if lowest_shared_version > highest_shared_version {
        return Err(NegotiationError::NoProtocolOverlap);
    }

    let codec_bits = local_capabilities.codecs().bits() & remote_capabilities.codecs().bits();
    if codec_bits == 0 {
        return Err(NegotiationError::NoCommonCodec);
    }

    Ok(NegotiatedSession {
        protocol_version: highest_shared_version,
        capabilities: NegotiatedCapabilities {
            max_width: local_capabilities
                .max_width()
                .min(remote_capabilities.max_width()),
            max_height: local_capabilities
                .max_height()
                .min(remote_capabilities.max_height()),
            max_refresh_millihz: local_capabilities
                .max_refresh_millihz()
                .min(remote_capabilities.max_refresh_millihz()),
            max_bitrate_kbps: local_capabilities
                .max_bitrate_kbps()
                .min(remote_capabilities.max_bitrate_kbps()),
            codec_bits,
            input_bits: local_capabilities.input().bits() & remote_capabilities.input().bits(),
            feature_bits: local_capabilities.features().bits()
                & remote_capabilities.features().bits(),
        },
    })
}

#[cfg(test)]
mod tests {
    use ladoflow_protocol::{Capabilities, CodecSet, FeatureFlags, Hello, InputCapabilities, Role};

    use super::{NegotiationError, negotiate};

    fn hello(min_protocol: u16, max_protocol: u16, role: Role) -> Hello {
        Hello::new(
            min_protocol,
            max_protocol,
            role,
            [role as u8; 16],
            "test endpoint",
        )
        .expect("valid test hello")
    }

    fn capabilities(
        width: u16,
        height: u16,
        refresh: u32,
        bitrate: u32,
        codecs: CodecSet,
        input: InputCapabilities,
        features: FeatureFlags,
    ) -> Capabilities {
        Capabilities::new(width, height, refresh, bitrate, codecs, input, features)
            .expect("valid test capabilities")
    }

    #[test]
    fn selects_highest_version_and_intersects_every_capability() {
        let local_hello = hello(1, 4, Role::Host);
        let remote_hello = hello(2, 3, Role::Display);
        let local = capabilities(
            3840,
            2160,
            120_000,
            50_000,
            CodecSet::H264 | CodecSet::HEVC,
            InputCapabilities::POINTER | InputCapabilities::KEYBOARD,
            FeatureFlags::AUDIO | FeatureFlags::REMOTE_CURSOR,
        );
        let remote = capabilities(
            2560,
            1600,
            90_000,
            30_000,
            CodecSet::H264 | CodecSet::AV1,
            InputCapabilities::POINTER | InputCapabilities::TOUCH,
            FeatureFlags::REMOTE_CURSOR | FeatureFlags::DYNAMIC_ROTATION,
        );

        let result =
            negotiate(&local_hello, local, &remote_hello, remote).expect("compatible endpoints");
        let shared = result.capabilities();

        assert_eq!(result.protocol_version(), 3);
        assert_eq!(shared.max_width(), 2560);
        assert_eq!(shared.max_height(), 1600);
        assert_eq!(shared.max_refresh_millihz(), 90_000);
        assert_eq!(shared.max_bitrate_kbps(), 30_000);
        assert_eq!(shared.codec_bits(), CodecSet::H264.bits());
        assert_eq!(shared.input_bits(), InputCapabilities::POINTER.bits());
        assert_eq!(shared.feature_bits(), FeatureFlags::REMOTE_CURSOR.bits());
    }

    #[test]
    fn rejects_matching_roles_disjoint_versions_and_disjoint_codecs() {
        let host = hello(1, 2, Role::Host);
        let other_host = hello(1, 2, Role::Host);
        let display_v3 = hello(3, 4, Role::Display);
        let display_v2 = hello(1, 2, Role::Display);
        let h264 = capabilities(
            1920,
            1080,
            60_000,
            20_000,
            CodecSet::H264,
            InputCapabilities::default(),
            FeatureFlags::default(),
        );
        let av1 = capabilities(
            1920,
            1080,
            60_000,
            20_000,
            CodecSet::AV1,
            InputCapabilities::default(),
            FeatureFlags::default(),
        );

        assert_eq!(
            negotiate(&host, h264, &other_host, h264),
            Err(NegotiationError::MatchingRoles)
        );
        assert_eq!(
            negotiate(&host, h264, &display_v3, h264),
            Err(NegotiationError::NoProtocolOverlap)
        );
        assert_eq!(
            negotiate(&host, h264, &display_v2, av1),
            Err(NegotiationError::NoCommonCodec)
        );
    }
}
