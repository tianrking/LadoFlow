# Android physical-device validation

This is the evidence checklist for the first Android display milestone. A green
build, emulator test, or successful APK installation is not evidence that AOA,
MediaCodec, touch return, or sustained display output works on a physical
device.

## Artifacts

From `apps/android` with JDK 17 and Android SDK Platform 36:

```powershell
./gradlew.bat --no-daemon testDebugUnitTest lintDebug assembleDebug assembleDebugAndroidTest
```

Outputs:

- app: `app/build/outputs/apk/debug/app-debug.apk`;
- instrumentation: `app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk`;
- unit report: `app/build/reports/tests/testDebugUnitTest/index.html`;
- lint report: `app/build/reports/lint-results-debug.html`.

The app APK can be copied to the device and opened for normal sideloading, or
installed from Android Studio. `adb install -r` and `adb logcat` are acceptable
developer installation/diagnostic tools, but ADB is never the LadoFlow product
transport. For the product-path proof, install first, then disable USB debugging
and confirm the session still uses Android Open Accessory.

The non-hardware instrumentation skeleton can be run with:

```powershell
./gradlew.bat --no-daemon connectedDebugAndroidTest
```

## Required host baseline

Use a PC build at or after `6fbfddf` (`Stream Windows capture through GPU
H264`). This supersedes the earlier synthetic test-pattern baseline: the Host
now runs cancellable Windows.Graphics.Capture, converts BGRA to NV12 with the
VideoProcessor on the same D3D11 device without CPU readback, encodes
low-latency H.264 Main with B-frames disabled, and sends each access unit as an
LDFL VideoFrame. Record the exact Host SHA; do not rely on a branch name alone.

No protocol field was added for this capture path. The current Host sets every
`VideoFrame.metadata.frame_id` to that frame's global LDFL header `sequence`.

The host must enter AOA with manufacturer `LadoFlow`, model `LadoFlow Host`, and
then carry raw LDFL v1 bytes with no USB-private header. Host writes remain at
or below 64 KiB.

## Functional run

1. Record Android manufacturer/model, Android build, API level, reported display
   modes, selected MediaCodec name, cable/adapter, PC OS/build, GPU, and Host SHA.
2. Start LadoFlow Android with USB debugging disabled. Connect a data-capable
   cable and start the PC host.
3. Confirm Android shows the system accessory permission dialog. Exercise both
   denial/retry and approval once.
4. Confirm the UI explicitly shows `Waiting for authorization`, `Connected`,
   `Reconnecting`, `Protocol error`, and `Device disconnected` for those real
   states. `Displaying` may appear only after the first MediaCodec output is
   released to Surface.
5. Confirm Host sequence `0/1/2` negotiates H.264 Main within the Android
   advertised size/rate/bitrate. The app must fail closed on a deliberately
   duplicated sequence and on VideoFrame before DisplayConfig.
6. Display live Windows capture at every resolution the Android capability
   permits. Do not force the PC encoder's full
   1280x800/1920x1080/2560x1440/2732x2048 matrix above the device's advertised
   maximum.
7. Verify SPS/PPS plus the LDFL `KEYFRAME` starts decode and P-frame access-unit
   boundaries are retained. Recreate the Activity, rotate portrait/landscape,
   and change the available window size while streaming. Confirm a stale old
   Surface destruction cannot clear the new Surface and each real Surface
   replacement waits for a fresh keyframe.
8. Confirm each Host `VideoFrame.metadata.frame_id` equals its global LDFL
   header `sequence`. Compare Android Telemetry `frame_id` with the latest Host
   frame ID actually released by MediaCodec to Surface. Compare `dropped_frames`
   with the Android session counter and `queue_depth` with the pre-Surface plus
   MediaCodec queues. Do not infer Presented from send count or claim physical
   panel presentation from a Surface release alone.
9. Exercise touch begin/move/end/cancel, mouse move/buttons/wheel, physical
   keyboard down/up with modifiers, focus loss, rotation/resolution
   reconfiguration, explicit disconnect, cable detach, foreground/background,
   and reconnect. After both transient I/O recovery and detach/reattach in the
   same Android process, confirm a fresh Host `Hello/0`, `Capabilities/1`, and
   `DisplayConfig/2` is accepted. Confirm the Host advertises each input family
   before Android sends it and releases tracked input state after focus loss or
   disconnect.
10. Run at least 30 minutes at the negotiated 60 Hz or downgraded 30 Hz. Record
    frame count, drops, maximum queue depth, decoder failures, reconnects,
    thermal state, and any visible corruption or latency observation.

## Evidence record

| Field | Result |
| --- | --- |
| Date/time and operator | |
| Android device / build / API | |
| Display modes | |
| MediaCodec name / acceleration evidence | |
| Host OS / GPU / exact SHA | |
| Cable / adapter | |
| Negotiated width × height @ refresh / bitrate | |
| Duration | |
| Host frames / Android Surface releases | |
| Dropped frames / max queue depth | |
| Permission, detach, in-process reconnect result | |
| Activity rebuild / Surface generation / resize result | |
| Touch / mouse / rotation result | |
| Logs, video, screenshots, trace paths | |
| Verdict and remaining failures | |

## Current evidence boundary

No row above has been completed with a physical Android device in this branch.

**未实机验证 / Not verified on a physical Android device.**
