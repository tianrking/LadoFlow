# Android display-side LDFL session

This controller consumes the unchanged LDFL v1 stream after AOA transport
framing. It adds no Android-only message, flag, or USB header.

## Negotiation contract

The host sender sequence is global across control and media:

1. `Hello` must be host sequence `0`.
2. `Capabilities` must be host sequence `1`.
3. The initial `DisplayConfig` must be host sequence `2`.
4. Every later host frame must have a strictly larger sequence than the
   preceding host frame, regardless of message family.

Android replies exactly once with `Hello` and `Capabilities`. Its independent
sender sequence begins at `0`; all later Pong, Ping, Input, Telemetry, and Error
frames continue that same monotonically increasing sequence. Priority is
resolved while the messages are still unnumbered payloads.

Android `Hello` is role `Display`, protocol range `1..1`, a fresh 16-byte
`SecureRandom` nonce, and implementation name `LadoFlow Android`.

The capability probe enumerates actual `MediaCodecList.REGULAR_CODECS` AVC
decoders, requires an advertised H.264 Main profile, intersects decoder size,
frame-rate, and bitrate ranges with the physical display modes, and prefers a
codec that Android reports as hardware accelerated. The current input mask is
`POINTER | TOUCH | KEYBOARD`. Input is sent only when its family appears in
both the Android and Host masks; an unnegotiated family is dropped locally.
The only feature flag is `DYNAMIC_ROTATION`.

## Configuration and Surface gate

The initial configuration is rejected unless it is H.264 Main and is within
both endpoints' advertised dimensions, refresh, and bitrate. The selected
MediaCodec must also accept the exact size/rate/bitrate tuple. A new compatible
`DisplayConfig` is the v1 mechanism for resolution or orientation changes.

`Configured` means the configuration was accepted but no usable decoder
Surface is confirmed. `Connected` is entered only after MediaCodec reports it
is awaiting a keyframe or running with a real Surface. Up to three access units
can wait for that Surface, beginning at a LDFL `KEYFRAME`. `Displaying` is
entered only after MediaCodec releases a decoded output buffer to the Surface;
this does not claim proof of physical scan-out.

Video before configuration, duplicate/stale sequence values, duplicate Hello
or Capabilities, an invalid handshake order, and active-only control messages
before DisplayConfig all fail closed and enqueue an LDFL Error when the USB
writer is still available. A host Error transitions the Android session to
`Failed`.

## Detach and reconnect generation

Transport recovery resets the peer Hello/Capabilities, active configuration,
decoder gate, input viewport, telemetry counters, and both sequence validators.
When the descriptor reopens, Android sends a new `Hello/0` and
`Capabilities/1`; the Host may likewise send a new `Hello/0`, `Capabilities/1`,
and `DisplayConfig/2`. Frames from a previous generation cannot be accepted by
the new outbound writer closure.

The Android UI classifies a physical accessory detach separately from bounded
I/O recovery. It also classifies LDFL validation failures as `Protocol error`
instead of a generic transport failure. These are local state/UI distinctions;
they add no LDFL field, flag, message, or compatibility rule.

Telemetry `frame_id` is the Host `VideoFrame.metadata.frame_id` most recently
released by MediaCodec to the Surface, never a locally guessed ordinal.
`dropped_frames` is cumulative for the current USB/LDFL session. `queue_depth`
is the sum of the three-frame pre-Surface buffer and the decoder's single
eight-access-unit bound spanning Handler admission, pending input, and codec
inputs waiting for output release. Decoder batch clears are counted by their
exact discarded access-unit count, not as one diagnostic event. `decode` timing
is measured from codec input queueing to output callback; `presentation` stays
zero because Surface release does not prove panel scan-out.

## Evidence boundary

Local JVM tests cover negotiation, exact reply identity and numbering,
configuration and Surface gates, cross-family duplicate sequence rejection,
priority-before-numbering, negotiated keyboard Input numbering, rejection of
an input family absent from the Host mask, detach, same-process reconnect with
fresh `0/1/2` Host numbering, and classified failure paths. MediaCodec
capability enumeration and actual Surface decode require an Android runtime.
The instrumentation suite exercises a real MediaCodec with a synthetic H.264
Main fixture and verifies Surface replacement/keyframe recovery; this remains
separate from physical-device evidence.

**未实机验证 / Not verified on a physical Android device.**
