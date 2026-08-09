# Android MediaCodec boundary

The Android decoder accepts the existing LDFL v1 `DisplayConfig` and
`VideoFrame` payloads without adding fields to the wire protocol. H.264 Main is
the only enabled codec/profile pair. HEVC and AV1 remain explicit future
implementations behind the codec-neutral `VideoDecoder` interface.

## H.264 access-unit contract

The Windows hardware probes in commits `a8b7950` and `cbdadbe` established that
Intel Quick Sync emits timestamped Annex-B H.264 Main access units and preserves
clean-point/IDR boundaries. Android therefore requires each LDFL `VideoFrame`
access unit to preserve Annex-B start codes. It does not wrap the payload in
another USB or Android-specific envelope.

For every new `DisplayConfig`, reconnect, output-Surface replacement, or dropped
dependent chain, the host must send:

1. Annex-B SPS and PPS NAL units before or inside the first independently
   decodable access unit;
2. a VCL access unit whose LDFL frame header has `KEYFRAME` set;
3. subsequent dependent access units in presentation order.

SPS/PPS may be carried in a parameter-only `VideoFrame` immediately before the
keyframe or in the same access unit. The Android gate extracts them into
MediaCodec `csd-0` and `csd-1`. LDFL v1 has no separate codec-configuration
message and no Annex-B/AVCC discriminator, so these rules are an interoperability
contract for the current H.264 implementation, not a new protocol field.

The LDFL `KEYFRAME` flag remains authoritative. Android also inspects H.264 NAL
types and reports a diagnostic warning if a flagged keyframe has no IDR NAL,
but it does not silently reinterpret the wire flag.

## Decoder and Surface lifecycle

- Android probes actual AVC Main profile, size, frame-rate, and bitrate ranges;
- the exact negotiated tuple is checked again before MediaCodec configuration;
- compatible decoders prefer one reported hardware-accelerated on API 29+;
- older releases report acceleration as unknown instead of inferring from a
  codec name;
- API 30+ low-latency mode is enabled only when the selected codec advertises
  the platform low-latency feature;
- all MediaCodec calls and callbacks run on one dedicated `HandlerThread`;
- pending input is bounded to three access units; overflow drops a dependent
  chain and waits for the next LDFL keyframe;
- output is released immediately to a caller-owned `SurfaceView` Surface;
- Surface loss, display reconfiguration, EOS, and codec errors release native
  codec resources and require a fresh keyframe before recovery.

The application-level display session owns a generation-numbered Surface
controller. Each Activity/Compose `SurfaceView` receives one lease. Replacing a
Surface installs the newer generation immediately; a delayed `surfaceDestroyed`
or composition dispose from the old Activity can release only its stale lease
and cannot clear the newer decoder Surface. `surfaceChanged` records positive
local view width/height on the active lease without rebuilding MediaCodec.

A local portrait/landscape or window-size change affects Surface scaling and
input mapping only. It does not invent a rotation/size field on the wire. A
change to the coded video dimensions still uses the existing compatible
`DisplayConfig`, followed by SPS/PPS and a frame marked `KEYFRAME`.

`OutputReleasedToSurface` means MediaCodec handed a decoded buffer to the
Surface. It deliberately does not claim that the device panel presented the
frame; physical presentation and latency require device-side measurements. The
event retains the exact Host `VideoFrame.metadata.frame_id` through the Annex-B
gate and MediaCodec timestamp correlation so session telemetry can report it.

## Evidence boundary

The Annex-B parser, recovery gate, session Surface gate, capability contract,
stale-lease protection, local portrait/landscape resize, and Activity recreation
ownership are covered by JVM or Android instrumentation tests. No phone or
tablet has decoded this stream yet.

**未实机验证 / Not verified on a physical Android device.**
