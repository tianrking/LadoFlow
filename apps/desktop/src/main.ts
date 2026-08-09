import { invoke, isTauri } from "@tauri-apps/api/core";

type SessionPhase =
  | "idle"
  | "negotiating"
  | "connected"
  | "streaming"
  | "recovering"
  | "stopped"
  | "failed";

type CapturePermission = "granted" | "required" | "unsupported";

interface DisplaySource {
  id: string;
  name: string;
  width: number;
  height: number;
  primary: boolean;
  virtualDisplay: boolean;
}

type VirtualDisplayState =
  | "unsupported"
  | "clientMissing"
  | "notInstalled"
  | "serviceStopped"
  | "ready"
  | "enabling"
  | "enabled"
  | "disabling"
  | "failed"
  | "stopping";

interface VirtualDisplayStatus {
  state: VirtualDisplayState;
  detail: string;
  serviceInstalled: boolean;
  serviceState: string;
  enabled: boolean;
  deviceInstanceId: string | null;
  lastError: string | null;
  generation: number;
}

interface HostSnapshot {
  appVersion: string;
  os: string;
  architecture: string;
  protocolVersion: number;
  session: {
    phase: SessionPhase;
    transport: string;
    peerName: string | null;
    lastError: string | null;
    configuredWidth: number | null;
    configuredHeight: number | null;
    configuredFps: number | null;
  };
  telemetry: {
    framesProduced: number;
    framesPresented: number;
    framesDropped: number;
    framesSuperseded: number;
    actualFps: number;
    p50LatencyMs: number | null;
    p95LatencyMs: number | null;
    queueDepth: number;
    uptimeMs: number;
  };
  platform: {
    captureBackend: string;
    encoderStatus: string;
    usbLinkState: "unsupported" | "ready" | "connecting" | "connected" | "failed";
    usbStatus: string;
    capturePermission: CapturePermission;
    virtualDisplay: VirtualDisplayStatus;
    displays: DisplaySource[];
  };
}

interface LoopbackConfig {
  width: number;
  height: number;
  fps: number;
}

interface CaptureProbeReport {
  backend: string;
  displayId: string;
  displayName: string;
  width: number;
  height: number;
  targetFps: number;
  elapsedMs: number;
  callbacks: number;
  contentFrames: number;
  idleFrames: number;
  incompleteFrames: number;
  framesWithSurface: number;
  dirtyRects: number;
  observedFps: number;
  startupLatencyMs: number | null;
  pixelFormat: string | null;
  passed: boolean;
}

interface UsbAccessoryProbeReport {
  passed: boolean;
  state: string;
  detail: string;
  protocolVersion: number | null;
  busNumber: number | null;
  deviceAddress: number | null;
  vendorId: number | null;
  productId: number | null;
  interfaceNumber: number | null;
  inputEndpoint: number | null;
  outputEndpoint: number | null;
  maxPacketSize: number | null;
}

interface TetherPairingReport {
  endpoint: string;
  detail: string;
}

interface TetherEndpointCandidate {
  endpoint: string;
  adapterName: string;
  gateway: string;
  evidence: string;
}

interface TetherDiscoveryReport {
  candidates: TetherEndpointCandidate[];
  detail: string;
}

interface VirtualDisplayActionReport {
  passed: boolean;
  status: VirtualDisplayStatus;
  selectedDisplayId: string | null;
  elapsedMs: number;
}

