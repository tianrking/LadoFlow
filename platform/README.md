# Native platform integration

This directory is reserved for components that must follow host operating-system APIs and packaging rules:

- Windows indirect display driver and service integration;
- macOS virtual-display adapter and signing/notarization configuration;
- Linux Wayland/X11/DRM adapters.

No shared abstraction will pretend these drivers are identical. Their narrow boundary is the frame/input/session contract exposed to the shared core.

