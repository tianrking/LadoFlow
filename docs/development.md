# Development setup

## Current requirements

- Git
- Rust 1.97.1 with `rustfmt` and `clippy` (pinned by `rust-toolchain.toml`)

Verify the foundation:

```bash
rustup toolchain install 1.97.1 --profile minimal --component rustfmt --component clippy
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

## Planned platform requirements

These are documented early but are not yet proof of a working build:

- Windows host/driver: Visual Studio Build Tools and the matching Windows Driver Kit;
- desktop UI: Node.js LTS, pnpm, Rust, and Tauri prerequisites;
- Android: Android Studio, JDK 17+, Android SDK, NDK, and a physical USB device;
- Apple display/host: a supported Mac, current Xcode, signing identity for device tests;
- Linux: distribution-specific Wayland/X11/DRM development packages.

Do not commit generated signing assets or machine-specific SDK paths.