const elements = {
  appVersion: getElement("app-version"),
  hostPlatform: getElement("host-platform"),
  hostStatusDot: getElement("host-status-dot"),
  protocolVersion: getElement("protocol-version"),
  sessionBadge: getElement("session-badge"),
  sessionTitle: getElement("connection-title"),
  sessionCopy: getElement("session-copy"),
  linkPath: getElement("link-path"),
  setupTitle: getElement("setup-title"),
  mediaMode: getElement("media-mode"),
  transportMode: getElement("transport-mode"),
  start: getButton("start-session"),
  stop: getButton("stop-session"),
  refresh: getButton("refresh-status"),
  requestPermission: getButton("request-permission"),
  runCaptureProbe: getButton("run-capture-probe"),
  captureProbeResult: getElement("capture-probe-result"),
  virtualDisplayReadiness: getElement("virtual-display-readiness"),
  virtualDisplayStatus: getElement("virtual-display-status"),
  enableVirtualDisplay: getButton("enable-virtual-display"),
  disableVirtualDisplay: getButton("disable-virtual-display"),
  virtualDisplayResult: getElement("virtual-display-result"),
  prepareAndroidUsb: getButton("prepare-android-usb"),
  pairAndroidTether: getButton("pair-android-tether"),
  discoverAndroidTether: getButton("discover-android-tether"),
  disconnectAndroidUsb: getButton("disconnect-android-usb"),
  tetherPairingForm: getElement("tether-pairing-form"),
  tetherEndpoint: getInput("tether-endpoint"),
  tetherCandidatesLabel: getElement("tether-candidates-label"),
  tetherCandidates: getSelect("tether-candidates"),
  tetherDiscoveryResult: getElement("tether-discovery-result"),
  tetherToken: getInput("tether-token"),
  tetherPairingResult: getElement("tether-pairing-result"),
  directUsbNote: getElement("direct-usb-note"),
  usbStatus: getElement("usb-status"),
  usbProbeResult: getElement("usb-probe-result"),
  resolution: getSelect("resolution"),
  framesPresented: getElement("frames-presented"),
  actualFps: getElement("actual-fps"),
  p50Latency: getElement("p50-latency"),
  p95Latency: getElement("p95-latency"),
  framesSuperseded: getElement("frames-superseded"),
  capturePermission: getElement("capture-permission"),
  captureBackend: getElement("capture-backend"),
  displayList: getElement("display-list"),
  errorBanner: getElement("error-banner"),
};

let selectedFps = 60;
let selectedDisplayId: string | null = null;
let pollingHandle: number | undefined;
let busy = false;
let lastSnapshot: HostSnapshot | null = null;
const nativeBridgeAvailable = isTauri();

function browserPreviewSnapshot(): HostSnapshot {
  return {
    appVersion: "0.1.0",
    os: "windows",
    architecture: "x86_64",
    protocolVersion: 1,
    session: {
      phase: "idle",
      transport: "No active transport",
      peerName: null,
      lastError: null,
      configuredWidth: null,
      configuredHeight: null,
      configuredFps: null,
    },
    telemetry: {
      framesProduced: 0,
      framesPresented: 0,
      framesDropped: 0,
      framesSuperseded: 0,
      actualFps: 0,
      p50LatencyMs: null,
      p95LatencyMs: null,
      queueDepth: 0,
      uptimeMs: 0,
    },
    platform: {
      captureBackend: "Windows Graphics Capture",
      encoderStatus: "Native encoder status is available in the desktop app",
      usbLinkState: "ready",
      usbStatus:
        "Connect Android by USB, enable USB tethering, and enter the address and one-time code shown by the app.",
      capturePermission: "granted",
      virtualDisplay: {
        state: "ready",
        detail: "LadoFlow virtual display is ready to enable in the desktop app.",
        serviceInstalled: true,
        serviceState: "stopped",
        enabled: false,
        deviceInstanceId: null,
        lastError: null,
        generation: 0,
      },
      displays: [
        {
          id: "preview-ladoflow-display",
          name: "LadoFlow Extended Display",
          width: 1920,
          height: 1080,
          primary: false,
          virtualDisplay: true,
        },
        {
          id: "preview-main-display",
          name: "Main display",
          width: 2560,
          height: 1440,
          primary: true,
          virtualDisplay: false,
        },
      ],
    },
  };
}

function getElement(id: string): HTMLElement {
  const element = document.getElementById(id);
  if (!element) throw new Error(`Missing required element #${id}`);
  return element;
}

function getButton(id: string): HTMLButtonElement {
  const element = getElement(id);
  if (!(element instanceof HTMLButtonElement)) {
    throw new Error(`#${id} is not a button`);
  }
  return element;
}

function getSelect(id: string): HTMLSelectElement {
  const element = getElement(id);
  if (!(element instanceof HTMLSelectElement)) {
    throw new Error(`#${id} is not a select`);
  }
  return element;
}

function getInput(id: string): HTMLInputElement {
  const element = getElement(id);
  if (!(element instanceof HTMLInputElement)) {
    throw new Error(`#${id} is not an input`);
  }
  return element;
}

function formatPlatform(snapshot: HostSnapshot): string {
  const osNames: Record<string, string> = {
    macos: "macOS",
    windows: "Windows",
    linux: "Linux",
  };
  return `${osNames[snapshot.os] ?? snapshot.os} · ${snapshot.architecture}`;
}

function formatMetric(value: number | null): string {
  if (value === null) return "—";
  if (value > 0 && value < 0.1) return "<0.1";
  return value.toFixed(1);
}

