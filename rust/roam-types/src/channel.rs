use std::convert::Infallible;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

use facet::Facet;
use facet_core::PtrConst;
#[cfg(not(target_arch = "wasm32"))]
use tokio::sync::{Semaphore, mpsc};

use crate::{ChannelClose, ChannelItem, ChannelReset, Metadata, Payload, SelfRef};

/// Create an unbound channel pair.
///
/// Both ends start hollow and must be hydrated by the session driver using
/// channel IDs from `Message::{Request,Response}.channels`.
pub fn channel<T>() -> (Tx<T>, Rx<T>) {
    (Tx::unbound(), Rx::unbound())
}

/// Runtime sink implemented by the session driver.
///
/// The contract is strict: successful completion means the item has gone
/// through the conduit to the link commit boundary.
#[cfg(not(target_arch = "wasm32"))]
pub trait ChannelSink: Send + Sync + 'static {
    fn send_payload<'payload>(
        &self,
        payload: Payload<'payload>,
    ) -> Pin<Box<dyn Future<Output = Result<(), TxError>> + Send + 'payload>>;

    fn close_channel(
        &self,
        metadata: Metadata,
    ) -> Pin<Box<dyn Future<Output = Result<(), TxError>> + Send + 'static>>;
}

// r[impl rpc.flow-control.credit]
// r[impl rpc.flow-control.credit.exhaustion]
/// A [`ChannelSink`] wrapper that enforces credit-based flow control.
///
/// Each `send_payload` acquires one permit from the semaphore, blocking if
/// credit is zero. The semaphore is shared with the driver so that incoming
/// `GrantCredit` messages can add permits via [`CreditSink::credit`].
#[cfg(not(target_arch = "wasm32"))]
pub struct CreditSink<S: ChannelSink> {
    inner: S,
    credit: Arc<Semaphore>,
}

#[cfg(not(target_arch = "wasm32"))]
impl<S: ChannelSink> CreditSink<S> {
    // r[impl rpc.flow-control.credit.initial]
    // r[impl rpc.flow-control.credit.initial.zero]
    /// Wrap `inner` with `initial_credit` permits (the const generic `N`).
    pub fn new(inner: S, initial_credit: u32) -> Self {
        Self {
            inner,
            credit: Arc::new(Semaphore::new(initial_credit as usize)),
        }
    }

    /// Returns the credit semaphore. The driver holds a clone so
    /// `GrantCredit` messages can call `add_permits`.
    pub fn credit(&self) -> &Arc<Semaphore> {
        &self.credit
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl<S: ChannelSink> ChannelSink for CreditSink<S> {
    fn send_payload<'payload>(
        &self,
        payload: Payload<'payload>,
    ) -> Pin<Box<dyn Future<Output = Result<(), TxError>> + Send + 'payload>> {
        let credit = self.credit.clone();
        let fut = self.inner.send_payload(payload);
        Box::pin(async move {
            let permit = credit
                .acquire()
                .await
                .map_err(|_| TxError::Transport("channel credit semaphore closed".into()))?;
            permit.forget();
            fut.await
        })
    }

    fn close_channel(
        &self,
        metadata: Metadata,
    ) -> Pin<Box<dyn Future<Output = Result<(), TxError>> + Send + 'static>> {
        // Close does not consume credit — it's a control message.
        self.inner.close_channel(metadata)
    }
}

/// Message delivered to an `Rx` by the driver.
#[cfg(not(target_arch = "wasm32"))]
pub enum IncomingChannelMessage {
    Item(SelfRef<ChannelItem<'static>>),
    Close(SelfRef<ChannelClose<'static>>),
    Reset(SelfRef<ChannelReset<'static>>),
}

/// Sender-side runtime slot.
#[derive(Facet)]
#[facet(opaque)]
pub(crate) struct SinkSlot {
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) inner: Option<Arc<dyn ChannelSink>>,
}

impl SinkSlot {
    pub(crate) fn empty() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            inner: None,
        }
    }
}

/// Receiver-side runtime slot.
#[derive(Facet)]
#[facet(opaque)]
pub(crate) struct ReceiverSlot {
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) inner: Option<mpsc::Receiver<IncomingChannelMessage>>,
}

impl ReceiverSlot {
    pub(crate) fn empty() -> Self {
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            inner: None,
        }
    }
}

/// Sender handle: "I send". The holder of a `Tx<T>` sends items of type `T`.
///
/// In arg position the handler holds it (handler sends → caller).
/// In return position the caller holds it (caller sends → handler).
///
/// Wire encoding is always unit (`()`), with channel IDs carried exclusively
/// in `Message::{Request,Response}.channels`.
// r[impl rpc.channel]
// r[impl rpc.channel.direction]
// r[impl rpc.channel.payload-encoding]
#[derive(Facet)]
#[facet(proxy = ())]
pub struct Tx<T, const N: usize = 16> {
    pub(crate) sink: SinkSlot,
    #[facet(opaque)]
    _marker: PhantomData<T>,
}

impl<T, const N: usize> Tx<T, N> {
    pub fn unbound() -> Self {
        Self {
            sink: SinkSlot::empty(),
            _marker: PhantomData,
        }
    }

