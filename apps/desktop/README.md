# LadoFlow desktop host

The desktop host is a Tauri 2 application with a small TypeScript presentation
layer and a Rust command/runtime layer. Shared protocol, session, transport, and
media policy stays in workspace crates. OS-specific capture and virtual-display
code stays behind target-gated adapters.

## Run on macOS

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

Native capture, encoder, driver, and Windows ownership boundaries are recorded
in the [platform handoff](../../docs/platform-handoff.md).

## Validate

```bash
pnpm check:desktop
cargo clippy -p ladoflow-desktop --all-targets -- -D warnings
cargo test -p ladoflow-desktop
```

Build an ad-hoc local macOS application bundle with:

```bash
pnpm --filter @ladoflow/desktop tauri build --bundles app
```
