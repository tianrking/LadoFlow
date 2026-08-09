# LadoFlow desktop host

The desktop host is a Tauri 2 application with a small TypeScript presentation
layer and a Rust command/runtime layer. Shared protocol, session, transport, and
media policy stays in workspace crates. OS-specific capture and virtual-display
code stays behind target-gated adapters.

## Run on macOS or Windows

Install the repository toolchain, then from the repository root:

```bash
pnpm install
pnpm dev:desktop
```

The first usable path is deterministic loopback. It performs capability
negotiation, produces synthetic frames, passes them through bounded media
queues, schedules presentation, and exposes latency/drop telemetry in the UI.

The macOS adapter provides screen-recording permission, active-display
discovery, and an explicit 750 ms ScreenCaptureKit probe. The probe receives
real callbacks and reports IOSurface dimensions, pixel format, dirty rectangles,
and startup timing without copying pixels into the webview. A long-running
capture/VideoToolbox stream and virtual-display creation remain separate native
milestones; the UI does not claim that the synthetic path is a usable extended
display.

The Windows adapter enumerates active monitors and runs the same bounded native
probe with `Windows.Graphics.Capture`, a hardware D3D11 device, and a
free-threaded frame pool. The UI reports actual GPU-surface callbacks, dirty
regions, dimensions, and startup timing. Its persistent native worker also
enumerates Media Foundation H.264 encoders that explicitly accept NV12 and are
registered as hardware MFTs. A bounded probe activates each candidate in order,
handles asynchronous input/output events and dynamic output renegotiation, and
requires non-empty Annex B H.264 Main output before reporting encode
verification. It preserves the MFT's access-unit boundaries, sample timestamps,
sample durations, and clean-point/IDR keyframe evidence. The probe has produced
a timestamped Intel Quick Sync bitstream with a verified keyframe on physical
hardware; it is not yet the long-running capture-to-encoder path or a virtual
display.

The Windows host also includes an explicit Android Open Accessory preparation
path. Shared Rust code validates the exact AOA protocol query, six terminated
identity strings, mode-switch request, and short-transfer failures. The Windows
adapter then waits for Google accessory re-enumeration, claims the app interface,
and keeps it owned by a cancellable duplex worker. Outbound control and media
queue heads are merged by their already-assigned global LDFL sequence, short
writes resume without interleaving frames, every write is capped at 64 KiB, and
inbound chunks feed the bounded LDFL incremental decoder. This prevents a later
control frame from overtaking an earlier encoded frame on the USB byte stream.
Read-only status never sends vendor requests; only the user's **Connect Android
USB** action attempts a mode switch. **Disconnect Android USB** joins the worker
and releases the interface. This path has not been verified with a physical
Android device, and Windows may need a signed WinUSB-compatible binding before
libusb can access the interface.

When the bulk link is connected, **Start session** selects the USB endpoint
instead of the local proof endpoint. It sends Host Hello and Capabilities,
accepts the display messages in either order while enforcing monotonically
increasing peer sequence numbers, computes a bounded H.264 Main configuration,
and sends DisplayConfig before entering the connected state. The host nonce is
filled from the operating-system random source. Active control traffic is
decoded and bounded; Ping receives Pong, Android Error fails the session, and
Input/Telemetry are validated without yet injecting native Windows input. A
separate native worker then continuously hardware-encodes timestamped synthetic
NV12 frames as H.264 Main access units. The session paces those units, marks
IDR/clean-point frames, and sends every interdependent H.264 frame reliably over
the same globally ordered LDFL stream while control remains responsive. This is
real encoded media rather than the loopback's fake bytes, but the pixels are
still a deterministic test pattern until the long-running D3D11 capture source
is attached.

Native capture, encoder, driver, and Windows ownership boundaries are recorded
in the [platform handoff](../../docs/platform-handoff.md).

## Validate

```bash
pnpm check:desktop
cargo clippy -p ladoflow-desktop --all-targets -- -D warnings
cargo test -p ladoflow-desktop
```

On an interactive Windows desktop, verify the real GPU capture path explicitly:

```powershell
cargo test -p ladoflow-desktop native_capture_probe_receives_a_gpu_surface -- --ignored --nocapture
```

Verify a physical hardware encoder produces Annex B H.264 bytes with:

```powershell
cargo test -p ladoflow-desktop hardware_h264_encoder_outputs_annex_b_stream -- --ignored --nocapture
```

Build an ad-hoc local macOS application bundle with:

```bash
pnpm --filter @ladoflow/desktop tauri build --bundles app
```

Build an unsigned Windows NSIS installer with:

```powershell
pnpm --filter @ladoflow/desktop tauri build --bundles nsis
```

CI uploads that unsigned installer as a short-lived workflow artifact. Code
signing remains a release gate; an unsigned CI artifact is not a public release.
