use std::{
    collections::HashSet,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Mutex, MutexGuard, OnceLock},
    thread,
    time::{Duration, Instant},
};

use serde::Deserialize;

use super::enumerate_display_sources;
use crate::platform::{VirtualDisplayActionReport, VirtualDisplayState, VirtualDisplayStatus};

const CONTROLLER_NAME: &str = "LadoFlowVirtualDisplay.exe";
const CONTROLLER_PROTOCOL_VERSION: u16 = 1;
const STATUS_CACHE_TTL: Duration = Duration::from_secs(1);
const STATUS_TIMEOUT: Duration = Duration::from_secs(5);
const ENABLE_TIMEOUT: Duration = Duration::from_secs(45);
const DISABLE_TIMEOUT: Duration = Duration::from_secs(15);
const MONITOR_APPEAR_TIMEOUT: Duration = Duration::from_secs(12);
const MONITOR_REMOVE_TIMEOUT: Duration = Duration::from_secs(8);
const MONITOR_POLL_INTERVAL: Duration = Duration::from_millis(100);
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(20);
const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const SUCCESS_HRESULT: &str = "0x00000000";

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ControllerReport {
    protocol_version: u16,
    service_installed: bool,
    service_state: String,
    state: String,
    request_result: String,
    last_error: String,
    device_instance_id: String,
    generation: u64,
    changed: bool,
}

struct ControllerOutcome {
    report: ControllerReport,
    process_succeeded: bool,
}

struct CachedStatus {
    measured_at: Instant,
    status: VirtualDisplayStatus,
}

pub fn status() -> VirtualDisplayStatus {
    {
        let cache = lock_cache();
        if let Some(cached) = cache.as_ref()
            && cached.measured_at.elapsed() < STATUS_CACHE_TTL
        {
            return cached.status.clone();
        }
    }

    let measured = status_uncached();
    store_status(measured.clone());
    measured
}

pub fn enable() -> Result<VirtualDisplayActionReport, String> {
    let started = Instant::now();
    let baseline_ids = enumerate_display_sources()
        .unwrap_or_default()
        .into_iter()
        .map(|display| display.id)
        .collect::<HashSet<_>>();
    let Some(controller) = find_controller() else {
        let status = client_missing_status();
        store_status(status.clone());
        return Ok(action_report(false, status, None, started));
    };

    let outcome = run_controller(&controller, "start", ENABLE_TIMEOUT)?;
    let mut status = status_from_report(&outcome.report)?;
    let command_succeeded = outcome.process_succeeded
        && hresult_succeeded(&outcome.report.request_result)?
        && status.enabled;
    if !command_succeeded {
        status.detail = format!(
            "Virtual display enable failed (request {}, device {}). {}",
            outcome.report.request_result, outcome.report.last_error, status.detail
        );
        store_status(status.clone());
        return Ok(action_report(false, status, None, started));
    }

    let deadline = Instant::now() + MONITOR_APPEAR_TIMEOUT;
    loop {
        match enumerate_display_sources() {
            Ok(displays) => {
                if let Some(display) = displays.iter().find(|display| display.virtual_display) {
                    let selected = display.id.clone();
                    status.detail = if baseline_ids.contains(&selected) {
                        "LadoFlow virtual display is enabled and selected.".to_owned()
                    } else {
                        "LadoFlow virtual display appeared and was selected automatically."
                            .to_owned()
                    };
                    store_status(status.clone());
                    return Ok(action_report(true, status, Some(selected), started));
                }
            }
            Err(error) => {
                if Instant::now() >= deadline {
                    status.detail = format!(
                        "The service enabled the device, but monitor enumeration failed: {error}"
                    );
                    store_status(status.clone());
                    return Ok(action_report(false, status, None, started));
                }
            }
        }

        if Instant::now() >= deadline {
            "The service enabled the software device, but no active LadoFlow HMONITOR appeared within 12 seconds. Check driver installation and Device Manager."
                .clone_into(&mut status.detail);
            store_status(status.clone());
            return Ok(action_report(false, status, None, started));
        }
        thread::sleep(MONITOR_POLL_INTERVAL);
    }
}

