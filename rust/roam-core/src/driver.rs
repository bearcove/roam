use std::collections::HashMap;
use std::sync::Arc;

use roam_types::{
    ChannelId, ConnectionId, Message, MessagePayload, Metadata, Parity, RequestId, SelfRef,
};
use tokio::sync::{mpsc, oneshot};

use crate::{ConnectionState, Session, SessionError, SessionEvent};

// ============================================================================
// Outbound message — callers send these to the driver task
// ============================================================================

/// A message the driver should send on the wire.
enum OutboundMsg {
    /// Send an RPC request and deliver the response to `reply`.
    Request {
        conn_id: ConnectionId,
        request_id: RequestId,
        msg: Vec<u8>, // serialized Message<'static>
        reply: oneshot::Sender<IncomingResponse>,
    },
    /// Cancel an in-flight request.
    Cancel {
        conn_id: ConnectionId,
        request_id: RequestId,
        metadata: Metadata,
    },
    /// Send a response (from a handler task back to the driver).
    Response {
        conn_id: ConnectionId,
        msg: Vec<u8>, // serialized Message<'static>
    },
    /// Open a new virtual connection.
    OpenConnection {
        conn_id: ConnectionId,
        local_parity: Parity,
        settings: roam_types::ConnectionSettings,
        metadata: Metadata,
        reply: oneshot::Sender<Result<ConnectionHandle, DriverError>>,
    },
    /// Accept a pending inbound virtual connection.
    AcceptConnection {
        conn_id: ConnectionId,
        settings: roam_types::ConnectionSettings,
        metadata: Metadata,
        reply: oneshot::Sender<Result<ConnectionHandle, DriverError>>,
    },
    /// Reject a pending inbound virtual connection.
    RejectConnection {
        conn_id: ConnectionId,
        metadata: Metadata,
    },
}

// ============================================================================
// Incoming response — driver delivers these to waiting callers
// ============================================================================

/// A response the driver received and is handing off to a waiting caller.
struct IncomingResponse {
    /// Raw serialized response payload bytes.
    payload: roam_types::SelfRef<Message<'static>>,
}

// ============================================================================
// Incoming request — driver delivers these to the handler
// ============================================================================

/// A request the driver received and is dispatching to a handler.
pub struct IncomingRequest {
    pub conn_id: ConnectionId,
    pub msg: SelfRef<Message<'static>>,
    /// Sender for the handler to push its response back to the driver.
    pub(crate) reply: mpsc::Sender<OutboundMsg>,
}

// ============================================================================
// Driver error
// ============================================================================

#[derive(Debug)]
pub enum DriverError {
    /// The driver task has exited (session closed or errored).
    Gone,
    /// A protocol violation or transport error occurred.
    Session(SessionError),
}

impl std::fmt::Display for DriverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Gone => write!(f, "driver is gone"),
            Self::Session(e) => write!(f, "session error: {e}"),
        }
    }
}

impl std::error::Error for DriverError {}

// ============================================================================
// ConnectionHandle — the caller side of a single connection
// ============================================================================

/// Handle to make outgoing requests on one connection.
///
/// Clone-able. Backed by a channel to the driver task.
/// Does NOT let you accept incoming connections — that's `AcceptHandle`.
#[derive(Clone)]
pub struct ConnectionHandle {
    conn_id: ConnectionId,
    /// Local parity for allocating request IDs and channel IDs.
    local_parity: Parity,
    /// Next request ID counter (odd or even depending on parity).
    // Shared across clones via Arc so they don't collide.
    next_request_id: Arc<std::sync::atomic::AtomicU64>,
    /// Channel to the driver task for sending outbound messages.
    tx: mpsc::Sender<OutboundMsg>,
}

