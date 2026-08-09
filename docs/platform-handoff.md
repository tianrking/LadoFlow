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
`Windows.Graphics.Capture` support, enumerates active monitor geometry, and
creates a hardware D3D11 device. Beyond the short probe, a cancellable worker
captures the UI-selected monitor, performs BGRA-to-NV12 scaling with the D3D11
video processor, and gives that GPU texture to a low-latency Media Foundation
hardware encoder through a shared DXGI device manager. It emits timestamped
Annex B H.264 Main access units, including explicit IDR/clean-point evidence,
into the ordered LDFL USB runtime. The real Windows screen-to-simulated-display
protocol path has passed on physical Intel Quick Sync hardware. A separate
one-monitor UMDF 2 IddCx project, LocalSystem lifecycle service, fixed v1 local
IPC contract, and JSON client now build and pass their non-installing tests plus
Universal API/INF/catalog validation. The Tauri host polls structured controller
state, performs bounded enable/disable calls, waits for the virtual monitor, and
selects it automatically. Trusted installation and physical Android USB proof
remain separate milestones.

### Phase A — capture/encode proof of concept

1. [x] Add a target-gated Windows adapter, capture support probe, and display
   source enumeration while preserving the existing `PlatformStatus` shape.
2. [x] Prove selected-monitor capture with a bounded, free-threaded D3D11 frame
   pool and expose real callback/surface/startup diagnostics in the desktop UI.
3. [x] Turn the probe into a cancellable long-running source with explicit UI
   selection and bounded callback queues. Resize recreation and source-removal
   handling are implemented; mode-change endurance and automatic D3D
   device-loss recovery still need dedicated physical tests.
4. [x] Enumerate NV12-to-H.264 Media Foundation transforms registered with the
   hardware MFT flag and report their real names without treating discovery as
   an encoding proof.
5. [x] Activate a hardware MFT, negotiate NV12 to H.264, handle asynchronous
   events and output stream changes, preserve access-unit timestamps/durations,
   and require actual Annex B Main bytes plus keyframe evidence. The physical
   validation currently covers Intel Quick Sync; NVIDIA discovery is not counted
   as a successful encode on this machine.
6. [x] Hand captured Direct3D surfaces to the selected Media Foundation encoder
   through GPU NV12 conversion without a CPU readback.
7. [x] Feed encoded access units into the existing protocol/transport/runtime;
   keyframe, wire ordering, cancellation, and restart have repeatable tests.
   Mode-change endurance, device-loss recovery, and physical Android transport
   remain open.
8. Record capture, encode, enqueue, dequeue, and presentation timestamps so the
   same latency model is comparable with macOS.

