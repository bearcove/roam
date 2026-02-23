//! SHM framing — the 8-byte header that wraps each BipBuffer entry.
//!
//! Every entry written to a BipBuffer is prefixed with a [`FrameHeader`].
//! The payload is either:
//!
//! - **Inline**: payload bytes follow the header directly. The entry is
//!   `align4(8 + payload_len)` bytes.
//! - **Slot-ref**: a [`SlotRefBody`] follows the header instead. The entry
//!   is exactly 20 bytes.
//!
//! All values are native-endian (little-endian on all supported platforms).

use shm_primitives::BipBufFull;
use shm_primitives::bipbuf::{BipBufConsumer, BipBufProducer};

use crate::varslot::SlotRef;

// ── constants ─────────────────────────────────────────────────────────────────

/// Bit 0 of `FrameHeader::flags`: payload is in VarSlotPool, not inline.
pub const FLAG_SLOT_REF: u8 = 0x01;

/// Size of [`FrameHeader`] in bytes.
pub const FRAME_HEADER_SIZE: usize = 8;

/// Size of [`SlotRefBody`] in bytes.
pub const SLOT_REF_BODY_SIZE: usize = 12;

/// Size of a slot-ref entry (header + body), always exactly 20 bytes.
pub const SLOT_REF_ENTRY_SIZE: u32 = 20;

/// Default inline threshold when the segment header field is 0.
pub const DEFAULT_INLINE_THRESHOLD: u32 = 256;

// ── wire types ────────────────────────────────────────────────────────────────

/// The 8-byte header that begins every BipBuffer entry.
///
/// r[impl shm.framing]
/// r[impl shm.framing.header]
/// r[impl shm.framing.alignment]
#[repr(C)]
pub struct FrameHeader {
    /// Total entry size in bytes, padded to a 4-byte boundary.
    /// Includes the 8-byte header itself.
    pub total_len: u32,
    /// Bit 0: `FLAG_SLOT_REF`. All other bits reserved and zero.
    pub flags: u8,
    pub _reserved: [u8; 3],
}

/// The 12-byte slot reference body that follows a `FrameHeader` when
/// `FLAG_SLOT_REF` is set.
///
/// r[impl shm.framing.slot-ref]
#[repr(C)]
pub struct SlotRefBody {
    pub class_idx: u8,
    pub extent_idx: u8,
    pub _reserved: [u8; 2],
    pub slot_idx: u32,
    pub generation: u32,
}

#[cfg(not(loom))]
const _: () = assert!(core::mem::size_of::<FrameHeader>() == FRAME_HEADER_SIZE);
#[cfg(not(loom))]
const _: () = assert!(core::mem::size_of::<SlotRefBody>() == SLOT_REF_BODY_SIZE);

// ── align helper ──────────────────────────────────────────────────────────────

#[inline]
const fn align4(n: u32) -> u32 {
    (n + 3) & !3
}

// ── writer ────────────────────────────────────────────────────────────────────

/// Write an inline frame to the producer.
///
/// Grants `align4(8 + payload.len())` bytes, writes the header and payload,
/// zeroes any padding bytes, then commits.
///
/// r[impl shm.framing.inline]
/// r[impl shm.framing.threshold]
pub fn write_inline(producer: &mut BipBufProducer<'_>, payload: &[u8]) -> Result<(), BipBufFull> {
    let entry_len = align4(8 + payload.len() as u32);
    let buf = producer.try_grant(entry_len).ok_or(BipBufFull)?;

    // Safety: buf is at least 4-byte aligned and large enough for FrameHeader.
    let hdr = unsafe { &mut *(buf.as_mut_ptr() as *mut FrameHeader) };
    hdr.total_len = entry_len;
    hdr.flags = 0;
    hdr._reserved = [0; 3];

    let payload_start = FRAME_HEADER_SIZE;
    let payload_end = payload_start + payload.len();
    buf[payload_start..payload_end].copy_from_slice(payload);
    buf[payload_end..entry_len as usize].fill(0);

    producer.commit(entry_len);
    Ok(())
}