function setBadge(element: HTMLElement, label: string, tone: "idle" | "good" | "warn") {
  element.textContent = label;
  element.className = `badge badge--${tone}`;
}

function sessionPresentation(
  phase: SessionPhase,
  transport: string,
  wiredReady: boolean,
) {
  const isWiredSession = transport.startsWith("Android");
  const isTetherSession = transport.includes("USB tether");
  switch (phase) {
    case "negotiating":
      return {
        title: "Negotiating the display",
        copy: "Exchanging protocol versions, capabilities, and stream configuration.",
        label: "Negotiating",
        tone: "warn" as const,
      };
    case "streaming":
      return isWiredSession
        ? {
            title: `${isTetherSession ? "USB tether" : "Direct USB"} H.264 stream is live`,
            copy: "The selected Windows display is GPU-captured, hardware-encoded as H.264 Main, and paced over the authenticated wired link.",
            label: "Streaming",
            tone: "good" as const,
          }
        : {
            title: "Loopback display is live",
            copy: "Synthetic frames are crossing the same bounded core path used by physical links.",
            label: "Streaming",
            tone: "good" as const,
          };
    case "connected":
      return {
        title: "Wired display negotiated",
        copy: "The host and Android display agreed on LDFL and H.264 Main settings. The Windows hardware encoder is preparing the first access-unit batch.",
        label: "Connected",
        tone: "good" as const,
      };
    case "recovering":
      return {
        title: "Restoring the display link",
        copy: "The session is preserving its negotiated configuration while transport recovers.",
        label: "Recovering",
        tone: "warn" as const,
      };
    case "failed":
      return {
        title: "The link needs attention",
        copy: "Open diagnostics, resolve the reported error, and start the session again.",
        label: "Failed",
        tone: "warn" as const,
      };
    case "stopped":
    case "idle":
      if (wiredReady) {
        return {
          title: "Android is authenticated",
          copy: "Choose the extended display and stream settings, then start the wired session. Pairing stays local and no account is required.",
          label: "Ready",
          tone: "good" as const,
        };
      }
      return {
        title: "Ready for a nearby screen",
        copy: "Start the deterministic loopback to validate negotiation, transport, pacing, and telemetry before attaching a physical device.",
        label: "Idle",
        tone: "idle" as const,
      };
  }
}

function renderDisplays(displays: DisplaySource[], disabled: boolean) {
  elements.displayList.replaceChildren();
  if (displays.length === 0) {
    selectedDisplayId = null;
    const empty = document.createElement("p");
    empty.className = "display-empty";
    empty.textContent = "No active displays reported by this platform adapter.";
    elements.displayList.append(empty);
    return;
  }

  if (!displays.some((display) => display.id === selectedDisplayId)) {
    selectedDisplayId =
      displays.find((display) => display.virtualDisplay)?.id ??
      displays.find((display) => display.primary)?.id ??
      displays[0]?.id ??
      null;
  }

  for (const display of displays) {
    const row = document.createElement("button");
    const selected = display.id === selectedDisplayId;
    row.type = "button";
    row.className = selected ? "display-row display-row--selected" : "display-row";
    row.classList.toggle("display-row--virtual", display.virtualDisplay);
    row.disabled = disabled;
    row.setAttribute("aria-pressed", String(selected));
    row.setAttribute("aria-label", `Use ${display.name} as the capture source`);
    row.addEventListener("click", () => {
      selectedDisplayId = display.id;
      elements.captureProbeResult.hidden = true;
      renderDisplays(displays, disabled);
    });

    const glyph = document.createElement("span");
    glyph.className = "display-glyph";
    glyph.setAttribute("aria-hidden", "true");

    const details = document.createElement("div");
    const name = document.createElement("strong");
    name.textContent = display.name;
    const resolution = document.createElement("small");
    const labels = [
      display.virtualDisplay ? "LadoFlow extended" : null,
      display.primary ? "Main" : null,
    ].filter((label): label is string => label !== null);
    resolution.textContent = `${display.width} × ${display.height}${labels.length > 0 ? ` · ${labels.join(" · ")}` : ""}`;
    details.append(name, resolution);

    row.append(glyph, details);
    elements.displayList.append(row);
  }
}

