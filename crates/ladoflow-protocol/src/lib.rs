//! Transport-independent wire protocol primitives for `LadoFlow`.
//!
//! The first functional milestone will add bounded frame parsing and
//! capability negotiation. This crate currently establishes the version
//! authority and workspace boundary only.

/// First protocol generation used during pre-alpha development.
pub const PROTOCOL_VERSION: u16 = 1;
