//! Variable-size slot pools with multiple size classes.
//!
//! This module implements shared variable-size slot pools as specified in
//! `docs/content/shm-spec/_index.md`. Unlike fixed-size per-guest pools,
//! variable-size pools are shared across all guests with per-slot ownership
//! tracking for crash recovery.

use core::sync::atomic::{AtomicU64, Ordering};

use shm_primitives::{Region, SlotState, VarSlotMeta};

use crate::layout::SizeClass;

/// Sentinel value indicating end of free list.
pub const FREE_LIST_END: u64 = u64::MAX;

/// Header for a single size class (64 bytes, cache-line aligned).
///
/// shm[impl shm.varslot.freelist]
#[repr(C, align(64))]
pub struct SizeClassHeader {
    /// Size of each slot in this class.
    pub slot_size: u32,
    /// Number of slots in this class.
    pub slot_count: u32,
    /// Free list head: packed (index in upper 32 bits, generation in lower 32 bits).
    /// Uses `FREE_LIST_END` as sentinel for empty list.
    pub free_head: AtomicU64,
    /// Reserved for alignment.
    pub _reserved: [u8; 48],
}

const _: () = assert!(core::mem::size_of::<SizeClassHeader>() == 64);

impl SizeClassHeader {
    /// Pack a slot index and generation into a free list head value.
    #[inline]
    pub fn pack(index: u32, generation: u32) -> u64 {
        ((index as u64) << 32) | (generation as u64)
    }

    /// Unpack a free list head value into (index, generation).
    #[inline]
    pub fn unpack(packed: u64) -> (u32, u32) {
        let index = (packed >> 32) as u32;
        let generation = packed as u32;
        (index, generation)
    }

    /// Initialize a size class header.
    pub fn init(&mut self, slot_size: u32, slot_count: u32) {
        self.slot_size = slot_size;
        self.slot_count = slot_count;
        self.free_head = AtomicU64::new(FREE_LIST_END);
        self._reserved = [0; 48];
    }
}

/// Handle to an allocated variable-size slot.
///
/// Encodes the size class index, slot index within that class, and generation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VarSlotHandle {
    /// Size class index (0-255).
    pub class_idx: u8,
    /// Slot index within the size class.
    pub slot_idx: u32,
    /// Generation counter for ABA detection.
    pub generation: u32,
}

impl VarSlotHandle {
    /// Sentinel value for inline payloads (no slot allocated).
    pub const INLINE: Self = Self {
        class_idx: 0xFF,
        slot_idx: 0x00FFFFFF,
        generation: 0,
    };

    /// Check if this is the inline sentinel.
    #[inline]
    pub fn is_inline(&self) -> bool {
        self.class_idx == 0xFF && self.slot_idx == 0x00FFFFFF
    }

    /// Pack into a u32 for MsgDesc.payload_slot.
    ///
    /// Format: class_idx (8 bits) | slot_idx (24 bits).
    #[inline]
    pub fn pack_slot(&self) -> u32 {
        ((self.class_idx as u32) << 24) | (self.slot_idx & 0x00FFFFFF)
    }

    /// Unpack from MsgDesc.payload_slot and payload_generation.
    #[inline]
    pub fn from_packed(payload_slot: u32, payload_generation: u32) -> Self {
        Self {
            class_idx: (payload_slot >> 24) as u8,
            slot_idx: payload_slot & 0x00FFFFFF,
            generation: payload_generation,
        }
    }
}

/// Errors from freeing a variable slot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarFreeError {
    /// The generation doesn't match (double-free or stale handle).
    GenerationMismatch { expected: u32, actual: u32 },
    /// The slot index is out of range.
    InvalidIndex,
    /// The class index is out of range.
    InvalidClass,
    /// The slot is not in the expected state.
    InvalidState {
        expected: SlotState,
        actual: SlotState,
    },
}