function render(snapshot: HostSnapshot) {
  lastSnapshot = snapshot;
  const usbConnected = snapshot.platform.usbLinkState === "connected";
  const isWiredSession = snapshot.session.transport.startsWith("Android");
  const isTetherSession = snapshot.session.transport.includes("USB tether");
  const presentation = sessionPresentation(
    snapshot.session.phase,
    snapshot.session.transport,
    usbConnected,
  );
  const isRunning =
    snapshot.session.phase === "streaming" ||
    snapshot.session.phase === "connected" ||
    snapshot.session.phase === "negotiating" ||
    snapshot.session.phase === "recovering";
  const usbMode = usbConnected || (isWiredSession && isRunning);

  elements.appVersion.textContent = `LadoFlow ${snapshot.appVersion}`;
  elements.hostPlatform.textContent = formatPlatform(snapshot);
  elements.hostStatusDot.className = "status-dot status-dot--cyan";
  elements.protocolVersion.textContent = `LDFL v${snapshot.protocolVersion}`;
  elements.sessionTitle.textContent = presentation.title;
  elements.sessionCopy.textContent = snapshot.session.lastError ?? presentation.copy;
  elements.start.textContent = usbMode ? "Start wired stream" : "Start loopback";
  elements.setupTitle.textContent = usbMode ? "Wired screen stream" : "Synthetic stream";
  elements.mediaMode.textContent = usbMode
    ? "GPU capture · hardware H.264 Main"
    : "Codec-neutral synthetic";
  elements.transportMode.textContent = usbMode
    ? isTetherSession
      ? "Authenticated USB tether TCP"
      : "Android Open Accessory USB"
    : "In-memory duplex";
  setBadge(elements.sessionBadge, presentation.label, presentation.tone);
  elements.linkPath.classList.toggle(
    "is-active",
    snapshot.session.phase === "streaming" ||
      snapshot.session.phase === "connected" ||
      snapshot.session.phase === "recovering",
  );
  elements.start.disabled = busy || isRunning;
  elements.stop.disabled = busy || !isRunning;
  elements.resolution.disabled = isRunning;

  elements.framesPresented.textContent = snapshot.telemetry.framesPresented.toLocaleString();
  elements.actualFps.textContent = snapshot.telemetry.actualFps.toFixed(1);
  elements.p50Latency.textContent = formatMetric(snapshot.telemetry.p50LatencyMs);
  elements.p95Latency.textContent = formatMetric(snapshot.telemetry.p95LatencyMs);
  elements.framesSuperseded.textContent = snapshot.telemetry.framesSuperseded.toLocaleString();

  const permissionGranted = snapshot.platform.capturePermission === "granted";
  const permissionUnsupported = snapshot.platform.capturePermission === "unsupported";
  setBadge(
    elements.capturePermission,
    permissionGranted ? "Allowed" : permissionUnsupported ? "N/A" : "Permission needed",
    permissionGranted ? "good" : permissionUnsupported ? "idle" : "warn",
  );
  elements.captureBackend.textContent = `${snapshot.platform.captureBackend}. ${snapshot.platform.encoderStatus}.`;
  const virtualDisplay = snapshot.platform.virtualDisplay;
  elements.virtualDisplayReadiness.hidden = snapshot.os !== "windows";
  elements.virtualDisplayStatus.textContent = virtualDisplay.detail;
  elements.enableVirtualDisplay.hidden = virtualDisplay.enabled;
  elements.disableVirtualDisplay.hidden = !virtualDisplay.enabled;
  const virtualDisplayUnavailable =
    virtualDisplay.state === "unsupported" ||
    virtualDisplay.state === "clientMissing" ||
    virtualDisplay.state === "notInstalled";
  const virtualDisplayTransitioning =
    virtualDisplay.state === "enabling" ||
    virtualDisplay.state === "disabling" ||
    virtualDisplay.state === "stopping";
  elements.enableVirtualDisplay.disabled =
    busy || isRunning || virtualDisplayUnavailable || virtualDisplayTransitioning;
  elements.disableVirtualDisplay.disabled = busy || isRunning || virtualDisplayTransitioning;
  elements.usbStatus.textContent = snapshot.platform.usbStatus;
  elements.requestPermission.hidden = permissionGranted || permissionUnsupported;
  elements.requestPermission.disabled = busy;
  const hasNativeCaptureProbe = snapshot.os === "macos" || snapshot.os === "windows";
  elements.runCaptureProbe.hidden = !hasNativeCaptureProbe || !permissionGranted;
  elements.runCaptureProbe.disabled = busy;
  elements.prepareAndroidUsb.hidden = snapshot.os !== "windows" || usbMode;
  elements.prepareAndroidUsb.disabled =
    busy || isRunning || snapshot.platform.usbLinkState === "connecting";
  elements.directUsbNote.hidden = snapshot.os !== "windows" || usbMode;
  elements.tetherPairingForm.hidden = usbMode;
  elements.tetherEndpoint.disabled = busy || isRunning;
  elements.discoverAndroidTether.hidden = snapshot.os !== "windows" || usbMode;
  elements.discoverAndroidTether.disabled = busy || isRunning;
  elements.tetherCandidates.disabled = busy || isRunning;
  elements.tetherToken.disabled = busy || isRunning;
  elements.pairAndroidTether.disabled = busy || isRunning;
  elements.disconnectAndroidUsb.hidden = !usbConnected;
  elements.disconnectAndroidUsb.disabled = busy;
  renderDisplays(snapshot.platform.displays, busy || isRunning);

  if (!nativeBridgeAvailable) {
    elements.start.disabled = true;
    elements.runCaptureProbe.disabled = true;
    elements.enableVirtualDisplay.disabled = true;
    elements.disableVirtualDisplay.disabled = true;
    elements.prepareAndroidUsb.disabled = true;
    elements.discoverAndroidTether.disabled = true;
    elements.pairAndroidTether.disabled = true;
    elements.disconnectAndroidUsb.disabled = true;
  }

  if (isRunning && pollingHandle === undefined) {
    pollingHandle = window.setInterval(() => void refreshSnapshot(false), 750);
  } else if (!isRunning && pollingHandle !== undefined) {
    window.clearInterval(pollingHandle);
    pollingHandle = undefined;
  }
}

