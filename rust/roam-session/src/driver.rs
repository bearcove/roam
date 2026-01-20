//! Bidirectional connection driver for message-based transports.
//!
//! This module provides the core connection handling for roam over transports
//! that already provide message framing (like WebSocket).
//!
//! For byte-stream transports (TCP, Unix sockets), see `roam-stream` which
//! wraps streams in COBS framing before using this driver.
//!
//! # Example
//!
//! ```ignore
//! use roam_session::{accept_framed, HandshakeConfig, NoDispatcher};
//! use roam_websocket::WsTransport;
//!
//! let transport = WsTransport::connect("ws://localhost:9000").await?;
//! let (handle, driver) = accept_framed(transport, HandshakeConfig::default(), NoDispatcher).await?;
//!
//! // Spawn the driver (uses runtime abstraction - works on native and WASM)
//! roam_session::runtime::spawn(async move {
//!     let _ = driver.run().await;
//! });
//!
//! // Use handle with generated client
//! let client = MyServiceClient::new(handle);
//! let response = client.echo("hello").await?;
//! ```

use std::collections::HashMap;
use std::future::Future;
use std::io;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use facet::Facet;

use crate::runtime::{Mutex, Receiver, channel, sleep, spawn};
use crate::{
    ChannelError, ChannelRegistry, ConnectionHandle, DriverMessage, MessageTransport, ResponseData,
    RoamError, Role, ServiceDispatcher, TransportError,
};
use roam_wire::{ConnectionId, Hello, Message};

/// Negotiated connection parameters after Hello exchange.
#[derive(Debug, Clone)]
pub struct Negotiated {
    /// Effective max payload size (min of both peers).
    pub max_payload_size: u32,
    /// Initial stream credit (min of both peers).
    pub initial_credit: u32,
}

/// Error during connection handling.
#[derive(Debug)]
pub enum ConnectionError {
    /// IO error.
    Io(std::io::Error),
    /// Protocol violation requiring Goodbye.
    ProtocolViolation {
        /// Rule ID that was violated.
        rule_id: &'static str,
        /// Human-readable context.
        context: String,
    },
    /// Dispatch error.
    Dispatch(String),
    /// Connection closed cleanly.
    Closed,
}

impl std::fmt::Display for ConnectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectionError::Io(e) => write!(f, "IO error: {e}"),
            ConnectionError::ProtocolViolation { rule_id, context } => {
                write!(f, "protocol violation: {rule_id}: {context}")
            }
            ConnectionError::Dispatch(msg) => write!(f, "dispatch error: {msg}"),
            ConnectionError::Closed => write!(f, "connection closed"),
        }
    }
}

impl std::error::Error for ConnectionError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConnectionError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<std::io::Error> for ConnectionError {
    fn from(e: std::io::Error) -> Self {
        ConnectionError::Io(e)
    }
}

/// Configuration for connection handshake.
#[derive(Debug, Clone)]
pub struct HandshakeConfig {
    /// Maximum payload size we support.
    pub max_payload_size: u32,
    /// Initial credit for channels.
    pub initial_channel_credit: u32,
}

impl Default for HandshakeConfig {
    fn default() -> Self {
        Self {
            max_payload_size: 1024 * 1024,     // 1 MiB
            initial_channel_credit: 64 * 1024, // 64 KiB
        }
    }
}

impl HandshakeConfig {
    /// Convert to Hello message (v2 format).
    pub fn to_hello(&self) -> Hello {
        Hello::V2 {
            max_payload_size: self.max_payload_size,
            initial_channel_credit: self.initial_channel_credit,
        }
    }
}

/// A factory that creates new message-based connections on demand.
///
/// Used by [`connect_framed()`] for reconnection with transports that
/// already provide message framing (like WebSocket).
pub trait MessageConnector: Send + Sync + 'static {
    /// The message transport type (e.g., `WsTransport`).
    type Transport: MessageTransport;

    /// Establish a new connection.
    fn connect(&self) -> impl Future<Output = io::Result<Self::Transport>> + Send;
}

/// Configuration for reconnection behavior.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    /// Maximum reconnection attempts before giving up.
    pub max_attempts: u32,
    /// Initial delay between reconnection attempts.
    pub initial_backoff: Duration,
    /// Maximum delay between reconnection attempts.
    pub max_backoff: Duration,
    /// Backoff multiplier.
    pub backoff_multiplier: f64,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_backoff: Duration::from_millis(100),
            max_backoff: Duration::from_secs(5),
            backoff_multiplier: 2.0,
        }
    }
}

impl RetryPolicy {
    /// Calculate the backoff duration for a given attempt number.
    pub fn backoff_for_attempt(&self, attempt: u32) -> Duration {
        let multiplier = self
            .backoff_multiplier
            .powi(attempt.saturating_sub(1) as i32);
        let backoff = self.initial_backoff.mul_f64(multiplier);
        backoff.min(self.max_backoff)
    }
}

/// Error from a reconnecting client.
#[derive(Debug)]
pub enum ConnectError {
    /// All retry attempts exhausted.
    RetriesExhausted {
        /// The original error.
        original: io::Error,
        /// Number of attempts made.
        attempts: u32,
    },
    /// Connection failed.
    ConnectFailed(io::Error),
    /// RPC error during connection setup.
    Rpc(TransportError),
    /// Virtual connection request was rejected by the remote peer.
    Rejected(String),
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectError::RetriesExhausted { original, attempts } => {
                write!(
                    f,
                    "reconnection failed after {attempts} attempts: {original}"
                )
            }
            ConnectError::ConnectFailed(e) => write!(f, "connection failed: {e}"),
            ConnectError::Rpc(e) => write!(f, "RPC error: {e}"),
            ConnectError::Rejected(reason) => write!(f, "connection rejected: {reason}"),
        }
    }
}