/// Variable-size slot pool with multiple size classes.
///
/// shm[impl shm.varslot.shared]
pub struct VarSlotPool {
    region: Region,
    /// Offset to the first size class header.
    base_offset: u64,
    /// Size class configurations.
    classes: Vec<SizeClass>,
    /// Computed offsets to each class's metadata array.
    meta_offsets: Vec<u64>,
    /// Computed offsets to each class's data array.
    data_offsets: Vec<u64>,
}

impl VarSlotPool {
    /// Create a new VarSlotPool view.
    ///
    /// This does not initialize the pool - use `init` for that.
    pub fn new(region: Region, base_offset: u64, classes: Vec<SizeClass>) -> Self {
        let mut meta_offsets = Vec::with_capacity(classes.len());
        let mut data_offsets = Vec::with_capacity(classes.len());

        // Headers are at the start
        let headers_size = classes.len() as u64 * 64;
        let mut offset = base_offset + headers_size;

        for class in &classes {
            // Align metadata array
            offset = align_up(offset, 16); // VarSlotMeta is 16 bytes
            meta_offsets.push(offset);
            offset += class.count as u64 * 16; // VarSlotMeta size

            // Align data array
            offset = align_up(offset, 64);
            data_offsets.push(offset);
            offset += class.count as u64 * class.slot_size as u64;
        }

        Self {
            region,
            base_offset,
            classes,
            meta_offsets,
            data_offsets,
        }
    }

    /// Calculate the total size needed for a variable slot pool.
    pub fn calculate_size(classes: &[SizeClass]) -> u64 {
        let headers_size = classes.len() as u64 * 64;
        let mut size = headers_size;

        for class in classes {
            // Align metadata array
            size = align_up(size, 16);
            size += class.count as u64 * 16; // VarSlotMeta size

            // Align data array
            size = align_up(size, 64);
            size += class.count as u64 * class.slot_size as u64;
        }

        align_up(size, 64)
    }

    /// Initialize the pool (call once during segment creation).
    ///
    /// # Safety
    ///
    /// Caller must ensure exclusive access during initialization.
    pub unsafe fn init(&self) {
        // Initialize size class headers
        for (i, class) in self.classes.iter().enumerate() {
            let header = self.class_header_mut(i);
            header.init(class.slot_size, class.count);
        }

        // Initialize slot metadata and build free lists
        for class_idx in 0..self.classes.len() {
            let class = &self.classes[class_idx];

            // Initialize all slot metadata
            for slot_idx in 0..class.count {
                let meta = self.slot_meta_mut(class_idx, slot_idx);
                meta.init();
            }

            // Build free list by linking slots together
            // Link 0 -> 1 -> 2 -> ... -> (n-1) -> END
            for slot_idx in 0..class.count {
                let meta = self.slot_meta_mut(class_idx, slot_idx);
                if slot_idx + 1 < class.count {
                    meta.next_free.store(slot_idx + 1, Ordering::Release);
                } else {
                    meta.next_free.store(u32::MAX, Ordering::Release);
                }
            }

            // Set head to first slot
            let header = self.class_header_mut(class_idx);
            if class.count > 0 {
                header
                    .free_head
                    .store(SizeClassHeader::pack(0, 0), Ordering::Release);
            }
        }
    }

    fn class_header_ptr(&self, class_idx: usize) -> *mut SizeClassHeader {
        let offset = self.base_offset as usize + class_idx * 64;
        self.region.offset(offset) as *mut SizeClassHeader
    }

    fn class_header(&self, class_idx: usize) -> &SizeClassHeader {
        unsafe { &*self.class_header_ptr(class_idx) }
    }

    fn class_header_mut(&self, class_idx: usize) -> &mut SizeClassHeader {
        unsafe { &mut *self.class_header_ptr(class_idx) }
    }

    fn slot_meta_ptr(&self, class_idx: usize, slot_idx: u32) -> *mut VarSlotMeta {
        let offset = self.meta_offsets[class_idx] as usize + slot_idx as usize * 16;
        self.region.offset(offset) as *mut VarSlotMeta
    }

    fn slot_meta(&self, class_idx: usize, slot_idx: u32) -> &VarSlotMeta {
        unsafe { &*self.slot_meta_ptr(class_idx, slot_idx) }
    }

