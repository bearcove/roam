use std::{collections::BTreeMap, pin::Pin, sync::Arc};

use moire::sync::mpsc;
use roam_types::{
    ChannelMessage, Conduit, ConduitRx, ConduitTx, ConduitTxPermit, ConnectionClose, ConnectionId,
    ConnectionOpen, ConnectionSettings, IdAllocator, MaybeSend, MaybeSync, Message, MessageFamily,
    MessagePayload, Metadata, Parity, RequestBody, RequestId, RequestMessage, RequestResponse,
    SelfRef, SessionRole,
};

mod builders;
pub use builders::*;

// r[impl session.handshake]
/// Current roam session protocol version.
pub const PROTOCOL_VERSION: u32 = 7;

// ---------------------------------------------------------------------------
// Connection acceptor trait
// ---------------------------------------------------------------------------

/// Callback for accepting or rejecting inbound virtual connections.
///
/// Registered on the session via the builder's `.on_connection()` method.
/// Called synchronously from the session run loop when a peer sends
/// `ConnectionOpen`. The acceptor returns either an `AcceptedConnection`
/// (with settings, metadata, and a setup callback that spawns the driver)
/// or rejection metadata.
// r[impl rpc.virtual-connection.accept]
pub trait ConnectionAcceptor: Send + 'static {
    fn accept(
        &self,
        conn_id: ConnectionId,
        peer_settings: &ConnectionSettings,
        metadata: &[roam_types::MetadataEntry],
    ) -> Result<AcceptedConnection, Metadata<'static>>;
}

/// Result of accepting a virtual connection.
pub struct AcceptedConnection {
    /// Our settings for this connection.
    pub settings: ConnectionSettings,
    /// Metadata to send back in ConnectionAccept.
    pub metadata: Metadata<'static>,
    /// Callback that receives the ConnectionHandle and spawns a Driver.
    pub setup: Box<dyn FnOnce(ConnectionHandle) + Send>,
}

// ---------------------------------------------------------------------------
// Open/close request types (from SessionHandle → run loop)
// ---------------------------------------------------------------------------

struct OpenRequest {
    settings: ConnectionSettings,
    metadata: Metadata<'static>,
    result_tx: moire::sync::oneshot::Sender<Result<ConnectionHandle, SessionError>>,
}

struct CloseRequest {
    conn_id: ConnectionId,
    metadata: Metadata<'static>,
    result_tx: moire::sync::oneshot::Sender<Result<(), SessionError>>,
}

// ---------------------------------------------------------------------------
// SessionHandle — cloneable handle for opening/closing virtual connections
// ---------------------------------------------------------------------------

/// Cloneable handle for opening and closing virtual connections.
///
/// Returned by the session builder alongside the `Session` and root
/// `ConnectionHandle`. The session's `run()` loop must be running
/// concurrently for requests to be processed.
// r[impl rpc.virtual-connection.open]
#[derive(Clone)]
pub struct SessionHandle {
    open_tx: mpsc::Sender<OpenRequest>,
    close_tx: mpsc::Sender<CloseRequest>,
}

impl SessionHandle {
    /// Open a new virtual connection on the session.
    ///
    /// Allocates a connection ID, sends `ConnectionOpen` to the peer, and
    /// waits for `ConnectionAccept` or `ConnectionReject`. The session's
    /// `run()` loop processes the response and completes the returned future.
    // r[impl connection.open]
    pub async fn open_connection(
        &self,
        settings: ConnectionSettings,
        metadata: Metadata<'static>,
    ) -> Result<ConnectionHandle, SessionError> {
        let (result_tx, result_rx) = moire::sync::oneshot::channel("session.open_result");
        self.open_tx
            .send(OpenRequest {
                settings,
                metadata,
                result_tx,
            })
            .await
            .map_err(|_| SessionError::Protocol("session closed".into()))?;
        result_rx
            .await
            .map_err(|_| SessionError::Protocol("session closed".into()))?
    }

    /// Close a virtual connection.
    ///
    /// Sends `ConnectionClose` to the peer and removes the connection slot.
    /// After this returns, no further messages will be routed to the
    /// connection's driver.
    // r[impl connection.close]
    pub async fn close_connection(
        &self,
        conn_id: ConnectionId,
        metadata: Metadata<'static>,
    ) -> Result<(), SessionError> {
        let (result_tx, result_rx) = moire::sync::oneshot::channel("session.close_result");
        self.close_tx
            .send(CloseRequest {
                conn_id,
                metadata,
                result_tx,
            })
            .await
            .map_err(|_| SessionError::Protocol("session closed".into()))?;
        result_rx
            .await
            .map_err(|_| SessionError::Protocol("session closed".into()))?
    }
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

/// Session state machine.
// r[impl session]
// r[impl rpc.one-service-per-connection]
pub struct Session<C: Conduit> {
    /// Conduit receiver
    rx: C::Rx,

