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
requires non-empty Annex B H.264 output before reporting encode verification.
The probe has produced a real Intel Quick Sync bitstream on physical hardware;
it is not yet the long-running capture-to-encoder path or a virtual display.

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