impl std::error::Error for ConnectError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            ConnectError::RetriesExhausted { original, .. } => Some(original),
            ConnectError::ConnectFailed(e) => Some(e),
            ConnectError::Rpc(e) => Some(e),
            ConnectError::Rejected(_) => None,
        }
    }
}

impl From<TransportError> for ConnectError {
    fn from(e: TransportError) -> Self {
        ConnectError::Rpc(e)
    }
}

// ============================================================================
// accept_framed() - For accepted connections (no reconnection)
// ============================================================================

/// Accept a connection with a pre-framed transport (e.g., WebSocket).
///
/// Use this when the transport already provides message framing.
/// Returns a handle for making calls and a driver that must be spawned.
pub async fn accept_framed<T, D>(
    transport: T,
    config: HandshakeConfig,
    dispatcher: D,
) -> Result<(ConnectionHandle, Driver<T, D>), ConnectionError>
where
    T: MessageTransport,
    D: ServiceDispatcher,
{
    establish(transport, config.to_hello(), dispatcher, Role::Acceptor).await
}

// ============================================================================
// connect_framed() - For message transports with reconnection
// ============================================================================

/// Connect using a message transport with automatic reconnection.
///
/// Returns a client that automatically reconnects on failure.
/// Implements [`Caller`](crate::Caller) so it works with generated service clients.
pub fn connect_framed<C, D>(
    connector: C,
    config: HandshakeConfig,
    dispatcher: D,
) -> FramedClient<C, D>
where
    C: MessageConnector,
    D: ServiceDispatcher + Clone,
{
    FramedClient {
        connector: Arc::new(connector),
        config,
        dispatcher,
        retry_policy: RetryPolicy::default(),
        state: Arc::new(Mutex::new(None)),
    }
}

/// Connect using a message transport with a custom retry policy.
pub fn connect_framed_with_policy<C, D>(
    connector: C,
    config: HandshakeConfig,
    dispatcher: D,
    retry_policy: RetryPolicy,
) -> FramedClient<C, D>
where
    C: MessageConnector,
    D: ServiceDispatcher + Clone,
{
    FramedClient {
        connector: Arc::new(connector),
        config,
        dispatcher,
        retry_policy,
        state: Arc::new(Mutex::new(None)),
    }
}

/// Internal connection state for FramedClient.
struct FramedClientState {
    handle: ConnectionHandle,
}

/// A client for message transports that automatically reconnects on failure.
///
/// Created by [`connect_framed()`]. Implements [`Caller`](crate::Caller) so it
/// works with generated service clients.
pub struct FramedClient<C, D> {
    connector: Arc<C>,
    config: HandshakeConfig,
    dispatcher: D,
    retry_policy: RetryPolicy,
    state: Arc<Mutex<Option<FramedClientState>>>,
}

impl<C, D> Clone for FramedClient<C, D>
where
    D: Clone,
{
    fn clone(&self) -> Self {
        Self {
            connector: self.connector.clone(),
            config: self.config.clone(),
            dispatcher: self.dispatcher.clone(),
            retry_policy: self.retry_policy.clone(),
            state: self.state.clone(),
        }
    }
}

impl<C, D> FramedClient<C, D>
where
    C: MessageConnector,
    D: ServiceDispatcher + Clone + 'static,
{
    /// Get the underlying handle if connected.
    pub async fn handle(&self) -> Result<ConnectionHandle, ConnectError> {
        self.ensure_connected().await
    }

    async fn ensure_connected(&self) -> Result<ConnectionHandle, ConnectError> {
        let mut state = self.state.lock().await;

        if let Some(ref conn) = *state {
            // Note: On WASM we can't detect dead connections via JoinHandle.
            // The connection will fail on next use if dead.
            return Ok(conn.handle.clone());
        }

        let conn = self.connect_internal().await?;
        let handle = conn.handle.clone();
        *state = Some(conn);
        Ok(handle)
    }

    async fn connect_internal(&self) -> Result<FramedClientState, ConnectError> {
        let transport = self
            .connector
            .connect()
            .await
            .map_err(ConnectError::ConnectFailed)?;

        let (handle, driver) = establish(
            transport,
            self.config.to_hello(),
            self.dispatcher.clone(),
            Role::Initiator,
        )
        .await
        .map_err(|e| ConnectError::ConnectFailed(connection_error_to_io(e)))?;

        // Spawn driver using runtime abstraction (works on native and WASM)
        spawn(async move {
            let _ = driver.run().await;
        });

        Ok(FramedClientState { handle })
    }

    /// Make a raw RPC call with automatic reconnection.
    pub async fn call_raw(
        &self,
        method_id: u64,
        payload: Vec<u8>,
    ) -> Result<Vec<u8>, ConnectError> {
        let mut last_error: Option<io::Error> = None;
        let mut attempt = 0u32;

        loop {
            let handle = match self.ensure_connected().await {
                Ok(h) => h,
                Err(ConnectError::ConnectFailed(e)) => {
                    attempt += 1;
                    if attempt >= self.retry_policy.max_attempts {
                        return Err(ConnectError::RetriesExhausted {
                            original: last_error.unwrap_or(e),
                            attempts: attempt,
                        });
                    }
                    last_error = Some(e);
                    let backoff = self.retry_policy.backoff_for_attempt(attempt);
                    sleep(backoff).await;
                    continue;
                }
                Err(e) => return Err(e),
            };

            match handle.call_raw(method_id, payload.clone()).await {
                Ok(response) => return Ok(response),
                Err(TransportError::Encode(e)) => {
                    return Err(ConnectError::Rpc(TransportError::Encode(e)));
                }
                Err(TransportError::ConnectionClosed) | Err(TransportError::DriverGone) => {
                    {
                        let mut state = self.state.lock().await;
                        *state = None;
                    }

                    attempt += 1;
                    if attempt >= self.retry_policy.max_attempts {
                        let error = last_error.unwrap_or_else(|| {
                            io::Error::new(io::ErrorKind::ConnectionReset, "connection closed")
                        });
                        return Err(ConnectError::RetriesExhausted {
                            original: error,
                            attempts: attempt,
                        });
                    }

                    last_error = Some(io::Error::new(
                        io::ErrorKind::ConnectionReset,
                        "connection closed",
                    ));
                    let backoff = self.retry_policy.backoff_for_attempt(attempt);
                    sleep(backoff).await;
                }
            }
        }
    }
}

