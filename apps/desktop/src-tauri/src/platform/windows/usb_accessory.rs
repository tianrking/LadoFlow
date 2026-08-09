//! Windows Android Open Accessory discovery and explicit mode switching.
//!
//! Read-only status never sends vendor requests. Mode switching only happens
//! after the user invokes the dedicated command, because probing endpoint zero
//! on unrelated USB devices would be an inappropriate background side effect.

use std::{
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    thread::JoinHandle,
    time::{Duration, Instant},
};

use ladoflow_protocol::{
    DecodeOutcome, FRAME_HEADER_LEN, Frame as WireFrame, FrameDecoder, MAX_CONTROL_PAYLOAD,
    MAX_MEDIA_PAYLOAD, MessageType,
};
use ladoflow_transport::{
    AccessoryControlIo, AccessoryIdentity, AoaNegotiationError, Channel, ConnectionState,
    LoopbackConfig, LoopbackEndpoint, Packet, PacketTransport, QueueLimits, ReceiveError,
    SendError, SendReport, is_aoa_app_accessory, loopback_pair, negotiate_accessory_mode,
};
use rusb::{ConfigDescriptor, Context, Device, DeviceHandle, Direction, TransferType, UsbContext};

use super::super::{UsbAccessoryProbeReport, UsbLinkState};

const REENUMERATION_TIMEOUT: Duration = Duration::from_secs(8);
const REENUMERATION_POLL_INTERVAL: Duration = Duration::from_millis(100);
const BULK_TRANSFER_BYTES: usize = 64 * 1_024;
const BULK_READ_TIMEOUT: Duration = Duration::from_millis(2);
const BULK_WRITE_TIMEOUT: Duration = Duration::from_secs(1);
const USB_QUEUE_CONFIG: LoopbackConfig = LoopbackConfig::new(
    valid_queue_limits(
        64,
        4 * 1_024 * 1_024,
        FRAME_HEADER_LEN + MAX_CONTROL_PAYLOAD,
    ),
    valid_queue_limits(3, 32 * 1_024 * 1_024, FRAME_HEADER_LEN + MAX_MEDIA_PAYLOAD),
);

const fn valid_queue_limits(
    max_packets: usize,
    max_queued_bytes: usize,
    max_packet_bytes: usize,
) -> QueueLimits {
    match QueueLimits::new(max_packets, max_queued_bytes, max_packet_bytes) {
        Ok(limits) => limits,
        Err(_) => panic!("built-in Android USB queue limits must be valid"),
    }
}

#[derive(Debug, Clone, Copy)]
struct BulkEndpoints {
    interface: u8,
    input: u8,
    output: u8,
    max_packet_size: u16,
}

#[derive(Debug, Clone)]
struct LinkStatus {
    phase: UsbLinkState,
    detail: String,
    bytes_read: u64,
    bytes_written: u64,
    frames_read: u64,
    frames_written: u64,
}

impl Default for LinkStatus {
    fn default() -> Self {
        Self {
            phase: UsbLinkState::Ready,
            detail: "Android USB has not been connected in this app session".to_owned(),
            bytes_read: 0,
            bytes_written: 0,
            frames_read: 0,
            frames_written: 0,
        }
    }
}

#[derive(Debug)]
struct AccessorySession {
    host_endpoint: LoopbackEndpoint,
    cancel: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl AccessorySession {
    fn stop(mut self) -> Result<(), String> {
        self.cancel.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| "Android USB bulk worker panicked while stopping".to_owned())?;
        }
        Ok(())
    }

    fn is_connected(&self) -> bool {
        self.host_endpoint.connection_state() == ConnectionState::Connected
    }
}

#[derive(Debug)]
struct OpenedAccessory {
    report: UsbAccessoryProbeReport,
    handle: DeviceHandle<Context>,
    endpoints: BulkEndpoints,
}

#[derive(Debug, Clone, Default)]
pub struct UsbAccessoryManager {
    inner: Arc<UsbAccessoryManagerInner>,
}

