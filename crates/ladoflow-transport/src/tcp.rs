use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    net::{Shutdown, SocketAddr, TcpStream},
    sync::{
        Arc, Mutex, MutexGuard,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use ladoflow_protocol::{FRAME_HEADER_LEN, MAX_CONTROL_PAYLOAD, MAX_MEDIA_PAYLOAD};

use crate::{
    Channel, ConnectionState, LdflPacketDecoder, LdflPacketMux, LoopbackConfig, LoopbackEndpoint,
    Packet, PacketTransport, QueueLimits, ReceiveError, SendError, SendReport, loopback_pair,
};

const TCP_IO_CHUNK_BYTES: usize = 64 * 1_024;
const TCP_IDLE_POLL_INTERVAL: Duration = Duration::from_millis(1);
const TCP_QUEUE_CONFIG: LoopbackConfig = LoopbackConfig::new(
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
        Err(_) => panic!("built-in TCP queue limits must be valid"),
    }
}

/// Observable state for one local TCP byte-stream worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TcpTransportStatus {
    state: ConnectionState,
    local_addr: SocketAddr,
    peer_addr: SocketAddr,
    bytes_read: u64,
    bytes_written: u64,
    frames_read: u64,
    frames_written: u64,
    last_error: Option<String>,
}

impl TcpTransportStatus {
    /// Whether packet operations are currently available.
    #[must_use]
    pub const fn state(&self) -> ConnectionState {
        self.state
    }

    /// Local socket address selected by the operating system.
    #[must_use]
    pub const fn local_addr(&self) -> SocketAddr {
        self.local_addr
    }

    /// Connected peer socket address.
    #[must_use]
    pub const fn peer_addr(&self) -> SocketAddr {
        self.peer_addr
    }

    /// Raw bytes received from the peer.
    #[must_use]
    pub const fn bytes_read(&self) -> u64 {
        self.bytes_read
    }

    /// Raw bytes written to the peer.
    #[must_use]
    pub const fn bytes_written(&self) -> u64 {
        self.bytes_written
    }

    /// Complete LDFL frames decoded from the peer.
    #[must_use]
    pub const fn frames_read(&self) -> u64 {
        self.frames_read
    }

    /// Complete LDFL frames fully written to the peer.
    #[must_use]
    pub const fn frames_written(&self) -> u64 {
        self.frames_written
    }

    /// Fatal socket, framing, or queue error, when the peer did not close cleanly.
    #[must_use]
    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
}

/// A bounded, nonblocking packet endpoint backed by one connected TCP stream.
///
/// The caller owns discovery, user consent, authentication, and any pairing
/// preface. Pass the stream here only after those steps finish. The worker then
/// carries raw, unchanged LDFL frames, with TCP providing reliable byte order.
/// Control and media remain independent bounded queues inside the process.
#[derive(Debug)]
pub struct TcpPacketTransport {
    endpoint: LoopbackEndpoint,
    cancel: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
    status: Arc<Mutex<TcpTransportStatus>>,
}

impl TcpPacketTransport {
    /// Start a worker on an already connected and authenticated TCP stream.
    ///
    /// # Errors
    ///
    /// Returns an error when socket metadata/options cannot be configured or
    /// the background worker thread cannot be created.
    pub fn from_authenticated_stream(stream: TcpStream) -> Result<Self, String> {
        let local_addr = stream
            .local_addr()
            .map_err(|error| format!("failed to read local TCP address: {error}"))?;
        let peer_addr = stream
            .peer_addr()
            .map_err(|error| format!("failed to read peer TCP address: {error}"))?;
        stream
            .set_nodelay(true)
            .map_err(|error| format!("failed to enable TCP_NODELAY: {error}"))?;
        stream
            .set_nonblocking(true)
            .map_err(|error| format!("failed to make the LDFL TCP stream nonblocking: {error}"))?;

        let (endpoint, network_endpoint) = loopback_pair(TCP_QUEUE_CONFIG);
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let status = Arc::new(Mutex::new(TcpTransportStatus {
            state: ConnectionState::Connected,
            local_addr,
            peer_addr,
            bytes_read: 0,
            bytes_written: 0,
            frames_read: 0,
            frames_written: 0,
            last_error: None,
        }));
        let worker_status = Arc::clone(&status);
        let worker = thread::Builder::new()
            .name("ladoflow-ldfl-tcp".to_owned())
            .spawn(move || {
                run_tcp_stream(stream, network_endpoint, &worker_cancel, &worker_status);
            })
            .map_err(|error| format!("failed to start the LDFL TCP worker: {error}"))?;

        Ok(Self {
            endpoint,
            cancel,
            worker: Some(worker),
            status,
        })
    }

