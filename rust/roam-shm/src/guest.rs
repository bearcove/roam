//! Guest-side SHM implementation.
//!
//! Guests attach to an existing shared memory segment created by the host.
//! Each guest gets a unique peer ID and communicates through its dedicated
//! rings and slot pool.

use std::io;
use std::ptr;

use roam_frame::{Frame, INLINE_PAYLOAD_LEN, INLINE_PAYLOAD_SLOT, MsgDesc, Payload};
use shm_primitives::{AllocResult, HeapRegion, Region, SlotHandle, TreiberSlabRaw};

use crate::channel::ChannelEntry;
use crate::layout::{
    CHANNEL_ENTRY_SIZE, DESC_SIZE, HEADER_SIZE, MAGIC, SegmentHeader, SegmentLayout, VERSION,
};
use crate::peer::{PeerEntry, PeerId};

/// Guest-side handle for a SHM segment.
///
/// shm[impl shm.topology.hub]
pub struct ShmGuest {
    /// Backing memory (heap-allocated for testing, mmap in production)
    #[allow(dead_code)]
    backing: Option<HeapRegion>,
    /// Region view into backing memory
    region: Region,
    /// Our peer ID
    peer_id: PeerId,
    /// Computed layout (reconstructed from header)
    layout: SegmentLayout,
    /// Our slot pool
    slots: TreiberSlabRaw,
    /// Local tail for our G→H ring (what we've published)
    g2h_local_head: u64,
    /// Local head for our H→G ring (what we've consumed)
    h2g_local_tail: u64,
    /// Pending slots waiting for acknowledgment
    pending_slots: Vec<SlotHandle>,
}

/// Errors when attaching to a SHM segment.
#[derive(Debug)]
pub enum AttachError {
    /// Invalid magic bytes
    InvalidMagic,
    /// Unsupported version
    UnsupportedVersion,
    /// No available peer slots
    NoPeerSlots,
    /// Host has signaled goodbye
    HostGoodbye,
    /// I/O error
    Io(io::Error),
}

impl std::fmt::Display for AttachError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AttachError::InvalidMagic => write!(f, "invalid magic bytes"),
            AttachError::UnsupportedVersion => write!(f, "unsupported segment version"),
            AttachError::NoPeerSlots => write!(f, "no available peer slots"),
            AttachError::HostGoodbye => write!(f, "host has signaled goodbye"),
            AttachError::Io(e) => write!(f, "I/O error: {}", e),
        }
    }
}

impl std::error::Error for AttachError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            AttachError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl ShmGuest {
    /// Attach to an existing SHM segment.
    ///
    /// This finds an empty peer slot, atomically claims it, and initializes
    /// the guest's local state.
    ///
    /// shm[impl shm.guest.attach]
    pub fn attach(region: Region) -> Result<Self, AttachError> {
        // Validate header
        let header = unsafe { &*(region.as_ptr() as *const SegmentHeader) };

        if header.magic != MAGIC {
            return Err(AttachError::InvalidMagic);
        }
        if header.version != VERSION || header.header_size != HEADER_SIZE as u32 {
            return Err(AttachError::UnsupportedVersion);
        }
        if header.is_host_goodbye() {
            return Err(AttachError::HostGoodbye);
        }

        // Reconstruct layout from header
        let config = crate::layout::SegmentConfig {
            max_payload_size: header.max_payload_size,
            initial_credit: header.initial_credit,
            max_guests: header.max_guests,
            ring_size: header.ring_size,
            slot_size: header.slot_size,
            slots_per_guest: header.slots_per_guest,
            max_channels: header.max_channels,
            heartbeat_interval: header.heartbeat_interval,
        };
        let layout = config
            .layout()
            .map_err(|_| AttachError::UnsupportedVersion)?;

        // Find and claim an empty peer slot
        // shm[impl shm.guest.attach.cas]
        let mut peer_id = None;
        for i in 1..=header.max_guests as u8 {
            let Some(id) = PeerId::from_index(i - 1) else {
                continue;
            };
            let offset = layout.peer_entry_offset(i);
            let entry = unsafe { &*(region.offset(offset as usize) as *const PeerEntry) };

            // Try to claim this slot with CAS
            if entry.try_attach().is_ok() {
                peer_id = Some(id);
                break;
            }
        }

        let peer_id = peer_id.ok_or(AttachError::NoPeerSlots)?;

        // Create slot pool view
        let slots = unsafe {
            Self::create_slot_pool_view(
                &region,
                layout.guest_slot_pool_offset(peer_id.get()),
                &config,
            )
        };

        // Initialize channel table entries to Free
        let channel_table_offset = layout.guest_channel_table_offset(peer_id.get());
        for i in 0..config.max_channels {
            let entry_offset = channel_table_offset as usize + i as usize * CHANNEL_ENTRY_SIZE;
            let entry = unsafe { &mut *(region.offset(entry_offset) as *mut ChannelEntry) };
            entry.init();
        }

        Ok(Self {
            backing: None,
            region,
            peer_id,
            layout,
            slots,
            g2h_local_head: 0,
            h2g_local_tail: 0,
            pending_slots: Vec::new(),
        })
    }

