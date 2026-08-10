# LadoFlow Android display

Native Kotlin and Jetpack Compose display endpoint for LadoFlow. It retains the
direct Android Open Accessory 2.0 path and now includes an explicit USB-tether
TCP fallback for Windows hosts that cannot safely claim the pre-AOA MTP device.
Both are normal-cable, no-ADB paths and both carry unchanged LDFL v1. Developer
options, accounts, and a cloud relay are not part of the display path.

## Implemented boundary

- Product UI explicitly distinguishes waiting for authorization, Connected,
  reconnecting, protocol error, physical device detach, and user disconnect.
- Exact LDFL v1 framing and all typed payloads from `docs/protocol.md`, including
  golden vectors, split/coalesced reads, corrupt input, and bounded decoding.
- AOA attach filter, temporary permission, duplex `ParcelFileDescriptor`, 64 KiB
  incremental reads, finite queues, descriptor-close cancellation, detach, and
  bounded in-process reconnect with a fresh LDFL generation.
- Foreground-only USB-tether TCP listener bound to an allow-listed tether
  interface address, with a memory-only 80-bit Crockford code, exact fixed-size
  HMAC pairing, two-minute/three-failure/single-host limits, bounded socket
  reads during pairing, explicit-close cancellation after authentication, and
  direct handoff of the untouched stream to the same LDFL session.
  Pairing authenticates the tethered host but does not encrypt LDFL and is not
  advertised as LAN transport.
- One global sender sequence across every message family. Outbound priority is
  decided before numbering; numbered frames enter one FIFO writer. Inbound
  control/media wire order is retained and duplicate/stale sequence values fail
  closed.
- Display-side negotiation with exact `Hello/0`, `Capabilities/1`, and initial
  `DisplayConfig/2` host contract. Android independently starts at sequence `0`.
- Device/display capability probing and exact H.264 Main size/rate/bitrate
  validation before MediaCodec configuration.
- Asynchronous MediaCodec Surface decode boundary with Annex-B SPS/PPS parsing,
  keyframe/discontinuity gating, one eight-access-unit bound spanning Handler
  admission through codec output, Surface recreation, and low-latency feature
  negotiation when the platform reports it. A process-owned Surface
  lease prevents a destroyed Activity from clearing a newer Surface, while
  local orientation/size changes remain outside the wire protocol.
- Pointer, touch, and physical-keyboard return through LDFL Input. The
  focusable decoder SurfaceView forwards key down/up as USB HID usages, and the
  session sends only input families advertised by both endpoints.
- Telemetry reports the latest Host metadata frame ID released to Surface,
  exact session-cumulative batch drops, the combined pre-Surface/decoder queue
  depth, and measured codec-input-to-output-callback decode time. Physical panel
  presentation time remains explicitly unmeasured.

Protocol and platform details:

- [display-side session](./docs/session-control.md)
- [USB accessory handoff](./docs/usb-accessory.md)
- [USB tethering fallback](./docs/usb-tether.md)
- [MediaCodec boundary](./docs/media-codec.md)
- [release build boundary](./docs/release-build.md)
- [input and rotation](./docs/input-and-rotation.md)
- [physical-device validation](./docs/device-validation.md)

## Local build

Requirements:

- JDK 17 (an Android Studio JBR 17+ or another JDK 17 is suitable);
- Android SDK Platform 36 and Build Tools 35.0.0;
- no checked-in `local.properties`, signing material, or machine-specific path.

From `apps/android`:

```powershell
$env:JAVA_HOME = "<path-to-jdk-17>"
$env:ANDROID_SDK_ROOT = "<path-to-android-sdk>"
./gradlew.bat --no-daemon testDebugUnitTest lintDebug lintRelease assembleDebug assembleRelease assembleDebugAndroidTest
```

Outputs:

- `app/build/outputs/apk/debug/app-debug.apk`;
- `app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk`;
- `app/build/outputs/apk/release/app-release-unsigned.apk`;
- `app/build/reports/tests/testDebugUnitTest/index.html`;
- `app/build/reports/lint-results-debug.html`;
- `app/build/reports/lint-results-release.html`.

The repository CI runs the same unit, lint, debug APK, unsigned release APK, and
instrumentation-APK assembly tasks. It rebuilds release on the same clean
runner and requires the two unsigned APK SHA-256 values to match. A physical or
emulated runtime can run the instrumentation suite with
`connectedDebugAndroidTest`.

`app-debug.apk` is development-only and uses the local/CI debug signing key.
`app-release-unsigned.apk` is deliberately unsigned: it contains no repository
or CI signing secret and cannot be distributed or installed until an external
release owner signs it. See the release boundary document for the artifact and
verification contract.

Automated tests and APK assembly do not prove USB permission behavior, AOA
bulk transfer, hardware MediaCodec behavior on a phone/tablet, physical
pointer/touch/keyboard return, or sustained operation on a physical device.

**未实机验证 / Not verified on a physical Android device.**