    /// Return a coherent copy of socket counters and terminal error state.
    #[must_use]
    pub fn status(&self) -> TcpTransportStatus {
        lock_status(&self.status).clone()
    }

    /// Stop the worker and close the socket without waiting on network I/O.
    ///
    /// # Errors
    ///
    /// Returns an error only if the worker thread panicked.
    pub fn shutdown(&mut self) -> Result<TcpTransportStatus, String> {
        self.cancel.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| "LDFL TCP worker panicked while stopping".to_owned())?;
        }
        Ok(self.status())
    }
}

impl PacketTransport for TcpPacketTransport {
    fn connection_state(&self) -> ConnectionState {
        self.endpoint.connection_state()
    }

    fn try_send(&mut self, packet: Packet) -> Result<SendReport, SendError> {
        self.endpoint.try_send(packet)
    }

    fn try_receive(&mut self, channel: Channel) -> Result<Option<Packet>, ReceiveError> {
        self.endpoint.try_receive(channel)
    }
}

impl Drop for TcpPacketTransport {
    fn drop(&mut self) {
        let _result = self.shutdown();
    }
}

struct PendingWrite {
    packet: Packet,
    offset: usize,
}

fn run_tcp_stream(
    mut stream: TcpStream,
    mut endpoint: LoopbackEndpoint,
    cancel: &AtomicBool,
    status: &Arc<Mutex<TcpTransportStatus>>,
) {
    let result = pump_tcp_stream(&mut stream, &mut endpoint, cancel, status);
    let _shutdown = stream.shutdown(Shutdown::Both);
    let _discarded = endpoint.disconnect();

    let mut current = lock_status(status);
    current.state = ConnectionState::Disconnected;
    if !cancel.load(Ordering::Acquire) {
        current.last_error = Some(match result {
            Ok(()) => "LDFL TCP peer closed the stream".to_owned(),
            Err(error) => error,
        });
    }
}

fn pump_tcp_stream(
    stream: &mut TcpStream,
    endpoint: &mut LoopbackEndpoint,
    cancel: &AtomicBool,
    status: &Arc<Mutex<TcpTransportStatus>>,
) -> Result<(), String> {
    let mut decoder = LdflPacketDecoder::new();
    let mut mux = LdflPacketMux::default();
    let mut outgoing = None::<PendingWrite>;
    let mut inbound = VecDeque::<Packet>::new();
    let mut read_buffer = vec![0_u8; TCP_IO_CHUNK_BYTES];

    while !cancel.load(Ordering::Acquire) {
        let mut made_progress = drain_inbound(endpoint, &mut inbound)?;

        if outgoing.is_none()
            && let Some(packet) = mux
                .next(endpoint)
                .map_err(|error| format!("outbound LDFL TCP queue is invalid: {error}"))?
        {
            outgoing = Some(PendingWrite { packet, offset: 0 });
        }
        if let Some(pending) = outgoing.as_mut() {
            made_progress |= write_pending(stream, pending, status)?;
            if pending.offset == pending.packet.len() {
                let mut current = lock_status(status);
                current.frames_written = current.frames_written.saturating_add(1);
                outgoing = None;
            }
        }

        if inbound.is_empty() {
            match stream.read(&mut read_buffer) {
                Ok(0) => return Ok(()),
                Ok(count) => {
                    let packets = decoder
                        .push(&read_buffer[..count])
                        .map_err(|error| format!("inbound LDFL TCP stream is invalid: {error}"))?;
                    let mut current = lock_status(status);
                    current.bytes_read = current
                        .bytes_read
                        .saturating_add(u64::try_from(count).unwrap_or(u64::MAX));
                    current.frames_read = current
                        .frames_read
                        .saturating_add(u64::try_from(packets.len()).unwrap_or(u64::MAX));
                    drop(current);
                    inbound.extend(packets);
                    made_progress = true;
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {}
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) => return Err(format!("LDFL TCP read failed: {error}")),
            }
        }

        if !made_progress {
            thread::sleep(TCP_IDLE_POLL_INTERVAL);
        }
    }
    Ok(())
}

fn drain_inbound(
    endpoint: &mut LoopbackEndpoint,
    inbound: &mut VecDeque<Packet>,
) -> Result<bool, String> {
    let mut made_progress = false;
    while let Some(packet) = inbound.pop_front() {
        match endpoint.try_send(packet) {
            Ok(_report) => made_progress = true,
            Err(SendError::Full { packet, .. }) => {
                inbound.push_front(packet);
                break;
            }
            Err(error) => return Err(format!("inbound LDFL TCP queue rejected a frame: {error}")),
        }
    }
    Ok(made_progress)
}

