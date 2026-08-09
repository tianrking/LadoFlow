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

mod loopback;
mod packet;
mod transport;

pub use loopback::{DisconnectReport, LoopbackConfig, LoopbackEndpoint, loopback_pair};
pub use packet::{Channel, Packet, SupersessionKey};
pub use transport::{
    ConnectionState, PacketTransport, QueueDepth, QueueLimits, QueueLimitsError, ReceiveError,
    SendError, SendReport,
};
