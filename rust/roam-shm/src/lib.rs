//! Shared-memory transport for roam.
//!
//! Implements [`Link`](roam_types::Link) over lock-free ring buffers in shared memory.
//! Inline bipbuf payloads are copied into boxed backing; slot-ref payloads are exposed
//! as shared zero-copy backing and freed when the backing is dropped.

use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::{Arc, Mutex};

use roam_types::{Backing, Link, LinkRx, LinkTx, LinkTxPermit, SharedBacking, WriteSlot};
use shm_primitives::{BIPBUF_HEADER_SIZE, BipBuf, Doorbell, HeapRegion, PeerId};

use crate::framing::{DEFAULT_INLINE_THRESHOLD, OwnedFrame};
use crate::segment::Segment;
use crate::varslot::{SizeClassConfig, SlotRef, VarSlotPool};

pub mod framing;
pub mod peer_table;
pub mod segment;
pub mod varslot;

const SLOT_LEN_PREFIX_SIZE: usize = 4;

#[derive(Clone)]
enum Backend {
    Segment(Arc<Segment>),
    Heap(Arc<HeapBackend>),
}

struct HeapBackend {
    _region: Arc<HeapRegion>,
    var_pool: VarSlotPool,
}

impl Backend {
    fn allocate_slot(&self, size: u32, owner_peer: u8) -> Option<SlotRef> {
        match self {
            Backend::Segment(segment) => segment.var_pool().allocate(size, owner_peer),
            Backend::Heap(heap) => heap.var_pool.allocate(size, owner_peer),
        }
    }

    fn free_slot(&self, slot_ref: SlotRef) {
        match self {
            Backend::Segment(segment) => {
                let _ = segment.var_pool().free(slot_ref);
            }
            Backend::Heap(heap) => {
                let _ = heap.var_pool.free(slot_ref);
            }
        }
    }

    unsafe fn slot_data<'a>(&self, slot_ref: &SlotRef) -> &'a [u8] {
        match self {
            Backend::Segment(segment) => unsafe { segment.var_pool().slot_data(slot_ref) },
            Backend::Heap(heap) => unsafe { heap.var_pool.slot_data(slot_ref) },
        }
    }

    unsafe fn slot_data_mut<'a>(&self, slot_ref: &SlotRef) -> &'a mut [u8] {
        match self {
            Backend::Segment(segment) => unsafe { segment.var_pool().slot_data_mut(slot_ref) },
            Backend::Heap(heap) => unsafe { heap.var_pool.slot_data_mut(slot_ref) },
        }
    }
}

struct TxShared {
    tx_bipbuf: Arc<BipBuf>,
    backend: Backend,
    owner_peer: u8,
    max_payload_size: u32,
    inline_threshold: u32,
    tx_lock: Mutex<()>,
    doorbell: Arc<Doorbell>,
    doorbell_dead: AtomicBool,
    stats: Arc<ShmTransportStats>,
}

/// A [`Link`] over shared memory ring buffers.
// r[impl transport.shm]
pub struct ShmLink {
    tx_shared: Arc<TxShared>,
    rx_bipbuf: Arc<BipBuf>,
    rx_backend: Backend,
    tx_closed: Arc<AtomicBool>,
    peer_closed: Arc<AtomicBool>,
}

