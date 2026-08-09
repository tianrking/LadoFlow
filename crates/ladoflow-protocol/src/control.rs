use crate::{MessageType, ProtocolError, WirePayload};

const PING_LEN: usize = 16;
const PONG_LEN: usize = 32;
const ERROR_PREFIX_LEN: usize = 5;

/// Maximum UTF-8 byte length of an error diagnostic.
pub const MAX_ERROR_DIAGNOSTIC_BYTES: usize = 1_024;

/// Liveness request carrying an opaque correlation token and send timestamp.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ping {
    token: u64,
    client_send_timestamp_micros: u64,
}

impl Ping {
    /// Construct a liveness request.
    #[must_use]
    pub const fn new(token: u64, client_send_timestamp_micros: u64) -> Self {
        Self {
            token,
            client_send_timestamp_micros,
        }
    }

    /// Opaque value that the responder must echo in its [`Pong`].
    #[must_use]
    pub const fn token(self) -> u64 {
        self.token
    }

    /// Request transmission time in the client's monotonic clock domain.
    #[must_use]
    pub const fn client_send_timestamp_micros(self) -> u64 {
        self.client_send_timestamp_micros
    }
}

impl WirePayload for Ping {
    const KIND: MessageType = MessageType::Ping;

    fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        let mut payload = Vec::with_capacity(PING_LEN);
        payload.extend_from_slice(&self.token.to_be_bytes());
        payload.extend_from_slice(&self.client_send_timestamp_micros.to_be_bytes());
        Ok(payload)
    }

    fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        if payload.len() != PING_LEN {
            return Err(ProtocolError::InvalidPayload(
                "ping payload must be exactly 16 bytes",
            ));
        }
        Ok(Self::new(read_u64(payload, 0), read_u64(payload, 8)))
    }
}

/// Liveness response with the four timestamps needed for clock-offset estimation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Pong {
    token: u64,
    client_send_timestamp_micros: u64,
    server_receive_timestamp_micros: u64,
    server_send_timestamp_micros: u64,
}

impl Pong {
    /// Construct a clock-estimation response.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidPayload`] when the server send time is
    /// earlier than its receive time. Client and server timestamps are not
    /// compared because they belong to different monotonic clocks.
    pub fn new(
        token: u64,
        client_send_timestamp_micros: u64,
        server_receive_timestamp_micros: u64,
        server_send_timestamp_micros: u64,
    ) -> Result<Self, ProtocolError> {
        if server_send_timestamp_micros < server_receive_timestamp_micros {
            return Err(ProtocolError::InvalidPayload(
                "pong server send timestamp precedes receive timestamp",
            ));
        }
        Ok(Self {
            token,
            client_send_timestamp_micros,
            server_receive_timestamp_micros,
            server_send_timestamp_micros,
        })
    }

    /// Correlation token copied from the request.
    #[must_use]
    pub const fn token(self) -> u64 {
        self.token
    }

    /// Request transmission time copied from the request.
    #[must_use]
    pub const fn client_send_timestamp_micros(self) -> u64 {
        self.client_send_timestamp_micros
    }

    /// Request arrival time in the server's monotonic clock domain.
    #[must_use]
    pub const fn server_receive_timestamp_micros(self) -> u64 {
        self.server_receive_timestamp_micros
    }

    /// Response transmission time in the server's monotonic clock domain.
    #[must_use]
    pub const fn server_send_timestamp_micros(self) -> u64 {
        self.server_send_timestamp_micros
    }
}

impl WirePayload for Pong {
    const KIND: MessageType = MessageType::Pong;

    fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        Self::new(
            self.token,
            self.client_send_timestamp_micros,
            self.server_receive_timestamp_micros,
            self.server_send_timestamp_micros,
        )?;

        let mut payload = Vec::with_capacity(PONG_LEN);
        payload.extend_from_slice(&self.token.to_be_bytes());
        payload.extend_from_slice(&self.client_send_timestamp_micros.to_be_bytes());
        payload.extend_from_slice(&self.server_receive_timestamp_micros.to_be_bytes());
        payload.extend_from_slice(&self.server_send_timestamp_micros.to_be_bytes());
        Ok(payload)
    }

    fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        if payload.len() != PONG_LEN {
            return Err(ProtocolError::InvalidPayload(
                "pong payload must be exactly 32 bytes",
            ));
        }
        Self::new(
            read_u64(payload, 0),
            read_u64(payload, 8),
            read_u64(payload, 16),
            read_u64(payload, 24),
        )
    }
}

/// Stable cross-platform error categories carried by an [`ErrorMessage`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum ErrorCode {
    /// A peer sent bytes that violate the negotiated protocol.
    ProtocolViolation = 1,
    /// A requested protocol feature or operation is unsupported.
    Unsupported = 2,
    /// A display or encoder configuration was rejected.
    ConfigurationRejected = 3,
    /// Authentication or local approval failed.
    Unauthorized = 4,
    /// The endpoint is temporarily unable to accept the operation.
    Busy = 5,
    /// Video encoding failed.
    EncoderFailure = 6,
    /// Video decoding failed.
    DecoderFailure = 7,
    /// A syntactically valid input event could not be applied.
    InputRejected = 8,
    /// A bounded resource was exhausted.
    ResourceExhausted = 9,
    /// An uncategorized endpoint failure occurred.
    Internal = 10,
}

