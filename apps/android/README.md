# LadoFlow Android display

Native Kotlin and Jetpack Compose display endpoint for LadoFlow. The production
connection is Android Open Accessory 2.0 over a normal USB data cable: the PC is
USB host and Android exposes the app as the accessory/device. ADB, developer
options, accounts, and a cloud relay are not part of the display path.

## Implemented boundary

- Product UI for waiting, Android permission, LDFL handshake, configured,
  Surface-ready Connected, Displaying, recovery, disconnect, and error states.
- Exact LDFL v1 framing and all typed payloads from `docs/protocol.md`, including
  golden vectors, split/coalesced reads, corrupt input, and bounded decoding.
- AOA attach filter, temporary permission, duplex `ParcelFileDescriptor`, 64 KiB
  incremental reads, finite queues, descriptor-close cancellation, detach, and
  bounded reconnect.
- One global sender sequence across every message family. Outbound priority is
  decided before numbering; numbered frames enter one FIFO writer. Inbound
  control/media wire order is retained and duplicate/stale sequence values fail
  closed.
- Display-side negotiation with exact `Hello/0`, `Capabilities/1`, and initial
  `DisplayConfig/2` host contract. Android independently starts at sequence `0`.
- Device/display capability probing and exact H.264 Main size/rate/bitrate
  validation before MediaCodec configuration.
- Asynchronous MediaCodec Surface decode boundary with Annex-B SPS/PPS parsing,
  keyframe gating, three-access-unit bounds, Surface recreation, and low-latency
  feature negotiation when the platform reports it.
- Pointer, touch, and physical-keyboard return through LDFL Input. The
  focusable decoder SurfaceView forwards key down/up as USB HID usages, and the
  session sends only input families advertised by both endpoints.
- Telemetry reports the latest Host metadata frame ID released to Surface,
  session-cumulative drops, and the combined pre-Surface/decoder queue depth.

Protocol and platform details:

- [display-side session](./docs/session-control.md)
- [USB accessory handoff](./docs/usb-accessory.md)
- [MediaCodec boundary](./docs/media-codec.md)
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
./gradlew.bat --no-daemon testDebugUnitTest lintDebug assembleDebug assembleDebugAndroidTest
```

Outputs:

- `app/build/outputs/apk/debug/app-debug.apk`;
- `app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk`;
- `app/build/reports/tests/testDebugUnitTest/index.html`;
- `app/build/reports/lint-results-debug.html`.

The repository CI runs the same unit, lint, debug APK, and instrumentation-APK
assembly tasks. A physical/emulated runtime can run the skeleton with
`connectedDebugAndroidTest`.

Automated tests and APK assembly do not prove USB permission behavior, AOA
bulk transfer, MediaCodec output, physical pointer/touch/keyboard return, or
sustained operation on a phone/tablet.

**未实机验证 / Not verified on a physical Android device.**