/// Write a slot-ref frame to the producer.
///
/// Always 20 bytes: 8-byte header + 12-byte [`SlotRefBody`].
///
/// r[impl shm.framing.slot-ref]
pub fn write_slot_ref(producer: &mut BipBufProducer<'_>, slot: &SlotRef) -> Result<(), BipBufFull> {
    let buf = producer.try_grant(SLOT_REF_ENTRY_SIZE).ok_or(BipBufFull)?;

    let hdr = unsafe { &mut *(buf.as_mut_ptr() as *mut FrameHeader) };
    hdr.total_len = SLOT_REF_ENTRY_SIZE;
    hdr.flags = FLAG_SLOT_REF;
    hdr._reserved = [0; 3];

    // Safety: buf[8..] is 4-byte aligned (8 % 4 == 0) and large enough.
    let body = unsafe { &mut *(buf[FRAME_HEADER_SIZE..].as_mut_ptr() as *mut SlotRefBody) };
    body.class_idx = slot.class_idx;
    body.extent_idx = slot.extent_idx;
    body._reserved = [0; 2];
    body.slot_idx = slot.slot_idx;
    body.generation = slot.generation;

    producer.commit(SLOT_REF_ENTRY_SIZE);
    Ok(())
}

// ── reader ────────────────────────────────────────────────────────────────────

/// A parsed frame from a BipBuffer entry.
pub enum Frame<'a> {
    /// Inline payload. The slice is `data[8..total_len]` and may include up
    /// to 3 trailing zero-padding bytes. The roam wire format is
    /// self-delimiting, so trailing zeros are harmless.
    Inline(&'a [u8]),
    /// The payload lives in the VarSlotPool at the given slot reference.
    SlotRef(SlotRef),
}

/// Parse the next frame from the front of `data`.
///
/// Returns `(frame, bytes_to_release)` on success, or `None` if `data` is
/// too short to contain a complete frame.  The caller must call
/// `consumer.release(bytes_to_release)` after processing the frame.
///
/// r[impl shm.framing.header]
pub fn peek_frame(data: &[u8]) -> Option<(Frame<'_>, u32)> {
    if data.len() < FRAME_HEADER_SIZE {
        return None;
    }

    // Safety: data is valid memory; FrameHeader is repr(C) with no padding.
    let hdr = unsafe { &*(data.as_ptr() as *const FrameHeader) };
    let total_len = hdr.total_len as usize;

    if data.len() < total_len {
        return None;
    }

    let frame = if hdr.flags & FLAG_SLOT_REF == 0 {
        Frame::Inline(&data[FRAME_HEADER_SIZE..total_len])
    } else {
        if total_len < FRAME_HEADER_SIZE + SLOT_REF_BODY_SIZE {
            return None;
        }
        let body = unsafe { &*(data[FRAME_HEADER_SIZE..].as_ptr() as *const SlotRefBody) };
        Frame::SlotRef(SlotRef {
            class_idx: body.class_idx,
            extent_idx: body.extent_idx,
            slot_idx: body.slot_idx,
            generation: body.generation,
        })
    };

    Some((frame, hdr.total_len))
}

/// Convenience wrapper: read the next frame from `consumer`.
///
/// Calls `consumer.try_read()`, parses the frame header, then calls
/// `consumer.release()`.  Returns `None` if the ring is empty or the data
/// is too short (shouldn't happen with a well-behaved writer).
pub fn read_frame(consumer: &mut BipBufConsumer<'_>) -> Option<OwnedFrame> {
    let data = consumer.try_read()?;
    let (frame, consumed) = peek_frame(data)?;
    let owned = match frame {
        Frame::Inline(payload) => OwnedFrame::Inline(payload.to_vec()),
        Frame::SlotRef(slot) => OwnedFrame::SlotRef(slot),
    };
    consumer.release(consumed);
    Some(owned)
}

