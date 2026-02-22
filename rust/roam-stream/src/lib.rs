#![deny(unsafe_code)]

//! Stream transport layer for roam RPC.
//!
//! This crate provides length-prefixed framing and byte-stream specific machinery for running
//! roam services over TCP, Unix sockets, or any async byte stream.
//!
//! For message-based transports (like WebSocket) that already provide framing,
//! use `roam_core` directly - it has the Driver and accept_framed/connect_framed.

mod driver;
mod framing;

// Byte-stream specific API (stays here)
pub use driver::{Client, Connector, accept, connect, connect_with_policy};

// length-prefixed framing
pub use framing::LengthPrefixedFramed;