impl ConnectionHandle {
    /// Send a request and wait for the response.
    ///
    /// The `payload` bytes are the already-serialized `Message<'call>`.
    /// Returns the raw `Message<'static>` containing the response.
    ///
    /// Backpressure: calling `.await` on the returned future blocks until
    /// the driver has capacity (bounded channel) and the response arrives.
    pub async fn call(
        &self,
        request_id: RequestId,
        msg: Vec<u8>,
    ) -> Result<SelfRef<Message<'static>>, DriverError> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(OutboundMsg::Request {
                conn_id: self.conn_id,
                request_id,
                msg,
                reply: reply_tx,
            })
            .await
            .map_err(|_| DriverError::Gone)?;

        let resp = reply_rx.await.map_err(|_| DriverError::Gone)?;
        Ok(resp.payload)
    }

    /// Allocate the next request ID using the local parity.
    pub fn alloc_request_id(&self) -> RequestId {
        let base = match self.local_parity {
            Parity::Odd => 1u64,
            Parity::Even => 2u64,
        };
        let prev = self
            .next_request_id
            .fetch_add(2, std::sync::atomic::Ordering::Relaxed);
        // First call: prev == 0, so result = base. Subsequent: prev + 2.
        RequestId(if prev == 0 { base } else { prev + 2 })
    }

    pub fn conn_id(&self) -> ConnectionId {
        self.conn_id
    }

    pub fn local_parity(&self) -> Parity {
        self.local_parity
    }
}

// ============================================================================
// AcceptHandle — accept incoming virtual connections
// ============================================================================

/// Handle to accept incoming virtual connections from the counterpart.
///
/// One per session. NOT clone-able — only one task drives accepts.
pub struct AcceptHandle {
    rx: mpsc::Receiver<IncomingVirtualConnection>,
}

/// An incoming virtual connection the counterpart has opened.
pub struct IncomingVirtualConnection {
    pub conn_id: ConnectionId,
    pub peer_parity: Parity,
    pub peer_settings: roam_types::ConnectionSettings,
    pub metadata: Metadata,
    /// Send back accept/reject decision.
    reply: mpsc::Sender<OutboundMsg>,
}

impl IncomingVirtualConnection {
    /// Accept this connection, providing local settings and a handler.
    ///
    /// Returns a `ConnectionHandle` for making outgoing requests on it.
    pub async fn accept(
        self,
        settings: roam_types::ConnectionSettings,
        metadata: Metadata,
    ) -> Result<ConnectionHandle, DriverError> {
        let (tx, rx) = oneshot::channel();
        self.reply
            .send(OutboundMsg::AcceptConnection {
                conn_id: self.conn_id,
                settings,
                metadata,
                reply: tx,
            })
            .await
            .map_err(|_| DriverError::Gone)?;
        rx.await.map_err(|_| DriverError::Gone)?
    }

    /// Reject this connection.
    pub async fn reject(self, metadata: Metadata) -> Result<(), DriverError> {
        self.reply
            .send(OutboundMsg::RejectConnection {
                conn_id: self.conn_id,
                metadata,
            })
            .await
            .map_err(|_| DriverError::Gone)
    }

    pub fn conn_id(&self) -> ConnectionId {
        self.conn_id
    }

    pub fn peer_parity(&self) -> Parity {
        self.peer_parity
    }

    pub fn metadata(&self) -> &Metadata {
        &self.metadata
    }
}

impl AcceptHandle {
    /// Wait for the next incoming virtual connection.
    ///
    /// Returns `None` when the session is closed.
    pub async fn accept_next(&mut self) -> Option<IncomingVirtualConnection> {
        self.rx.recv().await
    }
}

// ============================================================================
// Driver internal state
// ============================================================================

/// Per-connection state tracked by the driver.
struct ConnState {
    state: ConnectionState,
    /// In-flight requests from our callers: request_id → reply channel.
    pending_calls: HashMap<RequestId, oneshot::Sender<IncomingResponse>>,
    /// Channel for delivering incoming requests to the user's handler.
    /// None if no handler is set for this connection (reject incoming requests).
    handler_tx: Option<mpsc::Sender<IncomingRequest>>,
    /// Next channel ID counter for allocating channels on this connection.
    next_channel_id: u64,
}

impl ConnState {
    fn alloc_channel_id(&mut self) -> Option<ChannelId> {
        let base = match self.state.local_parity {
            Parity::Odd => 1u64,
            Parity::Even => 2u64,
        };
        if self.next_channel_id == 0 {
            self.next_channel_id = base;
        } else {
            self.next_channel_id += 2;
        }
        ChannelId::new(self.next_channel_id)
    }
}

// ============================================================================
// Driver task
// ============================================================================

