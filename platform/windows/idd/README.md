# LadoFlow Windows indirect display

This directory contains the native Windows 11 x64 virtual-display boundary:

- `driver/` is a UMDF 2 Indirect Display Driver built on IddCx;
- `service/` is the LocalSystem process that exclusively owns `HSWDEVICE`;
- `controller/` is an unprivileged, machine-readable service client;
- `setup/` is the administrator-only driver/service install and removal helper;
- `common/Protocol.h` is the fixed-size versioned local IPC contract;
- `build.ps1` restores pinned WDK packages, builds all four binaries, runs the
  Universal API validator, validates the generated INF, creates the catalog,
  smoke-tests every user-mode boundary, and assembles a development artifact.

The driver reports one stable LadoFlow monitor. Windows remembers its layout
because the monitor has a fixed container identity. The preferred mode is
1920×1080 at 60 Hz; the mode table also covers common 16:9, 16:10, 4:3, iPad,
and high-resolution tablet sizes through 2732×2048 at 60 Hz, with bounded
fallbacks down to 640×400 for lower-capability decoders. The desktop host only
changes this identity-verified virtual monitor, validates the exact advertised
mode first, and applies it for the current Windows session without persisting a
display profile.

## Frame path

IddCx gives the driver DWM swap-chain surfaces. The driver acknowledges those
surfaces on an MMCSS `Distribution` thread without CPU readback. The desktop
host then discovers the resulting virtual `HMONITOR`, captures it through the
existing `Windows.Graphics.Capture` D3D11 path, converts BGRA to NV12 on the
GPU, hardware-encodes H.264, and sends it through the LadoFlow session. This
keeps the UMDF frame loop small and isolates encoding/USB failures from the
display driver process.

This source and build proof do not by themselves prove a physically installed
extended desktop. Installation requires a trusted driver signature and a
controlled Windows test machine.

## Build

Prerequisites:

- Windows 11 x64;
- Visual Studio 2022 or 2026 with Desktop development with C++;
- `nuget.exe` on `PATH`, or pass `-NuGetPath`;
- network access to NuGet; a VS 2022 installation can use the pinned 46 KB
  build-integration VSIX fallback, while VS 2026 must include its matching
  Microsoft Windows Driver Kit component.

From PowerShell:

```powershell
.\platform\windows\idd\build.ps1
```

The build pins WDK `10.0.26100.6584` and SDK `10.0.26100.1`, the supported
VS 2022 toolchain pair. If Visual Studio does not have the WDK extension, the
script creates a repository-local MSBuild overlay under `.tools/`; it never
writes to Program Files and verifies the Microsoft payload SHA-256 before use.

If the MSVC Spectre libraries are missing, the script emits a warning and makes
a development-only non-Spectre build. Release automation must use:

```powershell
.\platform\windows\idd\build.ps1 -RequireSpectre
```

Output:

```text
platform/windows/idd/dist/Release/x64/
├── driver/
│   ├── LadoFlowIdd.cat
│   ├── LadoFlowIdd.dll
│   └── LadoFlowIdd.inf
├── symbols/
├── LadoFlowIdd.cer                 # development certificate, when generated
├── LadoFlowDisplayService.exe
├── LadoFlowVirtualDisplay.exe
└── LadoFlowWindowsSetup.exe
```

## Privileged lifecycle boundary

`LadoFlowDisplayService.exe` is designed to run as a Windows service under
LocalSystem. It is the only process that calls `SwDeviceCreate` and retains the
software-device handle. The Tauri host never loads driver code and never needs
an elevated webview process.

`LadoFlowVirtualDisplay.exe` is the ordinary-user client. It always prints one
JSON object, so the Tauri host can consume it without scraping localized text:

```powershell
.\LadoFlowVirtualDisplay.exe status
.\LadoFlowVirtualDisplay.exe start
.\LadoFlowVirtualDisplay.exe stop
```

`start` asks SCM to start the installed service when permitted, then sends an
`Enable` request. `status` is side-effect free. `stop` sends `Disable` but keeps
the service available for the next cable/session. JSON distinguishes SCM state,
request result, persistent device error, service PID, generation, and PnP device
instance ID. If the service is not installed, `status` succeeds while reporting
`serviceInstalled: false` and `0x80070424`.

The v1 pipe protocol has exact request/response sizes, a magic value, version,
command, correlation ID, reserved-field checks, and a 5-second client-I/O bound.
The server rejects remote clients, uses an explicit DACL that grants only the
required client rights to interactive users, and keeps the first/only pipe
instance open. The client verifies that the pipe server PID is the same PID SCM
reports for `LadoFlowVirtualDisplayService` before sending a command.

## Installer lifecycle boundary

`LadoFlowWindowsSetup.exe` is a static native helper invoked by the per-machine
Tauri NSIS installer. It never imports certificates, changes Secure Boot,
enables test-signing, or edits boot configuration. Its mutating commands require
an elevated token and are deliberately not exposed through the Tauri command
surface.

The post-install path validates every payload file and the exact LadoFlow INF
identity, stages/installs the driver package through Windows driver APIs,
creates or updates the LocalSystem service, enables delayed automatic start and
bounded restart recovery, then records the published `oemNN.inf` name and INF
SHA-256 under 64-bit HKLM. Uninstall first verifies every recorded package name,
INF identity, and hash; it never searches for an arbitrary `oemNN.inf` to
delete. Upgrade preparation stops only a service whose executable name, account,
display name, and command shape match the owned LadoFlow service.

These commands are always non-mutating and are executed by `build.ps1` where
applicable:

```powershell
.\LadoFlowWindowsSetup.exe self-test
.\LadoFlowWindowsSetup.exe plan-install
.\LadoFlowWindowsSetup.exe plan-uninstall
```

The NSIS hooks call `prepare-install`, `install`, and `uninstall`. Exit code
`3010` propagates a Windows restart request; every other nonzero result aborts
the corresponding installer step and leaves the helper available for recovery.

## Controlled development installation

The build script intentionally does **not** install the driver, add certificates
to trust stores, enable Windows test-signing mode, disable Secure Boot, or edit
boot configuration. Those are machine security changes and must be performed
only on an isolated development machine after explicit approval.

Once an appropriately trusted package is available, build the per-machine NSIS
installer from the repository root:

```powershell
pnpm --filter @ladoflow/desktop tauri build --bundles nsis
```

Run the resulting installer only on the approved test machine. Its Windows
uninstall entry invokes the same setup helper before Tauri removes application
files. For read-only diagnostics after installation:

```powershell
.\windows\LadoFlowVirtualDisplay.exe status
.\windows\LadoFlowWindowsSetup.exe plan-uninstall
```

The current build proof compiles the installer and all hooks without executing
them. Production distribution additionally requires a Microsoft-accepted
release signature plus clean-machine install, upgrade, rollback, reboot, and
uninstall evidence.

## Release gates

- Spectre-mitigated x64 MSVC libraries present;
- PREfast/static analysis enabled in the signing environment;
- Universal API and INF validation clean;
- production certificate and Microsoft driver-signing workflow complete;
- install/start/extend/capture/stop/uninstall tested on a clean supported host;
- sleep/wake, GPU reset, cable removal, user switch, and crash recovery tested;
- end-to-end frame pacing and latency measured with a physical Android device.

## Upstream notice

The IddCx lifecycle follows Microsoft's `Windows-driver-samples` IndirectDisplay
sample. See [THIRD_PARTY_NOTICES.md](./THIRD_PARTY_NOTICES.md). LadoFlow uses a
different device identity, one-monitor policy, mode table, controller, error
handling, diagnostics, and desktop capture architecture.
