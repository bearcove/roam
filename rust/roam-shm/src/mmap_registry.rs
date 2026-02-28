//! Mmap payload registry for large payloads that exceed the VarSlotPool.
//!
//! When a payload exceeds the largest VarSlotPool slot size, it is placed into a
//! separately memory-mapped file. The BipBuffer carries a 32-byte MMAP_REF frame
//! pointing to `(map_id, map_generation, map_offset, payload_len)`.
//!
//! The host (sender) side manages `MmapRegistry`, allocating space in mmap regions
//! and delivering file descriptors to the peer via a control socket.
//!
//! The guest (receiver) side manages `MmapAttachments`, receiving fds and resolving
//! mmap references to usable byte slices.

use std::collections::HashMap;
use std::io;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use shm_primitives::{MmapAttachMessage, MmapRegion};

/// r[impl shm.mmap]
/// r[impl shm.mmap.registry]
///
/// Host-side registry of mmap-backed payload regions.
///
/// Each slot holds an `MmapRegion` with a bump allocator for sub-allocations.
/// File descriptors are delivered to the peer via the control channel on first use.
pub struct MmapRegistry {
    slots: Vec<MmapSlot>,
    next_map_id: u32,
    channel: MmapChannelTx,
    default_region_size: usize,
}

struct MmapSlot {
    region: Arc<MmapRegion>,
    map_id: u32,
    map_generation: u32,
    delivered: bool,
    active_leases: Arc<AtomicU32>,
    offset: usize,
}

/// Result of an mmap allocation.
pub struct MmapAllocation {
    pub map_id: u32,
    pub map_generation: u32,
    pub map_offset: u64,
    pub region: Arc<MmapRegion>,
    pub lease_counter: Arc<AtomicU32>,
}

impl MmapAllocation {
    /// Get a mutable slice to write the payload into.
    ///
    /// # Safety
    /// The caller must ensure no other thread is reading this range concurrently.
    pub unsafe fn payload_mut(&mut self, len: usize) -> &mut [u8] {
        let region = self.region.region();
        let ptr = unsafe { region.as_ptr().add(self.map_offset as usize) };
        unsafe { std::slice::from_raw_parts_mut(ptr, len) }
    }
}

impl MmapRegistry {
    pub fn new(channel: MmapChannelTx, default_region_size: usize) -> Self {
        Self {
            slots: Vec::new(),
            next_map_id: 0,
            channel,
            default_region_size,
        }
    }

    /// r[impl shm.mmap.publish]
    ///
    /// Allocate space for a payload of `len` bytes.
    ///
    /// Creates a new mmap region if no existing slot has enough space.
    /// Delivers the fd to the peer if this is the first use of the slot.
    pub fn alloc(&mut self, len: usize) -> io::Result<MmapAllocation> {
        // Try to find an existing slot with enough space
        for slot in &mut self.slots {
            if slot.offset + len <= slot.region.len() {
                let offset = slot.offset;
                slot.offset += len;

                // r[impl shm.mmap.attach.once]
                if !slot.delivered {
                    self.channel.send_region(
                        &slot.region,
                        &MmapAttachMessage {
                            map_id: slot.map_id,
                            map_generation: slot.map_generation,
                            mapping_length: slot.region.len() as u64,
                        },
                    )?;
                    slot.delivered = true;
                }

                slot.active_leases.fetch_add(1, Ordering::Release);

                return Ok(MmapAllocation {
                    map_id: slot.map_id,
                    map_generation: slot.map_generation,
                    map_offset: offset as u64,
                    region: slot.region.clone(),
                    lease_counter: slot.active_leases.clone(),
                });
            }
        }

        // No existing slot fits — create a new one
        let region_size = self.default_region_size.max(len);
        let map_id = self.next_map_id;
        self.next_map_id += 1;
        let map_generation = 0;

        let region = create_mmap_region(region_size)?;
        let region = Arc::new(region);

        let active_leases = Arc::new(AtomicU32::new(1));

        // Deliver fd to peer
        self.channel.send_region(
            &region,
            &MmapAttachMessage {
                map_id,
                map_generation,
                mapping_length: region.len() as u64,
            },
        )?;

        let slot = MmapSlot {
            region: region.clone(),
            map_id,
            map_generation,
            delivered: true,
            active_leases: active_leases.clone(),
            offset: len,
        };
        self.slots.push(slot);

        Ok(MmapAllocation {
            map_id,
            map_generation,
            map_offset: 0,
            region,
            lease_counter: active_leases,
        })
    }

