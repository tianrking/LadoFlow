# Development setup

LadoFlow is a monorepo. Use the [source checkout guide](./source-checkout.md)
for a complete clone or a sparse Android/Desktop checkout before installing
toolchains.

## Current requirements

- Git;
- Rust 1.97.1 with `rustfmt` and `clippy` (pinned by `rust-toolchain.toml`);
- Node.js 22 or a newer active LTS release;
- pnpm 10.26.0;
- the official [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)
  for the host OS.

Android development additionally requires:

- JDK 17;
- Android SDK Platform 36 and Build Tools 35.0.0;
- the checked-in Gradle wrapper under `apps/android`.

Android Studio is convenient but not required for command-line validation. A
physical device is required only for the hardware/USB validation matrix, not
for JVM tests, lint, or APK assembly.

On macOS 13 or newer, the current ad-hoc local build works with the Xcode
command-line tools. Install them when needed with:

```bash
xcode-select --install
```

A full, current Xcode installation and an Apple signing identity are later
requirements for native capture/device work, signing, and notarization.

## Install and run

From the repository root:

```bash
rustup toolchain install 1.97.1 --profile minimal --component rustfmt --component clippy
corepack enable
pnpm install --frozen-lockfile
pnpm dev:desktop
```

The desktop host starts in an idle state. Select 30 or 60 Hz and start the
loopback to exercise negotiation, synthetic frame production, bounded media
delivery, presentation sequencing, and telemetry. The macOS capture-access
button invokes the native system permission request; use it only when you are
ready for macOS to update the app's privacy state.

## Validate

Run every current check with:

```bash
pnpm check
```

The equivalent explicit commands are:

```bash
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
pnpm check:desktop
```

Validate and assemble the Android application separately from `apps/android`:

```bash
./gradlew --no-daemon testDebugUnitTest lintDebug lintRelease assembleDebug assembleRelease assembleDebugAndroidTest
```

This produces a debug APK, an Android-test APK, and an intentionally unsigned
release APK. Repository CI verifies that two clean unsigned release builds are
byte-for-byte reproducible. A distributable release still requires an external
owner-controlled signing step.

Build an ad-hoc local macOS application bundle with:

```bash
pnpm --filter @ladoflow/desktop tauri build --bundles app
```

The bundle is written below `target/release/bundle/macos/`. It has only an
ad-hoc local signature, is not notarized, and does not yet create an extended
display. Its explicit native probe does capture frames only after the user grants
screen-recording access; the probe discards pixel contents in the native callback.

## Cross-platform CI

CI keeps shared-core, desktop-native, Android, and Windows-driver validation
separate:

- the protocol, core, transport, and media crates run on Linux;
- the Tauri frontend and Rust desktop crate build and test on macOS, Windows,
  and Linux;
- Linux installs WebKitGTK and AppIndicator development packages before the
  desktop checks;
- Android runs JVM tests, debug/release lint, and debug, unsigned-release, and
  instrumentation APK assembly;
- Windows separately builds and validates the IddCx development driver package
  and composes the unsigned NSIS installer resources.

The matrix proves that target-gated boundaries compile. Physical display,
driver, encoder, USB, permission, signing, and installer behavior still require
hardware/OS-specific test machines.

## Platform toolchains still needed for remaining milestones

- Windows host/driver: Visual Studio Build Tools and the matching Windows Driver
  Kit;
- Android physical validation: supported phone/tablet, USB data cable, and an
  OEM configuration exposing AOA or an allow-listed USB-tether interface;
- Apple display/host: current Xcode, a supported device, and signing identities;
- Linux: distribution-specific Wayland/X11/DRM development packages.

Do not commit generated signing assets, certificates, provisioning profiles, or
machine-specific SDK paths. See the [platform handoff](./platform-handoff.md)
before adding native capture or virtual-display code.
