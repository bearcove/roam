use std::convert::Infallible;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::sync::Arc;

use facet::Facet;
use roam_types::{ChannelId, ConnectionId, Message, Metadata, Payload, SelfRef};
use tokio::sync::mpsc;

/// Create an unbound channel pair.
///
/// Both ends start hollow and must be hydrated by the session driver using
/// channel IDs from `Message::{Request,Response}.channels`.
pub fn channel<T: 'static, const N: usize>() -> (Tx<T, N>, Rx<T, N>) {
    (Tx::unbound(), Rx::unbound())
}

type SendFut<'a> = Pin<Box<dyn Future<Output = Result<(), TxError>> + 'a>>;
type CloseFut = Pin<Box<dyn Future<Output = Result<(), TxError>> + 'static>>;

/// Runtime sink implemented by the session driver.
///
/// The contract is strict: successful completion means the item has gone
/// through the conduit to the link commit boundary.
pub trait ChannelSink: Send + Sync + 'static {
    fn send_payload<'payload>(
        &self,
        conn_id: ConnectionId,
        channel_id: ChannelId,
        payload: Payload<'payload>,
    ) -> SendFut<'payload>;

    fn close_channel(
        &self,
        conn_id: ConnectionId,
        channel_id: ChannelId,
        metadata: Metadata,
    ) -> CloseFut;
}

/// Message delivered to an `Rx` by the driver.
pub enum IncomingChannelMessage {
    Data(SelfRef<Message<'static>>),
    Close,
    Reset,
}

/// Sender-side runtime slot.
#[derive(Facet)]
#[facet(opaque)]
pub(crate) struct SinkSlot {
    pub(crate) inner: Option<Arc<dyn ChannelSink>>,
}

impl SinkSlot {
    pub(crate) fn empty() -> Self {
        Self { inner: None }
    }
}

/// Receiver-side runtime slot.
#[derive(Facet)]
#[facet(opaque)]
pub(crate) struct ReceiverSlot {
    pub(crate) inner: Option<mpsc::Receiver<IncomingChannelMessage>>,
}

impl ReceiverSlot {
    pub(crate) fn empty() -> Self {
        Self { inner: None }
    }
}

/// Caller-side sender handle (`caller -> callee`).
///
/// Wire encoding is always unit (`()`), with channel IDs carried exclusively
/// in `Message::{Request,Response}.channels`.
#[derive(Facet)]
#[facet(proxy = ())]
pub struct Tx<T: 'static, const N: usize = 16> {
    pub(crate) conn_id: ConnectionId,
    pub(crate) channel_id: ChannelId,
    pub(crate) sink: SinkSlot,
    #[facet(opaque)]
    _marker: PhantomData<T>,
}

impl<T: 'static, const N: usize> Tx<T, N> {
    pub fn unbound() -> Self {
        Self {
            conn_id: ConnectionId::ROOT,
            channel_id: ChannelId::RESERVED,
            sink: SinkSlot::empty(),
            _marker: PhantomData,
        }
    }

    pub fn is_bound(&self) -> bool {
        self.sink.inner.is_some() && self.channel_id != ChannelId::RESERVED
    }

    pub fn connection_id(&self) -> Option<ConnectionId> {
        self.is_bound().then_some(self.conn_id)
    }

    pub fn channel_id(&self) -> Option<ChannelId> {
        self.is_bound().then_some(self.channel_id)
    }

    pub async fn send<'value, U>(&self, value: &'value U) -> Result<(), TxError>
    where
        U: Facet<'value>,
        T: Facet<'static>,
    {
        if U::SHAPE != T::SHAPE {
            return Err(TxError::ShapeMismatch {
                expected: T::SHAPE,
                got: U::SHAPE,
            });
        }
        let sink = self.sink.inner.as_ref().ok_or(TxError::Unbound)?;
        sink.send_payload(self.conn_id, self.channel_id, Payload::borrowed::<U>(value))
            .await
    }

    pub async fn close(&self, metadata: Metadata) -> Result<(), TxError> {
        let sink = self.sink.inner.as_ref().ok_or(TxError::Unbound)?;
        sink.close_channel(self.conn_id, self.channel_id, metadata)
            .await
    }

    #[allow(dead_code)]
    pub(crate) fn bind(
        &mut self,
        conn_id: ConnectionId,
        channel_id: ChannelId,
        sink: Arc<dyn ChannelSink>,
    ) {
        self.conn_id = conn_id;
        self.channel_id = channel_id;
        self.sink.inner = Some(sink);
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

/// Error when sending on a `Tx`.
#[derive(Debug)]
pub enum TxError {
    Unbound,
    ShapeMismatch {
        expected: &'static facet_core::Shape,
        got: &'static facet_core::Shape,
    },
    Transport(String),
}

impl std::fmt::Display for TxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unbound => write!(f, "channel is not bound"),
            Self::ShapeMismatch { expected, got } => {
                write!(f, "shape mismatch: expected {expected}, got {got}")
            }
            Self::Transport(msg) => write!(f, "transport error: {msg}"),
        }
    }
}

