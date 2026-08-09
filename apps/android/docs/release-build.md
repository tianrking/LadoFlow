# Android release build boundary

The Android `release` build type is deliberately unsigned. Its Gradle
configuration sets `signingConfig = null`; the repository, CI workflow, and
artifacts contain no production keystore, private key, alias, or signing
password.

From `apps/android`, build and lint both variants with JDK 17 and Android SDK
Platform 36:

```powershell
./gradlew.bat --no-daemon testDebugUnitTest lintDebug lintRelease assembleDebug assembleRelease assembleDebugAndroidTest
```

The relevant outputs are:

- development APK: `app/build/outputs/apk/debug/app-debug.apk`;
- instrumentation APK:
  `app/build/outputs/apk/androidTest/debug/app-debug-androidTest.apk`;
- unsigned release APK:
  `app/build/outputs/apk/release/app-release-unsigned.apk`.

The debug APK is signed by Gradle's local or CI debug key and is suitable only
for development and test installation. The unsigned release APK is a
reproducible build input for a future controlled signing pipeline; Android will
not accept it as a production installation until an external release owner
signs it. Signing policy and key custody are intentionally outside this
milestone.

## CI contract

The `Android display` GitHub Actions job:

1. runs JVM tests plus debug and release lint;
2. assembles debug, instrumentation, and unsigned release APKs;
3. hashes the unsigned release APK, rebuilds release with `--rerun-tasks` in the
   same clean checkout/runner, and requires the second SHA-256 to match;
4. requires Android `apksigner verify` to reject the unsigned APK;
5. uploads `android-display-debug` and
   `android-display-release-unsigned` artifacts, with `SHA256SUMS` beside the
   unsigned release APK.

This is same-source, same-toolchain, same-runner byte reproducibility. It is not
yet a claim of cross-operating-system or independently reproduced builds.

## Evidence boundary

Build, lint, hash equality, and the negative signature check do not establish
physical Android decode, USB transport, input return, or long-run stability.

**未实机验证 / Not verified on a physical Android device.**
