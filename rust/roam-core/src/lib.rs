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

mod replay_buffer;

mod stable_conduit;
pub use stable_conduit::*;

mod memory_link;
pub use memory_link::*;

mod session;
pub use session::*;

/// Return a process-global cached `&'static RpcPlan` for type `T`.
pub fn rpc_plan<T: facet::Facet<'static>>() -> &'static roam_types::RpcPlan {
    roam_types::RpcPlan::for_type::<T>()
}

#[cfg(test)]
mod tests;
