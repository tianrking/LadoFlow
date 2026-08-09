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
regions, dimensions, and startup timing. Its production worker turns the
selected monitor into a cancellable, bounded capture stream, recreates the frame
pool and converter when the source size changes, converts BGRA to NV12 with the
D3D11 video processor, and hands the GPU texture to a Media Foundation hardware
encoder through a DXGI device manager without CPU readback. The low-latency H.264
Main encoder disables B-frames, handles asynchronous input/output events and
dynamic output renegotiation, and preserves access-unit boundaries, timestamps,
durations, and clean-point/IDR evidence. The complete capture-to-protocol path
has produced real screen access units with Intel Quick Sync on physical Windows
hardware.

The Windows shell also integrates the separate IddCx lifecycle boundary. It
reads structured controller status without blocking the UI, enables or disables
the LocalSystem-owned software device through bounded subprocess calls, waits
for the real virtual `HMONITOR`, and selects that monitor automatically. A
physical monitor remains an explicit fallback when the development driver and
service are not installed. This lifecycle code is build- and unit-tested, but a
trusted driver install and a physical extended-desktop run remain open proof
boundaries.

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
Input is decoded only after its pointer, touch, or keyboard family was included
in the negotiated capability intersection. Coordinates are bounded to the
configured stream and mapped onto the selected monitor within the complete
Windows virtual desktop. Pointer, button, wheel, and keyboard events use
`SendInput`; direct contacts use `InitializeTouchInjection`/`InjectTouchInput`.
Focus loss, disconnect, and worker teardown release every tracked button, key,
and touch contact. Windows UIPI can still block input into a process running at a
higher integrity level, which is surfaced as an error rather than hidden.

A separate native worker captures the UI-selected Windows monitor and
continuously hardware-encodes its GPU surfaces as timestamped H.264 Main access
units. The session paces those units, marks IDR/clean-point frames, and sends
every interdependent H.264 frame reliably over the same globally ordered LDFL
stream while control remains responsive. Capture cancellation, source removal,
resize, frame closure, and encoder shutdown are explicit. Automatic D3D
device-loss recovery remains open, and the combined video/input USB path still
requires proof with a physical Android device.

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

Run every interactive Windows capture, hardware-encoder, and simulated-display
integration test with:

```powershell
cargo test -p ladoflow-desktop -- --ignored --nocapture
```

Build an ad-hoc local macOS application bundle with:

```bash
pnpm --filter @ladoflow/desktop tauri build --bundles app
```

Build an unsigned Windows NSIS installer with:

```powershell
pnpm --filter @ladoflow/desktop tauri build --bundles nsis
```

The Windows-specific bundle first builds the IddCx driver, service, and
controller, then includes their binaries and driver package under the app's
`windows` resources directory. It does **not** yet register the service or
driver; installation, rollback, and exact driver removal are a separate
administrator-level milestone. CI uploads that unsigned installer as a
short-lived workflow artifact. Code signing remains a release gate; an unsigned
CI artifact is not a public release.
