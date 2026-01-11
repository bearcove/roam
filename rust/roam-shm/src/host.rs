//! Host-side SHM implementation.
//!
//! The host creates and owns the shared memory segment. It initializes
//! the segment header, peer table, and per-guest areas.

use std::collections::HashMap;
use std::io;
use std::ptr;

use roam_frame::{Frame, INLINE_PAYLOAD_LEN, INLINE_PAYLOAD_SLOT, MsgDesc, Payload};
use shm_primitives::{HeapRegion, Region, SlotHandle, TreiberSlabRaw};

use crate::channel::ChannelEntry;
use crate::layout::{
    CHANNEL_ENTRY_SIZE, DESC_SIZE, HEADER_SIZE, MAGIC, SegmentConfig, SegmentHeader, SegmentLayout,
    VERSION,
};
use crate::peer::{PeerEntry, PeerId, PeerState};

/// Host-side handle for a SHM segment.
///
/// shm[impl shm.topology.hub]
pub struct ShmHost {
    /// Backing memory (heap-allocated for now, mmap in production)
    #[allow(dead_code)]
    backing: HeapRegion,
    /// Region view into backing memory
    region: Region,
    /// Computed layout
    layout: SegmentLayout,
    /// Per-guest state tracked by the host
    guests: HashMap<PeerId, GuestState>,
    /// Host's slot pool
    host_slots: TreiberSlabRaw,
    /// Host's local head for each guest's H→G ring
    host_to_guest_heads: HashMap<PeerId, u64>,
}

/// Host-side state for a single guest.
struct GuestState {
    /// Last observed epoch
    last_epoch: u32,
    /// Slots we've allocated for messages to this guest
    pending_slots: Vec<SlotHandle>,
}

impl ShmHost {
    /// Create a new SHM segment with the given configuration.
    ///
    /// This allocates memory and initializes all data structures.
    pub fn create(config: SegmentConfig) -> io::Result<Self> {
        let layout = config
            .layout()
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;

        // Allocate backing memory
        let backing = HeapRegion::new_zeroed(layout.total_size as usize);
        let region = backing.region();

        // Initialize segment header
        // SAFETY: We just allocated this memory and it's zeroed
        unsafe {
            Self::init_header(&region, &layout);
            Self::init_peer_table(&region, &layout);
            Self::init_host_slots(&region, &layout);
            Self::init_guest_areas(&region, &layout);
        }

        // Create host slot pool view
        let host_slots = unsafe {
            Self::create_slot_pool_view(&region, layout.host_slot_pool_offset(), &config)
        };

        Ok(Self {
            backing,
            region,
            layout,
            guests: HashMap::new(),
            host_slots,
            host_to_guest_heads: HashMap::new(),
        })
    }

    /// Initialize the segment header.
    ///
    /// # Safety
    ///
    /// The region must be valid and exclusively owned.
    unsafe fn init_header(region: &Region, layout: &SegmentLayout) {
        let header = unsafe { &mut *(region.as_ptr() as *mut SegmentHeader) };
        header.magic = MAGIC;
        header.version = VERSION;
        header.header_size = HEADER_SIZE as u32;
        header.total_size = layout.total_size;
        header.max_payload_size = layout.config.max_payload_size;
        header.initial_credit = layout.config.initial_credit;
        header.max_guests = layout.config.max_guests;
        header.ring_size = layout.config.ring_size;
        header.peer_table_offset = layout.peer_table_offset;
        header.slot_region_offset = layout.slot_region_offset;
        header.slot_size = layout.config.slot_size;
        header.slots_per_guest = layout.config.slots_per_guest;
        header.max_channels = layout.config.max_channels;
        header.heartbeat_interval = layout.config.heartbeat_interval;
        // host_goodbye and reserved are already zeroed
    }

    /// Initialize the peer table.
    ///
    /// # Safety
    ///
    /// The region must be valid and exclusively owned.
    unsafe fn init_peer_table(region: &Region, layout: &SegmentLayout) {
        for i in 0..layout.config.max_guests {
            let peer_id = PeerId::from_index(i as u8).unwrap();
            let offset = layout.peer_entry_offset(peer_id.get()) as usize;
            let entry = unsafe { &mut *(region.offset(offset) as *mut PeerEntry) };

            entry.init(
                layout.guest_rings_offset(peer_id.get()),
                layout.guest_slot_pool_offset(peer_id.get()),
                layout.guest_channel_table_offset(peer_id.get()),
            );
        }
    }

    /// Initialize the host's slot pool.
    ///
    /// # Safety
    ///
    /// The region must be valid and exclusively owned.
    unsafe fn init_host_slots(region: &Region, layout: &SegmentLayout) {
        // SAFETY: caller guarantees region is valid and exclusively owned
        unsafe { Self::init_slot_pool(region, layout.host_slot_pool_offset(), &layout.config) };
    }

