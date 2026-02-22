//! Core implementations for the roam connectivity layer.
//!
//! This crate provides concrete implementations of the traits defined in
//! [`roam_types`]:
//!
//! - [`BareConduit`]: wraps a raw `Link` with postcard serialization.
//!   No reconnect, no reliability. For localhost, SHM, testing.
//! - `StableConduit` (TODO): wraps a Link + seq/ack/replay with
//!   bytes-based replay buffer. Handles reconnect transparently.

mod bare_conduit;
pub use bare_conduit::{BareConduit, BareConduitError};

mod memory_link;
pub use memory_link::{MemoryLink, MemoryLinkRx, MemoryLinkTx, memory_link_pair};

#[cfg(test)]
mod tests;
