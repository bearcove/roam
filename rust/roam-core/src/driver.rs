use std::collections::BTreeMap;

use roam_types::{
    ChannelBody, ChannelId, ChannelMessage, Handler, ReplySink, RequestBody, RequestCall,
    RequestId, RequestMessage, RequestResponse, RoamError, SelfRef, TxError,
};

use crate::session::{ConnectionHandle, ConnectionMessage};

// r[impl rpc.handler]
// r[impl rpc.request]
// r[impl rpc.response]
// r[impl rpc.pipelining]
/// Per-connection driver. Handles in-flight request tracking, dispatches
/// incoming calls to a Handler, and manages channel state/flow control.
pub struct Driver<R: ReplySink> {
    handle: ConnectionHandle,
    handler: Box<dyn Handler<R>>,
    /// In-flight outgoing requests (we sent a Call, waiting for Response).
    /// Maps request ID -> response slot.
    pending_responses:
        BTreeMap<RequestId, tokio::sync::oneshot::Sender<SelfRef<RequestResponse<'static>>>>,
    /// Channels we know about on this connection.
    channels: BTreeMap<ChannelId, ChannelState>,
}

struct ChannelState {
    /// Credit remaining (for sender side)
    credit: u32,
}

impl<R: ReplySink> Driver<R> {
    pub fn new(handle: ConnectionHandle, handler: Box<dyn Handler<R>>) -> Self {
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
        while let Some(msg) = self.handle.recv().await {
            match msg.as_ref() {
                ConnectionMessage::Request(req) => self.handle_request(&msg, req),
                ConnectionMessage::Channel(chan) => self.handle_channel(&msg, chan),
            }
        }
    }

    fn handle_request(
        &mut self,
        msg: &SelfRef<ConnectionMessage<'static>>,
        req: &RequestMessage<'_>,
    ) {
        match &req.body {
            // r[impl rpc.request]
            RequestBody::Call(_call) => {
                // TODO: create a ReplySink, wrap in SinkCall, dispatch to handler
                // Need to spawn the handler call so we don't block the driver loop
            }
            // r[impl rpc.response.one-per-request]
            RequestBody::Response(_resp) => {
                // Match against pending_responses, send to the oneshot
                if let Some(tx) = self.pending_responses.remove(&req.id) {
                    // TODO: repack the SelfRef to extract just the response
                    let _ = tx; // placeholder
                }
            }
            // r[impl rpc.cancel]
            RequestBody::Cancel(_cancel) => {
                // TODO: signal cancellation to in-flight handler task
            }
        }
    }

    fn handle_channel(
        &mut self,
        _msg: &SelfRef<ConnectionMessage<'static>>,
        chan: &ChannelMessage<'_>,
    ) {
        match &chan.body {
            // r[impl rpc.channel.item]
            ChannelBody::Item(_item) => {
                // TODO: route to the Rx's mpsc sender
            }
            // r[impl rpc.channel.close]
            ChannelBody::Close(_close) => {
                // TODO: signal end-of-stream to the Rx
            }
            // r[impl rpc.channel.reset]
            ChannelBody::Reset(_reset) => {
                // TODO: signal the Tx to stop sending
            }
            // r[impl rpc.flow-control.credit.grant]
            ChannelBody::GrantCredit(grant) => {
                if let Some(state) = self.channels.get_mut(&chan.id) {
                    state.credit = state.credit.saturating_add(grant.additional);
                    // TODO: wake any sender blocked on zero credit
                }
            }
        }
    }
}
