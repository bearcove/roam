use std::{collections::BTreeMap, pin::Pin, sync::Arc};

use moire::sync::SyncMutex;

use futures_util::StreamExt as _;
use futures_util::stream::FuturesUnordered;
use moire::task::FutureExt as _;
use roam_types::{
    Caller, ChannelBody, ChannelId, ChannelMessage, Handler, IdAllocator, Parity, ReplySink,
    RequestBody, RequestCall, RequestId, RequestMessage, RequestResponse, RoamError, SelfRef,
    TxError,
};

use crate::session::{ConnectionHandle, ConnectionMessage, ConnectionSender};

type ResponseSlot = moire::sync::oneshot::Sender<SelfRef<RequestMessage<'static>>>;

/// A boxed, Send future representing an in-flight handler task.
type HandlerFuture = Pin<Box<dyn std::future::Future<Output = ()> + Send>>;

/// State shared between the driver loop and any DriverCaller handles.
struct DriverShared {
    pending_responses: SyncMutex<BTreeMap<RequestId, ResponseSlot>>,
    request_ids: SyncMutex<IdAllocator<RequestId>>,
}

/// Concrete `ReplySink` implementation for the driver.
///
/// If dropped without `send_reply` being called, automatically sends
/// `RoamError::Cancelled` to the caller. This guarantees that every
/// request receives exactly one response (`rpc.response.one-per-request`),
/// even if the handler panics or forgets to reply.
pub struct DriverReplySink {
    sender: Option<ConnectionSender>,
    request_id: RequestId,
}

impl ReplySink for DriverReplySink {
    async fn send_reply(mut self, response: RequestResponse<'_>) {
        if let Err(e) = self
            .sender
            .take()
            .expect("unreachable: send_reply takes self by value")
            .send_response(self.request_id, response)
            .await
        {
            sender.mark_failure(self.request_id, "send_response failed")
        }
    }
}

// r[impl rpc.response.one-per-request]
impl Drop for DriverReplySink {
    fn drop(&mut self) {
        if let Some(sender) = self.sender.take() {
            sender.mark_failure(self.request_id, "no reply sent")
        }
    }
}

/// Cloneable handle for making outgoing calls through a connection.
///
/// Implements [`Caller`]: allocates a request ID, registers a response slot,
/// sends the call through the connection, and awaits the response.
#[derive(Clone)]
pub struct DriverCaller {
    sender: ConnectionSender,
    shared: Arc<DriverShared>,
}

impl Caller for DriverCaller {
    async fn call<'a>(
        &self,
        call: RequestCall<'a>,
    ) -> Result<SelfRef<RequestResponse<'static>>, RoamError> {
        async {
            // Allocate a request ID.
            let req_id = self.shared.request_ids.lock().next();

            // Register the response slot before sending, so the driver can
            // route the response even if it arrives before we start awaiting.
            let (tx, rx) = moire::sync::oneshot::channel("driver.response");
            self.shared.pending_responses.lock().insert(req_id, tx);

            // Send the call. This awaits the conduit permit and serializes
            // the borrowed payload all the way to the link's write buffer.
            let send_result = self
                .sender
                .send(ConnectionMessage::Request(RequestMessage {
                    id: req_id,
                    body: RequestBody::Call(call),
                }))
                .await;

            if send_result.is_err() {
                // Clean up the pending slot.
                self.shared.pending_responses.lock().remove(&req_id);
                return Err(RoamError::Cancelled);
            }

            // Await the response from the driver loop.
            let response_msg: SelfRef<RequestMessage<'static>> = rx
                .named("awaiting_response")
                .await
                .map_err(|_| RoamError::Cancelled)?;

            // Extract the Response variant from the RequestMessage.
            let response = response_msg.map(|m| match m.body {
                RequestBody::Response(r) => r,
                _ => unreachable!("pending_responses only gets Response variants"),
            });

            Ok(response)
        }
        .named("Caller::call")
        .await
    }
}

// r[impl rpc.handler]
// r[impl rpc.request]
// r[impl rpc.response]
// r[impl rpc.pipelining]
/// Per-connection driver. Handles in-flight request tracking, dispatches
/// incoming calls to a Handler, and manages channel state/flow control.
pub struct Driver<H: Handler<DriverReplySink>> {
    handle: ConnectionHandle,
    handler: Arc<H>,
    shared: Arc<DriverShared>,
    /// Channels we know about on this connection.
    channels: BTreeMap<ChannelId, ChannelState>,
}

