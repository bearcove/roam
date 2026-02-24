use std::{collections::BTreeMap, future::Future, pin::Pin, sync::Arc};

use roam_types::{
    ChannelMessage, Conduit, ConduitRx, ConduitTx, ConduitTxPermit, ConnectionId,
    ConnectionSettings, Message, MessageFamily, MessagePayload, Metadata, Parity, RequestBody,
    RequestId, RequestMessage, RequestResponse, SelfRef, SessionRole,
};
use tokio::sync::mpsc;

mod builders;
pub use builders::*;

// r[impl session.handshake]
/// Current roam session protocol version.
pub const PROTOCOL_VERSION: u32 = 7;

/// Session state machine,
// r[impl session]
pub struct Session<C: Conduit> {
    /// Conduit sender, torn down on protocol error
    tx: Option<C::Tx>,

    /// Conduit receiver
    rx: C::Rx,

    // r[impl session.role]
    role: SessionRole,

    /// Our local parity - when opening connections we'll use that.
    // r[impl session.parity]
    parity: Parity,

    /// Connection state (including inbound/outbound)
    conns: BTreeMap<ConnectionId, ConnectionSlot>,
}

// r[impl connection]
/// Static data for one active connection.
#[derive(Debug)]
pub struct ConnectionState {
    /// Unique connection identifier
    pub id: ConnectionId,

    /// Our settings
    pub local_settings: ConnectionSettings,

    /// The peer's settings
    pub peer_settings: ConnectionSettings,

    /// Sender for routing incoming request messages to the per-connection task.
    pub req_tx: mpsc::Sender<SelfRef<RequestMessage<'static>>>,
}

#[derive(Debug, Clone)]
struct PendingInboundOpen {
    peer_settings: ConnectionSettings,
}

#[derive(Debug, Clone)]
struct PendingOutboundOpen {
    local_settings: ConnectionSettings,
}

#[derive(Debug)]
enum ConnectionSlot {
    Active(ConnectionState),
    PendingInbound(PendingInboundOpen),
    PendingOutbound(PendingOutboundOpen),
}

#[derive(Clone)]
pub(crate) struct ConnectionSender {
    connection_id: ConnectionId,
    sess_core: Arc<SessionCore>,
}

impl ConnectionSender {
    pub async fn send<'a>(&self, msg: ConnectionMessage<'a>) -> Result<(), ()> {
        let payload = match msg {
            ConnectionMessage::Request(r) => MessagePayload::RequestMessage(r),
            ConnectionMessage::Channel(c) => MessagePayload::ChannelMessage(c),
        };
        let message = Message {
            connection_id: self.connection_id,
            payload,
        };
        self.sess_core.send(message).await.map_err(|_| ())
    }

    pub async fn send_response<'a>(
        &self,
        request_id: RequestId,
        response: RequestResponse<'a>,
    ) -> Result<(), ()> {
        self.send(ConnectionMessage::Request(RequestMessage {
            id: request_id,
            body: RequestBody::Response(response),
        }))
        .await
    }
}

pub(crate) struct ConnectionHandle {
    sender: ConnectionSender,
    rx: mpsc::Receiver<SelfRef<ConnectionMessage<'static>>>,
}

pub(crate) enum ConnectionMessage<'payload> {
    Request(RequestMessage<'payload>),
    Channel(ChannelMessage<'payload>),
}

impl ConnectionHandle {
    pub fn sender(&self) -> &ConnectionSender {
        &self.sender
    }

    pub async fn recv(&mut self) -> Option<SelfRef<ConnectionMessage<'static>>> {
        self.rx.recv().await
    }
}

/// Errors that can occur during session establishment or operation.
#[derive(Debug)]
pub enum SessionError {
    Io(std::io::Error),
    Protocol(String),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Protocol(msg) => write!(f, "protocol error: {msg}"),
        }
    }
}

impl std::error::Error for SessionError {}

impl<C> Session<C>
where
    C: Conduit<Msg = MessageFamily>,
{
    fn pre_handshake(_tx: C::Tx, _rx: C::Rx) -> Self {
        todo!()
    }

    async fn establish_as_initiator(
        &mut self,
        _settings: ConnectionSettings,
        _metadata: Metadata<'_>,
    ) -> Result<mpsc::Receiver<SelfRef<RequestMessage<'static>>>, SessionError> {
        todo!()
    }

    async fn establish_as_acceptor(
        &mut self,
        _settings: ConnectionSettings,
        _metadata: Metadata<'_>,
    ) -> Result<mpsc::Receiver<SelfRef<RequestMessage<'static>>>, SessionError> {
        todo!()
    }
}

pub(crate) struct SessionCore {
    tx: Box<dyn DynConduitTx>,
    conns: BTreeMap<ConnectionId, ConnectionSlot>,
}

impl SessionCore {
    pub(crate) async fn send<'a>(&self, msg: Message<'a>) -> Result<(), ()> {
        self.tx.send_msg(msg).await.map_err(|_| ())
    }
}

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

pub trait DynConduitTx: Send + Sync {
    fn send_msg<'a>(&'a self, msg: Message<'a>) -> BoxFuture<'a, std::io::Result<()>>;
}

impl<T> DynConduitTx for T
where
    T: ConduitTx<Msg = MessageFamily> + Send + Sync,
    for<'p> <T as ConduitTx>::Permit<'p>: Send,
{
    fn send_msg<'a>(&'a self, msg: Message<'a>) -> BoxFuture<'a, std::io::Result<()>> {
        Box::pin(async move {
            let permit = self.reserve().await?;
            permit.send(msg).map_err(std::io::Error::other)
        })
    }
}
