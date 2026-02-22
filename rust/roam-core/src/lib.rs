//! Core implementations for the roam connectivity layer.
//!
//! This crate provides concrete implementations of the traits defined in
//! [`roam_types::connectivity`]:
//!
//! - [`ReliableLink`]: wraps a raw `Link<Packet<T>, C>`, strips packet
//!   framing, exposes `Link<T, C>` with transparent reconnect + replay.
//! - [`ReliableAcceptor`]: server-side router that accepts raw connections,
//!   reads the reliability handshake, routes reconnects to existing sessions.

mod reliable_link;
pub use reliable_link::{ReliableLink, ReliableLinkError};

mod reliable_acceptor;
pub use reliable_acceptor::{ChannelLinkSource, IngestError, NewSession, ReliableAcceptor};

mod memory_link;
pub use memory_link::{MemoryLink, MemoryLinkRx, MemoryLinkTx, memory_link_pair};

#[cfg(test)]
mod tests;