Windows.Graphics.Capture is suitable for the capture proof of concept and
exposes display/window frame acquisition. Microsoft's
[screen-capture documentation](https://learn.microsoft.com/en-us/windows/uwp/audio-video-camera/screen-capture)
is the source of truth for support checks, picker behavior, frame pools, resize,
and device-loss handling.

### Phase C — Android Open Accessory USB

The shared transport crate now owns the side-effect-free AOA 1/2 control
contract: request `51`, all six request-`52` identity strings, request `53`,
little-endian protocol parsing, the 256-byte terminated-string bound, and
short-transfer rejection. The Windows adapter uses statically linked libusb and
only sends those vendor requests after explicit user action. It then waits for
`18d1:2d00/2d01/2d04/2d05`, finds a bulk IN/OUT pair, and must successfully
claim the app interface before reporting readiness.

1. [x] Match the Android app identity exactly: manufacturer `LadoFlow`, model
   `LadoFlow Host`; do not make app routing depend on host version.
2. [x] Unit-test the complete AOA control sequence, malformed protocol reply,
   unsupported version zero, identity bounds, and short writes.
3. [x] Add explicit Windows mode switching, re-enumeration timeout, descriptor
   inspection, and bulk-interface claim diagnostics.
4. [x] Keep the claimed handle open in a cancellable duplex worker, merge
   already-numbered control/media heads by global LDFL sequence, cap writes at
   64 KiB, retry short writes, and feed 64 KiB reads into the bounded LDFL
   incremental decoder. The decoder, channel classifier, and global-sequence
   mux now live in the shared transport crate so TCP can use the identical
   byte-stream contract. Any control priority must happen before sequence
   assignment; wire order is always monotonic.
5. [x] Compose the worker's bounded host endpoint into the runtime for
   Hello/Capabilities/DisplayConfig, monotonic peer sequencing, Ping/Pong,
   typed active control, cancellation, timeout, and failure diagnostics.
6. [x] Feed paced, timestamped, hardware-encoded H.264 Main access units from
   the selected live Windows display into the established USB session. The
   capture surface stays on the GPU through NV12 conversion and encoder input;
   interdependent frames are reliable rather than incorrectly superseded.
7. [x] Enforce the negotiated input mask, map remote coordinates only into the
   selected monitor, and inject pointer, wheel, keyboard, and direct-touch
   events through native Windows APIs. Focus loss and teardown release tracked
   state; an actual Android touch-return run remains part of physical proof.
8. [ ] Validate permission UI, sustained throughput, detach, input return, and reconnect on a
   physical Android device.
9. [ ] Replace development driver setup with a signed, installer-managed
   WinUSB-compatible binding and verify rollback/uninstall.

The AOA request and product-ID values follow the
[AOSP AOA 1.0 specification](https://source.android.com/docs/core/interaction/accessories/aoa)
and [AOA 2.0 additions](https://source.android.com/docs/core/interaction/accessories/aoa2).
On Windows, libusb documents that a non-HID interface generally needs WinUSB,
libusbK, or another compatible driver before user-mode access; the UI reports
this as an installation requirement rather than a protocol failure.

Windows mouse/keyboard synthesis follows Microsoft's
[`SendInput` contract](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-sendinput),
including its UIPI boundary. Absolute coordinates use the complete virtual
desktop as required by the
[`MOUSEINPUT` contract](https://learn.microsoft.com/en-us/windows/win32/api/winuser/ns-winuser-mouseinput).
Direct contacts are initialized and submitted through
[`InitializeTouchInjection`](https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-initializetouchinjection)
and `InjectTouchInput`.

### Phase B — real extended display

The supported IddCx user-mode indirect-display model is now implemented under
`platform/windows/idd`. The driver owns a stable one-monitor identity and its
tablet-oriented mode table, then consumes DWM swap-chain surfaces on an MMCSS
thread. The desktop process captures the exposed virtual `HMONITOR` through the
existing WGC/GPU H.264 path; encoding and USB work do not run in the UMDF host.
Follow Microsoft's [indirect display driver overview](https://learn.microsoft.com/en-us/windows-hardware/drivers/display/indirect-display-driver-model-overview)
and the official sample lineage recorded in the component's third-party notice.

The driver is outside the Tauri process. `LadoFlowDisplayService.exe` is the
LocalSystem owner of the software-device handle. `LadoFlowVirtualDisplay.exe`
is an ordinary-user JSON client for `start`, `status`, and `stop`; it reaches the
service through a fixed-size pipe protocol that rejects remote clients, checks
reserved fields and correlation IDs, and verifies the server PID against SCM.
The Tauri host now maps that JSON state into a typed platform status, caches
short-lived polling results, and performs bounded enable/disable transitions.
After enable it waits for the LadoFlow `HMONITOR` and selects it; after disable it
waits for removal. The per-machine Windows NSIS build includes the controller,
service, setup helper, DLL, INF, and catalog. Its hooks stop the owned service
before an upgrade, install after resource copy, and remove the service and only
the recorded hash-verified OEM INF packages before uninstall. The setup helper
has build-time self-tests and non-mutating plans, but its administrator path has
not yet run on a trusted clean test host. The remaining host and installation
work must cover:

- frame/surface handoff or encoded-stream handoff;
- health, backpressure, restart, and fatal-error events.

Once LDFL returns a `DisplayConfig`, the desktop resolves the selected monitor
back to its Win32 `\\.\DISPLAYn` name. Only an identity-verified LadoFlow
virtual monitor is eligible for a mode change. The host enumerates an exact
60 Hz `DEVMODE`, validates it with `CDS_TEST`, applies it to the live desktop
without `CDS_UPDATEREGISTRY`, waits up to five seconds for monitor
re-enumeration, and starts WGC only after the requested geometry is observable.
The 30 Hz video option keeps the virtual desktop at 60 Hz and paces encoding at
30 Hz. Physical-monitor selections are always a no-op. This path has compile
and unit-test evidence only until it runs with the driver installed on a
controlled host. Runtime orientation/rotation control remains separate work;
the host therefore does not advertise the `DYNAMIC_ROTATION` feature yet.

Driver crashes, upgrades, rollback, and uninstall must not corrupt desktop-app
state. The build produces a validated test-signed development catalog but does
not alter certificate stores, Secure Boot, test-signing mode, or boot settings.
Trusted install/remove and recovery evidence precede production signing.

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
