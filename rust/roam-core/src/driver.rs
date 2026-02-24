use std::collections::BTreeMap;

use roam_types::{
    ChannelBody, ChannelId, ChannelMessage, Handler, ReplySink, RequestBody, RequestId,
    RequestMessage, RequestResponse, SelfRef, TxError,
};

use crate::session::{ConnectionHandle, ConnectionMessage, ConnectionSender};

/// Owned response message, sent through a oneshot when a response arrives
/// for an outgoing call. The caller unpacks the `SelfRef` on the other side.
type ResponseSlot = tokio::sync::oneshot::Sender<SelfRef<RequestMessage<'static>>>;

/// Concrete `ReplySink` implementation for the driver.
/// Sends a response back through the connection's sender.
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
    handler: H,
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
            handler,
            pending_responses: BTreeMap::new(),
            channels: BTreeMap::new(),
        }
    }

    // r[impl rpc.pipelining]
    /// Main loop: receive messages from the session and dispatch them.
    pub async fn run(&mut self) {
        // [TODO] SelfRef should have a more ergonomic way to match-and-move
        // through enum variants without the peek-then-map dance.
        while let Some(msg) = self.handle.recv().await {
            self.handle_msg(msg).await;
        }
    }

    async fn handle_msg(&mut self, msg: SelfRef<ConnectionMessage<'static>>) {
        let is_request = matches!(&*msg, ConnectionMessage::Request(_));
        if is_request {
            let msg = msg.map(|m| match m {
                ConnectionMessage::Request(r) => r,
                _ => unreachable!(),
            });
            self.handle_request(msg);
        } else {
            let msg = msg.map(|m| match m {
                ConnectionMessage::Channel(c) => c,
                _ => unreachable!(),
            });
            self.handle_channel(msg);
        }
    }

    fn handle_request(&mut self, msg: SelfRef<RequestMessage<'static>>) {
        // [TODO] same peek-then-map dance — needs SelfRef ergonomics
        let req_id = msg.id;
        let is_call = matches!(&msg.body, RequestBody::Call(_));
        let is_response = matches!(&msg.body, RequestBody::Response(_));

        if is_call {
            // r[impl rpc.request]
            let _reply = DriverReplySink {
                sender: self.handle.sender().clone(),
                request_id: req_id,
            };
            let _call = msg.map(|m| match m.body {
                RequestBody::Call(c) => c,
                _ => unreachable!(),
            });
            // [TODO] spawn handler task so we don't block the driver loop
            // self.handler.handle(call, reply)
        } else if is_response {
            // r[impl rpc.response.one-per-request]
            if let Some(tx) = self.pending_responses.remove(&req_id) {
                let _ = tx.send(msg);
            }
        } else {
            // r[impl rpc.cancel]
            // [TODO] signal cancellation to in-flight handler task
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