/// An owned version of [`Frame`] returned by [`read_frame`].
#[derive(Debug)]
pub enum OwnedFrame {
    Inline(Vec<u8>),
    SlotRef(SlotRef),
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use shm_primitives::{BIPBUF_HEADER_SIZE, BipBuf, HeapRegion};

    fn make_bipbuf(capacity: usize) -> (HeapRegion, BipBuf) {
        let total = BIPBUF_HEADER_SIZE + capacity;
        let region = HeapRegion::new_zeroed(total);
        let bip = unsafe { BipBuf::init(region.region(), 0, capacity as u32) };
        (region, bip)
    }

    #[test]
    fn inline_roundtrip() {
        let (_region, bip) = make_bipbuf(1024);
        let (mut tx, mut rx) = bip.split();

        let payload = b"hello, world";
        write_inline(&mut tx, payload).unwrap();

        let frame = read_frame(&mut rx).unwrap();
        match frame {
            OwnedFrame::Inline(data) => {
                assert!(data.starts_with(payload));
            }
            _ => panic!("expected inline frame"),
        }
    }

    #[test]
    fn inline_alignment() {
        let (_region, bip) = make_bipbuf(1024);
        let (mut tx, mut rx) = bip.split();

        // Payload of 5 bytes → total_len should be align4(8+5) = 16
        write_inline(&mut tx, b"hello").unwrap();
        let (_, consumed) = peek_frame(rx.try_read().unwrap()).unwrap();
        assert_eq!(consumed, 16);
    }

    #[test]
    fn slot_ref_roundtrip() {
        let (_region, bip) = make_bipbuf(1024);
        let (mut tx, mut rx) = bip.split();

        let slot = SlotRef {
            class_idx: 2,
            extent_idx: 0,
            slot_idx: 7,
            generation: 42,
        };
        write_slot_ref(&mut tx, &slot).unwrap();

        let frame = read_frame(&mut rx).unwrap();
        match frame {
            OwnedFrame::SlotRef(s) => {
                assert_eq!(s.class_idx, 2);
                assert_eq!(s.slot_idx, 7);
                assert_eq!(s.generation, 42);
            }
            _ => panic!("expected slot-ref frame"),
        }
    }

    #[test]
    fn slot_ref_entry_size() {
        let (_region, bip) = make_bipbuf(1024);
        let (mut tx, mut rx) = bip.split();

        let slot = SlotRef {
            class_idx: 0,
            extent_idx: 0,
            slot_idx: 0,
            generation: 0,
        };
        write_slot_ref(&mut tx, &slot).unwrap();
        let (_, consumed) = peek_frame(rx.try_read().unwrap()).unwrap();
        assert_eq!(consumed, 20);
    }

    #[test]
    fn multiple_frames_sequential() {
        let (_region, bip) = make_bipbuf(1024);
        let (mut tx, mut rx) = bip.split();

        write_inline(&mut tx, b"first").unwrap();
        write_inline(&mut tx, b"second frame").unwrap();

        match read_frame(&mut rx).unwrap() {
            OwnedFrame::Inline(d) => assert!(d.starts_with(b"first")),
            _ => panic!(),
        }
        match read_frame(&mut rx).unwrap() {
            OwnedFrame::Inline(d) => assert!(d.starts_with(b"second frame")),
            _ => panic!(),
        }
        assert!(read_frame(&mut rx).is_none());
    }

    #[test]
    fn empty_payload() {
        let (_region, bip) = make_bipbuf(1024);
        let (mut tx, mut rx) = bip.split();

        write_inline(&mut tx, b"").unwrap();
        let (_, consumed) = peek_frame(rx.try_read().unwrap()).unwrap();
        // align4(8 + 0) = 8
        assert_eq!(consumed, 8);
    }

    #[test]
    fn ring_full_returns_err() {
        // Capacity just big enough for the header region but not a frame
        let (_region, bip) = make_bipbuf(8);
        let (mut tx, _rx) = bip.split();

        // First write succeeds (8 bytes fits exactly)
        write_inline(&mut tx, b"").unwrap();
        // Ring is now full
        assert!(write_inline(&mut tx, b"").is_err());
    }
}