impl std::error::Error for TxError {}

/// Caller-side receiver handle (`callee -> caller`).
///
/// Wire encoding is always unit (`()`), with channel IDs carried exclusively
/// in `Message::{Request,Response}.channels`.
#[derive(Facet)]
#[facet(proxy = ())]
pub struct Rx<T: 'static, const N: usize = 16> {
    pub(crate) conn_id: ConnectionId,
    pub(crate) channel_id: ChannelId,
    pub(crate) receiver: ReceiverSlot,
    #[facet(opaque)]
    _marker: PhantomData<T>,
}

impl<T: 'static, const N: usize> Rx<T, N> {
    pub fn unbound() -> Self {
        Self {
            conn_id: ConnectionId::ROOT,
            channel_id: ChannelId::RESERVED,
            receiver: ReceiverSlot::empty(),
            _marker: PhantomData,
        }
    }

    pub fn is_bound(&self) -> bool {
        self.receiver.inner.is_some() && self.channel_id != ChannelId::RESERVED
    }

    pub fn connection_id(&self) -> Option<ConnectionId> {
        self.is_bound().then_some(self.conn_id)
    }

    pub fn channel_id(&self) -> Option<ChannelId> {
        self.is_bound().then_some(self.channel_id)
    }

    pub async fn recv(&mut self) -> Result<Option<T>, RxError>
    where
        T: Facet<'static>,
    {
        let receiver = self.receiver.inner.as_mut().ok_or(RxError::Unbound)?;
        match receiver.recv().await {
            Some(IncomingChannelMessage::Close) | None => Ok(None),
            Some(IncomingChannelMessage::Reset) => Err(RxError::Reset),
            Some(IncomingChannelMessage::Data(msg)) => {
                let Some((conn_id, channel_id, payload)) = msg.as_channel_item() else {
                    return Err(RxError::Protocol("expected ChannelItem message".into()));
                };
                if conn_id != self.conn_id || channel_id != self.channel_id {
                    return Err(RxError::Protocol(format!(
                        "received item for unexpected channel: got ({conn_id}, {channel_id}) expected ({}, {})",
                        self.conn_id, self.channel_id
                    )));
                }

                let bytes = payload.as_incoming_bytes().ok_or_else(|| {
                    RxError::Protocol("incoming channel item payload was not decoded bytes".into())
                })?;
                let value = facet_postcard::from_slice(bytes).map_err(RxError::Deserialize)?;
                Ok(Some(value))
            }
        }
    }

    #[allow(dead_code)]
    pub(crate) fn bind(
        &mut self,
        conn_id: ConnectionId,
        channel_id: ChannelId,
        receiver: mpsc::Receiver<IncomingChannelMessage>,
    ) {
        self.conn_id = conn_id;
        self.channel_id = channel_id;
        self.receiver.inner = Some(receiver);
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
