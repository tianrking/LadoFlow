# LDFL TCP transport boundary

LadoFlow uses one TCP byte stream as the driver-independent wired fallback for
Android USB tethering and, later, as the data plane for trusted LAN sessions.
It does not change LDFL framing: the stream is a concatenation of complete LDFL
frames, and channel identity is derived from each frame's message type.

The shared `TcpPacketTransport` now accepts an already connected and
authenticated `TcpStream`. It enables `TCP_NODELAY`, moves socket I/O to a
cancellable nonblocking worker, caps each read/write turn at 64 KiB, and bridges
the socket to the same bounded control/media queues used by USB. The worker:

- restores global LDFL sequence order before writing control and media;
- incrementally decodes arbitrary TCP segmentation and coalescing;
- stops reading while an inbound queue is full so TCP applies backpressure;
- rejects malformed, trailing, wrong-channel, and duplicate-sequence frames;
- reports byte/frame counters and the terminal error;
- closes promptly without waiting on a blocking network call.

Unit tests cover split/coalesced framing, bidirectional control/media delivery,
counter convergence, clean shutdown, and malformed-stream disconnection over
real loopback TCP sockets. This proves the local transport primitive, not a
Windows-to-Android connection.

## Connection ownership

The Android display will listen; the desktop host will connect. That direction
fits USB tethering because Android is normally the tether gateway and avoids
opening a Windows listener or adding an inbound firewall rule. Discovery must
only consider an explicitly selected address or a route proven to belong to a
USB-tether interface; LadoFlow must not scan arbitrary LAN gateways in the
background.

Discovery and consent remain outside `TcpPacketTransport`. Before handing the
socket to the worker, the platform composition layer must:

1. obtain explicit user intent on both devices;
2. connect to the agreed endpoint with a bounded timeout;
3. authenticate a versioned pairing preface and bind it to both fresh nonces;
4. reject reflection, role mismatch, replay, and trailing preface bytes;
5. only then begin the existing LDFL Hello/Capabilities/DisplayConfig exchange.

The tether link can be local and isolated, but that is not an authentication
guarantee. LadoFlow authenticates it with the fixed pairing preface below. This
preface does **not** encrypt subsequent LDFL frames, so it is restricted to the
explicit USB-tether mode. General LAN support must add TLS or an equivalent
authenticated-encryption layer rather than silently reusing this boundary.

## USB-tether endpoint and token

Android listens on TCP port `49231` by default and may report a different port
if the user explicitly overrides it. The desktop connects only after the user
starts the Android foreground display session. On Windows, automatic discovery
is limited to the default gateway of an active adapter whose Plug and Play
ancestry contains a USB device. The UI always permits an explicit numeric
address and port.

Android generates a fresh 80-bit token from the operating-system CSPRNG for
every listening session. It displays the token as four groups of four Crockford
Base32 symbols, for example `000G-40R4-0M30-E209`. Input is case-insensitive,
hyphens and ASCII spaces are ignored, and `O`, `I`, and `L` are accepted as the
usual `0`, `1`, and `1` aliases. The normalized token is exactly 16 symbols and
decodes to exactly 10 bytes. Encoding consumes consecutive five-bit groups from
the most-significant bit of byte 0 through the least-significant bit of byte 9,
with no padding because 80 is divisible by five. It is secret material: neither
side logs, persists, nor sends it over the socket.

The Android owner of the listening session must enforce product policy around
this primitive: expire the token and close the listener after two minutes, stop
after three failed handshakes, accept only one authenticated host, and
invalidate the token as soon as that host succeeds or the user stops the
session.

## Pairing preface v1

The preface is four 56-byte records. All integer fields are unsigned big-endian;
there is no length prefix or text encoding.

| Offset | Size | Meaning |
| ---: | ---: | --- |
| 0 | 4 | ASCII magic `LDFP` |
| 4 | 2 | version `0x0001` |
| 6 | 1 | record kind |
| 7 | 1 | reserved, must be zero |
| 8 | 16 | nonce |
| 24 | 32 | HMAC-SHA-256 tag |

The ordered exchange is:

1. Host sends kind `1` (`HostHello`) with a fresh nonzero 16-byte host nonce
   and an all-zero tag.
2. Display sends kind `2` (`DisplayHello`) with a fresh nonzero 16-byte display
   nonce and its proof tag.
3. Host verifies that proof, then sends kind `3` (`HostFinished`) with an
   all-zero nonce and its proof tag.
4. Display verifies that proof, then sends kind `4` (`DisplayFinished`) with an
   all-zero nonce and its proof tag. The host verifies it before either side
   exposes the stream to LDFL.

Each proof is:

```text
HMAC-SHA-256(
  key = raw 10-byte token,
  message = "LadoFlow USB tether pairing v1\0"
          || 0x00 0x01
          || record-kind
          || host-nonce
          || display-nonce
)
```

The role-specific kind prevents reflection, both random nonces prevent reuse of
captured proofs, and the final two records give mutual authentication before
media begins. Implementations use constant-time tag verification, reject zero
hello nonces and every non-canonical field, apply a bounded handshake timeout,
and close immediately on any failure. Attempt limiting remains the listener's
responsibility because the preface operates on one accepted socket.

### Deterministic interoperability vector

Given token bytes `00010203040506070809`, host nonce
`11111111111111111111111111111111`, and display nonce
`22222222222222222222222222222222`, the tags are:

| Kind | Tag |
| ---: | --- |
| 2 | `73d8fcaffc575ef3fc87af45db2f900e3d497b2defa946d034f676b6735d3ddc` |
| 3 | `33eaeed1a55812212c0ae49c5b57b1fede4e0fdcd533d80bc0d1acba3f9d1ef5` |
| 4 | `6c110e9833d57f9a19cff9a21a3507dbb02cf6d97cfd69bfc1f043b6f08baded` |

The Rust tests assert these exact values and independently exercise both roles
over a real loopback socket. Successful pairing leaves the next byte untouched,
so the existing LDFL magic can follow the 224-byte preface immediately.

## Desktop integration status

The desktop now exposes a bounded composition path around the transport
primitive:

- accept a numeric IPv4 address with an optional port and default to `49231`;
- reject public, multicast, unspecified, IPv6, and hostname destinations;
- allow private, carrier-grade NAT, link-local, and loopback IPv4 only;
- cap TCP connection and pairing I/O at three seconds each;
- zeroize the Rust token input and parsed token without logging either value;
- retain the authenticated socket between **Pair** and **Start** without
  retaining the pairing token;
- move that socket into the existing negotiation, native-capture, H.264,
  input, telemetry, cancellation, and stop lifecycle;
- expose tether state in the desktop snapshot without presenting direct AOA as
  the default Windows path.

The **Find USB tether** action performs read-only Windows discovery. It
enumerates present network devices with SetupAPI, reads their
`NetCfgInstanceId`, walks each Config Manager parent chain with a strict depth
bound, and retains only devices with a `USB\\` ancestor. It then intersects
those adapter GUIDs with active IPv4 adapters and their reported gateways from
`GetAdaptersAddresses`. Only private, carrier-grade NAT, or link-local gateway
addresses are offered; loopback and public addresses are excluded. Enumeration
and result counts are bounded, and discovery never opens a socket, scans a
subnet, or probes a port. The user still confirms the candidate against the
address shown by Android before the authenticated pairing command connects.

The desktop UI uses a password input with autocomplete disabled, clears it
before awaiting the native command, and keeps direct Android Open Accessory
mode explicitly labelled experimental. A normal-browser preview uses read-only
sample status and never attempts native commands; the packaged Tauri app always
uses the real Rust snapshot.

Loopback socket tests prove mutual authentication and confirm that a mismatched
token is rejected without echoing it in the returned error. This is still not a
claim of physical Windows-to-Android cable interoperability.

## Remaining product work

- implement the Android listener as a foreground, user-visible display action;
- add a session-bound resumption policy before automatic tether reconnect;
- prove Windows-to-Android LDFL negotiation over a physical tether cable;
- record sustained bitrate, frame pacing, latency, cable removal, and recovery;
- add authenticated encryption before general LAN discovery is enabled.