    fn slot_meta_mut(&self, class_idx: usize, slot_idx: u32) -> &mut VarSlotMeta {
        unsafe { &mut *self.slot_meta_ptr(class_idx, slot_idx) }
    }

    /// Get a pointer to the slot's payload data area.
    pub fn payload_ptr(&self, handle: VarSlotHandle) -> Option<*mut u8> {
        if handle.class_idx as usize >= self.classes.len() {
            return None;
        }
        let class = &self.classes[handle.class_idx as usize];
        if handle.slot_idx >= class.count {
            return None;
        }
        let offset = self.data_offsets[handle.class_idx as usize] as usize
            + handle.slot_idx as usize * class.slot_size as usize;
        Some(self.region.offset(offset))
    }

    /// Get the slot size for a given class.
    pub fn slot_size(&self, class_idx: u8) -> Option<u32> {
        self.classes.get(class_idx as usize).map(|c| c.slot_size)
    }

    /// Allocate a slot that can hold `size` bytes.
    ///
    /// shm[impl shm.varslot.selection]
    ///
    /// Finds the smallest size class that fits, with fallback to larger classes
    /// if the preferred class is exhausted.
    pub fn alloc(&self, size: u32, owner: u8) -> Option<VarSlotHandle> {
        // Find smallest class that fits
        for (class_idx, class) in self.classes.iter().enumerate() {
            if class.slot_size >= size {
                if let Some(handle) = self.alloc_from_class(class_idx, owner) {
                    return Some(handle);
                }
                // Class exhausted, try next larger
            }
        }
        None // All classes exhausted
    }

    /// Allocate from a specific size class.
    ///
    /// shm[impl shm.varslot.allocation]
    pub fn alloc_from_class(&self, class_idx: usize, owner: u8) -> Option<VarSlotHandle> {
        if class_idx >= self.classes.len() {
            return None;
        }

        let header = self.class_header(class_idx);

        loop {
            let head = header.free_head.load(Ordering::Acquire);
            if head == FREE_LIST_END {
                return None; // Class exhausted
            }

            let (index, tag) = SizeClassHeader::unpack(head);
            let meta = self.slot_meta(class_idx, index);

            // Read next pointer before CAS
            let next = meta.next_free.load(Ordering::Acquire);
            let next_packed = if next == u32::MAX {
                FREE_LIST_END
            } else {
                SizeClassHeader::pack(next, tag.wrapping_add(1))
            };

            // Try to pop from free list
            match header.free_head.compare_exchange_weak(
                head,
                next_packed,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => {
                    // Success! Initialize slot metadata
                    let new_gen = meta
                        .generation
                        .fetch_add(1, Ordering::AcqRel)
                        .wrapping_add(1);
                    meta.state
                        .store(SlotState::Allocated as u32, Ordering::Release);
                    meta.owner_peer.store(owner as u32, Ordering::Release);

                    return Some(VarSlotHandle {
                        class_idx: class_idx as u8,
                        slot_idx: index,
                        generation: new_gen,
                    });
                }
                Err(_) => continue, // Retry
            }
        }
    }

    /// Mark a slot as in-flight (after enqueue).
    pub fn mark_in_flight(&self, handle: VarSlotHandle) -> Result<(), VarFreeError> {
        if handle.class_idx as usize >= self.classes.len() {
            return Err(VarFreeError::InvalidClass);
        }
        let class = &self.classes[handle.class_idx as usize];
        if handle.slot_idx >= class.count {
            return Err(VarFreeError::InvalidIndex);
        }

        let meta = self.slot_meta(handle.class_idx as usize, handle.slot_idx);

        // Verify generation
        let actual_gen = meta.generation.load(Ordering::Acquire);
        if actual_gen != handle.generation {
            return Err(VarFreeError::GenerationMismatch {
                expected: handle.generation,
                actual: actual_gen,
            });
        }

        // Transition Allocated -> InFlight
        match meta.state.compare_exchange(
            SlotState::Allocated as u32,
            SlotState::InFlight as u32,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => Ok(()),
            Err(actual) => Err(VarFreeError::InvalidState {
                expected: SlotState::Allocated,
                actual: SlotState::from_u32(actual).unwrap_or(SlotState::Free),
            }),
        }
    }