    /// Initialize all guest areas.
    ///
    /// # Safety
    ///
    /// The region must be valid and exclusively owned.
    unsafe fn init_guest_areas(region: &Region, layout: &SegmentLayout) {
        for i in 0..layout.config.max_guests {
            let peer_id = PeerId::from_index(i as u8).unwrap();

            // Initialize rings (zeroed by HeapRegion)
            // shm[impl shm.ring.initialization]

            // Initialize slot pool
            // SAFETY: caller guarantees region is valid
            unsafe {
                Self::init_slot_pool(
                    region,
                    layout.guest_slot_pool_offset(peer_id.get()),
                    &layout.config,
                )
            };

            // Initialize channel table
            // SAFETY: caller guarantees region is valid
            unsafe {
                Self::init_channel_table(
                    region,
                    layout.guest_channel_table_offset(peer_id.get()),
                    &layout.config,
                )
            };
        }
    }

    /// Initialize a slot pool at the given offset.
    ///
    /// # Safety
    ///
    /// The region must be valid and the offset must be correct.
    unsafe fn init_slot_pool(region: &Region, offset: u64, config: &SegmentConfig) {
        // Compute header size
        let bitmap_words = (config.slots_per_guest as u64 + 63) / 64;
        let bitmap_bytes = bitmap_words * 8;
        let header_size = ((bitmap_bytes + 63) / 64) * 64; // Align to 64

        // Initialize bitmap: all slots free (all bits set to 1)
        let bitmap_ptr = region.offset(offset as usize) as *mut u64;
        for i in 0..bitmap_words as usize {
            unsafe { ptr::write(bitmap_ptr.add(i), u64::MAX) };
        }

        // Clear any excess bits in the last word
        let excess = (config.slots_per_guest as u64) % 64;
        if excess != 0 {
            let last_word = unsafe { &mut *bitmap_ptr.add(bitmap_words as usize - 1) };
            *last_word = (1u64 << excess) - 1;
        }

        // Initialize slot generations to 0
        let slots_ptr = region.offset((offset + header_size) as usize);
        for i in 0..config.slots_per_guest {
            let slot_ptr =
                unsafe { slots_ptr.add(i as usize * config.slot_size as usize) } as *mut u32;
            unsafe { ptr::write(slot_ptr, 0) }; // Generation = 0
        }
    }

    /// Initialize a channel table at the given offset.
    ///
    /// # Safety
    ///
    /// The region must be valid and the offset must be correct.
    unsafe fn init_channel_table(region: &Region, offset: u64, config: &SegmentConfig) {
        for i in 0..config.max_channels {
            let entry_offset = offset as usize + i as usize * CHANNEL_ENTRY_SIZE;
            let entry = unsafe { &mut *(region.offset(entry_offset) as *mut ChannelEntry) };
            entry.init();
        }
    }

    /// Create a TreiberSlabRaw view for a slot pool.
    ///
    /// # Safety
    ///
    /// The pool must be initialized.
    unsafe fn create_slot_pool_view(
        region: &Region,
        offset: u64,
        config: &SegmentConfig,
    ) -> TreiberSlabRaw {
        use core::mem::size_of;
        use shm_primitives::SlotMeta;
        use shm_primitives::TreiberSlabHeader;

        let header_ptr = region.offset(offset as usize) as *mut TreiberSlabHeader;

        // Initialize the header
        unsafe { (*header_ptr).init(config.slot_size, config.slots_per_guest) };

        // Compute meta and data offsets
        let meta_offset = offset as usize + size_of::<TreiberSlabHeader>();
        let meta_offset = ((meta_offset + 7) / 8) * 8; // Align to 8
        let data_offset = meta_offset + config.slots_per_guest as usize * size_of::<SlotMeta>();
        let data_offset = ((data_offset + 3) / 4) * 4; // Align to 4

        let meta_ptr = region.offset(meta_offset) as *mut SlotMeta;
        let data_ptr = region.offset(data_offset);

        // Initialize slot metadata
        for i in 0..config.slots_per_guest {
            let meta = unsafe { &mut *meta_ptr.add(i as usize) };
            meta.init();
        }

        let slab = unsafe { TreiberSlabRaw::from_raw(header_ptr, meta_ptr, data_ptr) };

        // Initialize free list
        unsafe { slab.init_free_list() };

        slab
    }

    /// Get the segment header.
    fn header(&self) -> &SegmentHeader {
        unsafe { &*(self.region.as_ptr() as *const SegmentHeader) }
    }

    /// Get a peer entry.
    fn peer_entry(&self, peer_id: PeerId) -> &PeerEntry {
        let offset = self.layout.peer_entry_offset(peer_id.get()) as usize;
        unsafe { &*(self.region.offset(offset) as *const PeerEntry) }
    }

