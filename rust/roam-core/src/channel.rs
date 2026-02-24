use std::convert::Infallible;
use std::marker::PhantomData;

use facet::Facet;
use roam_types::{ChannelId, ConnectionId};

/// Create an unbound channel pair.
///
/// Both ends start hollow (`bound = false`) and are bound to session machinery
/// when request/response channel IDs are applied.
pub fn channel<T: 'static, const N: usize>() -> (Tx<T, N>, Rx<T, N>) {
    (Tx::unbound(), Rx::unbound())
}

/// Caller-side sender handle (`caller -> callee`).
///
/// Wire encoding is always unit (`()`), and channel IDs are carried only by
/// `Message::{Request,Response}.channels`.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
#[facet(proxy = ())]
pub struct Tx<T: 'static, const N: usize = 16> {
    pub(crate) conn_id: ConnectionId,
    pub(crate) channel_id: ChannelId,
    pub(crate) bound: bool,
    #[facet(opaque)]
    _marker: PhantomData<T>,
}

impl<T: 'static, const N: usize> Tx<T, N> {
    pub fn unbound() -> Self {
        Self {
            conn_id: ConnectionId::ROOT,
            channel_id: ChannelId::RESERVED,
            bound: false,
            _marker: PhantomData,
        }
    }

    pub fn is_bound(&self) -> bool {
        self.bound
    }

    pub fn connection_id(&self) -> Option<ConnectionId> {
        self.bound.then_some(self.conn_id)
    }

    pub fn channel_id(&self) -> Option<ChannelId> {
        self.bound.then_some(self.channel_id)
    }

    pub(crate) fn bind(&mut self, conn_id: ConnectionId, channel_id: ChannelId) {
        self.conn_id = conn_id;
        self.channel_id = channel_id;
        self.bound = true;
    }
}

#[allow(clippy::infallible_try_from)]
impl<T: 'static, const N: usize> TryFrom<&Tx<T, N>> for () {
    type Error = Infallible;

    fn try_from(_value: &Tx<T, N>) -> Result<Self, Self::Error> {
        Ok(())
    }
}

#[allow(clippy::infallible_try_from)]
impl<T: 'static, const N: usize> TryFrom<()> for Tx<T, N> {
    type Error = Infallible;

    fn try_from(_value: ()) -> Result<Self, Self::Error> {
        Ok(Self::unbound())
    }
}

/// Caller-side receiver handle (`callee -> caller`).
///
/// Wire encoding is always unit (`()`), and channel IDs are carried only by
/// `Message::{Request,Response}.channels`.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
#[facet(proxy = ())]
pub struct Rx<T: 'static, const N: usize = 16> {
    pub(crate) conn_id: ConnectionId,
    pub(crate) channel_id: ChannelId,
    pub(crate) bound: bool,
    #[facet(opaque)]
    _marker: PhantomData<T>,
}

impl<T: 'static, const N: usize> Rx<T, N> {
    pub fn unbound() -> Self {
        Self {
            conn_id: ConnectionId::ROOT,
            channel_id: ChannelId::RESERVED,
            bound: false,
            _marker: PhantomData,
        }
    }

    pub fn is_bound(&self) -> bool {
        self.bound
    }

    pub fn connection_id(&self) -> Option<ConnectionId> {
        self.bound.then_some(self.conn_id)
    }

    pub fn channel_id(&self) -> Option<ChannelId> {
        self.bound.then_some(self.channel_id)
    }

    pub(crate) fn bind(&mut self, conn_id: ConnectionId, channel_id: ChannelId) {
        self.conn_id = conn_id;
        self.channel_id = channel_id;
        self.bound = true;
    }
}

#[allow(clippy::infallible_try_from)]
impl<T: 'static, const N: usize> TryFrom<&Rx<T, N>> for () {
    type Error = Infallible;

    fn try_from(_value: &Rx<T, N>) -> Result<Self, Self::Error> {
        Ok(())
    }
}

#[allow(clippy::infallible_try_from)]
impl<T: 'static, const N: usize> TryFrom<()> for Rx<T, N> {
    type Error = Infallible;

    fn try_from(_value: ()) -> Result<Self, Self::Error> {
        Ok(Self::unbound())
    }
}

/// Check if a shape represents a `Tx` (caller -> callee) channel.
pub fn is_tx(shape: &facet_core::Shape) -> bool {
    shape.decl_id == Tx::<()>::SHAPE.decl_id
}

/// Check if a shape represents an `Rx` (callee -> caller) channel.
pub fn is_rx(shape: &facet_core::Shape) -> bool {
    shape.decl_id == Rx::<()>::SHAPE.decl_id
}

/// Check if a shape represents any channel type (`Tx` or `Rx`).
pub fn is_channel(shape: &facet_core::Shape) -> bool {
    is_tx(shape) || is_rx(shape)
}