    /// Free an in-flight slot back to its pool.
    ///
    /// shm[impl shm.varslot.freeing]
    pub fn free(&self, handle: VarSlotHandle) -> Result<(), VarFreeError> {
        if handle.class_idx as usize >= self.classes.len() {
            return Err(VarFreeError::InvalidClass);
        }
        let class = &self.classes[handle.class_idx as usize];
        if handle.slot_idx >= class.count {
            return Err(VarFreeError::InvalidIndex);
        }

        let meta = self.slot_meta(handle.class_idx as usize, handle.slot_idx);

        // Verify generation (detect double-free)
        let actual_gen = meta.generation.load(Ordering::Acquire);
        if actual_gen != handle.generation {
            return Err(VarFreeError::GenerationMismatch {
                expected: handle.generation,
                actual: actual_gen,
            });
        }

        // Transition InFlight -> Free
        match meta.state.compare_exchange(
            SlotState::InFlight as u32,
            SlotState::Free as u32,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {}
            Err(actual) => {
                return Err(VarFreeError::InvalidState {
                    expected: SlotState::InFlight,
                    actual: SlotState::from_u32(actual).unwrap_or(SlotState::Free),
                });
            }
        }

        // Push to free list
        self.push_to_free_list(handle.class_idx as usize, handle.slot_idx);
        Ok(())
    }

    /// Free an allocated (never sent) slot back to its pool.
    pub fn free_allocated(&self, handle: VarSlotHandle) -> Result<(), VarFreeError> {
        if handle.class_idx as usize >= self.classes.len() {
            return Err(VarFreeError::InvalidClass);
        }
        let class = &self.classes[handle.class_idx as usize];
        if handle.slot_idx >= class.count {
            return Err(VarFreeError::InvalidIndex);
        }

        let meta = self.slot_meta(handle.class_idx as usize, handle.slot_idx);

        // Verify generation
        let actual_gen = meta.generation.load(Ordering::Acquire);
        if actual_gen != handle.generation {
            return Err(VarFreeError::GenerationMismatch {
                expected: handle.generation,
                actual: actual_gen,
            });
        }

        // Transition Allocated -> Free
        match meta.state.compare_exchange(
            SlotState::Allocated as u32,
            SlotState::Free as u32,
            Ordering::AcqRel,
            Ordering::Acquire,
        ) {
            Ok(_) => {}
            Err(actual) => {
                return Err(VarFreeError::InvalidState {
                    expected: SlotState::Allocated,
                    actual: SlotState::from_u32(actual).unwrap_or(SlotState::Free),
                });
            }
        }

        // Push to free list
        self.push_to_free_list(handle.class_idx as usize, handle.slot_idx);
        Ok(())
    }

    fn push_to_free_list(&self, class_idx: usize, slot_idx: u32) {
        let header = self.class_header(class_idx);
        let meta = self.slot_meta(class_idx, slot_idx);

        loop {
            let head = header.free_head.load(Ordering::Acquire);
            let (head_idx, head_gen) = if head == FREE_LIST_END {
                (u32::MAX, 0u32)
            } else {
                SizeClassHeader::unpack(head)
            };

            // Set our next pointer
            meta.next_free.store(head_idx, Ordering::Release);

            // Try to become new head
            let new_head = SizeClassHeader::pack(slot_idx, head_gen.wrapping_add(1));

            match header.free_head.compare_exchange_weak(
                head,
                new_head,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return,
                Err(_) => continue, // Retry
            }
        }
    }