/// The driver task. Owns the session and runs the event loop.
///
/// Spawned by `Driver::spawn`. Drives `Session::recv_event` and routes
/// messages between callers, handlers, and the wire.
struct DriverTask<C: roam_types::Conduit<Msg = roam_types::MessageFamily>> {
    session: Session<C>,
    /// Inbound channel for outbound messages from callers and handlers.
    inbound: mpsc::Receiver<OutboundMsg>,
    /// Per-connection state.
    conns: HashMap<ConnectionId, ConnState>,
    /// Deliver incoming virtual connections to the accept handle.
    accept_tx: mpsc::Sender<IncomingVirtualConnection>,
    /// Clone of the outbound sender, handed to IncomingRequest and IncomingVirtualConnection.
    outbound_tx: mpsc::Sender<OutboundMsg>,
}

impl<C> DriverTask<C>
where
    C: roam_types::Conduit<Msg = roam_types::MessageFamily> + Send + 'static,
    C::Tx: Send + 'static,
    C::Rx: Send + 'static,
{
    async fn run(mut self) {
        loop {
            tokio::select! {
                // Drive the session receive loop.
                event = self.session.recv_event() => {
                    match event {
                        Ok(ev) => {
                            if let Err(e) = self.handle_session_event(ev).await {
                                // Protocol violation or transport error — session is dead.
                                tracing::error!("driver session error: {e}");
                                break;
                            }
                        }
                        Err(e) => {
                            tracing::error!("driver recv error: {e}");
                            break;
                        }
                    }
                }
                // Handle outbound messages from callers/handlers.
                msg = self.inbound.recv() => {
                    match msg {
                        Some(msg) => {
                            if let Err(e) = self.handle_outbound(msg).await {
                                tracing::error!("driver outbound error: {e}");
                                break;
                            }
                        }
                        None => {
                            // All senders dropped — session is done.
                            break;
                        }
                    }
                }
            }
        }
        // TODO: teardown — drain pending calls with Gone errors, close session.
    }

    async fn handle_session_event(&mut self, ev: SessionEvent) -> Result<(), SessionError> {
        match ev {
            SessionEvent::IncomingConnectionOpen {
                conn_id,
                peer_parity,
                peer_settings,
                metadata,
            } => {
                // Notify the AcceptHandle. If nobody is listening, we'll reject below.
                let incoming = IncomingVirtualConnection {
                    conn_id,
                    peer_parity,
                    peer_settings,
                    metadata,
                    reply: self.outbound_tx.clone(),
                };
                // Best-effort: if accept handle is gone, reject implicitly.
                let _ = self.accept_tx.try_send(incoming);
            }
            SessionEvent::OutgoingConnectionAccepted { conn_id } => {
                // The conn was already inserted as PendingOutbound in open_connection;
                // it's now Active in the session. Promote in our map too — handled
                // when we process the OutboundMsg::OpenConnection reply.
                // (The reply oneshot is resolved here.)
                // TODO: resolve the pending open reply
                let _ = conn_id;
            }
            SessionEvent::OutgoingConnectionRejected { conn_id, metadata } => {
                // TODO: resolve the pending open reply with an error
                let _ = (conn_id, metadata);
            }
            SessionEvent::ConnectionClosed { conn_id, metadata } => {
                // Drop all pending calls for this connection.
                self.conns.remove(&conn_id);
                let _ = metadata;
            }
            SessionEvent::IncomingMessage(msg) => {
                self.route_incoming(msg).await?;
            }
        }
        Ok(())
    }

    async fn route_incoming(&mut self, msg: SelfRef<Message<'static>>) -> Result<(), SessionError> {
        let conn_id = msg.connection_id();
        match msg.payload() {
            MessagePayload::Response(resp) => {
                let request_id = resp.request_id;
                if let Some(conn) = self.conns.get_mut(&conn_id) {
                    if let Some(reply) = conn.pending_calls.remove(&request_id) {
                        let _ = reply.send(IncomingResponse { payload: msg });
                    }
                    // else: cancelled or duplicate — drop silently
                }
            }
            MessagePayload::Request(_) => {
                // Route to handler.
                if let Some(conn) = self.conns.get(&conn_id) {
                    if let Some(tx) = &conn.handler_tx {
                        let req = IncomingRequest {
                            conn_id,
                            msg,
                            reply: self.outbound_tx.clone(),
                        };
                        // Backpressure: this blocks the driver recv loop until
                        // the handler consumes the request. TODO: consider bounded
                        // capacity tied to max_concurrent_requests.
                        let _ = tx.try_send(req); // TODO: handle full channel
                    }
                    // else: no handler — send UnknownMethod response
                }
            }
            MessagePayload::CancelRequest(cancel) => {
                // TODO: signal handler task to stop.
                let _ = cancel;
            }
            // Channel messages — TODO
            MessagePayload::ChannelItem(_)
            | MessagePayload::CloseChannel(_)
            | MessagePayload::ResetChannel(_)
            | MessagePayload::GrantCredit(_) => {}
            // These cannot appear here (session filters them).
            _ => {}
        }
        Ok(())
    }

    async fn handle_outbound(&mut self, msg: OutboundMsg) -> Result<(), SessionError> {
        match msg {
            OutboundMsg::Request {
                conn_id,
                request_id,
                msg: _payload,
                reply,
            } => {
                if let Some(conn) = self.conns.get_mut(&conn_id) {
                    conn.pending_calls.insert(request_id, reply);
                    // TODO: actually send the serialized message via session
                }
            }
            OutboundMsg::Response { conn_id: _, msg: _ } => {
                // TODO: send via session
            }
            OutboundMsg::Cancel {
                conn_id,
                request_id,
                metadata,
            } => {
                // Remove from pending and send CancelRequest on wire.
                if let Some(conn) = self.conns.get_mut(&conn_id) {
                    conn.pending_calls.remove(&request_id);
                }
                // TODO: session.send_rpc_message(CancelRequest { ... })
                let _ = metadata;
            }
            OutboundMsg::OpenConnection {
                conn_id,
                local_parity,
                settings,
                metadata,
                reply,
            } => {
                // TODO: track pending open, resolve reply when AcceptConnection arrives
                self.session
                    .open_connection(conn_id, local_parity, settings, metadata)
                    .await?;
                let _ = reply;
            }
            OutboundMsg::AcceptConnection {
                conn_id,
                settings,
                metadata,
                reply,
            } => {
                self.session
                    .accept_connection(conn_id, settings, metadata)
                    .await?;
                // Build and reply with ConnectionHandle.
                // TODO: derive local_parity from connection state
                let _ = reply;
            }
            OutboundMsg::RejectConnection { conn_id, metadata } => {
                self.session.reject_connection(conn_id, metadata).await?;
            }
        }
        Ok(())
    }
}