    /// r[impl shm.mmap.reclaim]
    ///
    /// Reclaim slots where all leases have been released and the region is fully consumed.
    pub fn try_reclaim(&mut self) {
        self.slots.retain(|slot| {
            let leases = slot.active_leases.load(Ordering::Acquire);
            leases > 0 || slot.offset < slot.region.len()
        });
    }
}

fn create_mmap_region(size: usize) -> io::Result<MmapRegion> {
    let dir = tempfile::tempdir()
        .map_err(|e| io::Error::other(format!("failed to create temp dir for mmap region: {e}")))?;
    let path = dir.path().join("mmap_payload.shm");
    MmapRegion::create(&path, size, shm_primitives::FileCleanup::Auto)
}

/// Sender half of the mmap control channel.
pub enum MmapChannelTx {
    #[cfg(unix)]
    Real(shm_primitives::MmapControlSender),
    InProcess(tokio::sync::mpsc::UnboundedSender<(Arc<MmapRegion>, MmapAttachMessage)>),
}

/// Receiver half of the mmap control channel.
pub enum MmapChannelRx {
    #[cfg(unix)]
    Real(shm_primitives::MmapControlReceiver),
    InProcess(tokio::sync::mpsc::UnboundedReceiver<(Arc<MmapRegion>, MmapAttachMessage)>),
}

impl MmapChannelTx {
    fn send_region(&self, region: &Arc<MmapRegion>, msg: &MmapAttachMessage) -> io::Result<()> {
        match self {
            #[cfg(unix)]
            MmapChannelTx::Real(sender) => sender.send(region.as_raw_fd(), msg),
            MmapChannelTx::InProcess(sender) => sender.send((region.clone(), *msg)).map_err(|_| {
                io::Error::new(io::ErrorKind::BrokenPipe, "mmap control channel closed")
            }),
        }
    }
}

/// r[impl shm.mmap.bounds]
///
/// Guest-side attachments: received mmap regions indexed by (map_id, map_generation).
pub struct MmapAttachments {
    mappings: HashMap<(u32, u32), Arc<AttachedMapping>>,
    channel: MmapChannelRx,
}

/// A single attached mmap region on the receiver side.
pub struct AttachedMapping {
    pub region: MmapRegion,
    pub map_id: u32,
    pub map_generation: u32,
    pub mapping_length: u64,
}

// SAFETY: MmapRegion is Send+Sync, and AttachedMapping only adds Copy fields
unsafe impl Send for AttachedMapping {}
unsafe impl Sync for AttachedMapping {}

impl MmapAttachments {
    pub fn new(channel: MmapChannelRx) -> Self {
        Self {
            mappings: HashMap::new(),
            channel,
        }
    }

    /// Drain all pending control messages, attaching new regions.
    pub fn drain_control(&mut self) {
        loop {
            match &mut self.channel {
                #[cfg(unix)]
                MmapChannelRx::Real(receiver) => match receiver.try_recv() {
                    Ok(Some((fd, msg))) => {
                        let key = (msg.map_id, msg.map_generation);
                        if self.mappings.contains_key(&key) {
                            continue;
                        }
                        match MmapRegion::attach_fd(fd, msg.mapping_length as usize) {
                            Ok(region) => {
                                self.mappings.insert(
                                    key,
                                    Arc::new(AttachedMapping {
                                        region,
                                        map_id: msg.map_id,
                                        map_generation: msg.map_generation,
                                        mapping_length: msg.mapping_length,
                                    }),
                                );
                            }
                            Err(_) => continue,
                        }
                    }
                    Ok(None) => break,
                    Err(_) => break,
                },
                MmapChannelRx::InProcess(receiver) => match receiver.try_recv() {
                    Ok((region, msg)) => {
                        let key = (msg.map_id, msg.map_generation);
                        if self.mappings.contains_key(&key) {
                            continue;
                        }
                        self.attach_in_process(&region, msg, key);
                    }
                    Err(tokio::sync::mpsc::error::TryRecvError::Empty) => break,
                    Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
                },
            }
        }
    }