impl<C, D> crate::Caller for FramedClient<C, D>
where
    C: MessageConnector,
    D: ServiceDispatcher + Clone + 'static,
{
    async fn call<T: Facet<'static>>(
        &self,
        method_id: u64,
        args: &mut T,
    ) -> Result<ResponseData, TransportError> {
        let mut attempt = 0u32;

        loop {
            let handle = match self.ensure_connected().await {
                Ok(h) => h,
                Err(ConnectError::ConnectFailed(_)) => {
                    attempt += 1;
                    if attempt >= self.retry_policy.max_attempts {
                        return Err(TransportError::ConnectionClosed);
                    }
                    let backoff = self.retry_policy.backoff_for_attempt(attempt);
                    sleep(backoff).await;
                    continue;
                }
                Err(ConnectError::RetriesExhausted { .. }) => {
                    return Err(TransportError::ConnectionClosed);
                }
                Err(ConnectError::Rpc(e)) => return Err(e),
                Err(ConnectError::Rejected(_)) => {
                    // Virtual connection rejected - this shouldn't happen for link-level connect
                    return Err(TransportError::ConnectionClosed);
                }
            };

            match handle.call(method_id, args).await {
                Ok(response) => return Ok(response),
                Err(TransportError::Encode(e)) => {
                    return Err(TransportError::Encode(e));
                }
                Err(TransportError::ConnectionClosed) | Err(TransportError::DriverGone) => {
                    {
                        let mut state = self.state.lock().await;
                        *state = None;
                    }

                    attempt += 1;
                    if attempt >= self.retry_policy.max_attempts {
                        return Err(TransportError::ConnectionClosed);
                    }

                    let backoff = self.retry_policy.backoff_for_attempt(attempt);
                    sleep(backoff).await;
                }
            }
        }
    }

    fn bind_response_streams<R: Facet<'static>>(&self, response: &mut R, channels: &[u64]) {
        // FramedClient wraps a ConnectionHandle, but we don't have direct access to it
        // during bind_response_streams. For reconnecting clients, response stream binding
        // would need to be handled at a higher level or the client would need to store
        // the current handle.
        // For now, this is a no-op - FramedClient users should use ConnectionHandle
        // directly if they need response stream binding.
        let _ = (response, channels);
    }
}

fn connection_error_to_io(e: ConnectionError) -> io::Error {
    match e {
        ConnectionError::Io(io_err) => io_err,
        ConnectionError::ProtocolViolation { rule_id, context } => io::Error::new(
            io::ErrorKind::InvalidData,
            format!("protocol violation: {rule_id}: {context}"),
        ),
        ConnectionError::Dispatch(msg) => io::Error::other(format!("dispatch error: {msg}")),
        ConnectionError::Closed => {
            io::Error::new(io::ErrorKind::ConnectionReset, "connection closed")
        }
    }
}

// ============================================================================
// Virtual Connection State
// ============================================================================

/// State for a single virtual connection on a link.
///
/// Each virtual connection has its own request ID space, channel ID space,
/// and dispatcher instance. Connection 0 (ROOT) is created implicitly on
/// link establishment. Additional connections are opened via Connect/Accept.
///
/// r[impl core.conn.independence]
struct ConnectionState {
    /// The connection ID.
    conn_id: ConnectionId,
    /// Client-side handle for making calls on this connection.
    handle: ConnectionHandle,
    /// Server-side channel registry for incoming Rx/Tx streams.
    server_channel_registry: ChannelRegistry,
    /// Pending responses (request_id -> response sender).
    pending_responses:
        HashMap<u64, crate::runtime::OneshotSender<Result<ResponseData, TransportError>>>,
    /// In-flight server requests (for duplicate detection).
    in_flight_server_requests: std::collections::HashSet<u64>,
}