#[derive(Debug, Default)]
struct UsbAccessoryManagerInner {
    session: Mutex<Option<AccessorySession>>,
    status: Arc<Mutex<LinkStatus>>,
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

impl UsbAccessoryManager {
    pub fn prepare(&self) -> UsbAccessoryProbeReport {
        if let Err(error) = self.stop_active_session() {
            return UsbAccessoryProbeReport::failed(error);
        }
        self.replace_status(LinkStatus {
            phase: UsbLinkState::Connecting,
            detail: "Negotiating Android Open Accessory mode".to_owned(),
            ..LinkStatus::default()
        });

        let opened = match open_android_accessory() {
            Ok(opened) => opened,
            Err(error) => {
                self.replace_status(LinkStatus {
                    phase: UsbLinkState::Failed,
                    detail: error.clone(),
                    ..LinkStatus::default()
                });
                return UsbAccessoryProbeReport::failed(error);
            }
        };
        let mut report = opened.report.clone();
        match start_bulk_session(opened, &self.inner.status) {
            Ok(session) => {
                *self.lock_session() = Some(session);
                "connected".clone_into(&mut report.state);
                "AOA interface remains claimed; the cancellable duplex bulk session is running with bounded LDFL framing"
                    .clone_into(&mut report.detail);
                report
            }
            Err(error) => {
                self.replace_status(LinkStatus {
                    phase: UsbLinkState::Failed,
                    detail: error.clone(),
                    ..LinkStatus::default()
                });
                UsbAccessoryProbeReport::failed(error)
            }
        }
    }

    pub fn disconnect(&self) -> Result<(), String> {
        self.stop_active_session()?;
        let stopped_status = self.lock_status().clone();
        if stopped_status.phase == UsbLinkState::Failed {
            return Err(stopped_status.detail);
        }
        self.replace_status(LinkStatus {
            phase: UsbLinkState::Ready,
            detail: "Android USB session disconnected by the user".to_owned(),
            ..LinkStatus::default()
        });
        Ok(())
    }

    pub fn runtime_status(&self) -> Option<(UsbLinkState, String)> {
        let session_connected = self
            .lock_session()
            .as_ref()
            .is_some_and(AccessorySession::is_connected);
        let status = self.lock_status().clone();
        if status.phase == UsbLinkState::Ready
            && status.detail == "Android USB has not been connected in this app session"
        {
            return None;
        }
        let detail = if session_connected && status.phase == UsbLinkState::Connected {
            format!(
                "{}; received {} frames / {} bytes, sent {} frames / {} bytes",
                status.detail,
                status.frames_read,
                status.bytes_read,
                status.frames_written,
                status.bytes_written
            )
        } else {
            status.detail
        };
        Some((status.phase, detail))
    }

    fn stop_active_session(&self) -> Result<(), String> {
        let session = self.lock_session().take();
        session.map_or(Ok(()), AccessorySession::stop)
    }

    fn lock_session(&self) -> MutexGuard<'_, Option<AccessorySession>> {
        self.inner
            .session
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn lock_status(&self) -> MutexGuard<'_, LinkStatus> {
        self.inner
            .status
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn replace_status(&self, status: LinkStatus) {
        *self.lock_status() = status;
    }
}

impl PacketTransport for UsbAccessoryManager {
    fn connection_state(&self) -> ConnectionState {
        self.lock_session()
            .as_ref()
            .map_or(ConnectionState::Disconnected, |session| {
                session.host_endpoint.connection_state()
            })
    }

    fn try_send(&mut self, packet: Packet) -> Result<SendReport, SendError> {
        let mut session = self.lock_session();
        let Some(session) = session.as_mut() else {
            return Err(SendError::Disconnected(packet));
        };
        session.host_endpoint.try_send(packet)
    }