    #[cfg(unix)]
    fn attach_in_process(
        &mut self,
        region: &Arc<MmapRegion>,
        msg: MmapAttachMessage,
        key: (u32, u32),
    ) {
        use std::os::unix::io::{FromRawFd, OwnedFd};

        let raw = region.as_raw_fd();
        let duped = unsafe { libc::dup(raw) };
        if duped < 0 {
            return;
        }
        let owned_fd = unsafe { OwnedFd::from_raw_fd(duped) };
        if let Ok(attached_region) = MmapRegion::attach_fd(owned_fd, msg.mapping_length as usize) {
            self.mappings.insert(
                key,
                Arc::new(AttachedMapping {
                    region: attached_region,
                    map_id: msg.map_id,
                    map_generation: msg.map_generation,
                    mapping_length: msg.mapping_length,
                }),
            );
        }
    }

    /// r[impl shm.mmap.bounds]
    /// r[impl shm.mmap.aba]
    ///
    /// Resolve an mmap reference to an attached mapping.
    pub fn resolve(
        &self,
        map_id: u32,
        map_generation: u32,
        map_offset: u64,
        payload_len: u32,
    ) -> Result<Arc<AttachedMapping>, MmapResolveError> {
        let key = (map_id, map_generation);
        let mapping = self
            .mappings
            .get(&key)
            .ok_or(MmapResolveError::UnknownMapping {
                map_id,
                map_generation,
            })?;

        let end =
            map_offset
                .checked_add(payload_len as u64)
                .ok_or(MmapResolveError::BoundsOverflow {
                    map_id,
                    map_generation,
                    map_offset,
                    payload_len,
                })?;

        if end > mapping.mapping_length {
            return Err(MmapResolveError::OutOfBounds {
                map_id,
                map_generation,
                map_offset,
                payload_len,
                mapping_length: mapping.mapping_length,
            });
        }

        Ok(mapping.clone())
    }
}

#[derive(Debug)]
pub enum MmapResolveError {
    UnknownMapping {
        map_id: u32,
        map_generation: u32,
    },
    OutOfBounds {
        map_id: u32,
        map_generation: u32,
        map_offset: u64,
        payload_len: u32,
        mapping_length: u64,
    },
    BoundsOverflow {
        map_id: u32,
        map_generation: u32,
        map_offset: u64,
        payload_len: u32,
    },
}

impl std::fmt::Display for MmapResolveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            MmapResolveError::UnknownMapping {
                map_id,
                map_generation,
            } => {
                write!(
                    f,
                    "unknown mmap mapping: map_id={map_id}, gen={map_generation}"
                )
            }
            MmapResolveError::OutOfBounds {
                map_id,
                map_generation,
                map_offset,
                payload_len,
                mapping_length,
            } => {
                write!(
                    f,
                    "mmap bounds check failed: map_id={map_id}, gen={map_generation}, \
                     offset={map_offset}, len={payload_len}, mapping_length={mapping_length}"
                )
            }
            MmapResolveError::BoundsOverflow {
                map_id,
                map_generation,
                map_offset,
                payload_len,
            } => {
                write!(
                    f,
                    "mmap offset+len overflow: map_id={map_id}, gen={map_generation}, \
                     offset={map_offset}, len={payload_len}"
                )
            }
        }
    }
}

impl std::error::Error for MmapResolveError {}

/// Create an in-process mmap channel pair for testing.
pub fn create_in_process_mmap_channel() -> (MmapChannelTx, MmapChannelRx) {
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    (MmapChannelTx::InProcess(tx), MmapChannelRx::InProcess(rx))
}
