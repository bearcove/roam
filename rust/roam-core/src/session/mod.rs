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

/// Session state machine.
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

    /// Sender for routing incoming messages to the per-connection driver task.
    conn_tx: mpsc::Sender<SelfRef<ConnectionMessage<'static>>>,
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
}

impl SessionCore {
    pub(crate) async fn send<'a>(&self, msg: Message<'a>) -> Result<(), ()> {
        self.tx.send_msg(msg).await.map_err(|_| ())
    }
}

/// Create a session that skips the handshake entirely, assuming a single
/// root connection (connection 0). Returns a ConnectionHandle for the
/// root connection and a future that runs the session recv loop.
///
/// Both sides of a MemoryLink (or any conduit) can call this independently.
pub(crate) fn session_no_handshake<C>(
    conduit: C,
    local_settings: ConnectionSettings,
    peer_settings: ConnectionSettings,
) -> (ConnectionHandle, impl Future<Output = ()>)
where
    C: Conduit<Msg = MessageFamily>,
    C::Tx: Send + Sync,
    for<'p> <C::Tx as ConduitTx>::Permit<'p>: Send,
    C::Tx: 'static,
    C::Rx: Send,
{
    let (conduit_tx, mut conduit_rx) = conduit.split();

    // Create the mpsc channel for routing incoming messages to the driver.
    let (conn_tx, conn_rx) = mpsc::channel::<SelfRef<ConnectionMessage<'static>>>(64);

    // Build the session core (shared, for sending).
    let sess_core = Arc::new(SessionCore {
        tx: Box::new(conduit_tx),
    });

    // Build the connection handle for the root connection.
    let handle = ConnectionHandle {
        sender: ConnectionSender {
            connection_id: ConnectionId::ROOT,
            sess_core,
        },
        rx: conn_rx,
    };

    // The recv loop: read from the conduit, demux by connection ID, route
    // to the appropriate connection's mpsc sender.
    let recv_loop = async move {
        // For now we only have connection 0. Store its sender.
        let root_tx = conn_tx;

        loop {
            let msg = match conduit_rx.recv().await {
                Ok(Some(msg)) => msg,
                Ok(None) => break, // peer closed
                Err(_) => break,   // conduit error
            };

            // Demux: peek at connection_id, then route.
            let conn_id = msg.connection_id;

            // For now, only connection 0 is supported.
            if !conn_id.is_root() {
                // [TODO] look up connection by ID, handle ConnectionOpen, etc.
                continue;
            }

            // Map the SelfRef<Message> into a SelfRef<ConnectionMessage>.
            let conn_msg = msg.map(|m| match m.payload {
                MessagePayload::RequestMessage(r) => ConnectionMessage::Request(r),
                MessagePayload::ChannelMessage(c) => ConnectionMessage::Channel(c),
                _ => {
                    // [TODO] handle control messages (Hello, ProtocolError, etc.)
                    // For now, skip them.
                    panic!("unexpected control message on connection 0");
                }
            });

            if root_tx.send(conn_msg).await.is_err() {
                // Driver dropped its receiver — connection is done.
                break;
            }
        }
    };

    (handle, recv_loop)
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
