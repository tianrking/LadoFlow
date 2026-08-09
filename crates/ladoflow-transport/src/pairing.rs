use std::{
    error::Error,
    fmt,
    io::{self, Read, Write},
    net::TcpStream,
    str::FromStr,
    time::Duration,
};

use hmac::{Hmac, KeyInit, Mac};
use sha2::Sha256;
use zeroize::{Zeroize, ZeroizeOnDrop};

/// Version of the fixed-size USB-tether pairing preface.
pub const TETHER_PAIRING_VERSION: u16 = 1;
/// Bytes in every pairing preface record.
pub const TETHER_PAIRING_RECORD_LEN: usize = 56;
/// User-entered Crockford Base32 symbols carrying 80 bits of entropy.
pub const TETHER_PAIRING_TOKEN_SYMBOLS: usize = 16;

const PAIRING_MAGIC: [u8; 4] = *b"LDFP";
const PAIRING_NONCE_LEN: usize = 16;
const PAIRING_TAG_LEN: usize = 32;
const PAIRING_TOKEN_BYTES: usize = 10;
const PAIRING_CONTEXT: &[u8] = b"LadoFlow USB tether pairing v1\0";
const CROCKFORD_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

type HmacSha256 = Hmac<Sha256>;

/// Short-lived high-entropy secret shown by the Android display and entered
/// on the host. Debug output is always redacted and memory is cleared on drop.
#[derive(PartialEq, Eq, Zeroize, ZeroizeOnDrop)]
pub struct TetherPairingToken([u8; PAIRING_TOKEN_BYTES]);

impl TetherPairingToken {
    /// Construct a token from exactly 80 random bits.
    #[must_use]
    pub const fn from_bytes(bytes: [u8; PAIRING_TOKEN_BYTES]) -> Self {
        Self(bytes)
    }

    /// Render four groups of four Crockford Base32 symbols for user entry.
    ///
    /// The returned string is secret material. Do not log or persist it.
    #[must_use]
    pub fn expose_grouped(&self) -> String {
        let mut output = String::with_capacity(TETHER_PAIRING_TOKEN_SYMBOLS + 3);
        let mut accumulator = 0_u32;
        let mut bits = 0_u8;
        let mut symbols = 0_usize;
        for byte in self.0 {
            accumulator = (accumulator << 8) | u32::from(byte);
            bits += 8;
            while bits >= 5 {
                bits -= 5;
                if symbols > 0 && symbols % 4 == 0 {
                    output.push('-');
                }
                let index =
                    usize::from(u8::try_from((accumulator >> bits) & 0x1f).unwrap_or_default());
                output.push(char::from(CROCKFORD_ALPHABET[index]));
                symbols += 1;
                accumulator &= if bits == 0 { 0 } else { (1 << bits) - 1 };
            }
        }
        debug_assert_eq!(bits, 0);
        debug_assert_eq!(symbols, TETHER_PAIRING_TOKEN_SYMBOLS);
        output
    }

    fn as_bytes(&self) -> &[u8; PAIRING_TOKEN_BYTES] {
        &self.0
    }
}

impl fmt::Debug for TetherPairingToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("TetherPairingToken([REDACTED])")
    }
}

impl FromStr for TetherPairingToken {
    type Err = TetherPairingTokenError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let symbols = value
            .chars()
            .filter(|character| !matches!(character, '-' | ' '))
            .collect::<Vec<_>>();
        if symbols.len() != TETHER_PAIRING_TOKEN_SYMBOLS {
            return Err(TetherPairingTokenError::WrongSymbolCount {
                actual: symbols.len(),
            });
        }

