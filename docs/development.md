# Development setup

## Current requirements

- Git;
- Rust 1.97.1 with `rustfmt` and `clippy` (pinned by `rust-toolchain.toml`);
- Node.js 22 or a newer active LTS release;
- pnpm 10.26.0;
- the official [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)
  for the host OS.

On macOS, the current unsigned local build works with the Xcode command-line
tools. Install them when needed with:

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

Build an unsigned local macOS application bundle with:

```bash
pnpm --filter @ladoflow/desktop tauri build --bundles app
```

The bundle is written below `target/release/bundle/macos/`. It is not signed or
notarized and does not yet capture or create an extended display.

## Cross-platform CI

CI keeps shared-core validation separate from desktop-native validation:

- the protocol, core, transport, and media crates run on Linux;
- the Tauri frontend and Rust desktop crate build and test on macOS, Windows,
  and Linux;
- Linux installs WebKitGTK and AppIndicator development packages before the
  desktop checks.

The matrix proves that target-gated boundaries compile. Physical display,
driver, encoder, USB, permission, signing, and installer behavior still require
hardware/OS-specific test machines.

## Future platform toolchains

- Windows host/driver: Visual Studio Build Tools and the matching Windows Driver
  Kit;
- Android: Android Studio, JDK 17+, Android SDK, NDK, and a physical USB device;
- Apple display/host: current Xcode, a supported device, and signing identities;
- Linux: distribution-specific Wayland/X11/DRM development packages.

Do not commit generated signing assets, certificates, provisioning profiles, or
machine-specific SDK paths. See the [platform handoff](./platform-handoff.md)
before adding native capture or virtual-display code.