    /// Attach to a segment using a HeapRegion (for testing).
    #[cfg(test)]
    pub fn attach_heap(backing: HeapRegion) -> Result<Self, AttachError> {
        let region = backing.region();
        let mut guest = Self::attach(region)?;
        guest.backing = Some(backing);
        Ok(guest)
    }

    /// Create a TreiberSlabRaw view for a slot pool.
    ///
    /// # Safety
    ///
    /// The pool must be initialized.
    unsafe fn create_slot_pool_view(
        region: &Region,
        offset: u64,
        config: &crate::layout::SegmentConfig,
    ) -> TreiberSlabRaw {
        use core::mem::size_of;
        use shm_primitives::{SlotMeta, TreiberSlabHeader};

        let header_ptr = region.offset(offset as usize) as *mut TreiberSlabHeader;

        // Compute meta and data offsets
        let meta_offset = offset as usize + size_of::<TreiberSlabHeader>();
        let meta_offset = ((meta_offset + 7) / 8) * 8; // Align to 8
        let data_offset = meta_offset + config.slots_per_guest as usize * size_of::<SlotMeta>();
        let data_offset = ((data_offset + 3) / 4) * 4; // Align to 4

        let meta_ptr = region.offset(meta_offset) as *mut SlotMeta;
        let data_ptr = region.offset(data_offset);

        // SAFETY: caller guarantees pool is initialized
        unsafe { TreiberSlabRaw::from_raw(header_ptr, meta_ptr, data_ptr) }
    }

    /// Get our peer ID.
    #[inline]
    pub fn peer_id(&self) -> PeerId {
        self.peer_id
    }

    /// Get the segment header.
    fn header(&self) -> &SegmentHeader {
        unsafe { &*(self.region.as_ptr() as *const SegmentHeader) }
    }

    /// Get our peer entry.
    fn peer_entry(&self) -> &PeerEntry {
        let offset = self.layout.peer_entry_offset(self.peer_id.get()) as usize;
        unsafe { &*(self.region.offset(offset) as *const PeerEntry) }
    }

    /// Check if the host has signaled goodbye.
    ///
    /// shm[impl shm.goodbye.host]
    #[inline]
    pub fn is_host_goodbye(&self) -> bool {
        self.header().is_host_goodbye()
    }

    /// Send a message to the host.
    ///
    /// shm[impl shm.topology.hub.calls]
    pub fn send(&mut self, frame: Frame) -> Result<(), SendError> {
        if self.is_host_goodbye() {
            return Err(SendError::HostGoodbye);
        }

        // Get G→H ring info
        let ring_offset = self.layout.guest_to_host_ring_offset(self.peer_id.get());
        let ring_size = self.layout.config.ring_size as u64;

        // Check if ring is full
        // shm[impl shm.ring.full]
        // Ring is full when (head + 1) % ring_size == tail
        // Using head - tail comparison: full when head - tail >= ring_size - 1
        let tail = self.peer_entry().g2h_tail() as u64;
        if self.g2h_local_head.wrapping_sub(tail) >= ring_size - 1 {
            return Err(SendError::RingFull);
        }

        // Write descriptor
        // shm[impl shm.ordering.ring-publish]
        let slot = (self.g2h_local_head % ring_size) as usize;
        let desc_offset = ring_offset as usize + slot * DESC_SIZE;

        // Handle payload - extract data slice
        let mut desc = frame.desc;
        let data: Option<&[u8]> = match &frame.payload {
            Payload::Inline => None,
            Payload::Owned(data) => Some(data.as_slice()),
            Payload::Bytes(data) => Some(data.as_ref()),
        };

        if let Some(data) = data {
            if data.len() <= INLINE_PAYLOAD_LEN {
                // Can inline
                desc.payload_slot = INLINE_PAYLOAD_SLOT;
                desc.payload_generation = 0;
                desc.payload_offset = 0;
                desc.payload_len = data.len() as u32;
                desc.inline_payload[..data.len()].copy_from_slice(data);
            } else {
                // Need slot from our pool
                // shm[impl shm.payload.slot]
                match self.slots.try_alloc() {
                    AllocResult::Ok(handle) => {
                        let slot_ptr = unsafe { self.slots.slot_data_ptr(handle) };
                        // Skip generation counter (4 bytes)
                        unsafe {
                            ptr::copy_nonoverlapping(data.as_ptr(), slot_ptr.add(4), data.len());
                        }
                        desc.payload_slot = handle.index;
                        desc.payload_generation = handle.generation;
                        desc.payload_offset = 0;
                        desc.payload_len = data.len() as u32;

                        // Mark as in-flight
                        let _ = self.slots.mark_in_flight(handle);

                        // Track for later freeing
                        self.pending_slots.push(handle);
                    }
                    AllocResult::WouldBlock => {
                        // shm[impl shm.slot.exhaustion]
                        return Err(SendError::SlotExhausted);
                    }
                }
            }
        }

        // Write descriptor
        unsafe {
            ptr::write(self.region.offset(desc_offset) as *mut MsgDesc, desc);
        }

        // Publish head
        self.g2h_local_head = self.g2h_local_head.wrapping_add(1);
        self.peer_entry()
            .g2h_publish_head(self.g2h_local_head as u32);

        Ok(())
    }