impl ConnectionState {
    /// Create a new connection state.
    fn new(
        conn_id: ConnectionId,
        driver_tx: crate::runtime::Sender<DriverMessage>,
        role: Role,
        initial_credit: u32,
        diagnostic_state: Option<Arc<crate::diagnostic::DiagnosticState>>,
    ) -> Self {
        let handle = ConnectionHandle::new_with_diagnostics(
            conn_id,
            driver_tx.clone(),
            role,
            initial_credit,
            diagnostic_state,
        );
        let server_channel_registry = ChannelRegistry::new_with_credit(initial_credit, driver_tx);
        Self {
            conn_id,
            handle,
            server_channel_registry,
            pending_responses: HashMap::new(),
            in_flight_server_requests: std::collections::HashSet::new(),
        }
    }

    /// Fail all pending responses (connection closing).
    fn fail_pending_responses(&mut self) {
        for (_, tx) in self.pending_responses.drain() {
            let _ = tx.send(Err(TransportError::ConnectionClosed));
        }
    }
}

/// An incoming virtual connection request.
///
/// Received via `take_incoming_connections()` on connection 0.
/// Call `accept()` to accept the connection and get a handle,
/// or `reject()` to refuse it.
pub struct IncomingConnection {
    /// The request ID for this Connect request.
    request_id: u64,
    /// Metadata from the Connect message.
    pub metadata: roam_wire::Metadata,
    /// Channel to send the Accept/Reject response.
    response_tx: crate::runtime::OneshotSender<IncomingConnectionResponse>,
}

impl IncomingConnection {
    /// Accept this connection and receive a handle for it.
    ///
    /// The `metadata` will be sent in the Accept message.
    pub async fn accept(
        self,
        metadata: roam_wire::Metadata,
    ) -> Result<ConnectionHandle, TransportError> {
        let (handle_tx, handle_rx) = crate::runtime::oneshot();
        let _ = self.response_tx.send(IncomingConnectionResponse::Accept {
            request_id: self.request_id,
            metadata,
            handle_tx,
        });
        handle_rx.await.map_err(|_| TransportError::DriverGone)?
    }

    /// Reject this connection with a reason.
    pub fn reject(self, reason: String, metadata: roam_wire::Metadata) {
        let _ = self.response_tx.send(IncomingConnectionResponse::Reject {
            request_id: self.request_id,
            reason,
            metadata,
        });
    }
}

/// Internal response for incoming connection handling.
enum IncomingConnectionResponse {
    Accept {
        request_id: u64,
        metadata: roam_wire::Metadata,
        handle_tx: crate::runtime::OneshotSender<Result<ConnectionHandle, TransportError>>,
    },
    Reject {
        request_id: u64,
        reason: String,
        metadata: roam_wire::Metadata,
    },
}

/// Pending outgoing Connect request.
struct PendingConnect {
    response_tx: crate::runtime::OneshotSender<Result<ConnectionHandle, ConnectError>>,
}

// ============================================================================
// Driver - The core connection loop
// ============================================================================

/// The connection driver - a future that handles bidirectional RPC.
///
/// This must be spawned or awaited to drive the connection forward.
///
/// The driver manages multiple virtual connections on a single link.
/// Connection 0 (ROOT) is created implicitly. Additional connections
/// can be opened via `Connect`/`Accept` messages.
pub struct Driver<T, D> {
    io: T,
    dispatcher: D,
    #[allow(dead_code)]
    role: Role,
    negotiated: Negotiated,
    /// Unified channel for all messages (Call/Data/Close/Response).
    driver_rx: Receiver<DriverMessage>,
    /// Sender for driver messages (passed to new connections).
    driver_tx: crate::runtime::Sender<DriverMessage>,
    /// All virtual connections on this link, keyed by conn_id.
    connections: HashMap<ConnectionId, ConnectionState>,
    /// Next connection ID to allocate (for Accept responses).
    /// r[impl core.conn.id-allocation]
    next_conn_id: u64,
    /// Pending outgoing Connect requests (request_id -> response channel).
    pending_connects: HashMap<u64, PendingConnect>,
    /// Next Connect request ID.
    next_connect_request_id: u64,
    /// Channel for incoming connection requests (only root can accept).
    /// r[impl core.conn.accept-required]
    incoming_connections_tx: Option<crate::runtime::Sender<IncomingConnection>>,
    /// Channel for incoming connection responses.
    incoming_response_rx: Option<Receiver<IncomingConnectionResponse>>,
    incoming_response_tx: crate::runtime::Sender<IncomingConnectionResponse>,
    /// Diagnostic state for debugging.
    diagnostic_state: Option<Arc<crate::diagnostic::DiagnosticState>>,
}

