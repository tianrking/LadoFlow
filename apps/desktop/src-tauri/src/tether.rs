use std::{
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream},
    str::FromStr,
    time::Duration,
};

use ladoflow_transport::{TcpPacketTransport, TetherPairingToken, authenticate_tether_host_stream};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

pub const DEFAULT_TETHER_PORT: u16 = 49_231;
const TETHER_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
const TETHER_PAIRING_TIMEOUT: Duration = Duration::from_secs(3);

/// User-provided endpoint and one-time pairing token.
///
/// This type intentionally does not implement `Debug` so diagnostics cannot
/// accidentally render the secret field.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TetherPairingRequest {
    endpoint: String,
    token: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TetherPairingReport {
    endpoint: String,
    detail: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TetherEndpointCandidate {
    pub(crate) endpoint: String,
    pub(crate) adapter_name: String,
    pub(crate) gateway: String,
    pub(crate) evidence: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TetherDiscoveryReport {
    pub(crate) candidates: Vec<TetherEndpointCandidate>,
    pub(crate) detail: String,
}

// The cross-platform Tauri command deliberately keeps one fallible signature:
// Windows performs fallible SetupAPI/IP Helper work, while other hosts return
// a successful manual-entry report without touching platform APIs.
#[cfg_attr(not(target_os = "windows"), allow(clippy::unnecessary_wraps))]
pub fn discover_tether_endpoints() -> Result<TetherDiscoveryReport, String> {
    #[cfg(target_os = "windows")]
    {
        crate::platform::discover_tether_endpoints()
    }
    #[cfg(not(target_os = "windows"))]
    {
        Ok(TetherDiscoveryReport {
            candidates: Vec::new(),
            detail: "Automatic USB-tether discovery is currently available on Windows; enter the Android address manually on this platform."
                .to_owned(),
        })
    }
}

#[derive(Debug)]
pub struct TetherConnection {
    endpoint: SocketAddr,
    transport: TcpPacketTransport,
}

impl TetherConnection {
    pub const fn endpoint(&self) -> SocketAddr {
        self.endpoint
    }

    pub const fn transport(&self) -> &TcpPacketTransport {
        &self.transport
    }

    pub fn into_transport(self) -> TcpPacketTransport {
        self.transport
    }
}

/// Connect and authenticate a user-approved USB-tether endpoint.
///
/// # Errors
///
/// Returns an error when the endpoint is outside the local-only address
/// boundary, the token is malformed, the bounded TCP connection fails, the
/// peer cannot prove the same token, or the packet worker cannot start.
pub fn pair_tether_connection(
    request: TetherPairingRequest,
) -> Result<(TetherConnection, TetherPairingReport), String> {
    let endpoint = parse_tether_endpoint(&request.endpoint)?;
    let token_text = Zeroizing::new(request.token);
    let token = TetherPairingToken::from_str(token_text.trim())
        .map_err(|error| format!("invalid USB-tether pairing code: {error}"))?;
    let stream = TcpStream::connect_timeout(&endpoint, TETHER_CONNECT_TIMEOUT)
        .map_err(|error| format!("could not connect to Android at {endpoint}: {error}"))?;
    let stream = authenticate_tether_host_stream(stream, &token, TETHER_PAIRING_TIMEOUT)
        .map_err(|error| format!("Android USB-tether authentication failed: {error}"))?;
    drop(token);
    drop(token_text);

    let transport = TcpPacketTransport::from_authenticated_stream(stream)?;
    let report = TetherPairingReport {
        endpoint: endpoint.to_string(),
        detail: format!(
            "Authenticated {endpoint} with the local USB-tether pairing preface. The socket is ready for LDFL."
        ),
    };
    Ok((
        TetherConnection {
            endpoint,
            transport,
        },
        report,
    ))
}

fn parse_tether_endpoint(value: &str) -> Result<SocketAddr, String> {
    let normalized = value.trim();
    if normalized.is_empty() {
        return Err("enter the Android USB-tether IPv4 address".to_owned());
    }

    let endpoint = normalized
        .parse::<SocketAddr>()
        .or_else(|_socket_error| {
            normalized
                .parse::<Ipv4Addr>()
                .map(|address| SocketAddr::V4(SocketAddrV4::new(address, DEFAULT_TETHER_PORT)))
        })
        .map_err(|_error| {
            format!(
                "USB-tether endpoint must be a numeric IPv4 address with an optional port, for example 192.168.42.129:{DEFAULT_TETHER_PORT}"
            )
        })?;

    let SocketAddr::V4(endpoint_v4) = endpoint else {
        return Err("USB-tether IPv6 endpoints are not supported in this build".to_owned());
    };
    if endpoint_v4.port() == 0 {
        return Err("USB-tether endpoint port must be between 1 and 65535".to_owned());
    }
    let address = *endpoint_v4.ip();
    if !is_local_tether_address(address) {
        return Err(format!(
            "refusing non-local address {address}; USB-tether mode only accepts private, carrier-grade NAT, link-local, or loopback IPv4 addresses"
        ));
    }
    Ok(SocketAddr::V4(endpoint_v4))
}

pub(crate) fn is_local_tether_address(address: Ipv4Addr) -> bool {
    let [first, second, _third, _fourth] = address.octets();
    let shared_carrier_nat = first == 100 && (64..=127).contains(&second);
    (address.is_private() || address.is_link_local() || address.is_loopback() || shared_carrier_nat)
        && !address.is_unspecified()
        && !address.is_broadcast()
        && !address.is_multicast()
}

#[cfg(test)]
mod tests {
    use std::{net::TcpListener, str::FromStr as _, sync::mpsc, thread, time::Duration};

    use ladoflow_transport::{
        ConnectionState, TetherPairingToken, authenticate_tether_display_stream,
    };

    use super::{
        DEFAULT_TETHER_PORT, TetherPairingRequest, pair_tether_connection, parse_tether_endpoint,
    };

    #[test]
    fn endpoint_defaults_port_and_rejects_public_or_ipv6_addresses() {
        assert_eq!(
            parse_tether_endpoint("192.168.42.129")
                .expect("private address")
                .port(),
            DEFAULT_TETHER_PORT
        );
        assert!(parse_tether_endpoint("8.8.8.8:49231").is_err());
        assert!(parse_tether_endpoint("[::1]:49231").is_err());
        assert!(parse_tether_endpoint("192.168.42.129:0").is_err());
        assert!(parse_tether_endpoint("android.local:49231").is_err());
    }

    #[test]
    fn pairing_connects_only_after_mutual_authentication() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind display listener");
        let endpoint = listener.local_addr().expect("listener endpoint");
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let display = thread::spawn(move || {
            let (stream, _peer) = listener.accept().expect("accept host");
            let token = TetherPairingToken::from_str("000G-40R4-0M30-E209").expect("display token");
            let authenticated =
                authenticate_tether_display_stream(stream, &token, Duration::from_secs(2))
                    .expect("authenticate host");
            release_rx.recv().expect("host inspected transport");
            drop(authenticated);
        });

        let request = TetherPairingRequest {
            endpoint: endpoint.to_string(),
            token: "000g 40r4-0m30 e2o9".to_owned(),
        };
        let (connection, report) = pair_tether_connection(request).expect("pair display");
        assert_eq!(connection.endpoint(), endpoint);
        assert_eq!(connection.transport().status().peer_addr(), endpoint);
        assert_eq!(
            connection.transport().status().state(),
            ConnectionState::Connected
        );
        assert_eq!(report.endpoint, endpoint.to_string());
        release_tx.send(()).expect("release display");
        drop(connection);
        display.join().expect("display worker");
    }

    #[test]
    fn mismatched_token_error_does_not_echo_secret() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind display listener");
        let endpoint = listener.local_addr().expect("listener endpoint");
        let display = thread::spawn(move || {
            let (stream, _peer) = listener.accept().expect("accept host");
            let token = TetherPairingToken::from_str("000G-40R4-0M30-E209").expect("token");
            authenticate_tether_display_stream(stream, &token, Duration::from_secs(2))
        });
        let rejected = pair_tether_connection(TetherPairingRequest {
            endpoint: endpoint.to_string(),
            token: "1111-1111-1111-1111".to_owned(),
        })
        .expect_err("token mismatch must fail");
        assert!(rejected.contains("authentication failed"));
        assert!(!rejected.contains("1111-1111-1111-1111"));
        assert!(display.join().expect("display worker").is_err());
    }

    #[test]
    fn numeric_endpoint_parser_accepts_an_explicit_loopback_port() {
        assert_eq!(
            parse_tether_endpoint("127.0.0.1:50123")
                .expect("loopback endpoint")
                .port(),
            50_123
        );
    }
}