#[derive(Default)]
struct ShmTransportStats {
    inline_sends: AtomicU64,
    slot_ref_sends: AtomicU64,
    inline_recvs: AtomicU64,
    slot_ref_recvs: AtomicU64,
    varslot_exhausted: AtomicU64,
    ring_exhausted: AtomicU64,
    reserve_waits: AtomicU64,
    commit_retries: AtomicU64,
    doorbell_peer_dead: AtomicU64,
    doorbell_wait_errors: AtomicU64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ShmTransportStatsSnapshot {
    pub inline_sends: u64,
    pub slot_ref_sends: u64,
    pub inline_recvs: u64,
    pub slot_ref_recvs: u64,
    pub varslot_exhausted: u64,
    pub ring_exhausted: u64,
    pub reserve_waits: u64,
    pub commit_retries: u64,
    pub doorbell_peer_dead: u64,
    pub doorbell_wait_errors: u64,
}

impl ShmTransportStats {
    fn snapshot(&self) -> ShmTransportStatsSnapshot {
        ShmTransportStatsSnapshot {
            inline_sends: self.inline_sends.load(AtomicOrdering::Relaxed),
            slot_ref_sends: self.slot_ref_sends.load(AtomicOrdering::Relaxed),
            inline_recvs: self.inline_recvs.load(AtomicOrdering::Relaxed),
            slot_ref_recvs: self.slot_ref_recvs.load(AtomicOrdering::Relaxed),
            varslot_exhausted: self.varslot_exhausted.load(AtomicOrdering::Relaxed),
            ring_exhausted: self.ring_exhausted.load(AtomicOrdering::Relaxed),
            reserve_waits: self.reserve_waits.load(AtomicOrdering::Relaxed),
            commit_retries: self.commit_retries.load(AtomicOrdering::Relaxed),
            doorbell_peer_dead: self.doorbell_peer_dead.load(AtomicOrdering::Relaxed),
            doorbell_wait_errors: self.doorbell_wait_errors.load(AtomicOrdering::Relaxed),
        }
    }
}

impl ShmLink {
    fn normalize_threshold(threshold: u32) -> u32 {
        if threshold == 0 {
            DEFAULT_INLINE_THRESHOLD
        } else {
            threshold
        }
    }

    fn from_parts(
        tx_bipbuf: Arc<BipBuf>,
        rx_bipbuf: Arc<BipBuf>,
        backend: Backend,
        doorbell: Arc<Doorbell>,
        owner_peer: u8,
        max_payload_size: u32,
        inline_threshold: u32,
        tx_closed: Arc<AtomicBool>,
        peer_closed: Arc<AtomicBool>,
    ) -> Self {
        let stats = Arc::new(ShmTransportStats::default());
        let tx_shared = Arc::new(TxShared {
            tx_bipbuf,
            backend: backend.clone(),
            owner_peer,
            max_payload_size,
            inline_threshold: Self::normalize_threshold(inline_threshold),
            tx_lock: Mutex::new(()),
            doorbell,
            doorbell_dead: AtomicBool::new(false),
            stats: stats.clone(),
        });

        Self {
            tx_shared,
            rx_bipbuf,
            rx_backend: backend,
            tx_closed,
            peer_closed,
        }
    }

    /// Build a guest-side SHM link from a shared segment.
    pub fn for_guest(segment: Arc<Segment>, peer_id: PeerId, doorbell: Doorbell) -> Self {
        let tx_bipbuf = Arc::new(segment.g2h_bipbuf(peer_id));
        let rx_bipbuf = Arc::new(segment.h2g_bipbuf(peer_id));
        let backend = Backend::Segment(segment.clone());

        Self::from_parts(
            tx_bipbuf,
            rx_bipbuf,
            backend,
            Arc::new(doorbell),
            peer_id.get(),
            segment.header().max_payload_size,
            segment.header().inline_threshold,
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
        )
    }

    /// Build a host-side SHM link for one peer from a shared segment.
    pub fn for_host(segment: Arc<Segment>, peer_id: PeerId, doorbell: Doorbell) -> Self {
        let tx_bipbuf = Arc::new(segment.h2g_bipbuf(peer_id));
        let rx_bipbuf = Arc::new(segment.g2h_bipbuf(peer_id));
        let backend = Backend::Segment(segment.clone());

        Self::from_parts(
            tx_bipbuf,
            rx_bipbuf,
            backend,
            Arc::new(doorbell),
            0,
            segment.header().max_payload_size,
            segment.header().inline_threshold,
            Arc::new(AtomicBool::new(false)),
            Arc::new(AtomicBool::new(false)),
        )
    }