function showError(error: unknown) {
  elements.errorBanner.hidden = false;
  elements.errorBanner.textContent = error instanceof Error ? error.message : String(error);
}

function clearError() {
  elements.errorBanner.hidden = true;
  elements.errorBanner.textContent = "";
}

async function refreshSnapshot(showFailure = true) {
  if (!nativeBridgeAvailable) {
    render(browserPreviewSnapshot());
    clearError();
    return;
  }
  try {
    const snapshot = await invoke<HostSnapshot>("get_host_snapshot");
    render(snapshot);
    clearError();
  } catch (error) {
    if (showFailure) showError(error);
  }
}

function selectedConfig(): LoopbackConfig {
  const [width, height] = elements.resolution.value.split("x").map(Number);
  if (!width || !height) throw new Error("Invalid display resolution");
  return { width, height, fps: selectedFps };
}

async function runAction(action: () => Promise<HostSnapshot>) {
  if (busy) return;
  busy = true;
  elements.start.disabled = true;
  elements.stop.disabled = true;
  elements.requestPermission.disabled = true;
  elements.runCaptureProbe.disabled = true;
  elements.enableVirtualDisplay.disabled = true;
  elements.disableVirtualDisplay.disabled = true;
  elements.prepareAndroidUsb.disabled = true;
  elements.discoverAndroidTether.disabled = true;
  elements.pairAndroidTether.disabled = true;
  elements.tetherEndpoint.disabled = true;
  elements.tetherToken.disabled = true;
  elements.disconnectAndroidUsb.disabled = true;
  clearError();
  try {
    const snapshot = await action();
    busy = false;
    render(snapshot);
  } catch (error) {
    busy = false;
    showError(error);
    await refreshSnapshot(false);
  }
}

function renderCaptureProbe(report: CaptureProbeReport) {
  elements.captureProbeResult.hidden = false;
  elements.captureProbeResult.className = report.passed
    ? "capture-probe-result capture-probe-result--good"
    : "capture-probe-result capture-probe-result--warn";
  const startup =
    report.startupLatencyMs === null ? "unknown" : `${formatMetric(report.startupLatencyMs)} ms`;
  const format = report.pixelFormat ?? "unknown format";
  elements.captureProbeResult.textContent = report.passed
    ? `${report.backend} delivered ${report.framesWithSurface} native ${format} surfaces (${report.width} × ${report.height}) across ${report.callbacks} callbacks. First surface: ${startup}; callback rate: ${formatMetric(report.observedFps)} fps.`
    : `${report.backend} started, but the ${report.elapsedMs} ms probe did not receive a native surface. Check permission, display availability, and system capture status.`;
}

