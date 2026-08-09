# LadoFlow Android display

Native Kotlin and Jetpack Compose display endpoint for LadoFlow. The user-facing
connection path is Android Open Accessory over a normal USB data cable; ADB is
not part of the product connection design.

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
hardware decode are intentionally reported as inactive in this initial UI
slice; implementation evidence is added in subsequent commits.
