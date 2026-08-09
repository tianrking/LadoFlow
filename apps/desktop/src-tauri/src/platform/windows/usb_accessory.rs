//! Windows Android Open Accessory discovery and explicit mode switching.
//!
//! Read-only status never sends vendor requests. Mode switching only happens
//! after the user invokes the dedicated command, because probing endpoint zero
//! on unrelated USB devices would be an inappropriate background side effect.

use std::{
    thread,
    time::{Duration, Instant},
};

use ladoflow_transport::{
    AccessoryControlIo, AccessoryIdentity, AoaNegotiationError, is_aoa_app_accessory,
    negotiate_accessory_mode,
};
use rusb::{ConfigDescriptor, Context, Device, DeviceHandle, Direction, TransferType, UsbContext};

use super::super::UsbAccessoryProbeReport;

const REENUMERATION_TIMEOUT: Duration = Duration::from_secs(8);
const REENUMERATION_POLL_INTERVAL: Duration = Duration::from_millis(100);

struct BulkEndpoints {
    interface: u8,
    input: u8,
    output: u8,
    max_packet_size: u16,
}

struct ControlHandle<'a>(&'a DeviceHandle<Context>);

impl AccessoryControlIo for ControlHandle<'_> {
    type Error = rusb::Error;

    fn read_control(
        &mut self,
        request_type: u8,
        request: u8,
        value: u16,
        index: u16,
        buffer: &mut [u8],
        timeout: Duration,
    ) -> Result<usize, Self::Error> {
        self.0
            .read_control(request_type, request, value, index, buffer, timeout)
    }

    fn write_control(
        &mut self,
        request_type: u8,
        request: u8,
        value: u16,
        index: u16,
        buffer: &[u8],
        timeout: Duration,
    ) -> Result<usize, Self::Error> {
        self.0
            .write_control(request_type, request, value, index, buffer, timeout)
    }
}

pub(super) fn collect_status() -> String {
    let context = match Context::new() {
        Ok(context) => context,
        Err(error) => return format!("USB host initialization failed: {error}"),
    };
    match find_accessory_devices(&context) {
        Ok(devices) if devices.is_empty() => {
            "USB host ready; no Android AOA app accessory is enumerated. Plug in Android and choose Prepare Android USB. Windows may require a signed WinUSB-compatible driver binding."
                .to_owned()
        }
        Ok(devices) => format!(
            "USB host sees {} Android AOA app accessor{}; use Prepare Android USB to verify driver access and bulk endpoints",
            devices.len(),
            if devices.len() == 1 { "y" } else { "ies" }
        ),
        Err(error) => format!("USB enumeration failed: {error}"),
    }
}

pub(super) fn prepare_android_accessory() -> UsbAccessoryProbeReport {
    match prepare_android_accessory_inner() {
        Ok(report) => report,
        Err(error) => UsbAccessoryProbeReport::failed(error),
    }
}

fn prepare_android_accessory_inner() -> Result<UsbAccessoryProbeReport, String> {
    let context =
        Context::new().map_err(|error| format!("failed to initialize libusb: {error}"))?;
    if let Some(device) = find_accessory_devices(&context)?.into_iter().next() {
        return verify_accessory(&device, None);
    }

    let identity = AccessoryIdentity::ladoflow(host_description(), env!("CARGO_PKG_VERSION"), "")
        .map_err(|error| format!("invalid LadoFlow AOA identity: {error}"))?;
    let devices = context
        .devices()
        .map_err(|error| format!("failed to enumerate USB devices: {error}"))?;
    let mut attempted = 0_usize;
    let mut inaccessible = 0_usize;
    let mut failures = Vec::new();

    for device in devices.iter() {
        let descriptor = match device.device_descriptor() {
            Ok(descriptor) => descriptor,
            Err(error) => {
                failures.push(format!("descriptor: {error}"));
                continue;
            }
        };
        if !is_android_probe_candidate(
            descriptor.vendor_id(),
            descriptor.product_id(),
            descriptor.class_code(),
        ) {
            continue;
        }
        let handle = match device.open() {
            Ok(handle) => handle,
            Err(error) => {
                inaccessible += 1;
                failures.push(format!(
                    "bus {} device {} {:04x}:{:04x} could not open: {error}",
                    device.bus_number(),
                    device.address(),
                    descriptor.vendor_id(),
                    descriptor.product_id()
                ));
                continue;
            }
        };
        attempted += 1;
        let negotiation = negotiate_accessory_mode(&mut ControlHandle(&handle), &identity);
        match negotiation {
            Ok(protocol) => {
                drop(handle);
                let started = Instant::now();
                while started.elapsed() < REENUMERATION_TIMEOUT {
                    if let Some(accessory) = find_accessory_devices(&context)?.into_iter().next() {
                        return verify_accessory(&accessory, Some(protocol.get()));
                    }
                    thread::sleep(REENUMERATION_POLL_INTERVAL);
                }
                return Err(format!(
                    "Android accepted AOA {} but did not re-enumerate as a Google accessory within {} seconds",
                    protocol.get(),
                    REENUMERATION_TIMEOUT.as_secs()
                ));
            }
            Err(AoaNegotiationError::Control {
                source: rusb::Error::Pipe,
                ..
            }) => {}
            Err(error) => failures.push(format!(
                "bus {} device {} {:04x}:{:04x}: {error}",
                device.bus_number(),
                device.address(),
                descriptor.vendor_id(),
                descriptor.product_id()
            )),
        }
    }

    let evidence = failures.into_iter().take(3).collect::<Vec<_>>().join("; ");
    Err(format!(
        "no connected USB device completed the AOA protocol query (attempted {attempted}, inaccessible {inaccessible}). On Windows, install the product's signed WinUSB-compatible binding for the Android interface before retrying{}{}",
        if evidence.is_empty() { "" } else { ": " },
        evidence
    ))
}

