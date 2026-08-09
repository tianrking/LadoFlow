# Applications

This directory owns user-facing applications:

- `desktop/` — runnable Tauri 2 shell shared across Windows, macOS, and Linux, with native platform services behind it;
- `android/` — native Kotlin application using Android hardware decoding and USB APIs;
- `apple/` — native Swift/SwiftUI application for iOS and iPadOS.

Only `desktop/` exists today. Mobile application directories will be created
when each scaffold has a reproducible local build; empty placeholder projects
are intentionally avoided.
