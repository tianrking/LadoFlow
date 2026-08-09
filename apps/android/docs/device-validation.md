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

Use a PC build containing at least the Windows host work through `3984efd`,
including global USB wire ordering (`d7cd44c`), H.264 Main configuration and
timestamped access-unit preservation (`cbdadbe`), and persistent test-pattern
VideoFrame transmission (`5116af5` and `3984efd`). Record the exact host SHA;
do not rely on a branch name alone.

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
4. Confirm the UI progresses through authorization, handshake, configured,
   Surface-ready Connected, and Displaying only after the first MediaCodec output
   is released to Surface.
5. Confirm Host sequence `0/1/2` negotiates H.264 Main within the Android
   advertised size/rate/bitrate. The app must fail closed on a deliberately
   duplicated sequence and on VideoFrame before DisplayConfig.
6. Display the PC test pattern at every resolution the Android capability permits.
   Do not force the PC encoder's full 1280x800/1920x1080/2560x1440/2732x2048
   matrix above the device's advertised maximum.
7. Verify SPS/PPS plus the LDFL `KEYFRAME` starts decode, P-frame access-unit
   boundaries are retained, and Surface loss/recreation waits for a fresh
   keyframe.
8. Compare Android Telemetry `frame_id` with the latest Host
   `VideoFrame.metadata.frame_id` actually released to Surface. Compare
   `dropped_frames` with the Android session counter and `queue_depth` with the
   pre-Surface plus MediaCodec queues. Do not infer Presented from send count.
9. Exercise touch begin/move/end/cancel, mouse move/buttons/wheel, focus loss,
   rotation/resolution reconfiguration, explicit disconnect, cable detach,
   foreground/background, and reconnect.
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
| Permission, detach, reconnect result | |
| Touch / mouse / rotation result | |
| Logs, video, screenshots, trace paths | |
| Verdict and remaining failures | |

## Current evidence boundary

No row above has been completed with a physical Android device in this branch.

**未实机验证 / Not verified on a physical Android device.**
