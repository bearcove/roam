//! Shared-memory transport binding for roam.
//!
//! This crate implements the SHM hub transport as specified in
//! `docs/content/shm-spec/_index.md`. It enables high-performance IPC
//! between a host and multiple guests via shared memory.
//!
//! # Architecture
//!
//! The SHM transport uses a hub topology with one host and multiple guests:
//!
//! ```text
//!          ┌─────────┐
//!          │  Host   │
//!          └────┬────┘
//!               │
//!     ┌─────────┼─────────┐
//!     │         │         │
//! ┌───┴───┐ ┌───┴───┐ ┌───┴───┐
//! │Guest 1│ │Guest 2│ │Guest 3│
//! └───────┘ └───────┘ └───────┘
//! ```
//!
//! Guests communicate only with the host, not with each other. Each guest
//! has its own rings and slot pool within the shared segment.
//!
//! # Usage
//!
//! ## Host side
//!
//! ```ignore
//! use roam_shm::{ShmHost, SegmentConfig};
//!
//! // Create a new SHM segment
//! let config = SegmentConfig::default();
//! let host = ShmHost::create("/dev/shm/myapp", config)?;
//!
//! // Poll for guest messages
//! while let Some((peer_id, frame)) = host.poll() {
//!     // Handle message from guest
//! }
//! ```
//!
//! ## Guest side
//!
//! ```ignore
//! use roam_shm::ShmGuest;
//!
//! // Attach to existing segment
//! let guest = ShmGuest::attach("/dev/shm/myapp")?;
//!
//! // Send message to host
//! guest.send(frame)?;
//!
//! // Receive message from host
//! if let Some(frame) = guest.recv() {
//!     // Handle message
//! }
//! ```
//!

// shm[impl shm.scope]
// shm[impl shm.architecture]

#[macro_use]
mod macros;

mod channel;
mod layout;
mod msg;
mod peer;
mod var_slot_pool;

mod auditable;
mod bootstrap;
mod cleanup;
mod driver;
mod guest;
mod host;
mod spawn;
mod transport;

// Re-export key types
pub use layout::{
    HEADER_SIZE, MAGIC, SegmentConfig, SegmentHeader, SegmentLayout, SizeClass, VERSION,
};
pub use msg::{ShmMsg, msg_type};
pub use peer::{PeerEntry, PeerId, PeerState};
pub use var_slot_pool::{SizeClassHeader, VarFreeError, VarSlotHandle, VarSlotPool};

// Re-export FileCleanup from shm-primitives
pub use shm_primitives::FileCleanup;

pub use guest::{AttachError, SendError as GuestSendError, ShmGuest};
pub use host::{GrowError, PollResult, SendError as HostSendError, ShmHost};

pub use transport::{
    ConvertError, ShmGuestTransport, ShmHostGuestTransport, message_to_shm_msg, shm_msg_to_message,
};

pub use spawn::{
    AddPeerOptions, DeathCallback, SpawnArgs, SpawnArgsError, SpawnTicket, die_with_parent,
};

pub use bootstrap::{
    BootstrapError, BootstrapTicket, SessionId, SessionIdError, SessionPaths, unix,
};
pub use driver::{
    IncomingConnection, IncomingConnectionResponse, IncomingConnections, MultiPeerHostDriver,
    MultiPeerHostDriverBuilder, MultiPeerHostDriverHandle, ShmConnectionError, ShmDriver,
    ShmNegotiated, establish_guest, establish_multi_peer_host,
};

/// Handshake is implicit via segment header.
///
/// shm[impl shm.handshake]
/// shm[impl shm.handshake.no-negotiation]
///
/// SHM does not use Hello messages. The segment header fields serve as the
/// host's unilateral configuration. Guests accept these values by attaching.
pub(crate) const fn _handshake_is_implicit() {}

/// Payload encoding is postcard.
///
/// shm[impl shm.payload.encoding]
pub(crate) const fn _payload_encoding_is_postcard() {}
