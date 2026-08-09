# LadoFlow Android display

Native Kotlin and Jetpack Compose display endpoint for LadoFlow. The user-facing
connection path is Android Open Accessory over a normal USB data cable; ADB is
not part of the product connection design.

The Kotlin protocol layer mirrors every LDFL v1 message family in
`docs/protocol.md`: bounded network-order framing, typed handshake, display,
video, input, telemetry, liveness, and error payloads, plus an incremental
decoder for arbitrarily split or coalesced transport reads.

## Local build

Requirements:

- JDK 17;
- Android SDK Platform 36 and Build Tools 35.0.0;
- no checked-in `local.properties` or machine-specific paths.

From this directory:

```powershell
$env:JAVA_HOME = "<path-to-jdk-17>"
$env:ANDROID_SDK_ROOT = "<path-to-android-sdk>"
./gradlew.bat testDebugUnitTest assembleDebug
```

The debug APK is written to `app/build/outputs/apk/debug/`. USB transport and
hardware decode remain intentionally reported as inactive until their own
verified slices land.