    /// Build an in-process SHM link pair over a heap-backed region.
    pub fn heap_pair(
        bipbuf_capacity: u32,
        max_payload_size: u32,
        inline_threshold: u32,
        size_classes: &[SizeClassConfig],
    ) -> io::Result<(Self, Self)> {
        fn align_up(n: usize, align: usize) -> usize {
            (n + align - 1) & !(align - 1)
        }

        let first_size = BIPBUF_HEADER_SIZE + bipbuf_capacity as usize;
        let second_offset = align_up(first_size, 64);
        let second_size = BIPBUF_HEADER_SIZE + bipbuf_capacity as usize;
        let pair_bytes = second_offset + second_size;
        let var_pool_offset = align_up(pair_bytes, 64);
        let var_pool_size = VarSlotPool::required_size(size_classes);
        let total_size = var_pool_offset + var_pool_size;

        let region = Arc::new(HeapRegion::new_zeroed(total_size));
        let g2h = Arc::new(unsafe { BipBuf::init(region.region(), 0, bipbuf_capacity) });
        let h2g =
            Arc::new(unsafe { BipBuf::init(region.region(), second_offset, bipbuf_capacity) });

        let var_pool = unsafe { VarSlotPool::init(region.region(), var_pool_offset, size_classes) };
        let backend = Backend::Heap(Arc::new(HeapBackend {
            _region: region,
            var_pool,
        }));
        let (a_doorbell, b_handle) = Doorbell::create_pair()?;
        let b_doorbell = Doorbell::from_handle(b_handle)?;

        let a_closed = Arc::new(AtomicBool::new(false));
        let b_closed = Arc::new(AtomicBool::new(false));

        let a = Self::from_parts(
            g2h.clone(),
            h2g.clone(),
            backend.clone(),
            Arc::new(a_doorbell),
            1,
            max_payload_size,
            inline_threshold,
            a_closed.clone(),
            b_closed.clone(),
        );
        let b = Self::from_parts(
            h2g,
            g2h,
            backend,
            Arc::new(b_doorbell),
            2,
            max_payload_size,
            inline_threshold,
            b_closed,
            a_closed,
        );
        Ok((a, b))
    }
}

/// Sending half of a [`ShmLink`].
pub struct ShmLinkTx {
    shared: Arc<TxShared>,
    tx_closed: Arc<AtomicBool>,
}

/// Receiving half of a [`ShmLink`].
pub struct ShmLinkRx {
    rx_bipbuf: Arc<BipBuf>,
    backend: Backend,
    peer_closed: Arc<AtomicBool>,
    doorbell: Arc<Doorbell>,
    stats: Arc<ShmTransportStats>,
}

pub struct ShmTxPermit {
    shared: Arc<TxShared>,
    tx_closed: Arc<AtomicBool>,
}

enum ShmWriteSlotInner {
    Inline {
        bytes: Vec<u8>,
    },
    VarSlot {
        slot_ref: Option<SlotRef>,
        payload_len: usize,
    },
}

pub struct ShmWriteSlot {
    shared: Arc<TxShared>,
    inner: ShmWriteSlotInner,
}

impl Drop for ShmWriteSlot {
    fn drop(&mut self) {
        if let ShmWriteSlotInner::VarSlot { slot_ref, .. } = &mut self.inner
            && let Some(slot_ref) = slot_ref.take()
        {
            self.shared.backend.free_slot(slot_ref);
        }
    }
}

impl Link for ShmLink {
    type Tx = ShmLinkTx;
    type Rx = ShmLinkRx;

    fn split(self) -> (Self::Tx, Self::Rx) {
        let tx_shared = self.tx_shared;
        let doorbell = tx_shared.doorbell.clone();
        let stats = tx_shared.stats.clone();
        (
            ShmLinkTx {
                shared: tx_shared,
                tx_closed: self.tx_closed,
            },
            ShmLinkRx {
                rx_bipbuf: self.rx_bipbuf,
                backend: self.rx_backend,
                peer_closed: self.peer_closed,
                doorbell,
                stats,
            },
        )
    }
}

impl LinkTx for ShmLinkTx {
    type Permit = ShmTxPermit;

    async fn reserve(&self) -> io::Result<Self::Permit> {
        loop {
            if self.tx_closed.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "shm tx is closed",
                ));
            }
            if self.shared.doorbell_dead.load(Ordering::Acquire) {
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "shm doorbell peer is closed",
                ));
            }
            if self
                .shared
                .tx_bipbuf
                .inner()
                .can_grant(framing::FRAME_HEADER_SIZE as u32)
            {
                return Ok(ShmTxPermit {
                    shared: self.shared.clone(),
                    tx_closed: self.tx_closed.clone(),
                });
            }

            self.shared
                .stats
                .reserve_waits
                .fetch_add(1, AtomicOrdering::Relaxed);
            if let Err(err) = self.shared.doorbell.wait().await {
                self.shared
                    .stats
                    .doorbell_wait_errors
                    .fetch_add(1, AtomicOrdering::Relaxed);
                return Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    format!("shm doorbell wait failed: {err}"),
                ));
            }
        }
    }

    async fn close(self) -> io::Result<()> {
        self.tx_closed.store(true, Ordering::Release);
        match self.shared.doorbell.signal_now() {
            shm_primitives::SignalResult::PeerDead => {
                self.shared.doorbell_dead.store(true, Ordering::Release);
                self.shared
                    .stats
                    .doorbell_peer_dead
                    .fetch_add(1, AtomicOrdering::Relaxed);
                Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "shm doorbell peer is closed",
                ))
            }
            _ => Ok(()),
        }
    }
}

