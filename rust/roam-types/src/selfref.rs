#![allow(unsafe_code)]

use std::mem::ManuallyDrop;

/// A decoded value `T` that may borrow from its own backing storage.
///
/// Transports decode into storage they own (heap buffer, VarSlot, mmap).
/// `SelfRef` keeps that storage alive so `T` can safely borrow from it.
///
/// Uses `ManuallyDrop` + custom `Drop` to guarantee drop order: value is
/// dropped before backing, so borrowed references in `T` remain valid
/// through `T`'s drop.
pub struct SelfRef<T: 'static> {
    /// The decoded value, potentially borrowing from `backing`.
    value: ManuallyDrop<T>,

    /// Backing storage keeping decoded bytes alive.
    backing: ManuallyDrop<Backing>,
}

/// Backing storage for a [`SelfRef`].
pub enum Backing {
    /// Heap-allocated buffer (TCP read, BipBuffer copy-out for small messages).
    Boxed(Box<[u8]>),
    // SHM VarSlot, pinned in shared memory:
    // VarSlot(Arc<VarSlot>),
    // Memory-mapped file region:
    // Mmap(Arc<MmapRegion>),
}

impl Backing {
    /// Access the backing bytes.
    pub fn as_bytes(&self) -> &[u8] {
        match self {
            Backing::Boxed(b) => b,
        }
    }
}

impl<T: 'static> Drop for SelfRef<T> {
    fn drop(&mut self) {
        // Drop value first (it may borrow from backing), then backing.
        unsafe {
            ManuallyDrop::drop(&mut self.value);
            ManuallyDrop::drop(&mut self.backing);
        }
    }
}

impl<T: 'static> SelfRef<T> {
    /// Construct a `SelfRef` from backing storage and a builder.
    ///
    /// The builder receives a `&'static [u8]` view of the backing bytes —
    /// sound because the backing is heap-allocated (stable address) and
    /// dropped after the value.
    pub fn try_new<E>(
        backing: Backing,
        builder: impl FnOnce(&'static [u8]) -> Result<T, E>,
    ) -> Result<Self, E> {
        // Create a 'static slice from the backing bytes.
        // Sound because:
        // - Backing is heap-allocated (stable address)
        // - We drop value before backing (custom Drop impl)
        let bytes: &'static [u8] = unsafe {
            let b = backing.as_bytes();
            std::slice::from_raw_parts(b.as_ptr(), b.len())
        };

        let value = builder(bytes)?;

        Ok(Self {
            value: ManuallyDrop::new(value),
            backing: ManuallyDrop::new(backing),
        })
    }

    /// Infallible variant of [`try_new`](Self::try_new).
    pub fn new(backing: Backing, builder: impl FnOnce(&'static [u8]) -> T) -> Self {
        Self::try_new(backing, |bytes| {
            Ok::<_, std::convert::Infallible>(builder(bytes))
        })
        .unwrap_or_else(|e: std::convert::Infallible| match e {})
    }
    /// Wrap an owned value that does NOT borrow from backing.
    ///
    /// No variance check — the value is fully owned. The backing is kept
    /// alive but the value doesn't reference it. Useful for in-memory
    /// transports (MemoryLink) where no deserialization occurs.
    pub fn owning(backing: Backing, value: T) -> Self {
        Self {
            value: ManuallyDrop::new(value),
            backing: ManuallyDrop::new(backing),
        }
    }

    /// Transform the contained value, keeping the same backing storage.
    ///
    /// Useful for projecting through wrapper types:
    /// `SelfRef<Frame<T>>` → `SelfRef<T>` by extracting the inner item.
    ///
    /// The closure receives the old value by move and returns the new value.
    /// Any references the new value holds into the backing storage (inherited
    /// from fields of `T`) remain valid — the backing is preserved.
    pub fn map<U: 'static>(mut self, f: impl FnOnce(T) -> U) -> SelfRef<U> {
        // SAFETY: we take both fields via ManuallyDrop::take, then forget
        // self to prevent its Drop impl from double-dropping them.
        let value = unsafe { ManuallyDrop::take(&mut self.value) };
        let backing = unsafe { ManuallyDrop::take(&mut self.backing) };
        core::mem::forget(self);

        SelfRef {
            value: ManuallyDrop::new(f(value)),
            backing: ManuallyDrop::new(backing),
        }
    }
}

impl<T: 'static> core::ops::Deref for SelfRef<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.value
    }
}

// No `into_inner()` — T may borrow from backing. Use Deref instead.
// No `DerefMut` — mutating T could invalidate borrowed references.
