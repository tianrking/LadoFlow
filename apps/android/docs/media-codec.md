# Android MediaCodec boundary

The first Android decoder slice accepts the existing LDFL v1 `DisplayConfig`
and `VideoFrame` payloads without adding fields to the wire protocol. H.264 is
the only enabled codec. HEVC and AV1 remain explicit future implementations
behind the codec-neutral `VideoDecoder` interface.

## H.264 access-unit contract

The Windows hardware probe in commit `a8b7950` established that its Intel Quick
Sync MFT output is Annex-B H.264. The Android decoder therefore requires each
LDFL `VideoFrame` access unit to preserve Annex-B start codes. It does not wrap
the payload in another USB or Android-specific envelope.

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

- all MediaCodec calls and callbacks run on one dedicated `HandlerThread`;
- Android enumerates compatible AVC decoders and prefers one reported as
  hardware-accelerated on API 29 or later;
- on older Android releases, hardware acceleration is reported as unknown
  rather than inferred from a codec name;
- API 30+ low-latency mode is enabled only when the selected codec advertises
  the platform low-latency feature;
- pending input is bounded to three access units; overflow drops a dependent
  chain and waits for the next LDFL keyframe;
- output is released immediately to a caller-owned `SurfaceView` Surface;
- Surface loss, display reconfiguration, EOS, and codec errors release native
  codec resources and require a fresh keyframe before recovery.

`OutputReleasedToSurface` means MediaCodec handed a decoded buffer to the
Surface. It deliberately does not claim that the device panel presented the
frame; physical presentation and latency require device-side measurements.

## Evidence boundary

The Annex-B parser and recovery gate are covered by local JVM tests. The Android
project compiles and packages the MediaCodec and Surface code against API 36.
No phone or tablet has decoded this stream yet.

**未实机验证 / Not verified on a physical Android device.**