    /// Recover all slots owned by a crashed peer.
    ///
    /// This scans all slots and frees any that were owned by the specified peer.
    /// Should be called when a peer crashes or disconnects unexpectedly.
    pub fn recover_peer(&self, peer_id: u8) {
        for class_idx in 0..self.classes.len() {
            let class = &self.classes[class_idx];
            for slot_idx in 0..class.count {
                let meta = self.slot_meta(class_idx, slot_idx);

                let owner = meta.owner_peer.load(Ordering::Acquire);
                let state = meta.state.load(Ordering::Acquire);

                if owner == peer_id as u32 && state != SlotState::Free as u32 {
                    // Force transition to Free
                    meta.state.store(SlotState::Free as u32, Ordering::Release);

                    // Push to free list
                    self.push_to_free_list(class_idx, slot_idx);
                }
            }
        }
    }

    /// Get the number of size classes.
    pub fn class_count(&self) -> usize {
        self.classes.len()
    }

    /// Get the size classes.
    pub fn classes(&self) -> &[SizeClass] {
        &self.classes
    }

    /// Approximate count of free slots in a class.
    pub fn free_count_approx(&self, class_idx: usize) -> u32 {
        if class_idx >= self.classes.len() {
            return 0;
        }

        let header = self.class_header(class_idx);
        let slot_count = self.classes[class_idx].count;
        let mut count = 0u32;
        let mut current = header.free_head.load(Ordering::Acquire);

        while current != FREE_LIST_END && count < slot_count {
            let (index, _) = SizeClassHeader::unpack(current);
            if index < slot_count {
                count += 1;
                let meta = self.slot_meta(class_idx, index);
                let next = meta.next_free.load(Ordering::Acquire);
                current = if next == u32::MAX {
                    FREE_LIST_END
                } else {
                    SizeClassHeader::pack(next, 0)
                };
            } else {
                break;
            }
        }

        count
    }
}

#[inline]
const fn align_up(value: u64, align: u64) -> u64 {
    (value + (align - 1)) & !(align - 1)
}

#[cfg(test)]
mod tests {
    use super::*;
    use shm_primitives::HeapRegion;

    fn create_test_pool() -> (HeapRegion, VarSlotPool) {
        let classes = vec![
            SizeClass::new(64, 16),  // Small: 64 bytes × 16 slots
            SizeClass::new(256, 8),  // Medium: 256 bytes × 8 slots
            SizeClass::new(1024, 4), // Large: 1 KB × 4 slots
        ];

        let size = VarSlotPool::calculate_size(&classes);
        let region = HeapRegion::new_zeroed(size as usize);
        let pool = VarSlotPool::new(region.region(), 0, classes);

        unsafe { pool.init() };

        (region, pool)
    }

    #[test]
    fn test_size_class_header_pack_unpack() {
        let packed = SizeClassHeader::pack(42, 123);
        let (index, generation) = SizeClassHeader::unpack(packed);
        assert_eq!(index, 42);
        assert_eq!(generation, 123);

        let packed_max = SizeClassHeader::pack(u32::MAX, u32::MAX);
        let (index_max, generation_max) = SizeClassHeader::unpack(packed_max);
        assert_eq!(index_max, u32::MAX);
        assert_eq!(generation_max, u32::MAX);
    }

    #[test]
    fn test_var_slot_handle_pack() {
        let handle = VarSlotHandle {
            class_idx: 2,
            slot_idx: 0x00123456,
            generation: 42,
        };
        let packed = handle.pack_slot();
        assert_eq!(packed, 0x02123456);

        let unpacked = VarSlotHandle::from_packed(packed, 42);
        assert_eq!(unpacked.class_idx, 2);
        assert_eq!(unpacked.slot_idx, 0x00123456);
        assert_eq!(unpacked.generation, 42);
    }

    #[test]
    fn test_alloc_smallest_fit() {
        let (_region, pool) = create_test_pool();

        // Small payload uses small class
        let small = pool.alloc(32, 0).unwrap();
        assert_eq!(small.class_idx, 0); // 64-byte class

        // Medium payload uses medium class
        let medium = pool.alloc(100, 0).unwrap();
        assert_eq!(medium.class_idx, 1); // 256-byte class

        // Large payload uses large class
        let large = pool.alloc(500, 0).unwrap();
        assert_eq!(large.class_idx, 2); // 1024-byte class
    }

