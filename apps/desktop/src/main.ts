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
  elements.captureBackend.textContent = `${snapshot.platform.captureBackend}. ${snapshot.platform.virtualDisplayStatus}`;
  elements.requestPermission.hidden = permissionGranted || permissionUnsupported;
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
  clearError();
  try {
    render(await action());
  } catch (error) {
    showError(error);
    await refreshSnapshot(false);
  } finally {
    busy = false;
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
