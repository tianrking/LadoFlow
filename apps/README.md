# Applications

This directory owns user-facing applications:

- `desktop/` — runnable Tauri 2 shell shared across Windows, macOS, and Linux, with native platform services behind it;
- `android/` — native Kotlin/Compose display endpoint with an independent LDFL v1 implementation, direct AOA and authenticated USB-tether transports, MediaCodec Surface decode, lifecycle recovery, telemetry, and input return;
- `apple/` — planned native Swift/SwiftUI application for iOS and iPadOS.

Application directories are added only when their scaffold has a reproducible
local build; empty placeholder projects are intentionally avoided.