pub fn disable() -> Result<VirtualDisplayActionReport, String> {
    let started = Instant::now();
    let Some(controller) = find_controller() else {
        let status = client_missing_status();
        store_status(status.clone());
        return Ok(action_report(false, status, None, started));
    };

    let outcome = run_controller(&controller, "stop", DISABLE_TIMEOUT)?;
    let mut status = status_from_report(&outcome.report)?;
    if !outcome.process_succeeded || !hresult_succeeded(&outcome.report.request_result)? {
        status.detail = format!(
            "Virtual display disable failed (request {}, device {}). {}",
            outcome.report.request_result, outcome.report.last_error, status.detail
        );
        store_status(status.clone());
        return Ok(action_report(false, status, None, started));
    }

    let deadline = Instant::now() + MONITOR_REMOVE_TIMEOUT;
    loop {
        match enumerate_display_sources() {
            Ok(displays) if displays.iter().all(|display| !display.virtual_display) => {
                status.detail = if outcome.report.changed {
                    "LadoFlow virtual display was disabled and removed from the active desktop."
                        .to_owned()
                } else {
                    "LadoFlow virtual display was already disabled.".to_owned()
                };
                store_status(status.clone());
                return Ok(action_report(true, status, None, started));
            }
            Ok(_) | Err(_) if Instant::now() < deadline => {
                thread::sleep(MONITOR_POLL_INTERVAL);
            }
            Ok(_) => {
                "The service closed the software-device handle, but Windows still reported the virtual monitor after 8 seconds."
                    .clone_into(&mut status.detail);
                store_status(status.clone());
                return Ok(action_report(false, status, None, started));
            }
            Err(error) => {
                status.detail = format!(
                    "The service disabled the device, but monitor removal could not be verified: {error}"
                );
                store_status(status.clone());
                return Ok(action_report(false, status, None, started));
            }
        }
    }
}

fn status_uncached() -> VirtualDisplayStatus {
    let Some(controller) = find_controller() else {
        return client_missing_status();
    };
    match run_controller(&controller, "status", STATUS_TIMEOUT) {
        Ok(outcome) => status_from_report(&outcome.report).unwrap_or_else(failed_status),
        Err(error) => failed_status(error),
    }
}

fn run_controller(
    path: &Path,
    command: &str,
    timeout: Duration,
) -> Result<ControllerOutcome, String> {
    use std::os::windows::process::CommandExt as _;

    let mut child = Command::new(path)
        .arg(command)
        .creation_flags(CREATE_NO_WINDOW)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| {
            format!(
                "failed to launch Windows virtual-display controller `{}`: {error}",
                path.display()
            )
        })?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => {
                let output = child.wait_with_output().map_err(|error| {
                    format!("failed to collect virtual-display controller output: {error}")
                })?;
                let stdout = String::from_utf8(output.stdout)
                    .map_err(|_| "virtual-display controller emitted non-UTF-8 JSON".to_owned())?;
                let report: ControllerReport =
                    serde_json::from_str(stdout.trim()).map_err(|error| {
                        let stderr = String::from_utf8_lossy(&output.stderr);
                        format!(
                            "invalid virtual-display controller JSON: {error}; stderr: {}",
                            stderr.trim()
                        )
                    })?;
                validate_report(&report)?;
                return Ok(ControllerOutcome {
                    report,
                    process_succeeded: output.status.success(),
                });
            }
            Ok(None) if Instant::now() < deadline => thread::sleep(PROCESS_POLL_INTERVAL),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "virtual-display controller `{command}` timed out after {} ms",
                    timeout.as_millis()
                ));
            }
            Err(error) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!(
                    "failed to wait for virtual-display controller: {error}"
                ));
            }
        }
    }
}

fn validate_report(report: &ControllerReport) -> Result<(), String> {
    if report.protocol_version != CONTROLLER_PROTOCOL_VERSION {
        return Err(format!(
            "virtual-display controller protocol {} is incompatible with host protocol {}",
            report.protocol_version, CONTROLLER_PROTOCOL_VERSION
        ));
    }
    let _request_result = parse_hresult(&report.request_result)?;
    let _last_error = parse_hresult(&report.last_error)?;
    if report.device_instance_id.encode_utf16().count() >= 260 {
        return Err("virtual-display controller returned an oversized device ID".to_owned());
    }
    Ok(())
}

