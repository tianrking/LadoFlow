import { invoke } from "@tauri-apps/api/core";

type SessionPhase =
  | "idle"
  | "negotiating"
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
    usbStatus: string;
    capturePermission: CapturePermission;
    virtualDisplayStatus: string;
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

const elements = {
  appVersion: getElement("app-version"),
  hostPlatform: getElement("host-platform"),
  hostStatusDot: getElement("host-status-dot"),
  protocolVersion: getElement("protocol-version"),
  sessionBadge: getElement("session-badge"),
  sessionTitle: getElement("connection-title"),
  sessionCopy: getElement("session-copy"),
  linkPath: getElement("link-path"),
  start: getButton("start-session"),
  stop: getButton("stop-session"),
  refresh: getButton("refresh-status"),
  requestPermission: getButton("request-permission"),
  runCaptureProbe: getButton("run-capture-probe"),
  captureProbeResult: getElement("capture-probe-result"),
  prepareAndroidUsb: getButton("prepare-android-usb"),
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

function sessionPresentation(phase: SessionPhase) {
  switch (phase) {
    case "negotiating":
      return {
        title: "Negotiating the display",
        copy: "Exchanging protocol versions, capabilities, and stream configuration.",
        label: "Negotiating",
        tone: "warn" as const,
      };
    case "streaming":
      return {
        title: "Loopback display is live",
        copy: "Synthetic frames are crossing the same bounded core path used by physical links.",
        label: "Streaming",
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
      return {
        title: "Ready for a nearby screen",
        copy: "Start the deterministic loopback to validate negotiation, transport, pacing, and telemetry before attaching a physical device.",
        label: "Idle",
        tone: "idle" as const,
      };
  }
}

function renderDisplays(displays: DisplaySource[]) {
  elements.displayList.replaceChildren();
  if (displays.length === 0) {
    const empty = document.createElement("p");
    empty.className = "display-empty";
    empty.textContent = "No active displays reported by this platform adapter.";
    elements.displayList.append(empty);
    return;
  }

  for (const display of displays) {
    const row = document.createElement("div");
    row.className = "display-row";

    const glyph = document.createElement("span");
    glyph.className = "display-glyph";
    glyph.setAttribute("aria-hidden", "true");

    const details = document.createElement("div");
    const name = document.createElement("strong");
    name.textContent = display.name;
    const resolution = document.createElement("small");
    resolution.textContent = `${display.width} × ${display.height}${display.primary ? " · Main" : ""}`;
    details.append(name, resolution);

    row.append(glyph, details);
    elements.displayList.append(row);
  }
}

function render(snapshot: HostSnapshot) {
  const presentation = sessionPresentation(snapshot.session.phase);
  const isRunning = snapshot.session.phase === "streaming" || snapshot.session.phase === "negotiating";

  elements.appVersion.textContent = `LadoFlow ${snapshot.appVersion}`;
  elements.hostPlatform.textContent = formatPlatform(snapshot);
  elements.hostStatusDot.className = "status-dot status-dot--cyan";
  elements.protocolVersion.textContent = `LDFL v${snapshot.protocolVersion}`;
  elements.sessionTitle.textContent = presentation.title;
  elements.sessionCopy.textContent = presentation.copy;
  setBadge(elements.sessionBadge, presentation.label, presentation.tone);
  elements.linkPath.classList.toggle("is-active", snapshot.session.phase === "streaming");
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
  elements.captureBackend.textContent = `${snapshot.platform.captureBackend}. ${snapshot.platform.encoderStatus}. ${snapshot.platform.virtualDisplayStatus}`;
  elements.usbStatus.textContent = snapshot.platform.usbStatus;
  elements.requestPermission.hidden = permissionGranted || permissionUnsupported;
  elements.requestPermission.disabled = busy;
  const hasNativeCaptureProbe = snapshot.os === "macos" || snapshot.os === "windows";
  elements.runCaptureProbe.hidden = !hasNativeCaptureProbe || !permissionGranted;
  elements.runCaptureProbe.disabled = busy;
  elements.prepareAndroidUsb.hidden = snapshot.os !== "windows";
  elements.prepareAndroidUsb.disabled = busy;
  selectedDisplayId =
    snapshot.platform.displays.find((display) => display.primary)?.id ??
    snapshot.platform.displays[0]?.id ??
    null;
  renderDisplays(snapshot.platform.displays);

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
  elements.prepareAndroidUsb.disabled = true;
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
  elements.prepareAndroidUsb.disabled = true;
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

function hexByte(value: number): string {
  return `0x${value.toString(16).padStart(2, "0")}`;
}

function hexWord(value: number): string {
  return value.toString(16).padStart(4, "0");
}

async function prepareAndroidUsb() {
  if (busy) return;
  busy = true;
  const idleLabel = elements.prepareAndroidUsb.textContent;
  elements.prepareAndroidUsb.disabled = true;
  elements.prepareAndroidUsb.textContent = "Preparing Android USB…";
  elements.start.disabled = true;
  elements.stop.disabled = true;
  elements.runCaptureProbe.disabled = true;
  clearError();

  try {
    const report = await invoke<UsbAccessoryProbeReport>("prepare_android_usb");
    renderUsbProbe(report);
  } catch (error) {
    showError(error);
  } finally {
    busy = false;
    elements.prepareAndroidUsb.textContent = idleLabel;
    await refreshSnapshot(false);
  }
}

elements.start.addEventListener("click", () => {
  void runAction(() => invoke<HostSnapshot>("start_loopback", { config: selectedConfig() }));
});

elements.stop.addEventListener("click", () => {
  void runAction(() => invoke<HostSnapshot>("stop_loopback"));
});

elements.refresh.addEventListener("click", () => void refreshSnapshot());

elements.requestPermission.addEventListener("click", () => {
  void runAction(() => invoke<HostSnapshot>("request_screen_capture_access"));
});

elements.runCaptureProbe.addEventListener("click", () => void runCaptureProbe());
elements.prepareAndroidUsb.addEventListener("click", () => void prepareAndroidUsb());

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