fn find_accessory_devices(context: &Context) -> Result<Vec<Device<Context>>, String> {
    let devices = context
        .devices()
        .map_err(|error| format!("failed to list USB devices: {error}"))?;
    let mut accessories = Vec::new();
    for device in devices.iter() {
        let descriptor = device
            .device_descriptor()
            .map_err(|error| format!("failed to read USB device descriptor: {error}"))?;
        if is_aoa_app_accessory(descriptor.vendor_id(), descriptor.product_id()) {
            accessories.push(device);
        }
    }
    Ok(accessories)
}

fn verify_accessory(
    device: &Device<Context>,
    protocol_version: Option<u16>,
) -> Result<UsbAccessoryProbeReport, String> {
    let descriptor = device
        .device_descriptor()
        .map_err(|error| format!("failed to read AOA descriptor: {error}"))?;
    let configuration = device
        .active_config_descriptor()
        .or_else(|_error| device.config_descriptor(0))
        .map_err(|error| format!("failed to read AOA configuration: {error}"))?;
    let endpoints = find_bulk_endpoints(&configuration).ok_or_else(|| {
        "AOA configuration has no interface with bulk IN and OUT endpoints".to_owned()
    })?;
    let handle = device.open().map_err(|error| {
        format!(
            "AOA device {:04x}:{:04x} is visible but cannot be opened ({error}). Windows needs a signed WinUSB-compatible driver for this interface",
            descriptor.vendor_id(), descriptor.product_id()
        )
    })?;
    let active = handle
        .active_configuration()
        .map_err(|error| format!("failed to query active AOA configuration: {error}"))?;
    if active != configuration.number() {
        handle
            .set_active_configuration(configuration.number())
            .map_err(|error| format!("failed to activate AOA configuration: {error}"))?;
    }
    handle.claim_interface(endpoints.interface).map_err(|error| {
        format!(
            "AOA device is visible but interface {} cannot be claimed ({error}). Another application or an incompatible Windows driver owns it",
            endpoints.interface
        )
    })?;
    handle
        .release_interface(endpoints.interface)
        .map_err(|error| format!("failed to release verified AOA interface: {error}"))?;

    Ok(UsbAccessoryProbeReport {
        passed: true,
        state: "ready".to_owned(),
        detail: "AOA app interface opened and its duplex bulk endpoints were claimed successfully"
            .to_owned(),
        protocol_version,
        bus_number: Some(device.bus_number()),
        device_address: Some(device.address()),
        vendor_id: Some(descriptor.vendor_id()),
        product_id: Some(descriptor.product_id()),
        interface_number: Some(endpoints.interface),
        input_endpoint: Some(endpoints.input),
        output_endpoint: Some(endpoints.output),
        max_packet_size: Some(endpoints.max_packet_size),
    })
}

fn find_bulk_endpoints(configuration: &ConfigDescriptor) -> Option<BulkEndpoints> {
    for interface in configuration.interfaces() {
        for descriptor in interface.descriptors() {
            let mut input = None;
            let mut output = None;
            let mut max_packet_size = 0_u16;
            for endpoint in descriptor.endpoint_descriptors() {
                if endpoint.transfer_type() != TransferType::Bulk {
                    continue;
                }
                max_packet_size = max_packet_size.max(endpoint.max_packet_size());
                match endpoint.direction() {
                    Direction::In => input.get_or_insert(endpoint.address()),
                    Direction::Out => output.get_or_insert(endpoint.address()),
                };
            }
            if let (Some(input), Some(output)) = (input, output) {
                return Some(BulkEndpoints {
                    interface: descriptor.interface_number(),
                    input,
                    output,
                    max_packet_size,
                });
            }
        }
    }
    None
}

const fn is_android_probe_candidate(vendor_id: u16, product_id: u16, class_code: u8) -> bool {
    vendor_id != 0
        && vendor_id != u16::MAX
        && !is_aoa_app_accessory(vendor_id, product_id)
        && matches!(class_code, 0x00 | 0xef | 0xff)
}

fn host_description() -> String {
    std::env::var("COMPUTERNAME")
        .ok()
        .filter(|name| !name.trim().is_empty())
        .map_or_else(
            || "LadoFlow Windows host".to_owned(),
            |name| format!("LadoFlow on {name}"),
        )
}

#[cfg(test)]
mod tests {
    use super::is_android_probe_candidate;

    #[test]
    fn background_probe_candidates_exclude_device_classes_with_other_roles() {
        assert!(is_android_probe_candidate(0x18d1, 0x4ee7, 0x00));
        assert!(is_android_probe_candidate(0x04e8, 0x6860, 0xef));
        assert!(!is_android_probe_candidate(0x18d1, 0x2d00, 0x00));
        assert!(!is_android_probe_candidate(0x1234, 0x5678, 0x03));
        assert!(!is_android_probe_candidate(0, 0, 0));
    }
}
