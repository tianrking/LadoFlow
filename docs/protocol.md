# Protocol principles

The wire protocol is transport-independent. USB and LAN adapters carry the same logical messages but may use different channel mappings.

## Requirements

- explicit magic bytes and protocol version;
- fixed-size, endian-defined frame header;
- bounded payload sizes before allocation;
- unknown message types rejected without desynchronizing the stream;
- capability negotiation before media starts;
- monotonic sequence numbers and timestamps;
- independent control and media backpressure;
- deterministic encode/decode tests and malformed-input tests.

## Initial message families

| Family | Purpose |
| --- | --- |
| Hello | Protocol range, implementation identity, session nonce |
| Capabilities | Resolution, refresh rate, codec, color, input support |
| DisplayConfig | Negotiated dimensions, refresh rate, bitrate, codec profile |
| VideoFrame | Frame identity, timestamps, keyframe flag, encoded bytes |
| Input | Touch, pointer, wheel, keyboard, and focus events |
| Telemetry | Stage timings, queue depth, loss, drops, thermal state |
| Ping/Pong | Liveness and clock-offset estimation |
| Error | Stable code, retryability, and bounded diagnostic text |

Version one defines typed payloads for every family in this table. Pairing authentication and encryption remain later protocol extensions.

### Session opening order

Sequence numbers are monotonic per sender across control and media. The host
opens a fresh connection with `Hello` sequence `0` followed by `Capabilities`
sequence `1`. The display replies with exactly one of each; their arrival order
may differ, but its own sequence must continue increasing. After role, version,
codec, dimension, refresh, and bitrate intersection succeeds, the host sends
`DisplayConfig` sequence `2`. No media is legal before that configuration.
Later host control responses consume the same sequence space, so video starts
at the next unused value rather than assuming it is always `3`.

The current desktop production boundary advertises H.264, selects H.264 Main,
and chooses 60 Hz when both endpoints support it or 30 Hz as the compatibility
fallback. A display below 30 Hz is rejected by this runtime instead of silently
creating an unsupported fractional refresh configuration.

## Version-one frame header

Every integer uses network byte order. The decoder validates the complete header before allocating payload storage.

| Offset | Bytes | Field |
| ---: | ---: | --- |
| 0 | 4 | ASCII magic `LDFL` |
| 4 | 2 | Protocol version |
| 6 | 2 | Header length (`24`) |
| 8 | 2 | Message type |
| 10 | 2 | Validated flags |
| 12 | 8 | Sender sequence number |
| 20 | 4 | Payload length |

Control payloads are limited to 64 KiB. A single encoded video payload is limited to 16 MiB. The incremental decoder also enforces a configurable total buffer ceiling.

## Version-one payload rules

Every multi-byte integer is in network byte order. Signed integers use two's-complement representation. Boolean fields accept only `0` or `1`. Decoders require the exact fixed or discriminant-selected length, reject unknown enum values and mask bits, and apply the bounds below even when called independently of frame decoding.

### Hello and capabilities

- `Hello`: supported version range, endpoint role, 16-byte nonce, and a non-empty UTF-8 implementation name of at most 64 bytes;
- `Capabilities`: maximum dimensions, refresh rate, bitrate, codec mask, input mask, and optional feature mask.

### DisplayConfig

`DisplayConfig` is exactly 14 bytes.

| Offset | Bytes | Field |
| ---: | ---: | --- |
| 0 | 2 | coded width in pixels, non-zero |
| 2 | 2 | coded height in pixels, non-zero |
| 4 | 4 | refresh rate in millihertz, non-zero |
| 8 | 4 | target bitrate in kilobits per second, non-zero |
| 12 | 1 | codec |
| 13 | 1 | codec profile |

Codec values are H.264 `1`, HEVC `2`, and AV1 `3`. Profile values are H.264 baseline/main/high `1`/`2`/`3`, HEVC main/main-10 `16`/`17`, and AV1 main `32`. A profile from a different codec family is invalid.

### VideoFrame

`VideoFrame` begins with 28 bytes of metadata followed by one non-empty encoded access unit. The complete payload remains subject to the 16 MiB media limit, so encoded bytes are limited to `16 MiB - 28`.

