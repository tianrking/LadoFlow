# USB tethering TCP fallback

This is the no-ADB, no-host-driver-replacement wired fallback for systems where
the PC cannot safely send the initial Android Open Accessory control requests.
It does not change LDFL v1. Android listens only after an explicit foreground
action, the PC connects through Android's USB-tether network, and the same TCP
socket carries raw LDFL immediately after pairing.

This is not encrypted LAN transport. The fixed handshake authenticates one
host that can reach the explicitly selected USB-tether interface; it does not
encrypt or provide forward secrecy for the subsequent LDFL stream.

## Product flow and local boundary

1. Connect a USB data cable and enable **USB tethering** in Android system
   settings. Developer options and USB debugging are not required.
2. Keep LadoFlow visible and tap **Use USB tethering fallback**, then
   **Create pairing code**. Lifecycle entry alone never starts the listener.
3. Android finds an active private/link-local IPv4 address only on an interface
   whose kernel name starts with `rndis`, `ncm`, `ecm`, or `usb`. It binds that
   exact address, never `0.0.0.0`. Wi-Fi, cellular, Ethernet, VPN, public,
   wildcard, and loopback addresses are not fallback candidates.
4. Android listens on TCP `49231`. The transport constructor accepts a numeric
   port override for controlled integration, while the product UI currently
   uses the fixed default.
5. Android generates a fresh CSPRNG 10-byte token in memory and displays its
   80 bits as 16 Crockford Base32 characters in four groups:
   `XXXX-XXXX-XXXX-XXXX`. It is never written to preferences, files, logs, or
   diagnostics. Token-bearing objects redact `toString()`.
6. The listener and token expire after two minutes, close after three failed
   handshakes, accept one authenticated host, and invalidate/zero the raw token
   on success, stop, expiry, lockout, bind failure, or backgrounding.
7. Each accepted pairing socket has a 10-second read timeout bounded again by
   the remaining two-minute listener lifetime. Pair and Start are separate Host
   actions, so the authenticated LDFL socket changes to infinite read timeout
   (`SO_TIMEOUT = 0`). Foreground stop, explicit disconnect, process
   backgrounding, or session failure closes the socket to release a blocked
   read; a pre-LDFL idle interval is not treated as a disconnect.
8. A stopped, expired, rejected, or disconnected session requires a new
   explicit pairing code. It starts a fresh LDFL generation; both senders begin
   their independent sequence spaces again. There is no private reconnect
   marker.

Failing to identify a known USB-tether interface is intentionally actionable:
the UI asks the user to enable tethering and retry. OEM kernel interface names
outside the allow-list require a reviewed compatibility addition; the app does
not broaden the listener to other networks as a fallback.

## Exact pairing records

Every pairing record is exactly 56 bytes:

| Offset | Size | Field | Canonical value |
| --- | ---: | --- | --- |
| 0 | 4 | magic | ASCII `LDFP` |
| 4 | 2 | version | big-endian `1` |
| 6 | 1 | kind | `1` through `4` below |
| 7 | 1 | reserved | zero |
| 8 | 16 | nonce | kind-specific |
| 24 | 32 | tag | kind-specific |

The four-record exchange is:

| Sender | Kind | Nonce | Tag |
| --- | ---: | --- | --- |
| Host | `1` HostHello | nonzero host nonce `H` | all zero |
| Display | `2` DisplayHello | nonzero display nonce `D` | HMAC |
| Host | `3` HostFinished | all zero | HMAC |
| Display | `4` DisplayFinished | all zero | HMAC |

The HMAC is SHA-256 with the raw 10 token bytes as key. Its message is the
exact concatenation:

```text
ASCII "LadoFlow USB tether pairing v1"
+ NUL
+ BE u16 1
+ kind u8
+ H[16]
+ D[16]
```

Android rejects unknown or out-of-order kinds, wrong magic/version/reserved,
zero Hello nonces, nonzero Finished nonces, noncanonical zero/nonzero tags,
short records, and an incorrect HostFinished tag. Tag comparison uses
`MessageDigest.isEqual`.

Golden vector with token `00010203040506070809`, `H = 11 * 16`, and
`D = 22 * 16`:

- kind 2: `73d8fcaffc575ef3fc87af45db2f900e3d497b2defa946d034f676b6735d3ddc`;
- kind 3: `33eaeed1a55812212c0ae49c5b57b1fede4e0fdcd533d80bc0d1acba3f9d1ef5`;
- kind 4: `6c110e9833d57f9a19cff9a21a3507dbb02cf6d97cfd69bfc1f043b6f08baded`.

## Transition to LDFL

The pairing code reads exactly 56 bytes at a time directly from the socket
stream and does not wrap it in a prefetching buffer. After DisplayFinished,
ownership of that same socket and its existing input/output streams transfers
to the bounded incremental LDFL I/O session. A Host byte already pipelined
after HostFinished remains unread for the LDFL decoder.

There is no TCP-private header, length, channel, or sequence. Framing, typed
payloads, global sender sequence, negotiation, MediaCodec input, telemetry,
and reverse input remain exactly the LDFL v1 behavior documented elsewhere.

## Automated and physical evidence boundary

JVM tests cover the Windows HMAC vectors, Crockford formatting/redaction,
canonical record corruption, short reads, wrong tags, split reads, the exact
four-record exchange, three-failure lockout, expiry, silent-peer timeout,
single-host listener closure, authenticated `SO_TIMEOUT = 0`, explicit-close
cancellation, and a byte pipelined for LDFL. Android
instrumentation checks manifest network permission and the explicit pairing
UI. These tests do not prove a physical phone's OEM tether interface name,
system tether toggle behavior, host reachability, sustained TCP throughput, or
MediaCodec output.

**未实机验证 / Not verified on a physical Android device.**