    /// Receive a message from the host.
    ///
    /// shm[impl shm.ordering.ring-consume]
    pub fn recv(&mut self) -> Option<Frame> {
        if self.is_host_goodbye() {
            return None;
        }

        // Get H→G ring info
        let ring_offset = self.layout.host_to_guest_ring_offset(self.peer_id.get());
        let ring_size = self.layout.config.ring_size as u64;

        // Check for messages
        let head = self.peer_entry().h2g_head() as u64;
        if self.h2g_local_tail >= head {
            return None; // Ring empty
        }

        let slot = (self.h2g_local_tail % ring_size) as usize;
        let desc_offset = ring_offset as usize + slot * DESC_SIZE;
        let desc = unsafe { ptr::read(self.region.offset(desc_offset) as *const MsgDesc) };

        // Get payload
        let payload = self.get_payload(&desc);
        let frame = Frame { desc, payload };

        // Advance tail
        self.h2g_local_tail = self.h2g_local_tail.wrapping_add(1);
        self.peer_entry()
            .h2g_advance_tail(self.h2g_local_tail as u32);

        Some(frame)
    }

    /// Get payload from a descriptor and free the slot.
    fn get_payload(&self, desc: &MsgDesc) -> Payload {
        if desc.payload_slot == INLINE_PAYLOAD_SLOT {
            Payload::Inline
        } else {
            // Read from host's slot pool
            let pool_offset = self.layout.host_slot_pool_offset();
            let slot_offset = pool_offset as usize
                + self.slot_header_size()
                + desc.payload_slot as usize * self.layout.config.slot_size as usize
                + 4; // Skip generation counter

            let payload_ptr = self
                .region
                .offset(slot_offset + desc.payload_offset as usize);
            let payload = unsafe {
                std::slice::from_raw_parts(payload_ptr, desc.payload_len as usize).to_vec()
            };

            // Free the slot (return to host's pool)
            // shm[impl shm.slot.free]
            let host_slots = unsafe {
                Self::create_slot_pool_view(&self.region, pool_offset, &self.layout.config)
            };
            let handle = SlotHandle {
                index: desc.payload_slot,
                generation: desc.payload_generation,
            };
            // Ignore errors - slot may already be freed
            let _ = host_slots.free(handle);

            Payload::Owned(payload)
        }
    }

    /// Compute slot pool header size.
    fn slot_header_size(&self) -> usize {
        use core::mem::size_of;
        use shm_primitives::{SlotMeta, TreiberSlabHeader};

        let meta_offset = size_of::<TreiberSlabHeader>();
        let meta_offset = ((meta_offset + 7) / 8) * 8;
        let data_offset =
            meta_offset + self.layout.config.slots_per_guest as usize * size_of::<SlotMeta>();
        ((data_offset + 3) / 4) * 4
    }

    /// Update heartbeat.
    ///
    /// shm[impl shm.heartbeat]
    pub fn heartbeat(&self) {
        let entry = self.peer_entry();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos() as u64;
        entry.update_heartbeat(now);
    }

    /// Initiate graceful detach.
    ///
    /// shm[impl shm.guest.detach]
    pub fn detach(&mut self) {
        let entry = self.peer_entry();

        // Set state to Goodbye
        entry.set_goodbye();

        // Drain any pending received messages
        while self.recv().is_some() {}

        // Free pending slots
        for handle in self.pending_slots.drain(..) {
            let _ = self.slots.free(handle);
        }
    }
}

impl Drop for ShmGuest {
    fn drop(&mut self) {
        // Ensure graceful detach
        self.detach();
    }
}

/// Errors from send operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
    /// Host has signaled goodbye
    HostGoodbye,
    /// Ring is full (backpressure)
    RingFull,
    /// No slots available (backpressure)
    SlotExhausted,
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::HostGoodbye => write!(f, "host goodbye"),
            SendError::RingFull => write!(f, "ring full"),
            SendError::SlotExhausted => write!(f, "slot exhausted"),
        }
    }
}

impl std::error::Error for SendError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::host::ShmHost;
    use crate::layout::SegmentConfig;

    #[test]
    fn attach_to_segment() {
        let config = SegmentConfig::default();
        let host = ShmHost::create(config).unwrap();
        let region = host.region();

        let guest = ShmGuest::attach(region).unwrap();
        assert_eq!(guest.peer_id().get(), 1);
    }

    #[test]
    fn multiple_guests_get_different_ids() {
        let config = SegmentConfig::default();
        let host = ShmHost::create(config).unwrap();
        let region = host.region();

        let guest1 = ShmGuest::attach(region).unwrap();
        let guest2 = ShmGuest::attach(region).unwrap();

        assert_eq!(guest1.peer_id().get(), 1);
        assert_eq!(guest2.peer_id().get(), 2);
    }
}