impl LinkTxPermit for ShmTxPermit {
    type Slot = ShmWriteSlot;

    fn alloc(self, len: usize) -> io::Result<Self::Slot> {
        if self.tx_closed.load(Ordering::Acquire) {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "shm tx is closed",
            ));
        }
        if len > self.shared.max_payload_size as usize {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "payload exceeds max_payload_size",
            ));
        }

        if len as u32 <= self.shared.inline_threshold {
            let entry_len = ((framing::FRAME_HEADER_SIZE + len + 3) & !3) as u32;
            if !self.shared.tx_bipbuf.inner().can_grant(entry_len) {
                self.shared
                    .stats
                    .ring_exhausted
                    .fetch_add(1, AtomicOrdering::Relaxed);
                return Err(io::Error::new(io::ErrorKind::WouldBlock, "ring is full"));
            }
            return Ok(ShmWriteSlot {
                shared: self.shared,
                inner: ShmWriteSlotInner::Inline {
                    bytes: vec![0; len],
                },
            });
        }

        if !self
            .shared
            .tx_bipbuf
            .inner()
            .can_grant(framing::SLOT_REF_ENTRY_SIZE)
        {
            self.shared
                .stats
                .ring_exhausted
                .fetch_add(1, AtomicOrdering::Relaxed);
            return Err(io::Error::new(io::ErrorKind::WouldBlock, "ring is full"));
        }

        let slot_payload_len = len.checked_add(SLOT_LEN_PREFIX_SIZE).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "payload length overflow")
        })?;
        let slot_ref = self
            .shared
            .backend
            .allocate_slot(slot_payload_len as u32, self.shared.owner_peer)
            .ok_or_else(|| {
                self.shared
                    .stats
                    .varslot_exhausted
                    .fetch_add(1, AtomicOrdering::Relaxed);
                io::Error::new(io::ErrorKind::WouldBlock, "varslot exhausted")
            })?;

        Ok(ShmWriteSlot {
            shared: self.shared,
            inner: ShmWriteSlotInner::VarSlot {
                slot_ref: Some(slot_ref),
                payload_len: len,
            },
        })
    }
}

impl WriteSlot for ShmWriteSlot {
    fn as_mut_slice(&mut self) -> &mut [u8] {
        match &mut self.inner {
            ShmWriteSlotInner::Inline { bytes } => bytes.as_mut_slice(),
            ShmWriteSlotInner::VarSlot {
                slot_ref,
                payload_len,
            } => {
                let slot_ref = slot_ref
                    .as_ref()
                    .expect("slot must be present while write slot is alive");
                let end = SLOT_LEN_PREFIX_SIZE + *payload_len;
                let data = unsafe { self.shared.backend.slot_data_mut(slot_ref) };
                &mut data[SLOT_LEN_PREFIX_SIZE..end]
            }
        }
    }

