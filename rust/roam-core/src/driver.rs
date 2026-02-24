use std::{collections::BTreeMap, pin::Pin, sync::Arc};

use futures_util::StreamExt as _;
use futures_util::stream::FuturesUnordered;
use roam_types::{
    ChannelBody, ChannelId, ChannelMessage, Handler, ReplySink, RequestBody, RequestId,
    RequestMessage, RequestResponse, SelfRef, TxError,
};

use crate::session::{ConnectionHandle, ConnectionMessage, ConnectionSender};

/// Owned response message, sent through a oneshot when a response arrives
/// for an outgoing call. The caller unpacks the `SelfRef` on the other side.
type ResponseSlot = tokio::sync::oneshot::Sender<SelfRef<RequestMessage<'static>>>;

/// A boxed, Send future representing an in-flight handler task.
type HandlerFuture = Pin<Box<dyn std::future::Future<Output = ()> + Send>>;

/// Concrete `ReplySink` implementation for the driver.
/// Sends a response back through the connection's sender.
// [TODO] Drop impl: if send_reply was never called, send RoamError::Cancelled
pub struct DriverReplySink {
    sender: ConnectionSender,
    request_id: RequestId,
}

impl ReplySink for DriverReplySink {
    async fn send_reply<'a>(&self, response: RequestResponse<'a>) -> Result<(), TxError> {
        self.sender
            .send_response(self.request_id, response)
            .await
            .map_err(|_| TxError::Transport("session closed".into()))
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
    /// In-flight outgoing requests (we sent a Call, waiting for Response).
    pending_responses: BTreeMap<RequestId, ResponseSlot>,
    /// Channels we know about on this connection.
    channels: BTreeMap<ChannelId, ChannelState>,
}

struct ChannelState {
    /// Credit remaining (for sender side)
    credit: u32,
}

impl<H: Handler<DriverReplySink>> Driver<H> {
    pub fn new(handle: ConnectionHandle, handler: H) -> Self {
        Self {
            handle,
            handler: Arc::new(handler),
            pending_responses: BTreeMap::new(),
            channels: BTreeMap::new(),
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
                sender: self.handle.sender().clone(),
                request_id: req_id,
            };
            let call = msg.map(|m| match m.body {
                RequestBody::Call(c) => c,
                _ => unreachable!(),
            });
            let handler = Arc::clone(&self.handler);
            Some(Box::pin(async move {
                handler.handle(call, reply).await;
            }))
        } else if is_response {
            // r[impl rpc.response.one-per-request]
            if let Some(tx) = self.pending_responses.remove(&req_id) {
                let _ = tx.send(msg);
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
