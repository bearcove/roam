//! Segment layout types.
//!
//! Defines the segment header structure and layout computation for SHM segments.

use core::mem::size_of;
use core::sync::atomic::{AtomicU32, Ordering};

/// Magic bytes for SHM segment identification.
///
/// shm[impl shm.segment.magic]
pub const MAGIC: [u8; 8] = *b"RAPAHUB\x01";

/// Segment header size in bytes.
///
/// shm[impl shm.segment.header-size]
pub const HEADER_SIZE: usize = 128;

/// Segment format version.
pub const VERSION: u32 = 1;

/// Peer entry size in bytes.
pub const PEER_ENTRY_SIZE: usize = 64;

/// Channel entry size in bytes.
pub const CHANNEL_ENTRY_SIZE: usize = 16;

/// Descriptor size in bytes (one cache line).
pub const DESC_SIZE: usize = 64;

/// Segment header at the start of the shared memory region.
///
/// shm[impl shm.segment.header]
#[repr(C)]
pub struct SegmentHeader {
    /// Magic bytes: "RAPAHUB\x01"
    pub magic: [u8; 8],
    /// Segment format version (1)
    pub version: u32,
    /// Size of this header (128)
    pub header_size: u32,
    /// Total segment size in bytes
    pub total_size: u64,
    /// Maximum payload per message
    pub max_payload_size: u32,
    /// Initial channel credit (bytes)
    pub initial_credit: u32,
    /// Maximum number of guests (≤ 255)
    ///
    /// shm[impl shm.topology.max-guests]
    pub max_guests: u32,
    /// Descriptor ring capacity (power of 2)
    pub ring_size: u32,
    /// Offset to peer table
    pub peer_table_offset: u64,
    /// Offset to payload slot region
    pub slot_region_offset: u64,
    /// Size of each payload slot
    pub slot_size: u32,
    /// Number of slots per guest
    pub slots_per_guest: u32,
    /// Max concurrent channels per guest
    pub max_channels: u32,
    /// Host goodbye flag (0 = active)
    ///
    /// shm[impl shm.goodbye.host]
    /// shm[impl shm.goodbye.host-atomic]
    pub host_goodbye: AtomicU32,
    /// Heartbeat interval in nanoseconds (0 = disabled)
    pub heartbeat_interval: u64,
    /// Reserved for future use (zero)
    pub reserved: [u8; 48],
}

const _: () = assert!(size_of::<SegmentHeader>() == HEADER_SIZE);

impl SegmentHeader {
    /// Validate the segment header.
    ///
    /// Returns `true` if magic and version are correct.
    pub fn validate(&self) -> bool {
        self.magic == MAGIC && self.version == VERSION && self.header_size == HEADER_SIZE as u32
    }

    /// Check if the host has signaled goodbye.
    #[inline]
    pub fn is_host_goodbye(&self) -> bool {
        self.host_goodbye.load(Ordering::Acquire) != 0
    }

    /// Signal host goodbye with a reason code.
    #[inline]
    pub fn set_host_goodbye(&self, reason: u32) {
        self.host_goodbye.store(reason, Ordering::Release);
    }
}

/// Configuration for creating a new SHM segment.
#[derive(Debug, Clone)]
pub struct SegmentConfig {
    /// Maximum payload per message
    pub max_payload_size: u32,
    /// Initial channel credit (bytes)
    pub initial_credit: u32,
    /// Maximum number of guests (1-255)
    pub max_guests: u32,
    /// Descriptor ring capacity (must be power of 2)
    pub ring_size: u32,
    /// Size of each payload slot
    pub slot_size: u32,
    /// Number of slots per guest
    pub slots_per_guest: u32,
    /// Max concurrent channels per guest
    pub max_channels: u32,
    /// Heartbeat interval in nanoseconds (0 = disabled)
    pub heartbeat_interval: u64,
}

