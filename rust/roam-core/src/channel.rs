use facet::Facet;

/// Placeholder for a caller→callee stream channel.
///
/// `Tx<T>` represents the sending end of a channel from the caller's perspective.
/// Full implementation pending the Requests/Channels spec section.
#[derive(Facet)]
pub struct Tx<T: 'static, const N: usize = 16> {
    _marker: std::marker::PhantomData<T>,
}

/// Placeholder for a callee→caller stream channel.
///
/// `Rx<T, N>` represents the receiving end of a channel from the caller's perspective.
/// `N` is the initial credit: the number of items the sender may send before
/// needing explicit credit grants. Full implementation pending the Requests/Channels
/// spec section.
#[derive(Facet)]
pub struct Rx<T: 'static, const N: usize = 16> {
    _marker: std::marker::PhantomData<T>,
}

/// Check if a shape represents a `Tx` (caller→callee) channel.
pub fn is_tx(shape: &facet_core::Shape) -> bool {
    shape.decl_id == Tx::<()>::SHAPE.decl_id
}

/// Check if a shape represents an `Rx` (callee→caller) channel.
pub fn is_rx(shape: &facet_core::Shape) -> bool {
    shape.decl_id == Rx::<()>::SHAPE.decl_id
}

/// Check if a shape represents any channel type (`Tx` or `Rx`).
pub fn is_channel(shape: &facet_core::Shape) -> bool {
    is_tx(shape) || is_rx(shape)
}