async function runCaptureProbe() {
  if (busy) return;
  busy = true;
  const idleLabel = elements.runCaptureProbe.textContent;
  elements.runCaptureProbe.disabled = true;
  elements.enableVirtualDisplay.disabled = true;
  elements.disableVirtualDisplay.disabled = true;
  elements.prepareAndroidUsb.disabled = true;
  elements.discoverAndroidTether.disabled = true;
  elements.pairAndroidTether.disabled = true;
  elements.tetherEndpoint.disabled = true;
  elements.tetherToken.disabled = true;
  elements.disconnectAndroidUsb.disabled = true;
  elements.runCaptureProbe.textContent = "Capturing for 0.75 s…";
  elements.start.disabled = true;
  elements.stop.disabled = true;
  clearError();

  try {
    const report = await invoke<CaptureProbeReport>("run_screen_capture_probe", {
      displayId: selectedDisplayId,
      fps: selectedFps,
    });
    renderCaptureProbe(report);
  } catch (error) {
    showError(error);
  } finally {
    busy = false;
    elements.runCaptureProbe.textContent = idleLabel;
    await refreshSnapshot(false);
  }
}

function renderUsbProbe(report: UsbAccessoryProbeReport) {
  elements.usbProbeResult.hidden = false;
  elements.usbProbeResult.className = report.passed
    ? "capture-probe-result capture-probe-result--good"
    : "capture-probe-result capture-probe-result--warn";
  if (!report.passed) {
    elements.usbProbeResult.textContent = report.detail;
    return;
  }
  const protocol = report.protocolVersion === null ? "already in accessory mode" : `AOA ${report.protocolVersion}`;
  const device =
    report.vendorId === null || report.productId === null
      ? "Android accessory"
      : `${hexWord(report.vendorId)}:${hexWord(report.productId)}`;
  const endpoints =
    report.inputEndpoint === null || report.outputEndpoint === null
      ? "bulk endpoints"
      : `bulk IN ${hexByte(report.inputEndpoint)} / OUT ${hexByte(report.outputEndpoint)}`;
  elements.usbProbeResult.textContent = `${protocol} ready: ${device}, interface ${report.interfaceNumber ?? "?"}, ${endpoints}, max packet ${report.maxPacketSize ?? "?"} bytes. ${report.detail}`;
}

function renderTetherPairing(report: TetherPairingReport) {
  elements.tetherPairingResult.hidden = false;
  elements.tetherPairingResult.className =
    "capture-probe-result capture-probe-result--good";
  elements.tetherPairingResult.textContent = report.detail;
}

function renderTetherDiscovery(report: TetherDiscoveryReport) {
  elements.tetherCandidates.replaceChildren();
  const hasCandidates = report.candidates.length > 0;
  const hasMultipleCandidates = report.candidates.length > 1;
  for (const candidate of report.candidates) {
    const option = document.createElement("option");
    option.value = candidate.endpoint;
    option.textContent = `${candidate.adapterName} · ${candidate.gateway}`;
    option.title = candidate.evidence;
    elements.tetherCandidates.append(option);
  }
  elements.tetherCandidatesLabel.hidden = !hasMultipleCandidates;
  elements.tetherCandidates.hidden = !hasMultipleCandidates;
  if (hasCandidates) {
    elements.tetherEndpoint.value = report.candidates[0]?.endpoint ?? "";
  }
  elements.tetherDiscoveryResult.hidden = false;
  elements.tetherDiscoveryResult.className = hasCandidates
    ? "capture-probe-result capture-probe-result--good"
    : "capture-probe-result capture-probe-result--warn";
  elements.tetherDiscoveryResult.textContent = report.detail;
}

function hexByte(value: number): string {
  return `0x${value.toString(16).padStart(2, "0")}`;
}

function hexWord(value: number): string {
  return value.toString(16).padStart(4, "0");
}

function renderVirtualDisplayAction(report: VirtualDisplayActionReport) {
  elements.virtualDisplayResult.hidden = false;
  elements.virtualDisplayResult.className = report.passed
    ? "capture-probe-result capture-probe-result--good"
    : "capture-probe-result capture-probe-result--warn";
  elements.virtualDisplayResult.textContent = `${report.status.detail} Completed in ${report.elapsedMs} ms.`;
  if (report.selectedDisplayId !== null) {
    selectedDisplayId = report.selectedDisplayId;
    elements.captureProbeResult.hidden = true;
  }
}