    pub fn is_bound(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        return self.sink.inner.is_some();
        #[cfg(target_arch = "wasm32")]
        return false;
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub async fn send<'value>(&self, value: T) -> Result<(), TxError>
    where
        T: Facet<'value>,
    {
        let sink = self.sink.inner.as_ref().ok_or(TxError::Unbound)?;
        let ptr = PtrConst::new((&value as *const T).cast::<u8>());
        // SAFETY: `value` is explicitly dropped only after `await`, so the pointer
        // remains valid for the whole send operation.
        let payload = unsafe { Payload::outgoing_unchecked(ptr, T::SHAPE) };
        let result = sink.send_payload(payload).await;
        drop(value);
        result
    }

    // r[impl rpc.channel.lifecycle]
    #[cfg(not(target_arch = "wasm32"))]
    pub async fn close<'value>(&self, metadata: Metadata<'value>) -> Result<(), TxError> {
        let sink = self.sink.inner.as_ref().ok_or(TxError::Unbound)?;
        sink.close_channel(metadata).await
    }

    #[doc(hidden)]
    #[cfg(not(target_arch = "wasm32"))]
    pub fn bind(&mut self, sink: Arc<dyn ChannelSink>) {
        self.sink.inner = Some(sink);
    }
}

#[allow(clippy::infallible_try_from)]
impl<T, const N: usize> TryFrom<&Tx<T, N>> for () {
    type Error = Infallible;

    fn try_from(_value: &Tx<T, N>) -> Result<Self, Self::Error> {
        Ok(())
    }
}

#[allow(clippy::infallible_try_from)]
impl<T, const N: usize> TryFrom<()> for Tx<T, N> {
    type Error = Infallible;

    fn try_from(_value: ()) -> Result<Self, Self::Error> {
        Ok(Self::unbound())
    }
}

/// Error when sending on a `Tx`.
#[derive(Debug)]
pub enum TxError {
    Unbound,
    Transport(String),
}

impl std::fmt::Display for TxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unbound => write!(f, "channel is not bound"),
            Self::Transport(msg) => write!(f, "transport error: {msg}"),
        }
    }
}

impl std::error::Error for TxError {}

/// Receiver handle: "I receive". The holder of an `Rx<T>` receives items of type `T`.
///
/// In arg position the handler holds it (handler receives ← caller).
/// In return position the caller holds it (caller receives ← handler).
///
/// Wire encoding is always unit (`()`), with channel IDs carried exclusively
/// in `Message::{Request,Response}.channels`.
#[derive(Facet)]
#[facet(proxy = ())]
pub struct Rx<T, const N: usize = 16> {
    pub(crate) receiver: ReceiverSlot,
    #[facet(opaque)]
    _marker: PhantomData<T>,
}

impl<T, const N: usize> Rx<T, N> {
    pub fn unbound() -> Self {
        Self {
            receiver: ReceiverSlot::empty(),
            _marker: PhantomData,
        }
    }

    pub fn is_bound(&self) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        return self.receiver.inner.is_some();
        #[cfg(target_arch = "wasm32")]
        return false;
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub async fn recv(&mut self) -> Result<Option<SelfRef<T>>, RxError>
    where
        T: Facet<'static>,
    {
        let receiver = self.receiver.inner.as_mut().ok_or(RxError::Unbound)?;
        match receiver.recv().await {
            Some(IncomingChannelMessage::Close(_)) | None => Ok(None),
            Some(IncomingChannelMessage::Reset(_)) => Err(RxError::Reset),
            Some(IncomingChannelMessage::Item(msg)) => msg
                .try_repack(|item, _backing_bytes| {
                    let Payload::Incoming(bytes) = item.item else {
                        return Err(RxError::Protocol(
                            "incoming channel item payload was not Incoming".into(),
                        ));
                    };
                    facet_postcard::from_slice_borrowed(bytes).map_err(RxError::Deserialize)
                })
                .map(Some),
        }
    }

    #[doc(hidden)]
    #[cfg(not(target_arch = "wasm32"))]
    pub fn bind(&mut self, receiver: mpsc::Receiver<IncomingChannelMessage>) {
        self.receiver.inner = Some(receiver);
    }
}

#[allow(clippy::infallible_try_from)]
impl<T, const N: usize> TryFrom<&Rx<T, N>> for () {
    type Error = Infallible;

    fn try_from(_value: &Rx<T, N>) -> Result<Self, Self::Error> {
        Ok(())
    }
}

#[allow(clippy::infallible_try_from)]
impl<T, const N: usize> TryFrom<()> for Rx<T, N> {
    type Error = Infallible;

    fn try_from(_value: ()) -> Result<Self, Self::Error> {
        Ok(Self::unbound())
    }
}

/// Error when receiving from an `Rx`.
#[derive(Debug)]
pub enum RxError {
    Unbound,
    Reset,
    Deserialize(facet_postcard::DeserializeError),
    Protocol(String),
}

impl std::fmt::Display for RxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unbound => write!(f, "channel is not bound"),
            Self::Reset => write!(f, "channel reset by peer"),
            Self::Deserialize(e) => write!(f, "deserialize error: {e}"),
            Self::Protocol(msg) => write!(f, "protocol error: {msg}"),
        }
    }
}

impl std::error::Error for RxError {}

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
