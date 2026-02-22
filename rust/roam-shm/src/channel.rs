//! Channel metadata table and flow control.
//!
//! Each guest-host pair has a channel table tracking active channels and their
//! flow control credits.

use core::mem::size_of;
use core::sync::atomic::{AtomicU32, Ordering};

/// Channel states.
///
/// shm[impl shm.flow.channel-table]
#[repr(u32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChannelState {
    /// Channel ID available for allocation
    Free = 0,
    /// Channel is active
    Active = 1,
    /// Channel has been closed
    Closed = 2,
}

impl ChannelState {
    /// Convert from u32, returning None for invalid values.
    #[inline]
    pub fn from_u32(v: u32) -> Option<Self> {
        match v {
            0 => Some(ChannelState::Free),
            1 => Some(ChannelState::Active),
            2 => Some(ChannelState::Closed),
            _ => None,
        }
    }
}

/// Channel table entry (16 bytes).
///
/// shm[impl shm.flow.channel-table]
#[repr(C)]
pub struct ChannelEntry {
    /// Channel state (Free, Active, Closed)
    pub state: AtomicU32,
    /// Cumulative bytes authorized by receiver
    ///
    /// shm[impl shm.flow.granted-total]
    pub granted_total: AtomicU32,
    /// Reserved (zero)
    pub _reserved: [u8; 8],
}

const _: () = assert!(size_of::<ChannelEntry>() == 16);

impl ChannelEntry {
    /// Initialize a channel entry to Free state.
    pub fn init(&mut self) {
        self.state = AtomicU32::new(ChannelState::Free as u32);
        self.granted_total = AtomicU32::new(0);
        self._reserved = [0; 8];
    }

    /// Get the current channel state.
    #[inline]
    pub fn state(&self) -> ChannelState {
        ChannelState::from_u32(self.state.load(Ordering::Acquire)).unwrap_or(ChannelState::Free)
    }

    /// Activate this channel with initial credit.
    ///
    /// shm[impl shm.flow.channel-activate]
    ///
    /// Returns Ok(()) if the channel was Free, Err(actual_state) otherwise.
    pub fn activate(&self, initial_credit: u32) -> Result<(), ChannelState> {
        // First set the granted_total
        self.granted_total.store(initial_credit, Ordering::Release);

        // Then transition state to Active
        match self.state.compare_exchange(
            ChannelState::Free as u32,
            ChannelState::Active as u32,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => Ok(()),
            Err(actual) => Err(ChannelState::from_u32(actual).unwrap_or(ChannelState::Free)),
        }
    }

    /// Mark this channel as closed.
    ///
    /// shm[impl shm.flow.channel-id-reuse]
    #[inline]
    pub fn close(&self) {
        self.state
            .store(ChannelState::Closed as u32, Ordering::Release);
    }

    /// Reset this channel to Free state for reuse.
    ///
    /// shm[impl shm.flow.channel-id-reuse]
    #[inline]
    pub fn reset_to_free(&self) {
        self.granted_total.store(0, Ordering::Release);
        self.state
            .store(ChannelState::Free as u32, Ordering::Release);
    }

    /// Get the granted_total counter (receiver side).
    ///
    /// shm[impl shm.flow.ordering.sender]
    #[inline]
    pub fn granted_total(&self) -> u32 {
        self.granted_total.load(Ordering::Acquire)
    }

    /// Grant additional credit (receiver side).
    ///
    /// shm[impl shm.flow.ordering.receiver]
    ///
    /// Adds `bytes` to granted_total and returns the new value.
    #[inline]
    pub fn grant_credit(&self, bytes: u32) -> u32 {
        self.granted_total.fetch_add(bytes, Ordering::Release) + bytes
    }

    /// Set granted_total directly (for initialization).
    #[inline]
    pub fn set_granted_total(&self, value: u32) {
        self.granted_total.store(value, Ordering::Release);
    }
}

/// Flow control state for a channel (sender side, kept locally).
///
/// The sender tracks `sent_total` locally and compares against `granted_total`
/// in shared memory to determine remaining credit.
#[derive(Debug, Clone, Copy)]
pub struct FlowControl {
    /// Cumulative bytes sent
    pub sent_total: u32,
}

impl FlowControl {
    /// Create a new flow control state.
    #[inline]
    pub fn new() -> Self {
        Self { sent_total: 0 }
    }

    /// Calculate remaining credit.
    ///
    /// shm[impl shm.flow.remaining-credit]
    /// shm[impl shm.flow.wrap-rule]
    ///
    /// Returns the remaining credit as i32. Negative values indicate corruption.
    #[inline]
    pub fn remaining_credit(&self, granted_total: u32) -> i32 {
        granted_total.wrapping_sub(self.sent_total) as i32
    }

    /// Check if we can send `bytes` worth of data.
    ///
    /// shm[impl shm.flow.zero-credit]
    #[inline]
    pub fn can_send(&self, granted_total: u32, bytes: u32) -> bool {
        let remaining = self.remaining_credit(granted_total);
        remaining >= 0 && (remaining as u32) >= bytes
    }

    /// Record that we sent `bytes` worth of data.
    ///
    /// shm[impl shm.bytes.what-counts]
    #[inline]
    pub fn record_sent(&mut self, bytes: u32) {
        self.sent_total = self.sent_total.wrapping_add(bytes);
    }
}

impl Default for FlowControl {
    fn default() -> Self {
        Self::new()
    }
}

use roam_types::{ChannelId, RequestId};

/// Channel ID allocator (SHM-specific, bounded).
///
/// shm[impl shm.id.channel-parity]
#[derive(Debug)]
pub struct ChannelIdAllocator {
    /// Next channel ID to allocate
    next: u32,
    /// Maximum channel ID (exclusive)
    max: u32,
}

impl ChannelIdAllocator {
    /// Create a new allocator for host-side channels (even IDs).
    pub fn for_host(max_channels: u32) -> Self {
        Self {
            next: 2, // First even ID
            max: max_channels,
        }
    }

    /// Create a new allocator for guest-side channels (odd IDs).
    pub fn for_guest(max_channels: u32) -> Self {
        Self {
            next: 1, // First odd ID
            max: max_channels,
        }
    }

    /// Allocate the next channel ID.
    ///
    /// Returns None if no more IDs are available.
    pub fn allocate(&mut self) -> Option<ChannelId> {
        if self.next >= self.max {
            return None;
        }
        let id = self.next;
        self.next += 2; // Skip to next ID with same parity
        Some(ChannelId(id))
    }
}

/// Request ID allocator (SHM-specific).
#[derive(Debug)]
pub struct RequestIdAllocator {
    next: u32,
}

impl RequestIdAllocator {
    /// Create a new request ID allocator.
    pub fn new() -> Self {
        Self { next: 1 }
    }

    /// Allocate the next request ID.
    pub fn allocate(&mut self) -> RequestId {
        let id = self.next;
        self.next = self.next.wrapping_add(1);
        if self.next == 0 {
            self.next = 1; // Skip 0
        }
        RequestId(id)
    }
}

impl Default for RequestIdAllocator {
    fn default() -> Self {
        Self::new()
    }
}