    fn commit(mut self) {
        fn ring_doorbell(shared: &TxShared) {
            if matches!(
                shared.doorbell.signal_now(),
                shm_primitives::SignalResult::PeerDead
            ) {
                shared.doorbell_dead.store(true, Ordering::Release);
                shared
                    .stats
                    .doorbell_peer_dead
                    .fetch_add(1, AtomicOrdering::Relaxed);
            }
        }

        match &mut self.inner {
            ShmWriteSlotInner::Inline { bytes } => loop {
                let lock = self.shared.tx_lock.lock().expect("tx lock poisoned");
                let (mut producer, _) = self.shared.tx_bipbuf.split();
                let result = framing::write_inline(&mut producer, bytes);
                drop(lock);
                match result {
                    Ok(()) => {
                        self.shared
                            .stats
                            .inline_sends
                            .fetch_add(1, AtomicOrdering::Relaxed);
                        ring_doorbell(&self.shared);
                        return;
                    }
                    Err(_) => {
                        self.shared
                            .stats
                            .commit_retries
                            .fetch_add(1, AtomicOrdering::Relaxed);
                        std::thread::yield_now();
                    }
                }
            },
            ShmWriteSlotInner::VarSlot {
                slot_ref,
                payload_len,
            } => {
                let Some(slot_ref_value) = *slot_ref else {
                    return;
                };

                {
                    let payload_len_bytes = (*payload_len as u32).to_le_bytes();
                    let data = unsafe { self.shared.backend.slot_data_mut(&slot_ref_value) };
                    data[..SLOT_LEN_PREFIX_SIZE].copy_from_slice(&payload_len_bytes);
                }

                loop {
                    let lock = self.shared.tx_lock.lock().expect("tx lock poisoned");
                    let (mut producer, _) = self.shared.tx_bipbuf.split();
                    let result = framing::write_slot_ref(&mut producer, &slot_ref_value);
                    drop(lock);
                    match result {
                        Ok(()) => {
                            *slot_ref = None;
                            self.shared
                                .stats
                                .slot_ref_sends
                                .fetch_add(1, AtomicOrdering::Relaxed);
                            ring_doorbell(&self.shared);
                            return;
                        }
                        Err(_) => {
                            self.shared
                                .stats
                                .commit_retries
                                .fetch_add(1, AtomicOrdering::Relaxed);
                            std::thread::yield_now();
                        }
                    }
                }
            }
        }
    }
}

#[derive(Debug)]
pub enum ShmLinkRxError {
    UnsupportedMmapRef,
    DoorbellWait(io::Error),
    MalformedSlotRefLength {
        slot_bytes: usize,
        payload_len: usize,
    },
}

impl std::fmt::Display for ShmLinkRxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ShmLinkRxError::UnsupportedMmapRef => write!(f, "mmap-ref frames are not implemented"),
            ShmLinkRxError::DoorbellWait(err) => write!(f, "doorbell wait failed: {err}"),
            ShmLinkRxError::MalformedSlotRefLength {
                slot_bytes,
                payload_len,
            } => write!(
                f,
                "malformed slot-ref payload length: payload_len={payload_len}, slot_bytes={slot_bytes}"
            ),
        }
    }
}

impl std::error::Error for ShmLinkRxError {}

impl LinkRx for ShmLinkRx {
    type Error = ShmLinkRxError;

    async fn recv(&mut self) -> Result<Option<Backing>, Self::Error> {
        loop {
            let (_, mut consumer) = self.rx_bipbuf.split();
            if let Some(frame) = framing::read_frame(&mut consumer) {
                return match frame {
                    OwnedFrame::Inline(bytes) => {
                        self.stats
                            .inline_recvs
                            .fetch_add(1, AtomicOrdering::Relaxed);
                        if matches!(
                            self.doorbell.signal_now(),
                            shm_primitives::SignalResult::PeerDead
                        ) {
                            self.peer_closed.store(true, Ordering::Release);
                            self.stats
                                .doorbell_peer_dead
                                .fetch_add(1, AtomicOrdering::Relaxed);
                        }
                        Ok(Some(Backing::Boxed(bytes.into_boxed_slice())))
                    }
                    OwnedFrame::SlotRef(slot_ref) => {
                        self.stats
                            .slot_ref_recvs
                            .fetch_add(1, AtomicOrdering::Relaxed);
                        if matches!(
                            self.doorbell.signal_now(),
                            shm_primitives::SignalResult::PeerDead
                        ) {
                            self.peer_closed.store(true, Ordering::Release);
                            self.stats
                                .doorbell_peer_dead
                                .fetch_add(1, AtomicOrdering::Relaxed);
                        }
                        let slot = unsafe { self.backend.slot_data(&slot_ref) };
                        if slot.len() < SLOT_LEN_PREFIX_SIZE {
                            self.backend.free_slot(slot_ref);
                            return Err(ShmLinkRxError::MalformedSlotRefLength {
                                slot_bytes: slot.len(),
                                payload_len: 0,
                            });
                        }

                        let payload_len =
                            u32::from_le_bytes([slot[0], slot[1], slot[2], slot[3]]) as usize;
                        if payload_len > slot.len().saturating_sub(SLOT_LEN_PREFIX_SIZE) {
                            self.backend.free_slot(slot_ref);
                            return Err(ShmLinkRxError::MalformedSlotRefLength {
                                slot_bytes: slot.len(),
                                payload_len,
                            });
                        }

                        Ok(Some(Backing::shared(Arc::new(ShmVarSlotBacking {
                            backend: self.backend.clone(),
                            slot_ref,
                            payload_len,
                        }))))
                    }
                    OwnedFrame::MmapRef(_) => Err(ShmLinkRxError::UnsupportedMmapRef),
                };
            }

            if self.peer_closed.load(Ordering::Acquire) && self.rx_bipbuf.inner().is_empty() {
                return Ok(None);
            }

            if let Err(err) = self.doorbell.wait().await {
                self.stats
                    .doorbell_wait_errors
                    .fetch_add(1, AtomicOrdering::Relaxed);
                return Err(ShmLinkRxError::DoorbellWait(err));
            }
        }
    }
}