    /// Poll all guest rings for incoming messages.
    ///
    /// shm[impl shm.host.poll-peers]
    ///
    /// Returns an iterator over (peer_id, frame) pairs.
    pub fn poll(&mut self) -> Vec<(PeerId, Frame)> {
        let mut messages = Vec::new();
        let mut crashed_guests = Vec::new();
        let mut goodbye_guests = Vec::new();

        for i in 0..self.layout.config.max_guests {
            let Some(peer_id) = PeerId::from_index(i as u8) else {
                continue;
            };
            let entry = self.peer_entry(peer_id);

            // Check peer state
            let state = entry.state();
            if state == PeerState::Empty {
                continue;
            }

            // Check for epoch change (crash detection)
            // shm[impl shm.crash.epoch]
            let current_epoch = entry.epoch();
            if let Some(guest_state) = self.guests.get(&peer_id) {
                if guest_state.last_epoch != current_epoch && guest_state.last_epoch != 0 {
                    // Epoch changed unexpectedly - previous guest crashed
                    crashed_guests.push(peer_id);
                    continue;
                }
            }

            if state == PeerState::Goodbye {
                // Guest is shutting down
                goodbye_guests.push(peer_id);
                continue;
            }

            // Dequeue messages from Guest→Host ring
            // shm[impl shm.ordering.ring-consume]
            let ring_offset = self.layout.guest_to_host_ring_offset(peer_id.get());
            let ring_size = self.layout.config.ring_size;

            loop {
                let tail = entry.g2h_tail();
                let head = entry.g2h_head();

                if tail >= head {
                    break; // Ring empty
                }

                let slot = (tail % ring_size) as usize;
                let desc_offset = ring_offset as usize + slot * DESC_SIZE;
                let desc = unsafe { ptr::read(self.region.offset(desc_offset) as *const MsgDesc) };

                // Get payload
                let payload = self.get_payload(&desc, peer_id);
                let frame = Frame { desc, payload };

                messages.push((peer_id, frame));

                // Advance tail
                entry.g2h_advance_tail(tail.wrapping_add(1));
            }

            // Update guest state
            self.guests
                .entry(peer_id)
                .or_insert(GuestState {
                    last_epoch: current_epoch,
                    pending_slots: Vec::new(),
                })
                .last_epoch = current_epoch;
        }

        // Handle crashed and goodbye guests after the loop
        for peer_id in crashed_guests {
            self.handle_guest_crash(peer_id);
        }
        for peer_id in goodbye_guests {
            self.handle_guest_goodbye(peer_id);
        }

        messages
    }

    /// Get payload from a descriptor.
    fn get_payload(&self, desc: &MsgDesc, peer_id: PeerId) -> Payload {
        if desc.payload_slot == INLINE_PAYLOAD_SLOT {
            Payload::Inline
        } else {
            // Read from guest's slot pool
            let pool_offset = self.layout.guest_slot_pool_offset(peer_id.get());
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

            // Free the slot (return to guest's pool)
            // TODO: Implement slot freeing

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

    /// Send a message to a specific guest.
    ///
    /// shm[impl shm.topology.hub.calls]
    pub fn send(&mut self, peer_id: PeerId, frame: Frame) -> Result<(), SendError> {
        // Check peer state and get needed values first
        let (peer_state, h2g_head, h2g_tail, current_epoch) = {
            let entry = self.peer_entry(peer_id);
            (
                entry.state(),
                entry.h2g_head(),
                entry.h2g_tail(),
                entry.epoch(),
            )
        };

        if peer_state != PeerState::Attached {
            return Err(SendError::PeerNotAttached);
        }

        // Get H→G ring info
        let ring_offset = self.layout.host_to_guest_ring_offset(peer_id.get());
        let ring_size = self.layout.config.ring_size;

        // Get or initialize local head
        let local_head = self
            .host_to_guest_heads
            .entry(peer_id)
            .or_insert_with(|| h2g_head as u64);

        // Check if ring is full
        // shm[impl shm.ring.full]
        let tail = h2g_tail as u64;
        if local_head.wrapping_sub(tail) >= ring_size as u64 {
            return Err(SendError::RingFull);
        }

        // Write descriptor
        // shm[impl shm.ordering.ring-publish]
        let slot = (*local_head % ring_size as u64) as usize;
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
                // Need slot from host pool
                // shm[impl shm.payload.slot]
                match self.host_slots.try_alloc() {
                    shm_primitives::AllocResult::Ok(handle) => {
                        let slot_ptr = unsafe { self.host_slots.slot_data_ptr(handle) };
                        // Skip generation counter (4 bytes)
                        unsafe {
                            ptr::copy_nonoverlapping(data.as_ptr(), slot_ptr.add(4), data.len());
                        }
                        desc.payload_slot = handle.index;
                        desc.payload_generation = handle.generation;
                        desc.payload_offset = 0;
                        desc.payload_len = data.len() as u32;

                        // Mark as in-flight
                        let _ = self.host_slots.mark_in_flight(handle);

                        // Track for later freeing
                        self.guests
                            .entry(peer_id)
                            .or_insert(GuestState {
                                last_epoch: current_epoch,
                                pending_slots: Vec::new(),
                            })
                            .pending_slots
                            .push(handle);
                    }
                    shm_primitives::AllocResult::WouldBlock => {
                        // shm[impl shm.slot.exhaustion]
                        return Err(SendError::SlotExhausted);
                    }
                }
            }
        }

        // Write descriptor with Release ordering
        unsafe {
            ptr::write(self.region.offset(desc_offset) as *mut MsgDesc, desc);
        }

        // Publish head
        let new_head = local_head.wrapping_add(1);
        *local_head = new_head;
        let _ = local_head; // Release the mutable borrow
        self.peer_entry(peer_id).h2g_publish_head(new_head as u32);

        Ok(())
    }

