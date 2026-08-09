# LadoFlow Windows indirect display

This directory contains the native Windows 11 x64 virtual-display boundary:

- `driver/` is a UMDF 2 Indirect Display Driver built on IddCx;
- `service/` is the LocalSystem process that exclusively owns `HSWDEVICE`;
- `controller/` is an unprivileged, machine-readable service client;
- `common/Protocol.h` is the fixed-size versioned local IPC contract;
- `build.ps1` restores pinned WDK packages, builds all three binaries, runs the
  Universal API validator, validates the generated INF, creates the catalog,
  smoke-tests the controller, and assembles a development artifact.

The driver reports one stable LadoFlow monitor. Windows remembers its layout
because the monitor has a fixed container identity. The preferred mode is
1920×1080 at 60 Hz; the mode table also covers common 16:9, 16:10, 4:3, iPad,
and high-resolution tablet sizes through 2732×2048 at 60 Hz.

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
- Visual Studio 2022 with Desktop development with C++;
- `nuget.exe` on `PATH`, or pass `-NuGetPath`;
- network access to NuGet and, when the VS WDK integration component is absent,
  Microsoft's pinned 46 KB build-integration VSIX.

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
└── LadoFlowVirtualDisplay.exe
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

## Controlled development installation

The build script intentionally does **not** install the driver, add certificates
to trust stores, enable Windows test-signing mode, disable Secure Boot, or edit
boot configuration. Those are machine security changes and must be performed
only on an isolated development machine after explicit approval.

Once an appropriately trusted package is available, a controlled development
installation from an elevated PowerShell is:

```powershell
pnputil /add-driver .\driver\LadoFlowIdd.inf /install
$serviceBinary = (Resolve-Path .\LadoFlowDisplayService.exe).Path
sc.exe create LadoFlowVirtualDisplayService binPath= "`"$serviceBinary`" service" start= auto obj= LocalSystem DisplayName= "LadoFlow Virtual Display Service"
sc.exe description LadoFlowVirtualDisplayService "Owns the local LadoFlow indirect display lifecycle."
sc.exe start LadoFlowVirtualDisplayService
.\LadoFlowVirtualDisplay.exe start
.\LadoFlowVirtualDisplay.exe status
```

Stop the virtual device before removing the package:

```powershell
.\LadoFlowVirtualDisplay.exe stop
sc.exe stop LadoFlowVirtualDisplayService
sc.exe delete LadoFlowVirtualDisplayService
pnputil /enum-drivers
pnputil /delete-driver oemNN.inf /uninstall
```

Replace `oemNN.inf` only with the published name that `pnputil /enum-drivers`
shows for `LadoFlow Project`. These commands are documentation, not actions
performed by the build. Production distribution additionally requires a
Microsoft-accepted release signature, recovery-configured service registration,
and installer-managed rollback.

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
