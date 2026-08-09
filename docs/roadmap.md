# Roadmap

This roadmap is ordered by evidence, not by marketing surface area.

## M0 — Repository foundation

- [x] Product boundaries and honest status matrix
- [x] Cross-platform repository layout
- [x] Original brand mark and multilingual README
- [x] Baseline CI configuration
- [ ] Confirm license and trademark strategy before paid distribution

## M1 — Protocol and loopback

- [ ] Versioned, bounded wire framing
- [ ] Capability and display-configuration messages
- [ ] Malformed-input and round-trip tests
- [ ] In-memory duplex transport
- [ ] Synthetic frame producer/consumer
- [ ] Latency and frame-pacing telemetry

## M2 — Windows host + Android display over USB

- [ ] Android native receiver shell
- [ ] App-compatible USB discovery and pairing without ADB
- [ ] H.264 hardware decoder and renderer
- [ ] Windows capture/encode proof of concept
- [ ] Touch and pointer return path
- [ ] Repeatable 30/60 Hz latency test

## M3 — Windows virtual extended display

- [ ] Supported virtual display driver path
- [ ] Resolution and rotation negotiation
- [ ] Driver/service isolation and recovery
- [ ] Signed development package
- [ ] Installer/uninstaller and rollback validation

## M4 — macOS host

- [ ] Native virtual-display adapter research spike
- [ ] Hardware capture/encode integration
- [ ] Android USB interoperability
- [ ] Signing and notarization pipeline

## M5 — iOS/iPadOS display

- [ ] Native Swift receiver and Metal presentation
- [ ] App Store-compatible wired transport
- [ ] Hardware decode, rotation, touch, and keyboard behavior
- [ ] Windows and macOS interoperability matrix

## M6 — Linux host

- [ ] Wayland compositor support matrix
- [ ] X11/DRM fallback path
- [ ] Debian package first, then RPM-family packaging

## M7 — Trusted LAN

- [ ] Discovery that does not leak screen content
- [ ] Explicit pairing and authenticated encryption
- [ ] Congestion control and adaptive bitrate
- [ ] Wired/wireless handoff and reconnect tests

Public internet relay is deliberately outside the USB-first roadmap.