impl TryFrom<u16> for ErrorCode {
    type Error = ProtocolError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::ProtocolViolation),
            2 => Ok(Self::Unsupported),
            3 => Ok(Self::ConfigurationRejected),
            4 => Ok(Self::Unauthorized),
            5 => Ok(Self::Busy),
            6 => Ok(Self::EncoderFailure),
            7 => Ok(Self::DecoderFailure),
            8 => Ok(Self::InputRejected),
            9 => Ok(Self::ResourceExhausted),
            10 => Ok(Self::Internal),
            _ => Err(ProtocolError::InvalidPayload("unknown error code")),
        }
    }
}

/// Stable error code, retry hint, and bounded human-readable diagnostic.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ErrorMessage {
    code: ErrorCode,
    retryable: bool,
    diagnostic: String,
}

impl ErrorMessage {
    /// Construct a validated wire error.
    ///
    /// The diagnostic is intended for logs and user support, not machine
    /// control flow. An empty diagnostic is valid.
    ///
    /// # Errors
    ///
    /// Returns [`ProtocolError::InvalidPayload`] when the diagnostic exceeds
    /// [`MAX_ERROR_DIAGNOSTIC_BYTES`] or contains a null byte.
    pub fn new(
        code: ErrorCode,
        retryable: bool,
        diagnostic: impl Into<String>,
    ) -> Result<Self, ProtocolError> {
        let diagnostic = diagnostic.into();
        validate_diagnostic(&diagnostic)?;
        Ok(Self {
            code,
            retryable,
            diagnostic,
        })
    }

    /// Stable programmatic category.
    #[must_use]
    pub const fn code(&self) -> ErrorCode {
        self.code
    }

    /// Whether retrying without changing the request may succeed.
    #[must_use]
    pub const fn retryable(&self) -> bool {
        self.retryable
    }

    /// Bounded UTF-8 diagnostic for logs or user support.
    #[must_use]
    pub fn diagnostic(&self) -> &str {
        &self.diagnostic
    }
}

impl WirePayload for ErrorMessage {
    const KIND: MessageType = MessageType::Error;

    fn encode(&self) -> Result<Vec<u8>, ProtocolError> {
        validate_diagnostic(&self.diagnostic)?;
        let diagnostic = self.diagnostic.as_bytes();
        let diagnostic_len = u16::try_from(diagnostic.len())
            .map_err(|_| ProtocolError::InvalidPayload("error diagnostic is too long"))?;

        let mut payload = Vec::with_capacity(ERROR_PREFIX_LEN + diagnostic.len());
        payload.extend_from_slice(&(self.code as u16).to_be_bytes());
        payload.push(u8::from(self.retryable));
        payload.extend_from_slice(&diagnostic_len.to_be_bytes());
        payload.extend_from_slice(diagnostic);
        Ok(payload)
    }

    fn decode(payload: &[u8]) -> Result<Self, ProtocolError> {
        if payload.len() < ERROR_PREFIX_LEN {
            return Err(ProtocolError::InvalidPayload("error payload is truncated"));
        }
        if payload.len() > ERROR_PREFIX_LEN + MAX_ERROR_DIAGNOSTIC_BYTES {
            return Err(ProtocolError::InvalidPayload(
                "error diagnostic is too long",
            ));
        }

        let code = ErrorCode::try_from(read_u16(payload, 0))?;
        let retryable = decode_bool(payload[2])?;
        let diagnostic_len = usize::from(read_u16(payload, 3));
        let expected_len = ERROR_PREFIX_LEN + diagnostic_len;
        if payload.len() != expected_len {
            return Err(ProtocolError::InvalidPayload(
                "error diagnostic length does not match payload",
            ));
        }
        let diagnostic = String::from_utf8(payload[ERROR_PREFIX_LEN..].to_vec())
            .map_err(|_| ProtocolError::InvalidUtf8)?;
        Self::new(code, retryable, diagnostic)
    }
}

fn validate_diagnostic(diagnostic: &str) -> Result<(), ProtocolError> {
    if diagnostic.len() > MAX_ERROR_DIAGNOSTIC_BYTES {
        Err(ProtocolError::InvalidPayload(
            "error diagnostic is too long",
        ))
    } else if diagnostic.contains('\0') {
        Err(ProtocolError::InvalidPayload(
            "error diagnostic contains a null byte",
        ))
    } else {
        Ok(())
    }
}

const fn decode_bool(value: u8) -> Result<bool, ProtocolError> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(ProtocolError::InvalidPayload(
            "boolean error field must be zero or one",
        )),
    }
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_be_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}
