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

The first implementation milestone covers bounded framing and negotiation types, not actual video transport.

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

Currently implemented typed control payloads:

- `Hello`: supported version range, endpoint role, 16-byte nonce, and a UTF-8 implementation name of at most 64 bytes;
- `Capabilities`: maximum dimensions, refresh rate, bitrate, codec mask, input mask, and optional feature mask.

Display configuration, video metadata, input events, telemetry, ping/pong timestamps, pairing authentication, and encryption remain subsequent milestones.

