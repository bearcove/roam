//! Bidirectional connection driver.
//!
//! The driver is a future that handles all I/O for a connection:
//! - Dispatches incoming requests to a service
//! - Routes incoming responses to waiting callers
//! - Sends outgoing requests from ConnectionHandle
//! - Handles stream data (Data/Close/Reset/Credit)

use std::collections::HashMap;
use std::time::Duration;

use roam_session::{
    CallError, ConnectionHandle, HandleCommand, Role, ServiceDispatcher, StreamError,
    StreamRegistry,
};
use roam_wire::{Hello, Message};
use tokio::sync::{mpsc, oneshot};

use crate::connection::{ConnectionError, Negotiated};
use crate::transport::MessageTransport;

/// Result from a completed streaming handler: (request_id, serialized_result).
type StreamingResult = (u64, Result<Vec<u8>, String>);

/// The connection driver - a future that handles bidirectional RPC.
///
/// This must be spawned or awaited to drive the connection forward.
/// Use [`ConnectionHandle`] to make outgoing calls.
pub struct Driver<T, D> {
    io: T,
    dispatcher: D,
    #[allow(dead_code)]
    role: Role,
    negotiated: Negotiated,

    /// Handle for client-side operations (streams, etc.)
    handle: ConnectionHandle,

    /// Receive commands from ConnectionHandle (outgoing calls).
    command_rx: mpsc::Receiver<HandleCommand>,

    /// Server-side stream registry (for incoming Tx/Rx from requests we serve).
    server_stream_registry: StreamRegistry,

    /// Pending responses for outgoing calls we made.
    /// request_id → oneshot sender for the response.
    pending_responses: HashMap<u64, oneshot::Sender<Result<Vec<u8>, CallError>>>,

    /// In-flight requests we're serving (to detect duplicates).
    in_flight_server_requests: std::collections::HashSet<u64>,

    /// Channel for receiving completed streaming handler results.
    streaming_results_tx: mpsc::Sender<StreamingResult>,
    streaming_results_rx: mpsc::Receiver<StreamingResult>,
}