async function changeVirtualDisplay(enable: boolean): Promise<VirtualDisplayActionReport | null> {
  if (busy) return null;
  busy = true;
  const button = enable ? elements.enableVirtualDisplay : elements.disableVirtualDisplay;
  const idleLabel = button.textContent;
  button.textContent = enable ? "Creating extended display…" : "Removing extended display…";
  elements.enableVirtualDisplay.disabled = true;
  elements.disableVirtualDisplay.disabled = true;
  elements.prepareAndroidUsb.disabled = true;
  elements.discoverAndroidTether.disabled = true;
  elements.pairAndroidTether.disabled = true;
  elements.tetherEndpoint.disabled = true;
  elements.tetherToken.disabled = true;
  elements.disconnectAndroidUsb.disabled = true;
  elements.runCaptureProbe.disabled = true;
  elements.start.disabled = true;
  elements.stop.disabled = true;
  clearError();

  try {
    const report = await invoke<VirtualDisplayActionReport>(
      enable ? "enable_virtual_display" : "disable_virtual_display",
    );
    renderVirtualDisplayAction(report);
    return report;
  } catch (error) {
    showError(error);
    return null;
  } finally {
    busy = false;
    button.textContent = idleLabel;
    await refreshSnapshot(false);
  }
}

async function prepareAndroidUsb() {
  if (busy) return;
  busy = true;
  const idleLabel = elements.prepareAndroidUsb.textContent;
  elements.prepareAndroidUsb.disabled = true;
  elements.discoverAndroidTether.disabled = true;
  elements.pairAndroidTether.disabled = true;
  elements.tetherEndpoint.disabled = true;
  elements.tetherToken.disabled = true;
  elements.disconnectAndroidUsb.disabled = true;
  elements.prepareAndroidUsb.textContent = "Preparing Android USB…";
  elements.start.disabled = true;
  elements.stop.disabled = true;
  elements.runCaptureProbe.disabled = true;
  elements.enableVirtualDisplay.disabled = true;
  elements.disableVirtualDisplay.disabled = true;
  clearError();
  let enableExtendedDisplay = false;

  try {
    const report = await invoke<UsbAccessoryProbeReport>("prepare_android_usb");
    renderUsbProbe(report);
    enableExtendedDisplay =
      report.passed === true &&
      lastSnapshot?.os === "windows" &&
      lastSnapshot.platform.virtualDisplay.serviceInstalled &&
      !lastSnapshot.platform.virtualDisplay.enabled;
  } catch (error) {
    showError(error);
  } finally {
    busy = false;
    elements.prepareAndroidUsb.textContent = idleLabel;
    await refreshSnapshot(false);
  }
  if (enableExtendedDisplay) {
    await changeVirtualDisplay(true);
  }
}

async function pairAndroidTether() {
  if (busy) return;
  const endpoint = elements.tetherEndpoint.value.trim();
  let token = elements.tetherToken.value.trim();
  if (endpoint.length === 0) {
    showError("Enter the Android address shown by the app.");
    elements.tetherEndpoint.focus();
    return;
  }
  if (token.replace(/[ -]/g, "").length !== 16) {
    showError("The one-time pairing code must contain 16 letters or digits.");
    elements.tetherToken.focus();
    return;
  }

  busy = true;
  const idleLabel = elements.pairAndroidTether.textContent;
  elements.pairAndroidTether.textContent = "Authenticating USB tether…";
  elements.pairAndroidTether.disabled = true;
  elements.discoverAndroidTether.disabled = true;
  elements.tetherEndpoint.disabled = true;
  elements.tetherToken.disabled = true;
  elements.tetherToken.value = "";
  elements.prepareAndroidUsb.disabled = true;
  elements.disconnectAndroidUsb.disabled = true;
  elements.start.disabled = true;
  elements.stop.disabled = true;
  elements.runCaptureProbe.disabled = true;
  elements.enableVirtualDisplay.disabled = true;
  elements.disableVirtualDisplay.disabled = true;
  elements.tetherPairingResult.hidden = true;
  clearError();
  let enableExtendedDisplay = false;

  try {
    const report = await invoke<TetherPairingReport>("pair_android_tether", {
      request: { endpoint, token },
    });
    renderTetherPairing(report);
    enableExtendedDisplay =
      lastSnapshot?.os === "windows" &&
      lastSnapshot.platform.virtualDisplay.serviceInstalled &&
      !lastSnapshot.platform.virtualDisplay.enabled;
  } catch (error) {
    showError(error);
  } finally {
    token = "";
    busy = false;
    elements.pairAndroidTether.textContent = idleLabel;
    await refreshSnapshot(false);
  }
  if (enableExtendedDisplay) {
    await changeVirtualDisplay(true);
  }
}

