# Native platform integration

This directory is reserved for components that must follow host operating-system APIs and packaging rules:

- Windows indirect display driver and service integration;
- macOS virtual-display adapter and signing/notarization configuration;
- Linux Wayland/X11/DRM adapters.

No shared abstraction will pretend these drivers are identical. Their narrow boundary is the frame/input/session contract exposed to the shared core.

Small target-gated adapters that can safely live in the desktop process begin in
`apps/desktop/src-tauri/src/platform/`. Privileged services, drivers, signing,
and installer projects belong here. Follow the concrete ownership and test
sequence in [`docs/platform-handoff.md`](../docs/platform-handoff.md).

The first privileged component now lives in
[`windows/idd`](./windows/idd/): a buildable Windows 11 x64 IddCx driver, a
LocalSystem lifecycle service, and an unprivileged versioned-IPC controller.
Their source/build validation is complete; trusted installation and physical
extended-display validation are separate release gates.