    // r[impl session.role]
    role: SessionRole,

    /// Our local parity — determines which connection IDs we allocate.
    // r[impl session.parity]
    parity: Parity,

    /// Shared core (for sending) — also held by all ConnectionSenders.
    sess_core: Arc<SessionCore>,

    /// Connection state (active, pending inbound, pending outbound).
    conns: BTreeMap<ConnectionId, ConnectionSlot>,

    /// Allocator for outbound virtual connection IDs (uses session parity).
    conn_ids: IdAllocator<ConnectionId>,

    /// Callback for accepting inbound virtual connections.
    on_connection: Option<Box<dyn ConnectionAcceptor>>,

    /// Receiver for open requests from SessionHandle.
    open_rx: mpsc::Receiver<OpenRequest>,

    /// Receiver for close requests from SessionHandle.
    close_rx: mpsc::Receiver<CloseRequest>,
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

#[derive(Debug)]
enum ConnectionSlot {
    Active(ConnectionState),
    PendingOutbound(PendingOutboundData),
}

/// Debug-printable wrapper that omits the oneshot sender.
struct PendingOutboundData {
    local_settings: ConnectionSettings,
    result_tx: Option<moire::sync::oneshot::Sender<Result<ConnectionHandle, SessionError>>>,
}

impl std::fmt::Debug for PendingOutboundData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PendingOutbound")
            .field("local_settings", &self.local_settings)
            .finish()
    }
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

impl std::fmt::Debug for ConnectionHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectionHandle")
            .field("connection_id", &self.sender.connection_id)
            .finish()
    }
}