impl<T, D> Driver<T, D>
where
    T: MessageTransport,
    D: ServiceDispatcher,
{
    /// Get the handle for the root connection (connection 0).
    ///
    /// This is the main handle returned from `establish()` and should be used
    /// for most operations. Virtual connections can be obtained via `connect()`.
    pub fn root_handle(&self) -> ConnectionHandle {
        self.connections
            .get(&ConnectionId::ROOT)
            .expect("root connection always exists")
            .handle
            .clone()
    }

    /// Start accepting incoming virtual connections.
    ///
    /// Returns a receiver that yields `IncomingConnection` for each `Connect`
    /// request received from the remote peer. Call `accept()` or `reject()`
    /// on each incoming connection.
    ///
    /// r[impl core.conn.accept-required]
    ///
    /// This can only be called once. Subsequent calls will panic.
    /// Only the root connection can accept incoming connections.
    ///
    /// # Example
    ///
    /// ```ignore
    /// let mut incoming = driver.take_incoming_connections();
    ///
    /// // In a separate task:
    /// while let Some(conn) = incoming.recv().await {
    ///     // Accept all incoming connections
    ///     let handle = conn.accept(vec![]).await?;
    ///     // Use handle...
    /// }
    /// ```
    pub fn take_incoming_connections(&mut self) -> Receiver<IncomingConnection> {
        if self.incoming_connections_tx.is_some() {
            panic!("take_incoming_connections() can only be called once");
        }
        let (tx, rx) = channel(64);
        self.incoming_connections_tx = Some(tx);
        rx
    }

    /// Run the driver until the connection closes.
    pub async fn run(mut self) -> Result<(), ConnectionError> {
        use futures_util::FutureExt;

        loop {
            futures_util::select! {
                msg = self.driver_rx.recv().fuse() => {
                    if let Some(msg) = msg {
                        self.handle_driver_message(msg).await?;
                    }
                }

                // Handle incoming connection accept/reject responses
                response = async {
                    if let Some(rx) = &mut self.incoming_response_rx {
                        rx.recv().await
                    } else {
                        std::future::pending().await
                    }
                }.fuse() => {
                    if let Some(response) = response {
                        self.handle_incoming_response(response).await?;
                    }
                }

                result = self.io.recv().fuse() => {
                    match self.handle_recv(result).await {
                        Ok(true) => continue,
                        Ok(false) => return Ok(()),
                        Err(e) => return Err(e),
                    }
                }
            }
        }
    }

    /// Handle an Accept/Reject response from application code.
    async fn handle_incoming_response(
        &mut self,
        response: IncomingConnectionResponse,
    ) -> Result<(), ConnectionError> {
        match response {
            IncomingConnectionResponse::Accept {
                request_id,
                metadata,
                handle_tx,
            } => {
                // Allocate a new connection ID
                // r[impl core.conn.id-allocation]
                let conn_id = ConnectionId::new(self.next_conn_id);
                self.next_conn_id += 1;

                // Create connection state
                let conn_state = ConnectionState::new(
                    conn_id,
                    self.driver_tx.clone(),
                    self.role,
                    self.negotiated.initial_credit,
                    self.diagnostic_state.clone(),
                );
                let handle = conn_state.handle.clone();
                self.connections.insert(conn_id, conn_state);

                // Send Accept message
                let msg = Message::Accept {
                    request_id,
                    conn_id,
                    metadata,
                };
                self.io.send(&msg).await?;

                // Return the handle to the caller
                let _ = handle_tx.send(Ok(handle));
            }
            IncomingConnectionResponse::Reject {
                request_id,
                reason,
                metadata,
            } => {
                let msg = Message::Reject {
                    request_id,
                    reason,
                    metadata,
                };
                self.io.send(&msg).await?;
            }
        }
        Ok(())
    }

    async fn handle_driver_message(&mut self, msg: DriverMessage) -> Result<(), ConnectionError> {
        match msg {
            DriverMessage::Call {
                conn_id,
                request_id,
                method_id,
                metadata,
                channels,
                payload,
                response_tx,
            } => {
                // Store pending response in the connection's state
                if let Some(conn) = self.connections.get_mut(&conn_id) {
                    conn.pending_responses.insert(request_id, response_tx);
                } else {
                    // Unknown connection - fail the call
                    let _ = response_tx.send(Err(TransportError::ConnectionClosed));
                    return Ok(());
                }
                let req = Message::Request {
                    conn_id,
                    request_id,
                    method_id,
                    metadata,
                    channels,
                    payload,
                };
                self.io.send(&req).await?;
            }
            DriverMessage::Data {
                conn_id,
                channel_id,
                payload,
            } => {
                let wire_msg = Message::Data {
                    conn_id,
                    channel_id,
                    payload,
                };
                self.io.send(&wire_msg).await?;
            }
            DriverMessage::Close {
                conn_id,
                channel_id,
            } => {
                let wire_msg = Message::Close {
                    conn_id,
                    channel_id,
                };
                self.io.send(&wire_msg).await?;
            }
            DriverMessage::Response {
                conn_id,
                request_id,
                channels,
                payload,
            } => {
                // Check that the request is in-flight for this connection
                let should_send = if let Some(conn) = self.connections.get_mut(&conn_id) {
                    conn.in_flight_server_requests.remove(&request_id)
                } else {
                    false
                };
                if !should_send {
                    return Ok(());
                }
                let wire_msg = Message::Response {
                    conn_id,
                    request_id,
                    metadata: vec![],
                    channels,
                    payload,
                };
                self.io.send(&wire_msg).await?;
            }
            DriverMessage::Connect {
                request_id,
                metadata,
                response_tx,
            } => {
                // Store pending connect request
                self.pending_connects
                    .insert(request_id, PendingConnect { response_tx });
                // Send Connect message
                let wire_msg = Message::Connect {
                    request_id,
                    metadata,
                };
                self.io.send(&wire_msg).await?;
            }
        }
        Ok(())
    }

    async fn handle_recv(
        &mut self,
        result: std::io::Result<Option<Message>>,
    ) -> Result<bool, ConnectionError> {
        let msg = match result {
            Ok(Some(m)) => m,
            Ok(None) => return Ok(false),
            Err(e) => {
                let raw = self.io.last_decoded();
                if raw.len() >= 2 && raw[0] == 0x00 && raw[1] != 0x00 {
                    return Err(self.goodbye("message.hello.unknown-version").await);
                }
                if !raw.is_empty() && raw[0] >= 12 {
                    return Err(self.goodbye("message.unknown-variant").await);
                }
                if e.kind() == std::io::ErrorKind::InvalidData {
                    return Err(self.goodbye("message.decode-error").await);
                }
                return Err(ConnectionError::Io(e));
            }
        };

        match self.handle_message(msg).await {
            Ok(()) => Ok(true),
            Err(ConnectionError::Closed) => Ok(false),
            Err(e) => Err(e),
        }
    }

    async fn handle_message(&mut self, msg: Message) -> Result<(), ConnectionError> {
        match msg {
            Message::Hello(_) => {
                // Already handled during handshake, ignore duplicates
            }
            Message::Connect {
                request_id,
                metadata,
            } => {
                // r[impl core.conn.accept-required]
                // Only root connection can accept incoming connections
                if let Some(tx) = &self.incoming_connections_tx {
                    // Create a oneshot that routes through incoming_response_tx
                    let (response_tx, response_rx) = crate::runtime::oneshot();
                    let incoming = IncomingConnection {
                        request_id,
                        metadata,
                        response_tx,
                    };
                    if tx.try_send(incoming).is_ok() {
                        // Spawn a task to forward the response
                        let incoming_response_tx = self.incoming_response_tx.clone();
                        spawn(async move {
                            if let Ok(response) = response_rx.await {
                                let _ = incoming_response_tx.send(response).await;
                            }
                        });
                    } else {
                        // Channel full or closed - reject
                        let msg = Message::Reject {
                            request_id,
                            reason: "not listening".into(),
                            metadata: vec![],
                        };
                        self.io.send(&msg).await?;
                    }
                } else {
                    // Not listening - reject
                    // r[impl message.reject.response]
                    let msg = Message::Reject {
                        request_id,
                        reason: "not listening".into(),
                        metadata: vec![],
                    };
                    self.io.send(&msg).await?;
                }
            }
            Message::Accept {
                request_id,
                conn_id,
                metadata: _,
            } => {
                // Handle response to our outgoing Connect request
                if let Some(pending) = self.pending_connects.remove(&request_id) {
                    // Create connection state for the new virtual connection
                    let conn_state = ConnectionState::new(
                        conn_id,
                        self.driver_tx.clone(),
                        self.role,
                        self.negotiated.initial_credit,
                        self.diagnostic_state.clone(),
                    );
                    let handle = conn_state.handle.clone();
                    self.connections.insert(conn_id, conn_state);
                    let _ = pending.response_tx.send(Ok(handle));
                }
                // Unknown request_id - ignore (may be late/duplicate)
            }
            Message::Reject {
                request_id,
                reason,
                metadata: _,
            } => {
                // Handle rejection of our outgoing Connect request
                if let Some(pending) = self.pending_connects.remove(&request_id) {
                    let _ = pending
                        .response_tx
                        .send(Err(ConnectError::Rejected(reason)));
                }
                // Unknown request_id - ignore
            }
            Message::Goodbye { conn_id, reason: _ } => {
                // r[impl message.goodbye.connection-zero]
                if conn_id.is_root() {
                    // Goodbye on root closes entire link
                    for (_, mut conn) in self.connections.drain() {
                        conn.fail_pending_responses();
                    }
                    return Err(ConnectionError::Closed);
                } else {
                    // Close just this virtual connection
                    // r[impl core.conn.lifecycle]
                    if let Some(mut conn) = self.connections.remove(&conn_id) {
                        conn.fail_pending_responses();
                    }
                }
            }
            Message::Request {
                conn_id,
                request_id,
                method_id,
                metadata,
                channels,
                payload,
            } => {
                self.handle_incoming_request(
                    conn_id, request_id, method_id, metadata, channels, payload,
                )
                .await?;
            }
            Message::Response {
                conn_id,
                request_id,
                channels,
                payload,
                ..
            } => {
                // Route to the correct connection
                if let Some(conn) = self.connections.get_mut(&conn_id) {
                    if let Some(tx) = conn.pending_responses.remove(&request_id) {
                        let _ = tx.send(Ok(ResponseData { payload, channels }));
                    }
                }
                // Unknown conn_id or request_id - ignore
            }
            Message::Cancel {
                conn_id: _,
                request_id: _,
            } => {
                // TODO: Implement cancellation
            }
            Message::Data {
                conn_id,
                channel_id,
                payload,
            } => {
                self.handle_data(conn_id, channel_id, payload).await?;
            }
            Message::Close {
                conn_id,
                channel_id,
            } => {
                self.handle_close(conn_id, channel_id).await?;
            }
            Message::Reset {
                conn_id,
                channel_id,
            } => {
                self.handle_reset(conn_id, channel_id)?;
            }
            Message::Credit {
                conn_id,
                channel_id,
                bytes,
            } => {
                self.handle_credit(conn_id, channel_id, bytes)?;
            }
        }
        Ok(())
    }

    async fn handle_incoming_request(
        &mut self,
        conn_id: ConnectionId,
        request_id: u64,
        method_id: u64,
        metadata: Vec<(String, roam_wire::MetadataValue)>,
        channels: Vec<u64>,
        payload: Vec<u8>,
    ) -> Result<(), ConnectionError> {
        // Get or validate the connection
        let conn = match self.connections.get_mut(&conn_id) {
            Some(c) => c,
            None => {
                // r[impl message.conn-id] - Unknown conn_id is a protocol error
                return Err(self.goodbye("message.conn-id").await);
            }
        };

        // r[impl call.request-id.duplicate-detection]
        if !conn.in_flight_server_requests.insert(request_id) {
            return Err(self.goodbye("call.request-id.duplicate-detection").await);
        }

        if let Err(rule_id) = roam_wire::validate_metadata(&metadata) {
            conn.in_flight_server_requests.remove(&request_id);
            return Err(self.goodbye(rule_id).await);
        }

        if payload.len() as u32 > self.negotiated.max_payload_size {
            conn.in_flight_server_requests.remove(&request_id);
            return Err(self.goodbye("flow.call.payload-limit").await);
        }

        let handler_fut = self.dispatcher.dispatch(
            conn_id,
            method_id,
            payload,
            channels,
            request_id,
            &mut conn.server_channel_registry,
        );
        spawn(async move {
            handler_fut.await;
        });
        Ok(())
    }

    async fn handle_data(
        &mut self,
        conn_id: ConnectionId,
        channel_id: u64,
        payload: Vec<u8>,
    ) -> Result<(), ConnectionError> {
        if channel_id == 0 {
            return Err(self.goodbye("channeling.id.zero-reserved").await);
        }

        if payload.len() as u32 > self.negotiated.max_payload_size {
            return Err(self.goodbye("flow.call.payload-limit").await);
        }

        // Find the connection and route data
        let conn = match self.connections.get_mut(&conn_id) {
            Some(c) => c,
            None => return Err(self.goodbye("message.conn-id").await),
        };

        let result = if conn.server_channel_registry.contains_incoming(channel_id) {
            conn.server_channel_registry
                .route_data(channel_id, payload)
                .await
        } else if conn.handle.contains_channel(channel_id) {
            conn.handle.route_data(channel_id, payload).await
        } else {
            Err(ChannelError::Unknown)
        };

        match result {
            Ok(()) => Ok(()),
            Err(ChannelError::Unknown) => Err(self.goodbye("channeling.unknown").await),
            Err(ChannelError::DataAfterClose) => {
                Err(self.goodbye("channeling.data-after-close").await)
            }
            Err(ChannelError::CreditOverrun) => {
                Err(self.goodbye("flow.channel.credit-overrun").await)
            }
        }
    }

    async fn handle_close(
        &mut self,
        conn_id: ConnectionId,
        channel_id: u64,
    ) -> Result<(), ConnectionError> {
        if channel_id == 0 {
            return Err(self.goodbye("channeling.id.zero-reserved").await);
        }

        let conn = match self.connections.get_mut(&conn_id) {
            Some(c) => c,
            None => return Err(self.goodbye("message.conn-id").await),
        };

        if conn.server_channel_registry.contains(channel_id) {
            conn.server_channel_registry.close(channel_id);
        } else if conn.handle.contains_channel(channel_id) {
            conn.handle.close_channel(channel_id);
        } else {
            return Err(self.goodbye("channeling.unknown").await);
        }
        Ok(())
    }

    fn handle_reset(
        &mut self,
        conn_id: ConnectionId,
        channel_id: u64,
    ) -> Result<(), ConnectionError> {
        if let Some(conn) = self.connections.get_mut(&conn_id) {
            if conn.server_channel_registry.contains(channel_id) {
                conn.server_channel_registry.reset(channel_id);
            } else if conn.handle.contains_channel(channel_id) {
                conn.handle.reset_channel(channel_id);
            }
        }
        Ok(())
    }

    fn handle_credit(
        &mut self,
        conn_id: ConnectionId,
        channel_id: u64,
        bytes: u32,
    ) -> Result<(), ConnectionError> {
        if let Some(conn) = self.connections.get_mut(&conn_id) {
            if conn.server_channel_registry.contains(channel_id) {
                conn.server_channel_registry
                    .receive_credit(channel_id, bytes);
            } else if conn.handle.contains_channel(channel_id) {
                conn.handle.receive_credit(channel_id, bytes);
            }
        }
        Ok(())
    }

    async fn goodbye(&mut self, rule_id: &'static str) -> ConnectionError {
        // Fail all pending responses on all connections
        for (_, conn) in self.connections.iter_mut() {
            conn.fail_pending_responses();
        }

        let _ = self
            .io
            .send(&Message::Goodbye {
                conn_id: ConnectionId::ROOT,
                reason: rule_id.into(),
            })
            .await;

        ConnectionError::ProtocolViolation {
            rule_id,
            context: String::new(),
        }
    }
}