        let mut output = [0_u8; PAIRING_TOKEN_BYTES];
        let mut accumulator = 0_u32;
        let mut bits = 0_u8;
        let mut output_index = 0_usize;
        for character in symbols {
            let value = decode_crockford(character)
                .ok_or(TetherPairingTokenError::InvalidSymbol(character))?;
            accumulator = (accumulator << 5) | u32::from(value);
            bits += 5;
            if bits >= 8 {
                bits -= 8;
                output[output_index] =
                    u8::try_from((accumulator >> bits) & 0xff).expect("eight-bit value fits u8");
                output_index += 1;
                accumulator &= if bits == 0 { 0 } else { (1 << bits) - 1 };
            }
        }
        debug_assert_eq!(bits, 0);
        debug_assert_eq!(output_index, PAIRING_TOKEN_BYTES);
        Ok(Self(output))
    }
}

/// Invalid user-facing tether pairing token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TetherPairingTokenError {
    /// The normalized token must contain exactly sixteen symbols.
    WrongSymbolCount {
        /// Number of non-separator symbols supplied.
        actual: usize,
    },
    /// A symbol is not part of Crockford Base32 or its input aliases.
    InvalidSymbol(char),
}

impl fmt::Display for TetherPairingTokenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::WrongSymbolCount { actual } => write!(
                formatter,
                "pairing token must contain {TETHER_PAIRING_TOKEN_SYMBOLS} symbols, got {actual}"
            ),
            Self::InvalidSymbol(symbol) => {
                write!(
                    formatter,
                    "pairing token contains invalid symbol `{symbol}`"
                )
            }
        }
    }
}

impl Error for TetherPairingTokenError {}

/// Failure while authenticating a connected USB-tether TCP stream.
#[derive(Debug)]
pub enum TetherPairingError {
    /// The pairing timeout must be nonzero.
    ZeroTimeout,
    /// Secure operating-system randomness was unavailable.
    Random(String),
    /// Socket I/O failed or exceeded the configured timeout.
    Io(io::Error),
    /// The record did not start with the LDFP magic.
    BadMagic,
    /// The peer uses a different pairing-preface version.
    UnsupportedVersion(u16),
    /// The peer sent a valid record at the wrong handshake step.
    UnexpectedKind {
        /// Record kind required at this step.
        expected: u8,
        /// Record kind received from the peer.
        actual: u8,
    },
    /// Reserved, nonce, or tag bytes were not canonical for the record kind.
    NonCanonicalRecord,
    /// The peer did not prove possession of the same pairing token.
    AuthenticationFailed,
}

impl fmt::Display for TetherPairingError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroTimeout => formatter.write_str("tether pairing timeout must be nonzero"),
            Self::Random(detail) => write!(formatter, "secure pairing randomness failed: {detail}"),
            Self::Io(error) => write!(formatter, "tether pairing I/O failed: {error}"),
            Self::BadMagic => formatter.write_str("peer sent invalid tether pairing magic"),
            Self::UnsupportedVersion(version) => write!(
                formatter,
                "peer uses unsupported tether pairing version {version}"
            ),
            Self::UnexpectedKind { expected, actual } => write!(
                formatter,
                "peer sent tether pairing record kind {actual}, expected {expected}"
            ),
            Self::NonCanonicalRecord => {
                formatter.write_str("peer sent a non-canonical tether pairing record")
            }
            Self::AuthenticationFailed => {
                formatter.write_str("tether pairing authentication failed")
            }
        }
    }
}

impl Error for TetherPairingError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for TetherPairingError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
enum PairingRecordKind {
    HostHello = 1,
    DisplayHello = 2,
    HostFinished = 3,
    DisplayFinished = 4,
}