    fn try_receive(&mut self, channel: Channel) -> Result<Option<Packet>, ReceiveError> {
        let mut session = self.lock_session();
        let Some(session) = session.as_mut() else {
            return Err(ReceiveError::Disconnected);
        };
        session.host_endpoint.try_receive(channel)
    }
}

impl Drop for UsbAccessoryManagerInner {
    fn drop(&mut self) {
        let session = self
            .session
            .get_mut()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        if let Some(session) = session {
            let _result = session.stop();
        }
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

fn open_android_accessory() -> Result<OpenedAccessory, String> {
    let context =
        Context::new().map_err(|error| format!("failed to initialize libusb: {error}"))?;
    if let Some(device) = find_accessory_devices(&context)?.into_iter().next() {
        return open_accessory(&device, None);
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
                        return open_accessory(&accessory, Some(protocol.get()));
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

fn open_accessory(
    device: &Device<Context>,
    protocol_version: Option<u16>,
) -> Result<OpenedAccessory, String> {
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
    Ok(OpenedAccessory {
        report: UsbAccessoryProbeReport {
            passed: true,
            state: "ready".to_owned(),
            detail:
                "AOA app interface opened and its duplex bulk endpoints were claimed successfully"
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
        },
        handle,
        endpoints,
    })
}

fn start_bulk_session(
    opened: OpenedAccessory,
    status: &Arc<Mutex<LinkStatus>>,
) -> Result<AccessorySession, String> {
    let (host_endpoint, device_endpoint) = loopback_pair(USB_QUEUE_CONFIG);
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    let worker_status = Arc::clone(status);
    *lock_link_status(status) = LinkStatus {
        phase: UsbLinkState::Connected,
        detail: "Android Open Accessory interface is claimed and the duplex bulk worker is active"
            .to_owned(),
        ..LinkStatus::default()
    };
    let worker = thread::Builder::new()
        .name("ladoflow-windows-usb".to_owned())
        .spawn(move || {
            run_bulk_session(
                opened.handle,
                opened.endpoints,
                device_endpoint,
                &worker_cancel,
                &worker_status,
            );
        })
        .map_err(|error| format!("failed to start Android USB bulk worker: {error}"))?;
    Ok(AccessorySession {
        host_endpoint,
        cancel,
        worker: Some(worker),
    })
}

fn run_bulk_session(
    handle: DeviceHandle<Context>,
    endpoints: BulkEndpoints,
    mut device_endpoint: LoopbackEndpoint,
    cancel: &AtomicBool,
    status: &Arc<Mutex<LinkStatus>>,
) {
    let result = pump_bulk_stream(&handle, endpoints, &mut device_endpoint, cancel, status);
    let _discarded = device_endpoint.disconnect();
    let release_result = handle.release_interface(endpoints.interface);

    let mut link_status = lock_link_status(status);
    if cancel.load(Ordering::Acquire) {
        if let Err(release_error) = release_result {
            link_status.phase = UsbLinkState::Failed;
            link_status.detail =
                format!("Android USB stopped, but releasing its interface failed: {release_error}");
        } else {
            link_status.phase = UsbLinkState::Ready;
            "Android USB bulk session stopped cleanly".clone_into(&mut link_status.detail);
        }
    } else {
        link_status.phase = UsbLinkState::Failed;
        link_status.detail = match (result, release_result) {
            (Err(error), Err(release_error)) => {
                format!("{error}; releasing USB interface also failed: {release_error}")
            }
            (Err(error), Ok(())) => error,
            (Ok(()), Err(release_error)) => {
                format!("Android USB worker ended; interface release failed: {release_error}")
            }
            (Ok(()), Ok(())) => "Android USB bulk worker ended unexpectedly".to_owned(),
        };
    }
    drop(handle);
}

fn pump_bulk_stream(
    handle: &DeviceHandle<Context>,
    endpoints: BulkEndpoints,
    device_endpoint: &mut LoopbackEndpoint,
    cancel: &AtomicBool,
    status: &Arc<Mutex<LinkStatus>>,
) -> Result<(), String> {
    let mut decoder = FrameDecoder::new();
    let mut read_buffer = vec![0_u8; BULK_TRANSFER_BYTES];
    let mut outgoing = OutgoingMux::default();

    while !cancel.load(Ordering::Acquire) {
        if let Some(packet) = outgoing.next(device_endpoint)? {
            let written = write_chunked(packet.payload(), |chunk| {
                handle.write_bulk(endpoints.output, chunk, BULK_WRITE_TIMEOUT)
            })?;
            let mut link_status = lock_link_status(status);
            link_status.bytes_written = link_status
                .bytes_written
                .saturating_add(u64::try_from(written).unwrap_or(u64::MAX));
            link_status.frames_written = link_status.frames_written.saturating_add(1);
        }

        match handle.read_bulk(endpoints.input, &mut read_buffer, BULK_READ_TIMEOUT) {
            Ok(0) | Err(rusb::Error::Timeout) => {}
            Ok(count) => {
                let frames = decoder
                    .push(&read_buffer[..count])
                    .map_err(|error| format!("Android USB LDFL stream is invalid: {error}"))?;
                let frame_count = frames.len();
                for frame in frames {
                    let packet = if frame.header().kind() == MessageType::VideoFrame {
                        Packet::media(frame.encode())
                    } else {
                        Packet::control(frame.encode())
                    };
                    device_endpoint.try_send(packet).map_err(|error| {
                        format!("Android USB inbound queue rejected a frame: {error}")
                    })?;
                }
                let mut link_status = lock_link_status(status);
                link_status.bytes_read = link_status
                    .bytes_read
                    .saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
                link_status.frames_read = link_status
                    .frames_read
                    .saturating_add(u64::try_from(frame_count).unwrap_or(u64::MAX));
            }
            Err(error) => return Err(format!("Android USB bulk read failed: {error}")),
        }
    }
    Ok(())
}

#[derive(Default)]
struct OutgoingMux {
    control: Option<Packet>,
    media: Option<Packet>,
}

impl OutgoingMux {
    fn next(&mut self, endpoint: &mut LoopbackEndpoint) -> Result<Option<Packet>, String> {
        if self.control.is_none() {
            self.control = endpoint
                .try_receive(Channel::Control)
                .map_err(|error| format!("Android USB outgoing control queue failed: {error}"))?;
        }
        if self.media.is_none() {
            self.media = endpoint
                .try_receive(Channel::Media)
                .map_err(|error| format!("Android USB outgoing media queue failed: {error}"))?;
        }

        match (&self.control, &self.media) {
            (Some(control), Some(media)) => {
                let control_sequence = outgoing_sequence(control)?;
                let media_sequence = outgoing_sequence(media)?;
                if control_sequence == media_sequence {
                    return Err(format!(
                        "Android USB outgoing queues contain duplicate LDFL sequence {control_sequence}"
                    ));
                }
                if control_sequence < media_sequence {
                    Ok(self.control.take())
                } else {
                    Ok(self.media.take())
                }
            }
            (Some(_), None) => Ok(self.control.take()),
            (None, Some(_)) => Ok(self.media.take()),
            (None, None) => Ok(None),
        }
    }
}

fn outgoing_sequence(packet: &Packet) -> Result<u64, String> {
    let DecodeOutcome::Complete { frame, consumed } = WireFrame::decode_prefix(packet.payload())
        .map_err(|error| format!("Android USB outgoing LDFL frame is invalid: {error}"))?
    else {
        return Err("Android USB outgoing queue contains a partial LDFL frame".to_owned());
    };
    if consumed != packet.len() {
        return Err("Android USB outgoing LDFL frame has trailing bytes".to_owned());
    }
    let expected_channel = if frame.header().kind() == MessageType::VideoFrame {
        Channel::Media
    } else {
        Channel::Control
    };
    if packet.channel() != expected_channel {
        return Err(format!(
            "Android USB outgoing {:?} is queued on the wrong {:?} channel",
            frame.header().kind(),
            packet.channel()
        ));
    }
    Ok(frame.header().sequence())
}

fn write_chunked<E: std::fmt::Display>(
    payload: &[u8],
    mut write: impl FnMut(&[u8]) -> Result<usize, E>,
) -> Result<usize, String> {
    let mut offset = 0_usize;
    while offset < payload.len() {
        let end = offset
            .saturating_add(BULK_TRANSFER_BYTES)
            .min(payload.len());
        let chunk = &payload[offset..end];
        let written =
            write(chunk).map_err(|error| format!("Android USB bulk write failed: {error}"))?;
        if written == 0 {
            return Err("Android USB bulk write made no progress".to_owned());
        }
        if written > chunk.len() {
            return Err(format!(
                "Android USB bulk write reported {written} bytes for a {}-byte chunk",
                chunk.len()
            ));
        }
        offset += written;
    }
    Ok(offset)
}

fn lock_link_status(status: &Arc<Mutex<LinkStatus>>) -> MutexGuard<'_, LinkStatus> {
    status
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
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
    use ladoflow_protocol::{Frame as WireFrame, FrameFlags, MessageType};
    use ladoflow_transport::{Packet, PacketTransport, loopback_pair};

    use super::{
        BULK_TRANSFER_BYTES, FRAME_HEADER_LEN, MAX_CONTROL_PAYLOAD, MAX_MEDIA_PAYLOAD, OutgoingMux,
        USB_QUEUE_CONFIG, is_android_probe_candidate, write_chunked,
    };

    #[test]
    fn background_probe_candidates_exclude_device_classes_with_other_roles() {
        assert!(is_android_probe_candidate(0x18d1, 0x4ee7, 0x00));
        assert!(is_android_probe_candidate(0x04e8, 0x6860, 0xef));
        assert!(!is_android_probe_candidate(0x18d1, 0x2d00, 0x00));
        assert!(!is_android_probe_candidate(0x1234, 0x5678, 0x03));
        assert!(!is_android_probe_candidate(0, 0, 0));
    }

    #[test]
    fn chunked_writer_caps_transfers_and_retries_short_writes() {
        let payload = (0..(BULK_TRANSFER_BYTES * 2 + 31))
            .map(|value| u8::try_from(value % 251).expect("value is bounded"))
            .collect::<Vec<_>>();
        let mut transferred = Vec::new();
        let mut largest_request = 0_usize;

        let written = write_chunked(&payload, |chunk| -> Result<usize, &'static str> {
            largest_request = largest_request.max(chunk.len());
            let accepted = chunk.len().min(7_919);
            transferred.extend_from_slice(&chunk[..accepted]);
            Ok(accepted)
        })
        .expect("all short writes are retried");

        assert_eq!(written, payload.len());
        assert_eq!(transferred, payload);
        assert!(largest_request <= BULK_TRANSFER_BYTES);
        assert!(write_chunked(b"blocked", |_chunk| Ok::<usize, &str>(0)).is_err());
    }

    #[test]
    fn usb_queues_accept_the_protocols_largest_encoded_frames() {
        assert_eq!(
            USB_QUEUE_CONFIG.control_limits().max_packet_bytes(),
            FRAME_HEADER_LEN + MAX_CONTROL_PAYLOAD
        );
        assert_eq!(
            USB_QUEUE_CONFIG.media_limits().max_packet_bytes(),
            FRAME_HEADER_LEN + MAX_MEDIA_PAYLOAD
        );
    }

    #[test]
    fn outgoing_frames_follow_global_sequence_across_channels() {
        let (mut host, mut device) = loopback_pair(USB_QUEUE_CONFIG);
        let media = WireFrame::new(MessageType::VideoFrame, FrameFlags::NONE, 3, b"video")
            .expect("valid media frame")
            .encode();
        let control = WireFrame::new(MessageType::Ping, FrameFlags::NONE, 4, b"control")
            .expect("valid control frame")
            .encode();
        host.try_send(Packet::media(media))
            .expect("media queue accepts frame");
        host.try_send(Packet::control(control))
            .expect("control queue accepts frame");

        let mut mux = OutgoingMux::default();
        let first = mux
            .next(&mut device)
            .expect("queue remains connected")
            .expect("media frame is available");
        let second = mux
            .next(&mut device)
            .expect("queue remains connected")
            .expect("control frame is available");
        assert_eq!(super::outgoing_sequence(&first), Ok(3));
        assert_eq!(super::outgoing_sequence(&second), Ok(4));
        assert!(
            mux.next(&mut device)
                .expect("queue remains connected")
                .is_none()
        );
    }

    #[test]
    fn outgoing_mux_rejects_wrong_channels_and_duplicate_sequences() {
        let (mut host, mut device) = loopback_pair(USB_QUEUE_CONFIG);
        let control = WireFrame::new(MessageType::Ping, FrameFlags::NONE, 9, b"control")
            .expect("valid control frame")
            .encode();
        let media = WireFrame::new(MessageType::VideoFrame, FrameFlags::NONE, 9, b"media")
            .expect("valid media frame")
            .encode();
        host.try_send(Packet::control(control))
            .expect("control queue accepts frame");
        host.try_send(Packet::media(media))
            .expect("media queue accepts frame");
        assert!(
            OutgoingMux::default()
                .next(&mut device)
                .expect_err("duplicate sequence is rejected")
                .contains("duplicate")
        );

        let wrong = WireFrame::new(MessageType::Ping, FrameFlags::NONE, 10, b"wrong")
            .expect("valid control frame")
            .encode();
        assert!(super::outgoing_sequence(&Packet::media(wrong)).is_err());
    }
}
