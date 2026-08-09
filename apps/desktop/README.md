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

The macOS adapter currently provides screen-recording permission and active
display discovery. Actual ScreenCaptureKit streaming and virtual-display
creation remain separate native milestones; the UI does not claim that the
synthetic path is a usable extended display.

## Validate

```bash
pnpm check:desktop
cargo test -p ladoflow-desktop
```

Build an unsigned local macOS application bundle with:

```bash
pnpm --filter @ladoflow/desktop tauri build --bundles app
```
