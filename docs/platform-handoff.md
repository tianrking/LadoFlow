# Desktop platform handoff

This document is the implementation handoff for replacing the deterministic
loopback with native host pipelines. It distinguishes code that can stay shared
from code that must be developed and tested on each operating system.

## Non-negotiable process boundaries

| Component | Responsibility | Must not own |
| --- | --- | --- |
| TypeScript/Tauri UI | User intent, configuration, status, diagnostics | Frame buffers, codecs, driver APIs, long-running media loops |
| Desktop Rust runtime | Command validation, lifecycle, composition of shared crates and native adapters | Platform driver implementation details |
| Shared Rust crates | Protocol, session, transport policy, codec-neutral media metadata and telemetry | CoreGraphics, Direct3D, USBKit/WinUSB, signing, installers |
| Native capture/encode adapter | OS frame acquisition, pixel surfaces, hardware codec integration | UI state or wire-session policy |
| Virtual-display driver/service | Virtual monitor lifecycle, privileged OS interaction, recovery | Tauri webview or frontend assets |

Native dependencies must remain target-gated. A macOS framework or Windows SDK
binding may not become an unconditional dependency of a shared crate.

## macOS: current state

The current Mac development slice is runnable and locally verified:

- `apps/desktop/src-tauri/src/platform/macos.rs` checks screen-recording access
  and enumerates active displays through CoreGraphics;
- an explicit 750 ms ScreenCaptureKit probe receives real frame callbacks,
  verifies IOSurface-backed pixel buffers, and reports format/dimensions/dirty
  rectangles without copying pixels into the webview;
- the Tauri UI can request capture access, select a 30/60 Hz synthetic mode,
  start/stop the loopback, and display live telemetry;
- the loopback passes typed protocol frames through the bounded media channel;
- `tauri build --bundles app` creates an ad-hoc `LadoFlow.app` with macOS 13.0
  as its configured minimum system version.

This is not yet a complete screen-streaming or extended-display implementation.
It does not create a virtual monitor, maintain a production capture stream,
invoke a hardware encoder, connect over USB, or feed a mobile decoder.

### macOS next slice

1. Promote the bounded ScreenCaptureKit probe into a long-running adapter that
   owns stream setup, callbacks, resizing, source removal, and shutdown.
2. Convert each production callback into codec-neutral metadata plus an opaque
   native surface. Keep native surface handles out of the wire protocol.
3. Add a VideoToolbox encoder boundary that accepts the native surface and emits
   H.264 access units with explicit keyframe information.
4. Convert encoded output into `ladoflow_protocol::VideoFrame`, preserving the
   monotonic capture/encode timestamps needed by telemetry.
5. Exercise the path through loopback first. Replace loopback with USB only
   after pacing, queue supersession, cancellation, and reconnect tests pass.
6. Investigate the supported virtual-display and distribution path separately;
   do not couple that experiment to capture or encoder correctness.

The first native-capture acceptance test should run for ten minutes at both 30
and 60 Hz, survive a display-mode change, stop without a callback after teardown,
and report bounded queue depth plus dropped/superseded frames.

Screen-recording permission is persistent OS state keyed to application identity.
Automated tests should preflight it but must not repeatedly prompt or mutate it.
Use Apple's [ScreenCaptureKit capture guidance](https://developer.apple.com/documentation/screencapturekit/capturing-screen-content-in-macos)
as the API baseline.

## Windows: implementation handoff

The Windows desktop crate now has a target-gated native adapter that checks
`Windows.Graphics.Capture` support, enumerates active monitor geometry, creates
a hardware D3D11 device, and runs a short free-threaded capture probe. The probe
has received real GPU surfaces on physical Windows hardware and shuts its event
handler, frame pool, and session down explicitly. A separate bounded Media
Foundation probe has activated Intel Quick Sync, submitted synthetic NV12
frames, handled its asynchronous format change, and verified Annex B H.264
bytes. The production capture-to-encode loop, service, and driver remain native
milestones.

### Phase A — capture/encode proof of concept

1. [x] Add a target-gated Windows adapter, capture support probe, and display
   source enumeration while preserving the existing `PlatformStatus` shape.
2. [x] Prove selected-monitor capture with a bounded, free-threaded D3D11 frame
   pool and expose real callback/surface/startup diagnostics in the desktop UI.
3. Turn the probe into a cancellable long-running capture source with explicit
   UI selection, resize handling, and device-loss recovery.
4. [x] Enumerate NV12-to-H.264 Media Foundation transforms registered with the
   hardware MFT flag and report their real names without treating discovery as
   an encoding proof.
5. [x] Activate a hardware MFT, negotiate NV12 to H.264, handle asynchronous
   events and output stream changes, and require actual Annex B bytes. The
   physical validation currently covers Intel Quick Sync; NVIDIA discovery is
   not counted as a successful encode on this machine.
6. Hand captured Direct3D surfaces to the selected Media Foundation encoder
   through an NV12 conversion path without a CPU readback.
7. Feed encoded access units into the existing protocol/transport/runtime and
   validate keyframe, resize, device-loss, stop, and restart behavior.
8. Record capture, encode, enqueue, dequeue, and presentation timestamps so the
   same latency model is comparable with macOS.

Windows.Graphics.Capture is suitable for the capture proof of concept and
exposes display/window frame acquisition. Microsoft's
[screen-capture documentation](https://learn.microsoft.com/en-us/windows/uwp/audio-video-camera/screen-capture)
is the source of truth for support checks, picker behavior, frame pools, resize,
and device-loss handling.

### Phase B — real extended display

Use the supported IddCx user-mode indirect-display model for the virtual monitor.
The driver owns adapter/monitor modes and receives desktop images through its
swapchain; this is a different boundary from capturing an existing display.
Follow Microsoft's [indirect display driver overview](https://learn.microsoft.com/en-us/windows-hardware/drivers/display/indirect-display-driver-model-overview)
and begin from the official driver sample rather than inventing an unsupported
kernel path.

Keep the driver and any privileged companion service outside the Tauri process.
Define a narrow, versioned local IPC contract for:

- connect/disconnect virtual monitor;
- supported modes and active mode;
- frame/surface handoff or encoded-stream handoff;
- health, backpressure, restart, and fatal-error events.

Driver crashes, upgrades, rollback, and uninstall must not corrupt desktop-app
state. Signing and installer work starts only after the unsigned development
package has repeatable install/remove and recovery tests.

## Commit and verification cadence

Keep native work reviewable and bisectable. A useful sequence is:

1. source enumeration and status;
2. cancellable capture with synthetic consumer;
3. hardware encoder and encoded-frame tests;
4. shared runtime integration and telemetry;
5. virtual-display service/driver protocol;
6. packaging, signing, and recovery.

Each commit should include its unit or integration test. Hardware-only behavior
must include a short manual test record with OS build, GPU, resolution, refresh
rate, duration, and observed drop/latency figures.