fn status_from_report(report: &ControllerReport) -> Result<VirtualDisplayStatus, String> {
    validate_report(report)?;
    let last_error = (report.last_error != SUCCESS_HRESULT).then(|| report.last_error.clone());
    let device_instance_id =
        (!report.device_instance_id.is_empty()).then(|| report.device_instance_id.clone());
    let state = if !report.service_installed {
        VirtualDisplayState::NotInstalled
    } else if report.service_state != "running" {
        VirtualDisplayState::ServiceStopped
    } else {
        match report.state.as_str() {
            "ready" => VirtualDisplayState::Ready,
            "enabling" => VirtualDisplayState::Enabling,
            "enabled" => VirtualDisplayState::Enabled,
            "disabling" => VirtualDisplayState::Disabling,
            "failed" => VirtualDisplayState::Failed,
            "stopping" => VirtualDisplayState::Stopping,
            "unavailable" => VirtualDisplayState::ServiceStopped,
            other => {
                return Err(format!(
                    "virtual-display controller returned unknown state `{other}`"
                ));
            }
        }
    };
    let detail = match state {
        VirtualDisplayState::NotInstalled => {
            "LadoFlow virtual-display service is not installed. Install the Windows host package to enable a true extended desktop."
                .to_owned()
        }
        VirtualDisplayState::ServiceStopped => {
            "LadoFlow virtual-display service is installed but not running.".to_owned()
        }
        VirtualDisplayState::Ready => {
            "LadoFlow virtual-display service is ready; the virtual monitor is disabled."
                .to_owned()
        }
        VirtualDisplayState::Enabling => "Windows is creating the LadoFlow virtual monitor."
            .to_owned(),
        VirtualDisplayState::Enabled => {
            "LadoFlow virtual monitor is enabled and available to Windows.".to_owned()
        }
        VirtualDisplayState::Disabling => {
            "Windows is removing the LadoFlow virtual monitor.".to_owned()
        }
        VirtualDisplayState::Failed => format!(
            "LadoFlow virtual-display device failed: {}.",
            last_error.as_deref().unwrap_or("unknown HRESULT")
        ),
        VirtualDisplayState::Stopping => {
            "LadoFlow virtual-display service is stopping.".to_owned()
        }
        VirtualDisplayState::Unsupported | VirtualDisplayState::ClientMissing => {
            unreachable!("controller reports cannot construct non-Windows states")
        }
    };
    Ok(VirtualDisplayStatus {
        state,
        detail,
        service_installed: report.service_installed,
        service_state: report.service_state.clone(),
        enabled: matches!(state, VirtualDisplayState::Enabled),
        device_instance_id,
        last_error,
        generation: report.generation,
    })
}

fn hresult_succeeded(value: &str) -> Result<bool, String> {
    Ok(parse_hresult(value)? & 0x8000_0000 == 0)
}

fn parse_hresult(value: &str) -> Result<u32, String> {
    let Some(hex) = value.strip_prefix("0x") else {
        return Err(format!("invalid HRESULT `{value}`"));
    };
    if hex.len() != 8 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(format!("invalid HRESULT `{value}`"));
    }
    u32::from_str_radix(hex, 16).map_err(|error| format!("invalid HRESULT `{value}`: {error}"))
}

fn find_controller() -> Option<PathBuf> {
    controller_candidates()
        .into_iter()
        .find(|path| path.is_file())
}

fn controller_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if cfg!(debug_assertions) {
        if let Some(override_path) = std::env::var_os("LADOFLOW_VIRTUAL_DISPLAY_CONTROLLER") {
            candidates.push(PathBuf::from(override_path));
        }
    }
    if let Ok(executable) = std::env::current_exe()
        && let Some(directory) = executable.parent()
    {
        candidates.extend([
            directory.join(CONTROLLER_NAME),
            directory.join("windows").join(CONTROLLER_NAME),
            directory
                .join("resources")
                .join("windows")
                .join(CONTROLLER_NAME),
        ]);
    }
    if cfg!(debug_assertions) {
        candidates.push(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../platform/windows/idd/dist/Release/x64")
                .join(CONTROLLER_NAME),
        );
    }

    let mut unique = HashSet::new();
    candidates
        .into_iter()
        .filter(|path| unique.insert(path.clone()))
        .collect()
}