async function discoverAndroidTether() {
  if (busy || !nativeBridgeAvailable) return;
  busy = true;
  const idleLabel = elements.discoverAndroidTether.textContent;
  elements.discoverAndroidTether.textContent = "Checking USB network…";
  elements.discoverAndroidTether.disabled = true;
  elements.tetherEndpoint.disabled = true;
  elements.tetherCandidates.disabled = true;
  elements.tetherToken.disabled = true;
  elements.pairAndroidTether.disabled = true;
  elements.prepareAndroidUsb.disabled = true;
  elements.disconnectAndroidUsb.disabled = true;
  elements.start.disabled = true;
  elements.stop.disabled = true;
  elements.tetherDiscoveryResult.hidden = true;
  clearError();
  try {
    const report = await invoke<TetherDiscoveryReport>("discover_android_tether");
    renderTetherDiscovery(report);
  } catch (error) {
    showError(error);
  } finally {
    busy = false;
    elements.discoverAndroidTether.textContent = idleLabel;
    if (lastSnapshot) render(lastSnapshot);
  }
}

async function startSession() {
  if (busy) return;
  const shouldEnableVirtualDisplay =
    lastSnapshot?.platform.usbLinkState === "connected" &&
    lastSnapshot.platform.virtualDisplay.serviceInstalled &&
    !lastSnapshot.platform.virtualDisplay.enabled;
  if (shouldEnableVirtualDisplay) {
    await changeVirtualDisplay(true);
  }
  await runAction(() =>
    invoke<HostSnapshot>("start_loopback", {
      config: selectedConfig(),
      displayId: selectedDisplayId,
    }),
  );
}

async function disconnectWiredDisplay() {
  await runAction(() => invoke<HostSnapshot>("disconnect_android_usb"));
  elements.tetherToken.value = "";
  elements.tetherPairingResult.hidden = true;
  elements.tetherDiscoveryResult.hidden = true;
  elements.tetherCandidatesLabel.hidden = true;
  elements.tetherCandidates.hidden = true;
  elements.usbProbeResult.hidden = true;
}

elements.start.addEventListener("click", () => {
  void startSession();
});

elements.stop.addEventListener("click", () => {
  void runAction(() => invoke<HostSnapshot>("stop_loopback"));
});

elements.refresh.addEventListener("click", () => void refreshSnapshot());

elements.requestPermission.addEventListener("click", () => {
  void runAction(() => invoke<HostSnapshot>("request_screen_capture_access"));
});

elements.runCaptureProbe.addEventListener("click", () => void runCaptureProbe());
elements.enableVirtualDisplay.addEventListener("click", () => void changeVirtualDisplay(true));
elements.disableVirtualDisplay.addEventListener("click", () => void changeVirtualDisplay(false));
elements.prepareAndroidUsb.addEventListener("click", () => void prepareAndroidUsb());
elements.pairAndroidTether.addEventListener("click", () => void pairAndroidTether());
elements.discoverAndroidTether.addEventListener("click", () => void discoverAndroidTether());
elements.tetherCandidates.addEventListener("change", () => {
  elements.tetherEndpoint.value = elements.tetherCandidates.value;
});
elements.tetherToken.addEventListener("input", () => {
  const symbols = elements.tetherToken.value
    .toUpperCase()
    .replace(/[ -]/g, "")
    .slice(0, 16);
  elements.tetherToken.value = symbols.match(/.{1,4}/g)?.join("-") ?? "";
});
elements.tetherToken.addEventListener("keydown", (event) => {
  if (event.key === "Enter") {
    event.preventDefault();
    void pairAndroidTether();
  }
});
elements.disconnectAndroidUsb.addEventListener("click", () => {
  void disconnectWiredDisplay();
});

document.querySelectorAll<HTMLButtonElement>("[data-fps]").forEach((button) => {
  button.addEventListener("click", () => {
    selectedFps = Number(button.dataset.fps);
    document.querySelectorAll<HTMLButtonElement>("[data-fps]").forEach((candidate) => {
      const selected = candidate === button;
      candidate.classList.toggle("is-selected", selected);
      candidate.setAttribute("aria-checked", String(selected));
    });
  });
});

window.addEventListener("DOMContentLoaded", () => void refreshSnapshot());
window.addEventListener("beforeunload", () => {
  if (pollingHandle !== undefined) window.clearInterval(pollingHandle);
});
