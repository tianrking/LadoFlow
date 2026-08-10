# Applications

This directory owns user-facing applications:

- `desktop/` — runnable Tauri 2 shell shared across Windows, macOS, and Linux, with native platform services behind it;
- `android/` — buildable native Kotlin/Compose display shell with a deterministic connection state model; USB Accessory transport and MediaCodec integration remain active implementation work;
- `apple/` — planned native Swift/SwiftUI application for iOS and iPadOS.

Application directories are added only when their scaffold has a reproducible
local build; empty placeholder projects are intentionally avoided.