    #[test]
    fn test_alloc_from_specific_class() {
        let (_region, pool) = create_test_pool();

        let handle = pool.alloc_from_class(1, 0).unwrap();
        assert_eq!(handle.class_idx, 1);
    }

    #[test]
    fn test_exhaustion_fallback() {
        let (_region, pool) = create_test_pool();

        // Exhaust small class (16 slots)
        let mut handles = Vec::new();
        for _ in 0..16 {
            handles.push(pool.alloc_from_class(0, 0).unwrap());
        }

        // Next small alloc should fail for this class
        assert!(pool.alloc_from_class(0, 0).is_none());

        // But alloc with size check should fall back to medium class
        let fallback = pool.alloc(32, 0).unwrap();
        assert_eq!(fallback.class_idx, 1); // Fell back to 256-byte class
    }

    #[test]
    fn test_free_and_realloc() {
        let (_region, pool) = create_test_pool();

        let handle1 = pool.alloc(32, 0).unwrap();
        pool.mark_in_flight(handle1).unwrap();
        pool.free(handle1).unwrap();

        // Should be able to allocate again
        let handle2 = pool.alloc(32, 0).unwrap();
        assert_eq!(handle2.class_idx, 0);
        // Generation should have increased
        assert!(handle2.generation > handle1.generation || handle2.slot_idx != handle1.slot_idx);
    }

    #[test]
    fn test_double_free_detected() {
        let (_region, pool) = create_test_pool();

        let handle = pool.alloc(32, 0).unwrap();
        pool.mark_in_flight(handle).unwrap();
        pool.free(handle).unwrap();

        // Second free should fail - the slot is now Free, not InFlight
        let result = pool.free(handle);
        assert!(matches!(
            result,
            Err(VarFreeError::InvalidState {
                expected: SlotState::InFlight,
                actual: SlotState::Free,
            })
        ));
    }

    #[test]
    fn test_owner_tracking() {
        let (_region, pool) = create_test_pool();

        let handle1 = pool.alloc(32, 1).unwrap(); // Owner: peer 1
        let handle2 = pool.alloc(32, 2).unwrap(); // Owner: peer 2

        let meta1 = pool.slot_meta(handle1.class_idx as usize, handle1.slot_idx);
        let meta2 = pool.slot_meta(handle2.class_idx as usize, handle2.slot_idx);

        assert_eq!(meta1.owner(), 1);
        assert_eq!(meta2.owner(), 2);
    }

    #[test]
    fn test_peer_recovery() {
        let (_region, pool) = create_test_pool();

        // Allocate slots for different peers
        let h1 = pool.alloc(32, 1).unwrap();
        let h2 = pool.alloc(32, 1).unwrap();
        let h3 = pool.alloc(32, 2).unwrap();

        // Mark as in-flight (simulating sent messages)
        pool.mark_in_flight(h1).unwrap();
        pool.mark_in_flight(h2).unwrap();
        pool.mark_in_flight(h3).unwrap();

        // Count free slots before recovery
        let free_before = pool.free_count_approx(0);

        // Recover peer 1's slots
        pool.recover_peer(1);

        // Peer 1's slots should now be free
        let free_after = pool.free_count_approx(0);
        assert_eq!(free_after, free_before + 2);

        // Peer 2's slot should still be in-flight (not recovered)
        let meta3 = pool.slot_meta(h3.class_idx as usize, h3.slot_idx);
        assert_eq!(meta3.state(), SlotState::InFlight);
        assert_eq!(meta3.owner(), 2);
    }

    #[test]
    fn test_payload_ptr() {
        let (_region, pool) = create_test_pool();

        let handle = pool.alloc(32, 0).unwrap();
        let ptr = pool.payload_ptr(handle);
        assert!(ptr.is_some());

        // Invalid handles should return None
        let invalid = VarSlotHandle {
            class_idx: 255,
            slot_idx: 0,
            generation: 0,
        };
        assert!(pool.payload_ptr(invalid).is_none());
    }
}
