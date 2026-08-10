# Roadmap

This roadmap is ordered by the current implementation sequence and by evidence,
not by marketing surface area. A checked item has a repeatable local validation
path; it is not automatically a release claim.

## M0 — Repository foundation

- [x] Product boundaries and honest status matrix
- [x] Cross-platform repository layout
- [x] Original brand mark and multilingual README
- [x] Baseline CI configuration
- [x] Cross-platform tray shell, close-to-tray lifecycle, and single-instance activation
- [ ] Confirm license and trademark strategy before paid distribution

## M1 — Protocol, shared runtime, and loopback

- [x] Versioned, bounded wire framing
- [x] Hello, capability, display-configuration, video, input, telemetry, ping/pong, and error messages
- [x] Malformed-input, chunking, and round-trip tests
- [x] Capability negotiation and reconnect-aware session state
- [x] Bounded in-memory duplex transport with obsolete-media supersession
- [x] Codec-neutral synthetic frame producer and 30/60 Hz pacing
- [x] Rolling latency, drop, queue-depth, and frame-pacing telemetry
- [x] Runnable Tauri 2 desktop loopback host

## M2 — macOS host proof of concept

- [x] Target-gated screen-recording permission and active-display discovery adapter
- [x] Bounded ScreenCaptureKit callback/IOSurface diagnostics probe
- [x] Ad-hoc local macOS application bundle
- [ ] ScreenCaptureKit frame stream with resize and display-removal handling
- [ ] IOSurface/Metal-friendly frame boundary with explicit pixel format
- [ ] VideoToolbox H.264 hardware-encode integration
- [ ] Native virtual-display feasibility spike and documented distribution constraints
- [ ] Repeatable 30/60 Hz capture-to-loopback latency test
- [ ] Signing and notarization pipeline

## M3 — Windows host proof of concept

- [x] Validate the Tauri shell on a physical Windows development machine
- [x] Windows Graphics Capture source enumeration and bounded GPU-surface probe
- [x] Media Foundation hardware-MFT discovery and real NV12-to-Annex-B H.264 probe
- [x] Long-running selected-display capture, GPU NV12 conversion, hardware encoder, and LDFL runtime handoff
- [x] Capability-gated pointer, wheel, keyboard, and direct-touch injection with selected-monitor coordinate mapping
- [x] Isolate privileged device ownership behind a LocalSystem service and bounded, PID-verified local IPC
- [ ] Repeatable 30/60 Hz capture-to-loopback latency test

## M4 — Android display and USB

- [ ] Native Kotlin receiver shell
- [x] Shared AOA 1/2 negotiation contract and Windows explicit mode-switch/endpoint-claim probe
- [x] Cancellable Windows bulk worker with bounded queues, 64 KiB transfer chunks, and LDFL stream decoding
- [x] Desktop runtime USB control exchange with Hello, Capabilities, DisplayConfig, Ping/Pong, and failure diagnostics
- [x] Real selected-display capture and hardware H.264 runtime composition with the USB worker
- [ ] Physical Windows-to-Android USB interoperability without ADB
- [ ] H.264 hardware decoder and renderer
- [ ] Touch and pointer return path
- [x] Host-side bounded/cancellable USB disconnect recovery with fresh protocol negotiation
- [x] Shared bounded TCP LDFL worker for an already authenticated connected socket
- [x] Windows USB-tether route discovery/UI with bounded USB device-tree evidence
- [ ] Android listener and mutually authenticated pairing preface
- [ ] Physical detach/reconnect endurance and cancellation test
- [ ] macOS and Windows USB interoperability test

## M5 — Windows virtual extended display

- [x] Supported, build-verified one-monitor IddCx indirect-display driver path
- [x] Stable monitor identity and tablet-oriented 60 Hz resolution table
- [x] LocalSystem software-device owner plus unprivileged JSON start/status/stop client
- [x] Versioned fixed-size IPC, explicit DACL, remote-client rejection, and SCM PID verification
- [x] Universal API/INF validation and test-signed development catalog build
- [x] Runtime resolution negotiation plus identity-gated Windows active-mode alignment
- [ ] Runtime orientation/rotation negotiation with the connected mobile display
- [ ] Trusted install plus sleep/wake, crash, GPU-reset, and rollback recovery
- [x] Automatic Tauri lifecycle and virtual-monitor selection integration
- [x] Per-machine NSIS hooks and hash-verified native setup-helper dry-run/build validation
- [ ] Installer/uninstaller and rollback validation

## M6 — iOS/iPadOS display

- [ ] Native Swift receiver and Metal presentation
- [ ] App Store-compatible wired transport
- [ ] Hardware decode, rotation, touch, and keyboard behavior
- [ ] Windows and macOS interoperability matrix

## M7 — Linux host

- [ ] Wayland compositor support matrix
- [ ] X11/DRM fallback path
- [ ] Debian package first, then RPM-family packaging

## M8 — Trusted LAN

- [ ] Discovery that does not leak screen content
- [ ] Explicit pairing and authenticated encryption
- [ ] Congestion control and adaptive bitrate
- [ ] Wired/wireless handoff and reconnect tests

Public internet relay is deliberately outside the USB-first roadmap.