impl Default for SegmentConfig {
    fn default() -> Self {
        Self {
            max_payload_size: 64 * 1024, // 64 KB
            initial_credit: 256 * 1024,  // 256 KB
            max_guests: 16,
            ring_size: 256,       // Power of 2
            slot_size: 64 * 1024, // 64 KB slots
            slots_per_guest: 16,
            max_channels: 256,
            heartbeat_interval: 0, // Disabled by default
        }
    }
}

impl SegmentConfig {
    /// Validate the configuration.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.max_guests == 0 || self.max_guests > 255 {
            return Err("max_guests must be 1-255");
        }
        if !self.ring_size.is_power_of_two() {
            return Err("ring_size must be power of 2");
        }
        if self.ring_size < 2 {
            return Err("ring_size must be at least 2");
        }
        if self.slot_size < 4 {
            return Err("slot_size must be at least 4");
        }
        if self.slots_per_guest == 0 {
            return Err("slots_per_guest must be > 0");
        }
        if self.max_channels == 0 {
            return Err("max_channels must be > 0");
        }
        Ok(())
    }

    /// Compute the segment layout from this configuration.
    pub fn layout(&self) -> Result<SegmentLayout, &'static str> {
        self.validate()?;
        Ok(SegmentLayout::new(self))
    }
}

/// Computed layout of a SHM segment.
///
/// All offsets are cache-line aligned (64 bytes).
#[derive(Debug, Clone)]
pub struct SegmentLayout {
    /// Configuration used to compute this layout
    pub config: SegmentConfig,
    /// Offset to peer table
    pub peer_table_offset: u64,
    /// Size of peer table in bytes
    pub peer_table_size: u64,
    /// Offset to slot region (host slots first, then guest slots)
    pub slot_region_offset: u64,
    /// Size of each slot pool (header + slots)
    ///
    /// shm[impl shm.segment.pool-size]
    pub pool_size: u64,
    /// Offset to first guest area
    pub guest_areas_offset: u64,
    /// Size of each guest area (rings + slot pool + channel table)
    pub guest_area_size: u64,
    /// Total segment size
    pub total_size: u64,
}

impl SegmentLayout {
    /// Compute the segment layout from configuration.
    fn new(config: &SegmentConfig) -> Self {
        // Peer table follows header
        let peer_table_offset = align_up(HEADER_SIZE as u64, 64);
        let peer_table_size = (config.max_guests as u64) * (PEER_ENTRY_SIZE as u64);

        // Slot region follows peer table
        // Host slot pool is at position 0 in the slot region
        let slot_region_offset = align_up(peer_table_offset + peer_table_size, 64);

        // Compute slot pool size (header + slots)
        // shm[impl shm.slot.pool-header-size]
        let bitmap_words = (config.slots_per_guest as u64 + 63) / 64;
        let bitmap_bytes = bitmap_words * 8;
        let slot_pool_header_size = align_up(bitmap_bytes, 64);
        let pool_size =
            slot_pool_header_size + (config.slots_per_guest as u64) * (config.slot_size as u64);

        // Guest areas follow host slot pool
        // shm[impl shm.segment.guest-slot-offset]
        let guest_areas_offset = align_up(slot_region_offset + pool_size, 64);

        // Each guest area contains:
        // - Guest→Host ring: ring_size * 64 bytes
        // - Host→Guest ring: ring_size * 64 bytes
        // - Slot pool: pool_size bytes
        // - Channel table: max_channels * 16 bytes
        //
        // shm[impl shm.ring.layout]
        let rings_size = 2 * (config.ring_size as u64) * (DESC_SIZE as u64);
        let channel_table_size = (config.max_channels as u64) * (CHANNEL_ENTRY_SIZE as u64);
        let guest_area_size =
            align_up(rings_size, 64) + align_up(pool_size, 64) + align_up(channel_table_size, 64);

        // Total size
        let total_size = guest_areas_offset + (config.max_guests as u64) * guest_area_size;

        Self {
            config: config.clone(),
            peer_table_offset,
            peer_table_size,
            slot_region_offset,
            pool_size,
            guest_areas_offset,
            guest_area_size,
            total_size,
        }
    }