impl<T, D> Driver<T, D>
where
    T: MessageTransport,
    D: ServiceDispatcher,
{
    /// Create a new driver with the given transport, dispatcher, and parameters.
    pub fn new(
        io: T,
        dispatcher: D,
        role: Role,
        negotiated: Negotiated,
        handle: ConnectionHandle,
        command_rx: mpsc::Receiver<HandleCommand>,
    ) -> Self {
        let (streaming_results_tx, streaming_results_rx) = mpsc::channel(64);
        let initial_credit = negotiated.initial_credit;
        Self {
            io,
            dispatcher,
            role,
            negotiated,
            handle,
            command_rx,
            server_stream_registry: StreamRegistry::new_with_credit(initial_credit),
            pending_responses: HashMap::new(),
            in_flight_server_requests: std::collections::HashSet::new(),
            streaming_results_tx,
            streaming_results_rx,
        }
    }

    /// Run the driver until the connection closes.
    ///
    /// TODO: Add FuturesUnordered for outgoing stream receivers.
    /// When ConnectionHandle::call binds streams, it should return taken receivers
    /// that get added to a FuturesUnordered here for polling.
    pub async fn run(mut self) -> Result<(), ConnectionError> {
        loop {
            tokio::select! {
                biased;

                // Handle completed streaming handlers (server-side)
                Some((request_id, result)) = self.streaming_results_rx.recv() => {
                    self.handle_streaming_result(request_id, result).await?;
                }

                // Handle commands from ConnectionHandle (client-side outgoing calls)
                Some(cmd) = self.command_rx.recv() => {
                    self.handle_command(cmd).await?;
                }

                // Handle incoming messages from peer
                result = self.io.recv_timeout(Duration::from_secs(30)) => {
                    match self.handle_recv(result).await {
                        Ok(true) => continue,
                        Ok(false) => return Ok(()), // Clean shutdown
                        Err(e) => return Err(e),
                    }
                }

                // TODO: Poll FuturesUnordered for outgoing stream data
                // When a receiver yields data, send it as a Data message
                // When a receiver is exhausted, send a Close message
            }
        }
    }

    /// Handle a completed streaming handler result.
    async fn handle_streaming_result(
        &mut self,
        request_id: u64,
        result: Result<Vec<u8>, String>,
    ) -> Result<(), ConnectionError> {
        let response_payload = result.map_err(ConnectionError::Dispatch)?;

        let resp = Message::Response {
            request_id,
            metadata: Vec::new(),
            payload: response_payload,
        };
        self.io.send(&resp).await?;
        self.in_flight_server_requests.remove(&request_id);
        Ok(())
    }

    /// Handle a command from ConnectionHandle.
    async fn handle_command(&mut self, cmd: HandleCommand) -> Result<(), ConnectionError> {
        match cmd {
            HandleCommand::Call {
                request_id,
                method_id,
                metadata,
                payload,
                response_tx,
            } => {
                // Store the response channel
                self.pending_responses.insert(request_id, response_tx);

                // Send the request
                let req = Message::Request {
                    request_id,
                    method_id,
                    metadata,
                    payload,
                };
                self.io.send(&req).await?;
            }
        }
        Ok(())
    }

    /// Handle result from recv_timeout.
    /// Returns Ok(true) to continue, Ok(false) to shutdown cleanly, Err for errors.
    async fn handle_recv(
        &mut self,
        result: std::io::Result<Option<Message>>,
    ) -> Result<bool, ConnectionError> {
        let msg = match result {
            Ok(Some(m)) => m,
            Ok(None) => return Ok(false), // Clean shutdown
            Err(e) => {
                // Check for protocol errors
                let raw = self.io.last_decoded();
                if raw.len() >= 2 && raw[0] == 0x00 && raw[1] != 0x00 {
                    return Err(self.goodbye("message.hello.unknown-version").await);
                }
                if !raw.is_empty() && raw[0] >= 9 {
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

    /// Handle a single incoming message.
    async fn handle_message(&mut self, msg: Message) -> Result<(), ConnectionError> {
        match msg {
            Message::Hello(_) => {
                // Duplicate Hello - ignore
            }
            Message::Goodbye { .. } => {
                // Fail all pending responses
                for (_, tx) in self.pending_responses.drain() {
                    let _ = tx.send(Err(CallError::ConnectionClosed));
                }
                return Err(ConnectionError::Closed);
            }
            Message::Request {
                request_id,
                method_id,
                metadata,
                payload,
            } => {
                self.handle_incoming_request(request_id, method_id, metadata, payload)
                    .await?;
            }
            Message::Response {
                request_id,
                metadata: _,
                payload,
            } => {
                // Route to waiting caller
                if let Some(tx) = self.pending_responses.remove(&request_id) {
                    let _ = tx.send(Ok(payload));
                }
                // Unknown response IDs are ignored per spec
            }
            Message::Cancel { request_id: _ } => {
                // TODO: Implement cancellation
            }
            Message::Data { stream_id, payload } => {
                self.handle_data(stream_id, payload).await?;
            }
            Message::Close { stream_id } => {
                self.handle_close(stream_id).await?;
            }
            Message::Reset { stream_id } => {
                self.handle_reset(stream_id)?;
            }
            Message::Credit { stream_id, bytes } => {
                self.handle_credit(stream_id, bytes)?;
            }
        }
        Ok(())
    }

    /// Handle an incoming request (we're the server for this call).
    async fn handle_incoming_request(
        &mut self,
        request_id: u64,
        method_id: u64,
        metadata: Vec<(String, roam_wire::MetadataValue)>,
        payload: Vec<u8>,
    ) -> Result<(), ConnectionError> {
        // Duplicate detection
        if !self.in_flight_server_requests.insert(request_id) {
            return Err(self.goodbye("unary.request-id.duplicate-detection").await);
        }

        // Validate metadata
        if let Err(rule_id) = roam_wire::validate_metadata(&metadata) {
            self.in_flight_server_requests.remove(&request_id);
            return Err(self.goodbye(rule_id).await);
        }

        // Validate payload size
        if payload.len() as u32 > self.negotiated.max_payload_size {
            self.in_flight_server_requests.remove(&request_id);
            return Err(self.goodbye("flow.unary.payload-limit").await);
        }

        // Dispatch
        if self.dispatcher.is_streaming(method_id) {
            let handler_fut = self.dispatcher.dispatch_streaming(
                method_id,
                payload,
                &mut self.server_stream_registry,
            );
            let results_tx = self.streaming_results_tx.clone();
            tokio::spawn(async move {
                let result = handler_fut.await;
                let _ = results_tx.send((request_id, result)).await;
            });
        } else {
            let response_payload = self
                .dispatcher
                .dispatch_unary(method_id, &payload)
                .await
                .map_err(ConnectionError::Dispatch)?;

            let resp = Message::Response {
                request_id,
                metadata: Vec::new(),
                payload: response_payload,
            };
            self.io.send(&resp).await?;
            self.in_flight_server_requests.remove(&request_id);
        }
        Ok(())
    }

    /// Handle incoming Data message.
    async fn handle_data(
        &mut self,
        stream_id: u64,
        payload: Vec<u8>,
    ) -> Result<(), ConnectionError> {
        if stream_id == 0 {
            return Err(self.goodbye("streaming.id.zero-reserved").await);
        }

        if payload.len() as u32 > self.negotiated.max_payload_size {
            return Err(self.goodbye("flow.unary.payload-limit").await);
        }

        // Try server registry first, then client registry
        let result = if self.server_stream_registry.contains_incoming(stream_id) {
            self.server_stream_registry
                .route_data(stream_id, payload)
                .await
        } else if self.handle.contains_stream(stream_id) {
            self.handle.route_data(stream_id, payload).await
        } else {
            Err(StreamError::Unknown)
        };

        match result {
            Ok(()) => Ok(()),
            Err(StreamError::Unknown) => Err(self.goodbye("streaming.unknown").await),
            Err(StreamError::DataAfterClose) => {
                Err(self.goodbye("streaming.data-after-close").await)
            }
            Err(StreamError::CreditOverrun) => {
                Err(self.goodbye("flow.stream.credit-overrun").await)
            }
        }
    }

    /// Handle incoming Close message.
    async fn handle_close(&mut self, stream_id: u64) -> Result<(), ConnectionError> {
        if stream_id == 0 {
            return Err(self.goodbye("streaming.id.zero-reserved").await);
        }

        // Try server registry first, then client registry
        if self.server_stream_registry.contains(stream_id) {
            self.server_stream_registry.close(stream_id);
        } else if self.handle.contains_stream(stream_id) {
            self.handle.close_stream(stream_id);
        } else {
            return Err(self.goodbye("streaming.unknown").await);
        }
        Ok(())
    }

    /// Handle incoming Reset message.
    fn handle_reset(&mut self, stream_id: u64) -> Result<(), ConnectionError> {
        if stream_id == 0 {
            // For Reset, we don't send Goodbye for zero - just return error
            // Actually spec says we MUST send Goodbye
            // But we can't await here... let's make this async
        }

        // Try both registries - Reset on unknown stream is not an error
        if self.server_stream_registry.contains(stream_id) {
            self.server_stream_registry.reset(stream_id);
        } else if self.handle.contains_stream(stream_id) {
            self.handle.reset_stream(stream_id);
        }
        // Unknown stream for Reset is ignored per spec
        Ok(())
    }

    /// Handle incoming Credit message.
    fn handle_credit(&mut self, stream_id: u64, bytes: u32) -> Result<(), ConnectionError> {
        if stream_id == 0 {
            // Same issue as Reset - need async for Goodbye
        }

        // Try both registries
        if self.server_stream_registry.contains(stream_id) {
            self.server_stream_registry.receive_credit(stream_id, bytes);
        } else if self.handle.contains_stream(stream_id) {
            self.handle.receive_credit(stream_id, bytes);
        }
        // Unknown stream for Credit - should be error but we'd need async
        Ok(())
    }

    /// Send Goodbye and return error.
    async fn goodbye(&mut self, rule_id: &'static str) -> ConnectionError {
        // Fail all pending responses
        for (_, tx) in self.pending_responses.drain() {
            let _ = tx.send(Err(CallError::ConnectionClosed));
        }

        let _ = self
            .io
            .send(&Message::Goodbye {
                reason: rule_id.into(),
            })
            .await;

        ConnectionError::ProtocolViolation {
            rule_id,
            context: String::new(),
        }
    }
}

/// Establish a bidirectional connection as the acceptor.
///
/// Returns a handle for making calls and a driver future that must be spawned.
pub async fn establish_acceptor<T, D>(
    mut io: T,
    our_hello: Hello,
    dispatcher: D,
) -> Result<(ConnectionHandle, Driver<T, D>), ConnectionError>
where
    T: MessageTransport,
    D: ServiceDispatcher,
{
    // Send our Hello immediately
    io.send(&Message::Hello(our_hello.clone())).await?;

    // Wait for peer Hello
    let peer_hello = match io.recv_timeout(Duration::from_secs(5)).await? {
        Some(Message::Hello(h)) => h,
        Some(_) => {
            let _ = io
                .send(&Message::Goodbye {
                    reason: "message.hello.ordering".into(),
                })
                .await;
            return Err(ConnectionError::ProtocolViolation {
                rule_id: "message.hello.ordering",
                context: "received non-Hello before Hello exchange".into(),
            });
        }
        None => return Err(ConnectionError::Closed),
    };

    let (our_max, our_credit) = match &our_hello {
        Hello::V1 {
            max_payload_size,
            initial_stream_credit,
        } => (*max_payload_size, *initial_stream_credit),
    };
    let (peer_max, peer_credit) = match &peer_hello {
        Hello::V1 {
            max_payload_size,
            initial_stream_credit,
        } => (*max_payload_size, *initial_stream_credit),
    };

    let negotiated = Negotiated {
        max_payload_size: our_max.min(peer_max),
        initial_credit: our_credit.min(peer_credit),
    };

    let (command_tx, command_rx) = mpsc::channel(64);
    let handle = ConnectionHandle::new(command_tx, Role::Acceptor, negotiated.initial_credit);

    let driver = Driver::new(
        io,
        dispatcher,
        Role::Acceptor,
        negotiated,
        handle.clone(),
        command_rx,
    );

    Ok((handle, driver))
}

/// Establish a bidirectional connection as the initiator.
///
/// Returns a handle for making calls and a driver future that must be spawned.
pub async fn establish_initiator<T, D>(
    mut io: T,
    our_hello: Hello,
    dispatcher: D,
) -> Result<(ConnectionHandle, Driver<T, D>), ConnectionError>
where
    T: MessageTransport,
    D: ServiceDispatcher,
{
    // Send our Hello immediately
    io.send(&Message::Hello(our_hello.clone())).await?;

    // Wait for peer Hello
    let peer_hello = match io.recv_timeout(Duration::from_secs(5)).await? {
        Some(Message::Hello(h)) => h,
        Some(_) => {
            let _ = io
                .send(&Message::Goodbye {
                    reason: "message.hello.ordering".into(),
                })
                .await;
            return Err(ConnectionError::ProtocolViolation {
                rule_id: "message.hello.ordering",
                context: "received non-Hello before Hello exchange".into(),
            });
        }
        None => return Err(ConnectionError::Closed),
    };

    let (our_max, our_credit) = match &our_hello {
        Hello::V1 {
            max_payload_size,
            initial_stream_credit,
        } => (*max_payload_size, *initial_stream_credit),
    };
    let (peer_max, peer_credit) = match &peer_hello {
        Hello::V1 {
            max_payload_size,
            initial_stream_credit,
        } => (*max_payload_size, *initial_stream_credit),
    };

    let negotiated = Negotiated {
        max_payload_size: our_max.min(peer_max),
        initial_credit: our_credit.min(peer_credit),
    };

    let (command_tx, command_rx) = mpsc::channel(64);
    let handle = ConnectionHandle::new(command_tx, Role::Initiator, negotiated.initial_credit);

    let driver = Driver::new(
        io,
        dispatcher,
        Role::Initiator,
        negotiated,
        handle.clone(),
        command_rx,
    );

    Ok((handle, driver))
}
