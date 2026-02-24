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
pub use bare_conduit::*;

mod channel;
pub use channel::*;

mod replay_buffer;

mod stable_conduit;
pub use stable_conduit::*;

mod memory_link;
pub use memory_link::*;

mod session;
pub use session::*;

/// Build a `&'static RpcPlan` for type `T`, using `Tx<()>` / `Rx<()>` as the
/// channel sentinels, via the process-global RpcPlan cache.
///
/// Kept as a convenience helper for call sites that need a plan on demand.
pub fn rpc_plan<T: facet::Facet<'static>>() -> &'static roam_types::RpcPlan {
    roam_types::RpcPlan::for_type_cached::<T, crate::Tx<()>, crate::Rx<()>>()
}

#[cfg(test)]
mod tests;