    /// Get the offset to a peer entry.
    ///
    /// shm[impl shm.topology.peer-id]
    #[inline]
    pub fn peer_entry_offset(&self, peer_id: u8) -> u64 {
        assert!(peer_id >= 1 && peer_id <= self.config.max_guests as u8);
        let index = (peer_id - 1) as u64;
        self.peer_table_offset + index * (PEER_ENTRY_SIZE as u64)
    }

    /// Get the offset to the host's slot pool.
    ///
    /// shm[impl shm.segment.host-slots]
    #[inline]
    pub fn host_slot_pool_offset(&self) -> u64 {
        self.slot_region_offset
    }

    /// Get the offset to a guest's area.
    #[inline]
    pub fn guest_area_offset(&self, peer_id: u8) -> u64 {
        assert!(peer_id >= 1 && peer_id <= self.config.max_guests as u8);
        let index = (peer_id - 1) as u64;
        self.guest_areas_offset + index * self.guest_area_size
    }

    /// Get the offset to a guest's rings.
    ///
    /// shm[impl shm.segment.guest-rings]
    #[inline]
    pub fn guest_rings_offset(&self, peer_id: u8) -> u64 {
        self.guest_area_offset(peer_id)
    }

    /// Get the offset to a guest's Guest→Host ring.
    #[inline]
    pub fn guest_to_host_ring_offset(&self, peer_id: u8) -> u64 {
        self.guest_rings_offset(peer_id)
    }

    /// Get the offset to a guest's Host→Guest ring.
    #[inline]
    pub fn host_to_guest_ring_offset(&self, peer_id: u8) -> u64 {
        self.guest_rings_offset(peer_id) + (self.config.ring_size as u64) * (DESC_SIZE as u64)
    }

    /// Get the offset to a guest's slot pool.
    ///
    /// shm[impl shm.segment.guest-slot-offset]
    #[inline]
    pub fn guest_slot_pool_offset(&self, peer_id: u8) -> u64 {
        let rings_size = 2 * (self.config.ring_size as u64) * (DESC_SIZE as u64);
        self.guest_area_offset(peer_id) + align_up(rings_size, 64)
    }

    /// Get the offset to a guest's channel table.
    ///
    /// shm[impl shm.flow.channel-table-location]
    #[inline]
    pub fn guest_channel_table_offset(&self, peer_id: u8) -> u64 {
        self.guest_slot_pool_offset(peer_id) + align_up(self.pool_size, 64)
    }
}

/// Align a value up to the given alignment.
#[inline]
const fn align_up(value: u64, align: u64) -> u64 {
    (value + (align - 1)) & !(align - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_size_is_128() {
        assert_eq!(size_of::<SegmentHeader>(), 128);
    }

    #[test]
    fn default_config_is_valid() {
        let config = SegmentConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn layout_offsets_are_aligned() {
        let config = SegmentConfig::default();
        let layout = config.layout().unwrap();

        assert_eq!(layout.peer_table_offset % 64, 0);
        assert_eq!(layout.slot_region_offset % 64, 0);
        assert_eq!(layout.guest_areas_offset % 64, 0);

        for peer_id in 1..=config.max_guests as u8 {
            assert_eq!(layout.guest_area_offset(peer_id) % 64, 0);
            assert_eq!(layout.guest_rings_offset(peer_id) % 64, 0);
            assert_eq!(layout.guest_slot_pool_offset(peer_id) % 64, 0);
            assert_eq!(layout.guest_channel_table_offset(peer_id) % 64, 0);
        }
    }

    #[test]
    fn invalid_configs_are_rejected() {
        let mut config = SegmentConfig::default();

        config.max_guests = 0;
        assert!(config.validate().is_err());

        config.max_guests = 256;
        assert!(config.validate().is_err());

        config.max_guests = 16;
        config.ring_size = 3; // Not power of 2
        assert!(config.validate().is_err());
    }
}
