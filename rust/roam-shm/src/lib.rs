//! Shared-memory transport for roam.
//!
//! Implements [`Link<T, C>`](roam_types::Link) over lock-free ring buffers
//! in shared memory. Zero-copy: the receiver's [`SelfRef`](roam_types::SelfRef)
//! borrows directly from the shared region.

/// A [`Link`](roam_types::Link) over shared memory ring buffers.
pub struct ShmLink {
    // Will hold: BipBuffer pair, signaling fds, etc.
}

/// Sending half of a [`ShmLink`].
pub struct ShmLinkTx {
    // Will hold: write-side BipBuffer handle
}

/// Receiving half of a [`ShmLink`].
pub struct ShmLinkRx {
    // Will hold: read-side BipBuffer handle
}
