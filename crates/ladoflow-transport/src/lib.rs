//! Bounded packet channels and transport abstractions for `LadoFlow`.
//!
//! Control and media traffic use independent queues. Control packets are
//! reliable while a connection is alive: a full queue returns the packet to
//! the caller for retry and never evicts an earlier packet. Media packets can
//! opt into latest-value behavior with a [`SupersessionKey`].
//!
//! [`loopback_pair`] provides a deterministic, in-memory duplex implementation
//! for session and protocol tests. Platform USB and network adapters can
//! implement [`PacketTransport`] using the same packet and error types.

#![forbid(unsafe_code)]

mod aoa;
mod ldfl_stream;
mod loopback;
mod packet;
mod tcp;
mod transport;

pub use aoa::{
    AOA_ACCESSORY_ADB_PRODUCT_ID, AOA_ACCESSORY_AUDIO_ADB_PRODUCT_ID,
    AOA_ACCESSORY_AUDIO_PRODUCT_ID, AOA_ACCESSORY_PRODUCT_ID, AOA_CONTROL_READ_TYPE,
    AOA_CONTROL_TIMEOUT, AOA_CONTROL_WRITE_TYPE, AOA_GET_PROTOCOL, AOA_GOOGLE_VENDOR_ID,
    AOA_MAX_IDENTIFICATION_BYTES, AOA_SEND_IDENTIFICATION, AOA_START_ACCESSORY, AccessoryControlIo,
    AccessoryIdentity, AccessoryIdentityError, AoaNegotiationError, AoaProtocolVersion,
    is_aoa_app_accessory, negotiate_accessory_mode,
};
pub use ldfl_stream::{LdflPacketDecoder, LdflPacketMux, LdflStreamError, ldfl_packet_sequence};
pub use loopback::{DisconnectReport, LoopbackConfig, LoopbackEndpoint, loopback_pair};
pub use packet::{Channel, Packet, SupersessionKey};
pub use tcp::{TcpPacketTransport, TcpTransportStatus};
pub use transport::{
    ConnectionState, PacketTransport, QueueDepth, QueueLimits, QueueLimitsError, ReceiveError,
    SendError, SendReport,
};