// ============================================================================
// initiate_framed() - For initiator role
// ============================================================================

/// Initiate a connection with a pre-framed transport (e.g., WebSocket).
///
/// Use this when establishing a connection as the initiator (client).
/// Returns a handle for making calls and a driver that must be spawned.
pub async fn initiate_framed<T, D>(
    transport: T,
    config: HandshakeConfig,
    dispatcher: D,
) -> Result<(ConnectionHandle, Driver<T, D>), ConnectionError>
where
    T: MessageTransport,
    D: ServiceDispatcher,
{
    establish(transport, config.to_hello(), dispatcher, Role::Initiator).await
}

// ============================================================================
// establish() - Perform handshake and create driver (internal)
// ============================================================================

async fn establish<T, D>(
    mut io: T,
    our_hello: Hello,
    dispatcher: D,
    role: Role,
) -> Result<(ConnectionHandle, Driver<T, D>), ConnectionError>
where
    T: MessageTransport,
    D: ServiceDispatcher,
{
    // Send Hello
    io.send(&Message::Hello(our_hello.clone())).await?;

    // Wait for peer Hello with timeout
    let peer_hello = match io.recv_timeout(Duration::from_secs(5)).await {
        Ok(Some(Message::Hello(h))) => h,
        Ok(Some(_)) => {
            let _ = io
                .send(&Message::Goodbye {
                    conn_id: ConnectionId::ROOT,
                    reason: "message.hello.ordering".into(),
                })
                .await;
            return Err(ConnectionError::ProtocolViolation {
                rule_id: "message.hello.ordering",
                context: "received non-Hello before Hello exchange".into(),
            });
        }
        Ok(None) => return Err(ConnectionError::Closed),
        Err(e) => {
            let raw = io.last_decoded();
            let is_unknown_hello = raw.len() >= 2 && raw[0] == 0x00 && raw[1] != 0x00;
            let version = if is_unknown_hello { raw[1] } else { 0 };

            if is_unknown_hello {
                let _ = io
                    .send(&Message::Goodbye {
                        conn_id: ConnectionId::ROOT,
                        reason: "message.hello.unknown-version".into(),
                    })
                    .await;
                return Err(ConnectionError::ProtocolViolation {
                    rule_id: "message.hello.unknown-version",
                    context: format!("unknown Hello version: {version}"),
                });
            }
            return Err(ConnectionError::Io(e));
        }
    };

    // Negotiate parameters
    let (our_max, our_credit) = match &our_hello {
        Hello::V1 {
            max_payload_size,
            initial_channel_credit,
        }
        | Hello::V2 {
            max_payload_size,
            initial_channel_credit,
        } => (*max_payload_size, *initial_channel_credit),
    };
    let (peer_max, peer_credit) = match &peer_hello {
        Hello::V1 {
            max_payload_size,
            initial_channel_credit,
        }
        | Hello::V2 {
            max_payload_size,
            initial_channel_credit,
        } => (*max_payload_size, *initial_channel_credit),
    };

    let negotiated = Negotiated {
        max_payload_size: our_max.min(peer_max),
        initial_credit: our_credit.min(peer_credit),
    };

    // Create unified channel for all messages
    let (driver_tx, driver_rx) = channel(256);

    // Create the root connection (connection 0)
    // r[impl core.link.connection-zero]
    let root_conn = ConnectionState::new(
        ConnectionId::ROOT,
        driver_tx.clone(),
        role,
        negotiated.initial_credit,
        None, // No diagnostic state by default
    );
    let handle = root_conn.handle.clone();

    let mut connections = HashMap::new();
    connections.insert(ConnectionId::ROOT, root_conn);

    // Create channel for incoming connection responses (Accept/Reject from app code)
    let (incoming_response_tx, incoming_response_rx) = channel(64);

    let driver = Driver {
        io,
        dispatcher,
        role,
        negotiated: negotiated.clone(),
        driver_rx,
        driver_tx,
        connections,
        next_conn_id: 1, // 0 is ROOT, start allocating at 1
        pending_connects: HashMap::new(),
        next_connect_request_id: 1,
        incoming_connections_tx: None, // Not listening by default
        incoming_response_rx: Some(incoming_response_rx),
        incoming_response_tx,
        diagnostic_state: None,
    };

    Ok((handle, driver))
}

