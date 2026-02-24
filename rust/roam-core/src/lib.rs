//! Core implementations for the roam connectivity layer.
//!
//! This crate provides concrete implementations of the traits defined in
//! [`roam_types`]:
//!
//! - [`BareConduit`]: wraps a raw `Link` with postcard serialization.
//!   No reconnect, no reliability. For localhost, SHM, testing.
//! - `StableConduit` (TODO): wraps a Link + seq/ack/replay with
//!   bytes-based replay buffer. Handles reconnect transparently.

mod bare_conduit;
pub use bare_conduit::*;

mod replay_buffer;

mod stable_conduit;
pub use stable_conduit::*;

mod memory_link;
pub use memory_link::*;

mod session;
pub use session::*;

/// Return a process-global cached `&'static RpcPlan` for type `T`.
/// [FIXME] requiring 'static here is wrong
/// [FIXME] this function is now useless since we have RpcPlan::for_type
pub fn rpc_plan<T: facet::Facet<'static>>() -> &'static roam_types::RpcPlan {
    roam_types::RpcPlan::for_type::<T>()
}

/// Deserialize postcard-encoded `backing` bytes into `T` in place, returning a
/// [`roam_types::SelfRef`] that keeps the backing storage alive for the value.
///
/// # Safety contract
///
/// The caller must ensure `shape` is exactly `T::SHAPE` (or the lifetime-erased
/// equivalent for types where the shape is lifetime-independent). Deserializes
/// directly into a stack-allocated `MaybeUninit<T>` via
/// `Partial::from_raw_with_shape`, then calls `finish_in_place` before
/// `assume_init` — `T` is fully initialized iff deserialization succeeds.
pub(crate) fn deserialize_postcard<T: 'static>(
    backing: roam_types::Backing,
    shape: &'static facet_core::Shape,
) -> Result<roam_types::SelfRef<T>, facet_format::DeserializeError> {
    use facet_format::{FormatDeserializer, MetaSource};
    use facet_postcard::PostcardParser;
    use facet_reflect::Partial;

    // SAFETY: backing is heap-allocated with a stable address.
    // The SelfRef::try_new contract guarantees value is dropped before backing.
    roam_types::SelfRef::try_new(backing, |bytes| {
        let mut value = std::mem::MaybeUninit::<T>::uninit();
        let ptr = facet_core::PtrUninit::from_maybe_uninit(&mut value);

        // SAFETY: ptr points to valid, aligned, properly-sized memory for T;
        // shape is T's shape.
        #[allow(unsafe_code)]
        let partial: Partial<'_, false> = unsafe { Partial::from_raw_with_shape(ptr, shape) }
            .map_err(facet_format::DeserializeError::from)?;

        let mut parser = PostcardParser::new(bytes);
        let mut deserializer = FormatDeserializer::new_owned(&mut parser);
        let partial = deserializer.deserialize_into(partial, MetaSource::FromEvents)?;

        partial
            .finish_in_place()
            .map_err(facet_format::DeserializeError::from)?;

        // SAFETY: finish_in_place succeeded, so value is fully initialized.
        #[allow(unsafe_code)]
        Ok(unsafe { value.assume_init() })
    })
}

#[cfg(test)]
mod tests;