struct ChannelState {
    /// Credit remaining (for sender side)
    credit: u32,
}

impl<H: Handler<DriverReplySink>> Driver<H> {
    pub fn new(handle: ConnectionHandle, handler: H, parity: Parity) -> Self {
        Self {
            handle,
            handler: Arc::new(handler),
            shared: Arc::new(DriverShared {
                pending_responses: SyncMutex::new("driver.pending_responses", BTreeMap::new()),
                request_ids: SyncMutex::new("driver.request_ids", IdAllocator::new(parity)),
            }),
            channels: BTreeMap::new(),
        }
    }

    /// Get a cloneable caller handle for making outgoing calls.
    pub fn caller(&self) -> DriverCaller {
        DriverCaller {
            sender: self.handle.sender().clone(),
            shared: Arc::clone(&self.shared),
        }
    }

    // r[impl rpc.pipelining]
    /// Main loop: receive messages from the session and dispatch them.
    /// Handler calls run concurrently in a FuturesUnordered — we don't
    /// block the driver loop waiting for a handler to finish.
    pub async fn run(&mut self) {
        let mut in_flight: FuturesUnordered<HandlerFuture> = FuturesUnordered::new();

        loop {
            tokio::select! {
                msg = self.handle.recv() => {
                    match msg {
                        Some(msg) => {
                            if let Some(fut) = self.handle_msg(msg) {
                                in_flight.push(fut);
                            }
                        }
                        None => break,
                    }
                }
                Some(()) = in_flight.next() => {}
                Some((req_id, _reason)) = self.handle.failures_rx.recv() => {
                    self.shared.pending_responses.lock().remove(&req_id);
                }
            }
        }
    }

    fn handle_msg(&mut self, msg: SelfRef<ConnectionMessage<'static>>) -> Option<HandlerFuture> {
        let is_request = matches!(&*msg, ConnectionMessage::Request(_));
        if is_request {
            let msg = msg.map(|m| match m {
                ConnectionMessage::Request(r) => r,
                _ => unreachable!(),
            });
            self.handle_request(msg)
        } else {
            let msg = msg.map(|m| match m {
                ConnectionMessage::Channel(c) => c,
                _ => unreachable!(),
            });
            self.handle_channel(msg);
            None
        }
    }

    fn handle_request(&mut self, msg: SelfRef<RequestMessage<'static>>) -> Option<HandlerFuture> {
        let req_id = msg.id;
        let is_call = matches!(&msg.body, RequestBody::Call(_));
        let is_response = matches!(&msg.body, RequestBody::Response(_));

        if is_call {
            // r[impl rpc.request]
            let reply = DriverReplySink {
                sender: Some(self.handle.sender().clone()),
                request_id: req_id,
            };
            let call = msg.map(|m| match m.body {
                RequestBody::Call(c) => c,
                _ => unreachable!(),
            });
            let handler = Arc::clone(&self.handler);
            Some(Box::pin(
                async move {
                    handler.handle(call, reply).await;
                }
                .named("handler"),
            ))
        } else if is_response {
            // r[impl rpc.response.one-per-request]
            if let Some(tx) = self.shared.pending_responses.lock().remove(&req_id) {
                let _: Result<(), _> = tx.send(msg);
            }
            None
        } else {
            // r[impl rpc.cancel]
            // [TODO] signal cancellation to in-flight handler task
            None
        }
    }

    fn handle_channel(&mut self, msg: SelfRef<ChannelMessage<'static>>) {
        let chan_id = msg.id;
        match &msg.body {
            // r[impl rpc.channel.item]
            ChannelBody::Item(_item) => {
                // [TODO] route to the Rx's mpsc sender
            }
            // r[impl rpc.channel.close]
            ChannelBody::Close(_close) => {
                // [TODO] signal end-of-stream to the Rx
            }
            // r[impl rpc.channel.reset]
            ChannelBody::Reset(_reset) => {
                // [TODO] signal the Tx to stop sending
            }
            // r[impl rpc.flow-control.credit.grant]
            ChannelBody::GrantCredit(grant) => {
                if let Some(state) = self.channels.get_mut(&chan_id) {
                    state.credit = state.credit.saturating_add(grant.additional);
                    // [TODO] wake any sender blocked on zero credit
                }
            }
        }
    }
}
