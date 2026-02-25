use std::{collections::BTreeMap, pin::Pin, sync::Arc};

use moire::sync::mpsc;
use roam_types::{
    ChannelMessage, Conduit, ConduitRx, ConduitTx, ConduitTxPermit, ConnectionId,
    ConnectionSettings, Message, MessageFamily, MessagePayload, Metadata, Parity, RequestBody,
    RequestId, RequestMessage, RequestResponse, SelfRef, SessionRole,
};

mod builders;
pub use builders::*;

// r[impl session.handshake]
/// Current roam session protocol version.
pub const PROTOCOL_VERSION: u32 = 7;

/// Session state machine.
// r[impl session]
pub struct Session<C: Conduit> {
    /// Conduit receiver
    rx: C::Rx,

    // r[impl session.role]
    role: SessionRole,

    /// Our local parity - when opening connections we'll use that.
    // r[impl session.parity]
    parity: Parity,

    /// Shared core (for sending) — also held by all ConnectionSenders.
    sess_core: Arc<SessionCore>,

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
    failures: Arc<mpsc::UnboundedSender<(RequestId, &'static str)>>,
}

impl ConnectionSender {
    /// Send an arbitrary connection message
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

    /// Send a response specifically
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

    /// Mark a request as failed by removing any pending response slot.
    /// Called when a send error occurs or no reply was sent.
    pub fn mark_failure(&self, request_id: RequestId, reason: &'static str) {
        let _ = self.failures.send((request_id, reason));
    }
}

pub struct ConnectionHandle {
    pub(crate) sender: ConnectionSender,
    pub(crate) rx: mpsc::Receiver<SelfRef<ConnectionMessage<'static>>>,
    pub(crate) failures_rx: mpsc::UnboundedReceiver<(RequestId, &'static str)>,
}

pub(crate) enum ConnectionMessage<'payload> {
    Request(RequestMessage<'payload>),
    Channel(ChannelMessage<'payload>),
}

impl ConnectionHandle {
    pub(crate) fn sender(&self) -> &ConnectionSender {
        &self.sender
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
    C::Tx: Send + Sync + 'static,
    for<'p> <C::Tx as ConduitTx>::Permit<'p>: Send,
    C::Rx: Send,
{
    fn pre_handshake(tx: C::Tx, rx: C::Rx) -> Self {
        let sess_core = Arc::new(SessionCore { tx: Box::new(tx) });
        Session {
            rx,
            role: SessionRole::Initiator, // overwritten in establish_as_*
            parity: Parity::Odd,          // overwritten in establish_as_*
            sess_core,
            conns: BTreeMap::new(),
        }
    }

    // r[impl session.handshake]
    async fn establish_as_initiator(
        &mut self,
        settings: ConnectionSettings,
        metadata: Metadata<'_>,
    ) -> Result<ConnectionHandle, SessionError> {
        use roam_types::{Hello, MessagePayload};

        self.role = SessionRole::Initiator;
        self.parity = settings.parity;

        // Send Hello
        self.sess_core
            .send(Message {
                connection_id: ConnectionId::ROOT,
                payload: MessagePayload::Hello(Hello {
                    version: PROTOCOL_VERSION,
                    connection_settings: settings.clone(),
                    metadata,
                }),
            })
            .await
            .map_err(|_| SessionError::Protocol("failed to send Hello".into()))?;

        // Receive HelloYourself
        let peer_settings = match self.rx.recv().await {
            Ok(Some(msg)) => {
                let payload = msg.map(|m| m.payload);
                match &*payload {
                    MessagePayload::HelloYourself(hy) => hy.connection_settings.clone(),
                    MessagePayload::ProtocolError(e) => {
                        return Err(SessionError::Protocol(e.description.to_owned()));
                    }
                    _ => {
                        return Err(SessionError::Protocol("expected HelloYourself".into()));
                    }
                }
            }
            Ok(None) => {
                return Err(SessionError::Protocol(
                    "peer closed during handshake".into(),
                ));
            }
            Err(e) => return Err(SessionError::Protocol(e.to_string())),
        };

        Ok(self.make_root_handle(settings, peer_settings))
    }

    // r[impl session.handshake]
    #[moire::instrument]
    async fn establish_as_acceptor(
        &mut self,
        settings: ConnectionSettings,
        metadata: Metadata<'_>,
    ) -> Result<ConnectionHandle, SessionError> {
        use roam_types::{HelloYourself, MessagePayload};

        self.role = SessionRole::Acceptor;

        // Receive Hello
        let peer_settings = match self.rx.recv().await {
            Ok(Some(msg)) => {
                let payload = msg.map(|m| m.payload);
                match &*payload {
                    MessagePayload::Hello(h) => {
                        if h.version != PROTOCOL_VERSION {
                            return Err(SessionError::Protocol(format!(
                                "version mismatch: got {}, expected {PROTOCOL_VERSION}",
                                h.version
                            )));
                        }
                        h.connection_settings.clone()
                    }
                    MessagePayload::ProtocolError(e) => {
                        return Err(SessionError::Protocol(e.description.to_owned()));
                    }
                    _ => {
                        return Err(SessionError::Protocol("expected Hello".into()));
                    }
                }
            }
            Ok(None) => {
                return Err(SessionError::Protocol(
                    "peer closed during handshake".into(),
                ));
            }
            Err(e) => return Err(SessionError::Protocol(e.to_string())),
        };

        // Acceptor parity is opposite of initiator
        let our_settings = ConnectionSettings {
            parity: peer_settings.parity.other(),
            ..settings
        };
        self.parity = our_settings.parity;

        // Send HelloYourself
        self.sess_core
            .send(Message {
                connection_id: ConnectionId::ROOT,
                payload: MessagePayload::HelloYourself(HelloYourself {
                    connection_settings: our_settings.clone(),
                    metadata,
                }),
            })
            .await
            .map_err(|_| SessionError::Protocol("failed to send HelloYourself".into()))?;

        Ok(self.make_root_handle(our_settings, peer_settings))
    }

    fn make_root_handle(
        &mut self,
        local_settings: ConnectionSettings,
        peer_settings: ConnectionSettings,
    ) -> ConnectionHandle {
        let (conn_tx, conn_rx) =
            mpsc::channel::<SelfRef<ConnectionMessage<'static>>>("session.conn0", 64);
        let (failures_tx, failures_rx) = mpsc::unbounded_channel("session.conn0.failures");

        let sender = ConnectionSender {
            connection_id: ConnectionId::ROOT,
            sess_core: Arc::clone(&self.sess_core),
            failures: Arc::new(failures_tx),
        };

        self.conns.insert(
            ConnectionId::ROOT,
            ConnectionSlot::Active(ConnectionState {
                id: ConnectionId::ROOT,
                local_settings,
                peer_settings,
                conn_tx,
            }),
        );

        ConnectionHandle {
            sender,
            rx: conn_rx,
            failures_rx,
        }
    }

    /// Run the session recv loop: read from the conduit, demux by connection
    /// ID, route to the appropriate connection's mpsc sender.
    pub async fn run(&mut self) {
        loop {
            let msg = match self.rx.recv().await {
                Ok(Some(msg)) => msg,
                Ok(None) => break,
                Err(_) => break,
            };

            let conn_id = msg.connection_id;

            let conn_tx = match self.conns.get(&conn_id) {
                Some(ConnectionSlot::Active(state)) => state.conn_tx.clone(),
                _ => {
                    // [TODO] handle ConnectionOpen and other control messages
                    continue;
                }
            };

            let conn_msg = msg.map(|m| match m.payload {
                MessagePayload::RequestMessage(r) => ConnectionMessage::Request(r),
                MessagePayload::ChannelMessage(c) => ConnectionMessage::Channel(c),
                _ => {
                    // [TODO] handle control messages
                    panic!("unexpected control message on active connection");
                }
            });

            if conn_tx.send(conn_msg).await.is_err() {
                // Driver dropped — connection is done
                self.conns.remove(&conn_id);
            }
        }
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

type BoxFuture<'a, T> = Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;

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