impl ShmLinkTx {
    pub fn stats(&self) -> ShmTransportStatsSnapshot {
        self.shared.stats.snapshot()
    }
}

impl ShmLinkRx {
    pub fn stats(&self) -> ShmTransportStatsSnapshot {
        self.stats.snapshot()
    }
}

struct ShmVarSlotBacking {
    backend: Backend,
    slot_ref: SlotRef,
    payload_len: usize,
}

impl SharedBacking for ShmVarSlotBacking {
    fn as_bytes(&self) -> &[u8] {
        let slot = unsafe { self.backend.slot_data(&self.slot_ref) };
        let end = SLOT_LEN_PREFIX_SIZE + self.payload_len;
        &slot[SLOT_LEN_PREFIX_SIZE..end]
    }
}

impl Drop for ShmVarSlotBacking {
    fn drop(&mut self) {
        self.backend.free_slot(self.slot_ref);
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use roam_types::{LinkRx as _, LinkTx as _, LinkTxPermit as _};
    use tokio::time::timeout;

    use super::*;

    const CLASSES: &[SizeClassConfig] = &[SizeClassConfig {
        slot_size: 256,
        slot_count: 1,
    }];

    #[tokio::test]
    async fn inline_payload_roundtrip_is_boxed() {
        let (a, b) = ShmLink::heap_pair(4096, 1024, 128, CLASSES).unwrap();
        let (a_tx, _a_rx) = a.split();
        let (_b_tx, mut b_rx) = b.split();

        let payload = b"inline hello";
        let permit = a_tx.reserve().await.unwrap();
        let mut slot = permit.alloc(payload.len()).unwrap();
        slot.as_mut_slice().copy_from_slice(payload);
        slot.commit();

        let backing = b_rx.recv().await.unwrap().unwrap();
        match backing {
            Backing::Boxed(bytes) => assert_eq!(&*bytes, payload),
            Backing::Shared(_) => panic!("inline path must be boxed"),
        }
    }

    #[tokio::test]
    async fn slot_ref_payload_is_zero_copy_shared_backing() {
        let (a, b) = ShmLink::heap_pair(4096, 1024, 64, CLASSES).unwrap();
        let (a_tx, _a_rx) = a.split();
        let (_b_tx, mut b_rx) = b.split();

        let payload = vec![7_u8; 200];
        let permit = a_tx.reserve().await.unwrap();
        let mut slot = permit.alloc(payload.len()).unwrap();
        slot.as_mut_slice().copy_from_slice(&payload);
        slot.commit();

        let backing = b_rx.recv().await.unwrap().unwrap();
        match backing {
            Backing::Shared(shared) => assert_eq!(shared.as_bytes(), payload.as_slice()),
            Backing::Boxed(_) => panic!("slot-ref path must be shared"),
        }
    }

    #[tokio::test]
    async fn shared_backing_drop_releases_slot() {
        let (a, b) = ShmLink::heap_pair(4096, 1024, 64, CLASSES).unwrap();
        let (a_tx, _a_rx) = a.split();
        let (_b_tx, mut b_rx) = b.split();

        let payload = vec![1_u8; 200];
        let permit = a_tx.reserve().await.unwrap();
        let mut slot = permit.alloc(payload.len()).unwrap();
        slot.as_mut_slice().copy_from_slice(&payload);
        slot.commit();

        let backing = b_rx.recv().await.unwrap().unwrap();

        // single-slot pool: second allocation fails until backing is dropped
        let permit2 = a_tx.reserve().await.unwrap();
        let err = match permit2.alloc(payload.len()) {
            Ok(_) => panic!("expected slot allocation to fail while shared backing is alive"),
            Err(err) => err,
        };
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);

        drop(backing);

        let permit3 = a_tx.reserve().await.unwrap();
        let _slot3 = permit3
            .alloc(payload.len())
            .expect("slot must be released after drop");
    }