fn write_pending(
    stream: &mut TcpStream,
    pending: &mut PendingWrite,
    status: &Arc<Mutex<TcpTransportStatus>>,
) -> Result<bool, String> {
    let end = pending
        .offset
        .saturating_add(TCP_IO_CHUNK_BYTES)
        .min(pending.packet.len());
    match stream.write(&pending.packet.payload()[pending.offset..end]) {
        Ok(0) => Err("LDFL TCP write made no progress".to_owned()),
        Ok(written) => {
            pending.offset = pending.offset.saturating_add(written);
            let mut current = lock_status(status);
            current.bytes_written = current
                .bytes_written
                .saturating_add(u64::try_from(written).unwrap_or(u64::MAX));
            Ok(true)
        }
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(false),
        Err(error) if error.kind() == io::ErrorKind::Interrupted => Ok(false),
        Err(error) => Err(format!("LDFL TCP write failed: {error}")),
    }
}

fn lock_status(status: &Arc<Mutex<TcpTransportStatus>>) -> MutexGuard<'_, TcpTransportStatus> {
    status
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use std::{
        io::Write as _,
        net::{TcpListener, TcpStream},
        thread,
        time::{Duration, Instant},
    };

    use ladoflow_protocol::{FRAME_HEADER_LEN, Frame as WireFrame, FrameFlags, MessageType};

    use crate::{
        Channel, ConnectionState, Packet, PacketTransport, ReceiveError, TcpPacketTransport,
    };

    fn frame(kind: MessageType, sequence: u64, payload: &[u8]) -> Vec<u8> {
        WireFrame::new(kind, FrameFlags::NONE, sequence, payload)
            .expect("valid frame")
            .encode()
    }

    fn transport_pair() -> (TcpPacketTransport, TcpPacketTransport) {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback listener");
        let address = listener.local_addr().expect("listener address");
        let connector = thread::spawn(move || TcpStream::connect(address).expect("connect peer"));
        let (accepted, _peer) = listener.accept().expect("accept peer");
        let connected = connector.join().expect("connector thread");
        (
            TcpPacketTransport::from_authenticated_stream(connected).expect("client transport"),
            TcpPacketTransport::from_authenticated_stream(accepted).expect("server transport"),
        )
    }

    fn receive(transport: &mut TcpPacketTransport, channel: Channel, timeout: Duration) -> Packet {
        let deadline = Instant::now() + timeout;
        loop {
            match transport.try_receive(channel) {
                Ok(Some(packet)) => return packet,
                Ok(None) => {}
                Err(ReceiveError::Disconnected) => panic!("transport disconnected before packet"),
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for {channel:?}"
            );
            thread::sleep(Duration::from_millis(1));
        }
    }

    #[test]
    fn tcp_transport_round_trips_control_and_media_ldfl_frames() {
        let (mut host, mut display) = transport_pair();
        let control = frame(MessageType::Ping, 3, b"ping");
        let media = frame(MessageType::VideoFrame, 4, b"video");
        host.try_send(Packet::control(control.clone()))
            .expect("control queued");
        host.try_send(Packet::media(media.clone()))
            .expect("media queued");

        assert_eq!(
            receive(&mut display, Channel::Control, Duration::from_secs(2)).payload(),
            control
        );
        assert_eq!(
            receive(&mut display, Channel::Media, Duration::from_secs(2)).payload(),
            media
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while host.status().frames_written() < 2 || display.status().frames_read() < 2 {
            assert!(Instant::now() < deadline, "TCP counters did not converge");
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(
            host.status().bytes_written(),
            control.len() as u64 + media.len() as u64
        );
        assert_eq!(
            display.status().bytes_read(),
            control.len() as u64 + media.len() as u64
        );
        host.shutdown().expect("client shutdown");
        display.shutdown().expect("server shutdown");
    }

    #[test]
    fn malformed_tcp_stream_disconnects_without_exposing_a_packet() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind loopback listener");
        let address = listener.local_addr().expect("listener address");
        let sender = thread::spawn(move || {
            let mut stream = TcpStream::connect(address).expect("connect raw peer");
            stream
                .write_all(&[0_u8; FRAME_HEADER_LEN])
                .expect("send malformed header");
        });
        let (stream, _peer) = listener.accept().expect("accept raw peer");
        let mut transport =
            TcpPacketTransport::from_authenticated_stream(stream).expect("start transport");
        sender.join().expect("raw sender");

        let deadline = Instant::now() + Duration::from_secs(2);
        while transport.connection_state() == ConnectionState::Connected {
            assert!(
                Instant::now() < deadline,
                "malformed stream stayed connected"
            );
            thread::sleep(Duration::from_millis(1));
        }
        assert!(
            transport
                .status()
                .last_error()
                .expect("terminal error")
                .contains("invalid")
        );
        assert!(matches!(
            transport.try_receive(Channel::Control),
            Err(ReceiveError::Disconnected)
        ));
        transport.shutdown().expect("worker joins");
    }
}