impl PairingRecordKind {
    const fn value(self) -> u8 {
        self as u8
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PairingRecord {
    kind: u8,
    nonce: [u8; PAIRING_NONCE_LEN],
    tag: [u8; PAIRING_TAG_LEN],
}

/// Authenticate the host side of a connected Android USB-tether socket.
///
/// The token never crosses the socket. Four fixed-size records bind both roles
/// and fresh 128-bit nonces into HMAC-SHA-256 proofs. On success, socket
/// timeouts are cleared and the stream is ready for raw LDFL framing.
///
/// # Errors
///
/// Returns an error for timeout/I/O failure, malformed records, incompatible
/// versions, or a peer that does not hold the same token.
pub fn authenticate_tether_host_stream(
    stream: TcpStream,
    token: &TetherPairingToken,
    timeout: Duration,
) -> Result<TcpStream, TetherPairingError> {
    let host_nonce = generate_nonce()?;
    authenticate_tether_host_stream_with_nonce(stream, token, timeout, host_nonce)
}

/// Authenticate the display side of a connected USB-tether socket.
///
/// Android implements this record layout independently; this Rust mirror keeps
/// cross-role tests deterministic and can support future native display tools.
///
/// # Errors
///
/// Returns the same failures as [`authenticate_tether_host_stream`].
pub fn authenticate_tether_display_stream(
    stream: TcpStream,
    token: &TetherPairingToken,
    timeout: Duration,
) -> Result<TcpStream, TetherPairingError> {
    let display_nonce = generate_nonce()?;
    authenticate_tether_display_stream_with_nonce(stream, token, timeout, display_nonce)
}

fn authenticate_tether_host_stream_with_nonce(
    mut stream: TcpStream,
    token: &TetherPairingToken,
    timeout: Duration,
    host_nonce: [u8; PAIRING_NONCE_LEN],
) -> Result<TcpStream, TetherPairingError> {
    configure_pairing_timeouts(&stream, timeout)?;
    write_record(
        &mut stream,
        PairingRecord {
            kind: PairingRecordKind::HostHello.value(),
            nonce: host_nonce,
            tag: [0; PAIRING_TAG_LEN],
        },
    )?;

    let display_hello = read_record(&mut stream)?;
    validate_record_kind(display_hello, PairingRecordKind::DisplayHello)?;
    if display_hello.nonce == [0; PAIRING_NONCE_LEN] {
        return Err(TetherPairingError::NonCanonicalRecord);
    }
    verify_pairing_tag(
        token,
        PairingRecordKind::DisplayHello,
        &host_nonce,
        &display_hello.nonce,
        &display_hello.tag,
    )?;

    write_record(
        &mut stream,
        finished_record(
            token,
            PairingRecordKind::HostFinished,
            &host_nonce,
            &display_hello.nonce,
        )?,
    )?;
    let display_finished = read_record(&mut stream)?;
    validate_finished_record(
        token,
        display_finished,
        PairingRecordKind::DisplayFinished,
        &host_nonce,
        &display_hello.nonce,
    )?;
    clear_pairing_timeouts(&stream)?;
    Ok(stream)
}

fn authenticate_tether_display_stream_with_nonce(
    mut stream: TcpStream,
    token: &TetherPairingToken,
    timeout: Duration,
    display_nonce: [u8; PAIRING_NONCE_LEN],
) -> Result<TcpStream, TetherPairingError> {
    configure_pairing_timeouts(&stream, timeout)?;
    let host_hello = read_record(&mut stream)?;
    validate_record_kind(host_hello, PairingRecordKind::HostHello)?;
    if host_hello.nonce == [0; PAIRING_NONCE_LEN] || host_hello.tag != [0; PAIRING_TAG_LEN] {
        return Err(TetherPairingError::NonCanonicalRecord);
    }

    write_record(
        &mut stream,
        PairingRecord {
            kind: PairingRecordKind::DisplayHello.value(),
            nonce: display_nonce,
            tag: pairing_tag(
                token,
                PairingRecordKind::DisplayHello,
                &host_hello.nonce,
                &display_nonce,
            )?,
        },
    )?;

    let host_finished = read_record(&mut stream)?;
    validate_finished_record(
        token,
        host_finished,
        PairingRecordKind::HostFinished,
        &host_hello.nonce,
        &display_nonce,
    )?;
    write_record(
        &mut stream,
        finished_record(
            token,
            PairingRecordKind::DisplayFinished,
            &host_hello.nonce,
            &display_nonce,
        )?,
    )?;
    clear_pairing_timeouts(&stream)?;
    Ok(stream)
}

fn configure_pairing_timeouts(
    stream: &TcpStream,
    timeout: Duration,
) -> Result<(), TetherPairingError> {
    if timeout.is_zero() {
        return Err(TetherPairingError::ZeroTimeout);
    }
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    Ok(())
}

fn clear_pairing_timeouts(stream: &TcpStream) -> Result<(), TetherPairingError> {
    stream.set_read_timeout(None)?;
    stream.set_write_timeout(None)?;
    Ok(())
}

fn generate_nonce() -> Result<[u8; PAIRING_NONCE_LEN], TetherPairingError> {
    loop {
        let mut nonce = [0_u8; PAIRING_NONCE_LEN];
        getrandom::fill(&mut nonce)
            .map_err(|error| TetherPairingError::Random(error.to_string()))?;
        if nonce != [0; PAIRING_NONCE_LEN] {
            return Ok(nonce);
        }
    }
}

fn finished_record(
    token: &TetherPairingToken,
    kind: PairingRecordKind,
    host_nonce: &[u8; PAIRING_NONCE_LEN],
    display_nonce: &[u8; PAIRING_NONCE_LEN],
) -> Result<PairingRecord, TetherPairingError> {
    Ok(PairingRecord {
        kind: kind.value(),
        nonce: [0; PAIRING_NONCE_LEN],
        tag: pairing_tag(token, kind, host_nonce, display_nonce)?,
    })
}

fn validate_finished_record(
    token: &TetherPairingToken,
    record: PairingRecord,
    expected: PairingRecordKind,
    host_nonce: &[u8; PAIRING_NONCE_LEN],
    display_nonce: &[u8; PAIRING_NONCE_LEN],
) -> Result<(), TetherPairingError> {
    validate_record_kind(record, expected)?;
    if record.nonce != [0; PAIRING_NONCE_LEN] {
        return Err(TetherPairingError::NonCanonicalRecord);
    }
    verify_pairing_tag(token, expected, host_nonce, display_nonce, &record.tag)
}

fn validate_record_kind(
    record: PairingRecord,
    expected: PairingRecordKind,
) -> Result<(), TetherPairingError> {
    if record.kind == expected.value() {
        Ok(())
    } else {
        Err(TetherPairingError::UnexpectedKind {
            expected: expected.value(),
            actual: record.kind,
        })
    }
}

fn pairing_tag(
    token: &TetherPairingToken,
    kind: PairingRecordKind,
    host_nonce: &[u8; PAIRING_NONCE_LEN],
    display_nonce: &[u8; PAIRING_NONCE_LEN],
) -> Result<[u8; PAIRING_TAG_LEN], TetherPairingError> {
    let mut mac = HmacSha256::new_from_slice(token.as_bytes())
        .map_err(|_error| TetherPairingError::AuthenticationFailed)?;
    mac.update(PAIRING_CONTEXT);
    mac.update(&TETHER_PAIRING_VERSION.to_be_bytes());
    mac.update(&[kind.value()]);
    mac.update(host_nonce);
    mac.update(display_nonce);
    Ok(mac.finalize().into_bytes().into())
}

fn verify_pairing_tag(
    token: &TetherPairingToken,
    kind: PairingRecordKind,
    host_nonce: &[u8; PAIRING_NONCE_LEN],
    display_nonce: &[u8; PAIRING_NONCE_LEN],
    tag: &[u8; PAIRING_TAG_LEN],
) -> Result<(), TetherPairingError> {
    let mut mac = HmacSha256::new_from_slice(token.as_bytes())
        .map_err(|_error| TetherPairingError::AuthenticationFailed)?;
    mac.update(PAIRING_CONTEXT);
    mac.update(&TETHER_PAIRING_VERSION.to_be_bytes());
    mac.update(&[kind.value()]);
    mac.update(host_nonce);
    mac.update(display_nonce);
    mac.verify_slice(tag)
        .map_err(|_error| TetherPairingError::AuthenticationFailed)
}

fn write_record(stream: &mut TcpStream, record: PairingRecord) -> Result<(), TetherPairingError> {
    stream.write_all(&encode_record(record))?;
    Ok(())
}

fn read_record(stream: &mut TcpStream) -> Result<PairingRecord, TetherPairingError> {
    let mut bytes = [0_u8; TETHER_PAIRING_RECORD_LEN];
    stream.read_exact(&mut bytes)?;
    decode_record(&bytes)
}

fn encode_record(record: PairingRecord) -> [u8; TETHER_PAIRING_RECORD_LEN] {
    let mut bytes = [0_u8; TETHER_PAIRING_RECORD_LEN];
    bytes[..4].copy_from_slice(&PAIRING_MAGIC);
    bytes[4..6].copy_from_slice(&TETHER_PAIRING_VERSION.to_be_bytes());
    bytes[6] = record.kind;
    bytes[7] = 0;
    bytes[8..24].copy_from_slice(&record.nonce);
    bytes[24..].copy_from_slice(&record.tag);
    bytes
}

fn decode_record(
    bytes: &[u8; TETHER_PAIRING_RECORD_LEN],
) -> Result<PairingRecord, TetherPairingError> {
    if bytes[..4] != PAIRING_MAGIC {
        return Err(TetherPairingError::BadMagic);
    }
    let version = u16::from_be_bytes([bytes[4], bytes[5]]);
    if version != TETHER_PAIRING_VERSION {
        return Err(TetherPairingError::UnsupportedVersion(version));
    }
    if bytes[7] != 0 {
        return Err(TetherPairingError::NonCanonicalRecord);
    }
    let mut nonce = [0_u8; PAIRING_NONCE_LEN];
    nonce.copy_from_slice(&bytes[8..24]);
    let mut tag = [0_u8; PAIRING_TAG_LEN];
    tag.copy_from_slice(&bytes[24..]);
    Ok(PairingRecord {
        kind: bytes[6],
        nonce,
        tag,
    })
}

fn decode_crockford(character: char) -> Option<u8> {
    let normalized = character.to_ascii_uppercase();
    match normalized {
        'O' => Some(0),
        'I' | 'L' => Some(1),
        _ => CROCKFORD_ALPHABET
            .iter()
            .position(|candidate| char::from(*candidate) == normalized)
            .and_then(|index| u8::try_from(index).ok()),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{Read as _, Write as _},
        net::{TcpListener, TcpStream},
        str::FromStr as _,
        thread,
        time::Duration,
    };

    use super::{
        PAIRING_NONCE_LEN, PairingRecordKind, TetherPairingError, TetherPairingToken,
        authenticate_tether_display_stream_with_nonce, authenticate_tether_host_stream_with_nonce,
        pairing_tag,
    };

    const TIMEOUT: Duration = Duration::from_secs(2);

    fn connected_streams() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind listener");
        let address = listener.local_addr().expect("listener address");
        let connector = thread::spawn(move || TcpStream::connect(address).expect("connect"));
        let (accepted, _peer) = listener.accept().expect("accept");
        (connector.join().expect("connector"), accepted)
    }

    #[test]
    fn crockford_token_round_trips_grouping_case_and_aliases() {
        let token = TetherPairingToken::from_bytes([0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        assert_eq!(token.expose_grouped(), "000G-40R4-0M30-E209");
        assert_eq!(
            TetherPairingToken::from_str("000g 40r4-0m30 e2o9").expect("aliases parse"),
            token
        );
        assert!(TetherPairingToken::from_str("TOO-SHORT").is_err());
        assert!(TetherPairingToken::from_str("000G-40R4-0M30-E20U").is_err());
        assert_eq!(format!("{token:?}"), "TetherPairingToken([REDACTED])");
    }

    #[test]
    fn deterministic_proofs_bind_role_and_both_nonces() {
        let token = TetherPairingToken::from_bytes([0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
        let host_nonce = [0x11; PAIRING_NONCE_LEN];
        let display_nonce = [0x22; PAIRING_NONCE_LEN];
        let display = pairing_tag(
            &token,
            PairingRecordKind::DisplayHello,
            &host_nonce,
            &display_nonce,
        )
        .expect("display proof");
        let host = pairing_tag(
            &token,
            PairingRecordKind::HostFinished,
            &host_nonce,
            &display_nonce,
        )
        .expect("host proof");
        let finished = pairing_tag(
            &token,
            PairingRecordKind::DisplayFinished,
            &host_nonce,
            &display_nonce,
        )
        .expect("display finished proof");
        assert_eq!(
            display,
            [
                0x73, 0xd8, 0xfc, 0xaf, 0xfc, 0x57, 0x5e, 0xf3, 0xfc, 0x87, 0xaf, 0x45, 0xdb, 0x2f,
                0x90, 0x0e, 0x3d, 0x49, 0x7b, 0x2d, 0xef, 0xa9, 0x46, 0xd0, 0x34, 0xf6, 0x76, 0xb6,
                0x73, 0x5d, 0x3d, 0xdc,
            ]
        );
        assert_eq!(
            host,
            [
                0x33, 0xea, 0xee, 0xd1, 0xa5, 0x58, 0x12, 0x21, 0x2c, 0x0a, 0xe4, 0x9c, 0x5b, 0x57,
                0xb1, 0xfe, 0xde, 0x4e, 0x0f, 0xdc, 0xd5, 0x33, 0xd8, 0x0b, 0xc0, 0xd1, 0xac, 0xba,
                0x3f, 0x9d, 0x1e, 0xf5,
            ]
        );
        assert_eq!(
            finished,
            [
                0x6c, 0x11, 0x0e, 0x98, 0x33, 0xd5, 0x7f, 0x9a, 0x19, 0xcf, 0xf9, 0xa2, 0x1a, 0x35,
                0x07, 0xdb, 0xb0, 0x2c, 0xf6, 0xd9, 0x7c, 0xfd, 0x69, 0xbf, 0xc1, 0xf0, 0x43, 0xb6,
                0xf0, 0x8b, 0xad, 0xed,
            ]
        );
    }

    #[test]
    fn four_record_handshake_authenticates_then_preserves_stream_bytes() {
        let (host_stream, display_stream) = connected_streams();
        let host_token = TetherPairingToken::from_bytes([0x5a; 10]);
        let display_token = TetherPairingToken::from_bytes([0x5a; 10]);
        let display = thread::spawn(move || {
            let mut stream = authenticate_tether_display_stream_with_nonce(
                display_stream,
                &display_token,
                TIMEOUT,
                [0x22; PAIRING_NONCE_LEN],
            )
            .expect("display authenticates");
            stream.write_all(b"LDFL follows").expect("write payload");
        });
        let mut stream = authenticate_tether_host_stream_with_nonce(
            host_stream,
            &host_token,
            TIMEOUT,
            [0x11; PAIRING_NONCE_LEN],
        )
        .expect("host authenticates");
        let mut payload = [0_u8; 12];
        stream
            .read_exact(&mut payload)
            .expect("read post-preface bytes");
        assert_eq!(&payload, b"LDFL follows");
        display.join().expect("display thread");
    }

    #[test]
    fn mismatched_token_fails_without_starting_ldfl() {
        let (host_stream, display_stream) = connected_streams();
        let host_token = TetherPairingToken::from_bytes([0x11; 10]);
        let display_token = TetherPairingToken::from_bytes([0x22; 10]);
        let display = thread::spawn(move || {
            authenticate_tether_display_stream_with_nonce(
                display_stream,
                &display_token,
                TIMEOUT,
                [0x44; PAIRING_NONCE_LEN],
            )
        });
        let host_error = authenticate_tether_host_stream_with_nonce(
            host_stream,
            &host_token,
            TIMEOUT,
            [0x33; PAIRING_NONCE_LEN],
        )
        .expect_err("host rejects display proof");
        assert!(matches!(
            host_error,
            TetherPairingError::AuthenticationFailed
        ));
        assert!(display.join().expect("display thread").is_err());
    }
}