    #[tokio::test]
    async fn mixed_payload_stress_roundtrip() {
        let classes = [SizeClassConfig {
            slot_size: 4096,
            slot_count: 32,
        }];
        let (a, b) = ShmLink::heap_pair(1 << 16, 1 << 20, 256, &classes).unwrap();
        let (a_tx, _a_rx) = a.split();
        let (_b_tx, mut b_rx) = b.split();

        for i in 0..400 {
            let len = if i % 3 == 0 { 48 } else { 1500 };
            let payload = vec![(i % 239) as u8; len];
            let permit = a_tx.reserve().await.unwrap();
            let mut slot = permit.alloc(payload.len()).unwrap();
            slot.as_mut_slice().copy_from_slice(&payload);
            slot.commit();

            let backing = b_rx.recv().await.unwrap().unwrap();
            assert_eq!(backing.as_bytes(), payload.as_slice());
        }
    }

    #[tokio::test]
    async fn reserve_waits_until_rx_frees_ring_space() {
        let (a, b) = ShmLink::heap_pair(32, 1024, 256, CLASSES).unwrap();
        let (a_tx, _a_rx) = a.split();
        let (_b_tx, mut b_rx) = b.split();

        let payload = vec![9_u8; 24]; // align4(8 + 24) = 32 (fills ring)
        let permit = a_tx.reserve().await.unwrap();
        let mut slot = permit.alloc(payload.len()).unwrap();
        slot.as_mut_slice().copy_from_slice(&payload);
        slot.commit();

        let reserve_fut = a_tx.reserve();
        tokio::pin!(reserve_fut);
        assert!(
            timeout(Duration::from_millis(20), &mut reserve_fut)
                .await
                .is_err(),
            "reserve should wait while ring is full"
        );

        let _ = b_rx.recv().await.unwrap().unwrap();
        let permit2 = timeout(Duration::from_secs(1), &mut reserve_fut)
            .await
            .expect("reserve should wake after recv")
            .expect("reserve should succeed");
        drop(permit2);
    }

    #[tokio::test]
    async fn transport_stats_track_send_recv_and_exhaustion() {
        let (a, b) = ShmLink::heap_pair(4096, 1024, 64, CLASSES).unwrap();
        let (a_tx, _a_rx) = a.split();
        let (_b_tx, mut b_rx) = b.split();

        let inline = b"hello";
        let permit = a_tx.reserve().await.unwrap();
        let mut slot = permit.alloc(inline.len()).unwrap();
        slot.as_mut_slice().copy_from_slice(inline);
        slot.commit();

        let large = vec![1_u8; 200];
        let permit = a_tx.reserve().await.unwrap();
        let mut slot = permit.alloc(large.len()).unwrap();
        slot.as_mut_slice().copy_from_slice(&large);
        slot.commit();

        let backing1 = b_rx.recv().await.unwrap().unwrap();
        assert!(backing1.as_bytes().starts_with(inline));
        let backing2 = b_rx.recv().await.unwrap().unwrap();
        assert_eq!(backing2.as_bytes(), large.as_slice());

        let permit = a_tx.reserve().await.unwrap();
        let err = match permit.alloc(large.len()) {
            Ok(_) => panic!("expected varslot exhaustion"),
            Err(err) => err,
        };
        assert_eq!(err.kind(), io::ErrorKind::WouldBlock);
        drop(backing2); // free slot for future sends

        let tx_stats = a_tx.stats();
        let rx_stats = b_rx.stats();

        assert_eq!(tx_stats.inline_sends, 1);
        assert_eq!(tx_stats.slot_ref_sends, 1);
        assert_eq!(tx_stats.varslot_exhausted, 1);
        assert_eq!(rx_stats.inline_recvs, 1);
        assert_eq!(rx_stats.slot_ref_recvs, 1);
    }
}