fn client_missing_status() -> VirtualDisplayStatus {
    VirtualDisplayStatus {
        state: VirtualDisplayState::ClientMissing,
        detail: "LadoFlow virtual-display controller is not bundled with this host build. Existing-monitor capture remains available."
            .to_owned(),
        service_installed: false,
        service_state: "unavailable".to_owned(),
        enabled: false,
        device_instance_id: None,
        last_error: None,
        generation: 0,
    }
}

fn failed_status(error: String) -> VirtualDisplayStatus {
    VirtualDisplayStatus {
        state: VirtualDisplayState::Failed,
        detail: error.clone(),
        service_installed: false,
        service_state: "unknown".to_owned(),
        enabled: false,
        device_instance_id: None,
        last_error: Some(error),
        generation: 0,
    }
}

fn action_report(
    passed: bool,
    status: VirtualDisplayStatus,
    selected_display_id: Option<String>,
    started: Instant,
) -> VirtualDisplayActionReport {
    VirtualDisplayActionReport {
        passed,
        status,
        selected_display_id,
        elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
    }
}

fn status_cache() -> &'static Mutex<Option<CachedStatus>> {
    static CACHE: OnceLock<Mutex<Option<CachedStatus>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(None))
}

fn lock_cache() -> MutexGuard<'static, Option<CachedStatus>> {
    status_cache()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn store_status(status: VirtualDisplayStatus) {
    *lock_cache() = Some(CachedStatus {
        measured_at: Instant::now(),
        status,
    });
}

#[cfg(test)]
mod tests {
    use super::{
        CONTROLLER_PROTOCOL_VERSION, ControllerReport, VirtualDisplayState, hresult_succeeded,
        parse_hresult, status_from_report,
    };

    fn report() -> ControllerReport {
        ControllerReport {
            protocol_version: CONTROLLER_PROTOCOL_VERSION,
            service_installed: true,
            service_state: "running".to_owned(),
            state: "enabled".to_owned(),
            request_result: "0x00000000".to_owned(),
            last_error: "0x00000000".to_owned(),
            device_instance_id: "SWD\\LadoFlowVirtualDisplay\\1".to_owned(),
            generation: 4,
            changed: true,
        }
    }

    #[test]
    fn controller_report_maps_enabled_state() {
        let status = status_from_report(&report()).expect("valid controller report");
        assert_eq!(status.state, VirtualDisplayState::Enabled);
        assert!(status.enabled);
        assert!(status.service_installed);
        assert_eq!(status.generation, 4);
        assert_eq!(
            status.device_instance_id.as_deref(),
            Some("SWD\\LadoFlowVirtualDisplay\\1")
        );
    }

    #[test]
    fn missing_service_is_distinct_from_device_failure() {
        let mut missing = report();
        missing.service_installed = false;
        missing.service_state = "stopped".to_owned();
        missing.state = "unavailable".to_owned();
        missing.request_result = "0x80070424".to_owned();
        missing.last_error = "0x80070424".to_owned();
        let status = status_from_report(&missing).expect("valid missing-service report");
        assert_eq!(status.state, VirtualDisplayState::NotInstalled);
        assert!(!status.enabled);
        assert_eq!(status.last_error.as_deref(), Some("0x80070424"));
    }

    #[test]
    fn hresult_parser_enforces_exact_wire_shape_and_severity() {
        assert_eq!(parse_hresult("0x80070005").expect("HRESULT"), 0x8007_0005);
        assert!(!hresult_succeeded("0x80070005").expect("failure HRESULT"));
        assert!(hresult_succeeded("0x00000001").expect("success HRESULT"));
        assert!(parse_hresult("80070005").is_err());
        assert!(parse_hresult("0x123").is_err());
        assert!(parse_hresult("0xZZZZZZZZ").is_err());
    }

    #[test]
    fn incompatible_controller_versions_are_rejected() {
        let mut incompatible = report();
        incompatible.protocol_version += 1;
        assert!(status_from_report(&incompatible).is_err());
    }
}