    /// Handle a guest crash.
    ///
    /// shm[impl shm.crash.recovery]
    fn handle_guest_crash(&mut self, peer_id: PeerId) {
        let entry = self.peer_entry(peer_id);

        // Set state to Goodbye
        entry.set_goodbye();

        // Reset rings
        entry.reset();

        // Free pending slots
        if let Some(state) = self.guests.remove(&peer_id) {
            for handle in state.pending_slots {
                let _ = self.host_slots.free(handle);
            }
        }

        // Reset channel table entries
        let channel_table_offset = self.layout.guest_channel_table_offset(peer_id.get());
        for i in 0..self.layout.config.max_channels {
            let entry_offset = channel_table_offset as usize + i as usize * CHANNEL_ENTRY_SIZE;
            let channel_entry =
                unsafe { &*(self.region.offset(entry_offset) as *const ChannelEntry) };
            channel_entry.reset_to_free();
        }

        // Clear local head tracking
        self.host_to_guest_heads.remove(&peer_id);
    }

    /// Handle a guest goodbye.
    fn handle_guest_goodbye(&mut self, peer_id: PeerId) {
        // Drain remaining messages before cleanup
        // (handled by poll loop)

        // Free pending slots
        if let Some(state) = self.guests.remove(&peer_id) {
            for handle in state.pending_slots {
                let _ = self.host_slots.free(handle);
            }
        }

        // Clear local head tracking
        self.host_to_guest_heads.remove(&peer_id);
    }

    /// Initiate host goodbye (graceful shutdown).
    ///
    /// shm[impl shm.goodbye.host]
    pub fn goodbye(&self, _reason: &str) {
        self.header().set_host_goodbye(1);
    }

    /// Check if host has said goodbye.
    pub fn is_goodbye(&self) -> bool {
        self.header().is_host_goodbye()
    }

    /// Get the number of attached guests.
    pub fn attached_guest_count(&self) -> usize {
        (0..self.layout.config.max_guests)
            .filter(|&i| {
                PeerId::from_index(i as u8)
                    .map(|id| self.peer_entry(id).state() == PeerState::Attached)
                    .unwrap_or(false)
            })
            .count()
    }

    /// Get the segment layout.
    pub fn layout(&self) -> &SegmentLayout {
        &self.layout
    }

    /// Get a raw pointer to the segment for testing.
    #[cfg(test)]
    pub fn region(&self) -> Region {
        self.region
    }
}

/// Errors from send operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendError {
    /// Peer is not attached
    PeerNotAttached,
    /// Ring is full (backpressure)
    RingFull,
    /// No slots available (backpressure)
    SlotExhausted,
}

impl std::fmt::Display for SendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SendError::PeerNotAttached => write!(f, "peer not attached"),
            SendError::RingFull => write!(f, "ring full"),
            SendError::SlotExhausted => write!(f, "slot exhausted"),
        }
    }
}

impl std::error::Error for SendError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_host() {
        let config = SegmentConfig::default();
        let host = ShmHost::create(config).unwrap();

        // Verify header
        let header = host.header();
        assert_eq!(header.magic, MAGIC);
        assert_eq!(header.version, VERSION);
        assert!(!host.is_goodbye());
    }

    #[test]
    fn host_goodbye() {
        let config = SegmentConfig::default();
        let host = ShmHost::create(config).unwrap();

        assert!(!host.is_goodbye());
        host.goodbye("test shutdown");
        assert!(host.is_goodbye());
    }

    #[test]
    fn poll_empty() {
        let config = SegmentConfig::default();
        let mut host = ShmHost::create(config).unwrap();

        let messages = host.poll();
        assert!(messages.is_empty());
    }
}
