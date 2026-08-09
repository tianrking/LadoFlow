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