| Offset | Bytes | Field |
| ---: | ---: | --- |
| 0 | 8 | sender-assigned frame ID |
| 8 | 8 | capture-complete timestamp in microseconds |
| 16 | 8 | target presentation timestamp in microseconds |
| 24 | 4 | intended presentation duration in microseconds, non-zero |
| 28 | remaining | encoded access unit |

Timestamps are values from the sender's monotonic clock. Codec and profile come from the active `DisplayConfig`. The frame header's `KEYFRAME` flag identifies independently decodable access units.

### Input

One `Input` payload carries one event. Bytes `0..8` are the event timestamp in the sender's monotonic clock and byte `8` is the event kind. The frame sequence number is the event identity. The kind selects an exact body layout:

| Kind | Event | Body beginning at offset 9 | Total bytes |
| ---: | --- | --- | ---: |
| 1 | absolute pointer move | `x: u16`, `y: u16` pixel coordinates | 13 |
| 2 | pointer button | `button: u8`, `state: u8` | 11 |
| 3 | wheel | `delta_x: i16`, `delta_y: i16` | 13 |
| 4 | keyboard | USB HID `usage: u16`, `state: u8`, `modifiers: u16` | 14 |
| 5 | direct touch | `contact: u8`, `phase: u8`, `x: u16`, `y: u16`, `pressure: u16` | 17 |
| 6 | focus | `focused: bool` | 10 |

Button/key state is released `0` or pressed `1`. Pointer buttons are primary `1`, secondary `2`, middle `3`, back `4`, and forward `5`. Keyboard modifier bits are Shift, Control, Alt, Meta, Caps Lock, and Num Lock in bits `0..5`; HID usage zero is reserved. Touch phases are begin `1`, move `2`, end `3`, and cancel `4`; version one permits contact IDs `0..15`. Pressure is normalized over the full `u16` range. Larger batches, text composition, relative pointer motion, stylus metadata, and game controllers require later message versions.

### Telemetry

`Telemetry` is exactly 51 bytes. Stage durations are independently bounded to 60 seconds; zero means unmeasured. Queue depth is at most 4096 and loss is at most 1,000,000 parts per million. Drop and late counters are cumulative within a session.

| Offset | Bytes | Field |
| ---: | ---: | --- |
| 0 | 8 | sample timestamp in sender-clock microseconds |
| 8 | 8 | associated frame ID, or zero |
| 16 | 4 | capture duration in microseconds |
| 20 | 4 | encode duration in microseconds |
| 24 | 4 | transport enqueue-to-dequeue duration in microseconds |
| 28 | 4 | decode duration in microseconds |
| 32 | 4 | decoded-to-presented duration in microseconds |
| 36 | 2 | queue depth |
| 38 | 4 | loss in parts per million |
| 42 | 4 | dropped-frame count |
| 46 | 4 | late-frame count |
| 50 | 1 | thermal state |

Thermal states are unknown `0`, nominal `1`, fair `2`, serious `3`, and critical `4`.

### Ping and Pong

`Ping` is exactly 16 bytes: an opaque `u64` correlation token followed by the client's `u64` send timestamp in microseconds.

`Pong` is exactly 32 bytes: the echoed token, echoed client-send timestamp, server-receive timestamp, and server-send timestamp, all `u64`. Server send must not precede server receive. The client combines its local response-receive timestamp with these fields for NTP-style round-trip and clock-offset estimation; timestamps from different endpoints are not directly ordered.

### Error

`Error` has a five-byte prefix followed by zero to 1024 bytes of UTF-8 diagnostic text.

| Offset | Bytes | Field |
| ---: | ---: | --- |
| 0 | 2 | stable error code |
| 2 | 1 | retryable boolean |
| 3 | 2 | diagnostic byte length |
| 5 | remaining | UTF-8 diagnostic without null bytes |

Codes are protocol violation `1`, unsupported `2`, configuration rejected `3`, unauthorized `4`, busy `5`, encoder failure `6`, decoder failure `7`, input rejected `8`, resource exhausted `9`, and internal failure `10`. Control flow relies on the code and retry flag, never on diagnostic text.