// ============================================================================
// Public entry point
// ============================================================================

/// A spawned driver. Holds the accept handle and the root connection handle.
pub struct Driver {
    /// Make calls on the root connection.
    pub root: ConnectionHandle,
    /// Accept incoming virtual connections.
    pub accept: AcceptHandle,
}

impl Driver {
    /// Spawn a driver task for an already-established session.
    ///
    /// `root_state` is the `ConnectionState` for the root connection (conn 0),
    /// obtained from `session.connection(ConnectionId::ROOT)`.
    ///
    /// `root_handler_tx`: if `Some`, incoming requests on the root connection
    /// are delivered here. If `None`, the root connection rejects all requests.
    pub fn spawn<C>(
        session: Session<C>,
        root_state: ConnectionState,
        root_handler_tx: Option<mpsc::Sender<IncomingRequest>>,
    ) -> Self
    where
        C: roam_types::Conduit<Msg = roam_types::MessageFamily> + Send + 'static,
        C::Tx: Send + 'static,
        C::Rx: Send + 'static,
    {
        // Bounded channel for outbound messages. Capacity = max concurrent requests
        // from the root connection; clamp to at least 8.
        let cap = (root_state.peer_settings.max_concurrent_requests as usize).max(8);
        let (outbound_tx, outbound_rx) = mpsc::channel(cap);
        let (accept_tx, accept_rx) = mpsc::channel(16);

        let root_handle = ConnectionHandle {
            conn_id: ConnectionId::ROOT,
            local_parity: root_state.local_parity,
            next_request_id: Arc::new(std::sync::atomic::AtomicU64::new(0)),
            tx: outbound_tx.clone(),
        };

        let mut conns = HashMap::new();
        conns.insert(
            ConnectionId::ROOT,
            ConnState {
                state: root_state,
                pending_calls: HashMap::new(),
                handler_tx: root_handler_tx,
                next_channel_id: 0,
            },
        );

        let task = DriverTask {
            session,
            inbound: outbound_rx,
            conns,
            accept_tx,
            outbound_tx,
        };

        tokio::spawn(task.run());

        Driver {
            root: root_handle,
            accept: AcceptHandle { rx: accept_rx },
        }
    }
}