// ============================================================================
// NoDispatcher - For client-only connections
// ============================================================================

/// A no-op dispatcher for client-only connections.
///
/// Returns UnknownMethod for all requests since we don't serve any methods.
pub struct NoDispatcher;

impl ServiceDispatcher for NoDispatcher {
    fn method_ids(&self) -> Vec<u64> {
        vec![]
    }

    fn dispatch(
        &self,
        conn_id: roam_wire::ConnectionId,
        _method_id: u64,
        _payload: Vec<u8>,
        _channels: Vec<u64>,
        request_id: u64,
        registry: &mut ChannelRegistry,
    ) -> Pin<Box<dyn Future<Output = ()> + Send + 'static>> {
        let driver_tx = registry.driver_tx();
        Box::pin(async move {
            let response: Result<(), RoamError<()>> = Err(RoamError::UnknownMethod);
            let payload = facet_postcard::to_vec(&response).unwrap_or_default();
            let _ = driver_tx
                .send(DriverMessage::Response {
                    conn_id,
                    request_id,
                    channels: Vec::new(),
                    payload,
                })
                .await;
        })
    }
}

impl Clone for NoDispatcher {
    fn clone(&self) -> Self {
        NoDispatcher
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_backoff_calculation() {
        let policy = RetryPolicy::default();
        assert_eq!(policy.backoff_for_attempt(1), Duration::from_millis(100));
        assert_eq!(policy.backoff_for_attempt(2), Duration::from_millis(200));
        assert_eq!(policy.backoff_for_attempt(3), Duration::from_millis(400));
        assert_eq!(policy.backoff_for_attempt(10), Duration::from_secs(5));
    }
}