pub(crate) enum ConnectionMessage<'payload> {
    Request(RequestMessage<'payload>),
    Channel(ChannelMessage<'payload>),
}

impl ConnectionHandle {
    /// Returns the connection ID for this handle.
    pub fn connection_id(&self) -> ConnectionId {
        self.sender.connection_id
    }
}

/// Errors that can occur during session establishment or operation.
#[derive(Debug)]
pub enum SessionError {
    Io(std::io::Error),
    Protocol(String),
    Rejected(Metadata<'static>),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::Protocol(msg) => write!(f, "protocol error: {msg}"),
            Self::Rejected(_) => write!(f, "connection rejected"),
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
    fn pre_handshake(
        tx: C::Tx,
        rx: C::Rx,
        on_connection: Option<Box<dyn ConnectionAcceptor>>,
        open_rx: mpsc::Receiver<OpenRequest>,
        close_rx: mpsc::Receiver<CloseRequest>,
    ) -> Self {
        let sess_core = Arc::new(SessionCore { tx: Box::new(tx) });
        Session {
            rx,
            role: SessionRole::Initiator, // overwritten in establish_as_*
            parity: Parity::Odd,          // overwritten in establish_as_*
            sess_core,
            conns: BTreeMap::new(),
            conn_ids: IdAllocator::new(Parity::Odd), // overwritten in establish_as_*
            on_connection,
            open_rx,
            close_rx,
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
        self.conn_ids = IdAllocator::new(settings.parity);

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
        self.conn_ids = IdAllocator::new(our_settings.parity);

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
        self.make_connection_handle(ConnectionId::ROOT, local_settings, peer_settings)
    }

    fn make_connection_handle(
        &mut self,
        conn_id: ConnectionId,
        local_settings: ConnectionSettings,
        peer_settings: ConnectionSettings,
    ) -> ConnectionHandle {
        let label = format!("session.conn{}", conn_id.0);
        let (conn_tx, conn_rx) = mpsc::channel::<SelfRef<ConnectionMessage<'static>>>(&label, 64);
        let (failures_tx, failures_rx) = mpsc::unbounded_channel(&format!("{label}.failures"));

        let sender = ConnectionSender {
            connection_id: conn_id,
            sess_core: Arc::clone(&self.sess_core),
            failures: Arc::new(failures_tx),
        };

        self.conns.insert(
            conn_id,
            ConnectionSlot::Active(ConnectionState {
                id: conn_id,
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
    /// ID, and route to the appropriate connection's driver. Also processes
    /// open/close requests from the SessionHandle.
    // r[impl zerocopy.framing.pipeline.incoming]
    pub async fn run(&mut self) {
        loop {
            tokio::select! {
                msg = self.rx.recv() => {
                    match msg {
                        Ok(Some(msg)) => self.handle_message(msg).await,
                        _ => break,
                    }
                }
                Some(req) = self.open_rx.recv() => {
                    self.handle_open_request(req).await;
                }
                Some(req) = self.close_rx.recv() => {
                    self.handle_close_request(req).await;
                }
            }
        }
    }

    async fn handle_message(&mut self, msg: SelfRef<Message<'static>>) {
        let conn_id = msg.connection_id;

        // Check what kind of payload this is before looking up the connection.
        let is_connection_open = matches!(&msg.payload, MessagePayload::ConnectionOpen(_));
        let is_connection_accept = matches!(&msg.payload, MessagePayload::ConnectionAccept(_));
        let is_connection_reject = matches!(&msg.payload, MessagePayload::ConnectionReject(_));
        let is_connection_close = matches!(&msg.payload, MessagePayload::ConnectionClose(_));

        // --- Connection control messages ---

        if is_connection_open {
            self.handle_inbound_open(conn_id, msg).await;
            return;
        }

        if is_connection_accept {
            self.handle_inbound_accept(conn_id, msg);
            return;
        }

        if is_connection_reject {
            self.handle_inbound_reject(conn_id, msg);
            return;
        }

        // r[impl connection.close.semantics]
        if is_connection_close {
            // Remove the connection — dropping conn_tx causes the Driver's rx
            // to return None, which exits its run loop. All in-flight handlers
            // are dropped, triggering DriverReplySink::drop → Cancelled responses.
            self.conns.remove(&conn_id);
            return;
        }

        // --- Data messages on active connections ---

        let conn_tx = match self.conns.get(&conn_id) {
            Some(ConnectionSlot::Active(state)) => state.conn_tx.clone(),
            _ => {
                // Unknown connection or pending — drop the message.
                return;
            }
        };

        let conn_msg = msg.map(|m| match m.payload {
            MessagePayload::RequestMessage(r) => ConnectionMessage::Request(r),
            MessagePayload::ChannelMessage(c) => ConnectionMessage::Channel(c),
            _ => {
                // Control messages were handled above.
                unreachable!("control messages handled above");
            }
        });

        if conn_tx.send(conn_msg).await.is_err() {
            // Driver dropped — connection is done
            self.conns.remove(&conn_id);
        }
    }

    async fn handle_inbound_open(&mut self, conn_id: ConnectionId, msg: SelfRef<Message<'static>>) {
        // Extract the ConnectionOpen payload.
        let open = msg.map(|m| match m.payload {
            MessagePayload::ConnectionOpen(o) => o,
            _ => unreachable!(),
        });

        // Validate: connection ID must match peer's parity (opposite of ours).
        let peer_parity = self.parity.other();
        if !conn_id.has_parity(peer_parity) {
            // Protocol error: wrong parity. For now, just reject.
            let _ = self
                .sess_core
                .send(Message {
                    connection_id: conn_id,
                    payload: MessagePayload::ConnectionReject(roam_types::ConnectionReject {
                        metadata: vec![],
                    }),
                })
                .await;
            return;
        }

        // Validate: connection ID must not already be in use.
        if self.conns.contains_key(&conn_id) {
            // Protocol error: duplicate connection ID.
            let _ = self
                .sess_core
                .send(Message {
                    connection_id: conn_id,
                    payload: MessagePayload::ConnectionReject(roam_types::ConnectionReject {
                        metadata: vec![],
                    }),
                })
                .await;
            return;
        }

        // r[impl connection.open.rejection]
        // Call the acceptor callback. If none is registered, reject.
        let acceptor = match &self.on_connection {
            Some(a) => a,
            None => {
                let _ = self
                    .sess_core
                    .send(Message {
                        connection_id: conn_id,
                        payload: MessagePayload::ConnectionReject(roam_types::ConnectionReject {
                            metadata: vec![],
                        }),
                    })
                    .await;
                return;
            }
        };

        match acceptor.accept(conn_id, &open.connection_settings, &open.metadata) {
            Ok(accepted) => {
                // Create the connection handle and activate it.
                let handle = self.make_connection_handle(
                    conn_id,
                    accepted.settings.clone(),
                    open.connection_settings.clone(),
                );

                // Send ConnectionAccept to the peer.
                let _ = self
                    .sess_core
                    .send(Message {
                        connection_id: conn_id,
                        payload: MessagePayload::ConnectionAccept(roam_types::ConnectionAccept {
                            connection_settings: accepted.settings,
                            metadata: accepted.metadata,
                        }),
                    })
                    .await;

                // Let the acceptor set up its driver.
                (accepted.setup)(handle);
            }
            Err(reject_metadata) => {
                let _ = self
                    .sess_core
                    .send(Message {
                        connection_id: conn_id,
                        payload: MessagePayload::ConnectionReject(roam_types::ConnectionReject {
                            metadata: reject_metadata,
                        }),
                    })
                    .await;
            }
        }
    }

    fn handle_inbound_accept(&mut self, conn_id: ConnectionId, msg: SelfRef<Message<'static>>) {
        let slot = self.conns.remove(&conn_id);
        match slot {
            Some(ConnectionSlot::PendingOutbound(mut pending)) => {
                let accept = msg.map(|m| match m.payload {
                    MessagePayload::ConnectionAccept(a) => a,
                    _ => unreachable!(),
                });

                let handle = self.make_connection_handle(
                    conn_id,
                    pending.local_settings.clone(),
                    accept.connection_settings.clone(),
                );

                if let Some(tx) = pending.result_tx.take() {
                    let _ = tx.send(Ok(handle));
                }
            }
            Some(other) => {
                // Not pending outbound — put it back and ignore.
                self.conns.insert(conn_id, other);
            }
            None => {
                // No pending open for this ID — ignore.
            }
        }
    }

    fn handle_inbound_reject(&mut self, conn_id: ConnectionId, msg: SelfRef<Message<'static>>) {
        let slot = self.conns.remove(&conn_id);
        match slot {
            Some(ConnectionSlot::PendingOutbound(mut pending)) => {
                let reject = msg.map(|m| match m.payload {
                    MessagePayload::ConnectionReject(r) => r,
                    _ => unreachable!(),
                });

                if let Some(tx) = pending.result_tx.take() {
                    let _ = tx.send(Err(SessionError::Rejected(reject.metadata.to_vec())));
                }
            }
            Some(other) => {
                self.conns.insert(conn_id, other);
            }
            None => {}
        }
    }

    // r[impl connection.open]
    async fn handle_open_request(&mut self, req: OpenRequest) {
        let conn_id = self.conn_ids.next();

        // Send ConnectionOpen to the peer.
        let send_result = self
            .sess_core
            .send(Message {
                connection_id: conn_id,
                payload: MessagePayload::ConnectionOpen(ConnectionOpen {
                    connection_settings: req.settings.clone(),
                    metadata: req.metadata,
                }),
            })
            .await;

        if send_result.is_err() {
            let _ = req.result_tx.send(Err(SessionError::Protocol(
                "failed to send ConnectionOpen".into(),
            )));
            return;
        }

        // Store the pending state. The run loop will complete the oneshot
        // when ConnectionAccept or ConnectionReject arrives.
        self.conns.insert(
            conn_id,
            ConnectionSlot::PendingOutbound(PendingOutboundData {
                local_settings: req.settings,
                result_tx: Some(req.result_tx),
            }),
        );
    }

    // r[impl connection.close]
    async fn handle_close_request(&mut self, req: CloseRequest) {
        if req.conn_id.is_root() {
            let _ = req.result_tx.send(Err(SessionError::Protocol(
                "cannot close root connection".into(),
            )));
            return;
        }

        // Remove the connection slot — this drops conn_tx and causes the
        // Driver to exit cleanly.
        if self.conns.remove(&req.conn_id).is_none() {
            let _ = req
                .result_tx
                .send(Err(SessionError::Protocol("connection not found".into())));
            return;
        }

        // Send ConnectionClose to the peer.
        let send_result = self
            .sess_core
            .send(Message {
                connection_id: req.conn_id,
                payload: MessagePayload::ConnectionClose(ConnectionClose {
                    metadata: req.metadata,
                }),
            })
            .await;

        if send_result.is_err() {
            let _ = req.result_tx.send(Err(SessionError::Protocol(
                "failed to send ConnectionClose".into(),
            )));
            return;
        }

        let _ = req.result_tx.send(Ok(()));
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

#[cfg(not(target_arch = "wasm32"))]
type BoxFuture<'a, T> = Pin<Box<dyn std::future::Future<Output = T> + Send + 'a>>;
#[cfg(target_arch = "wasm32")]
type BoxFuture<'a, T> = Pin<Box<dyn std::future::Future<Output = T> + 'a>>;

#[cfg(not(target_arch = "wasm32"))]
pub trait DynConduitTx: Send + Sync {
    fn send_msg<'a>(&'a self, msg: Message<'a>) -> BoxFuture<'a, std::io::Result<()>>;
}
#[cfg(target_arch = "wasm32")]
pub trait DynConduitTx {
    fn send_msg<'a>(&'a self, msg: Message<'a>) -> BoxFuture<'a, std::io::Result<()>>;
}

// r[impl zerocopy.send]
// r[impl zerocopy.framing.pipeline.outgoing]
impl<T> DynConduitTx for T
where
    T: ConduitTx<Msg = MessageFamily> + MaybeSend + MaybeSync,
    for<'p> <T as ConduitTx>::Permit<'p>: MaybeSend,
{
    fn send_msg<'a>(&'a self, msg: Message<'a>) -> BoxFuture<'a, std::io::Result<()>> {
        Box::pin(async move {
            let permit = self.reserve().await?;
            permit
                .send(msg)
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))
        })
    }
}
