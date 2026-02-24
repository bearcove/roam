use std::{collections::BTreeMap, sync::Arc};

use facet::Facet;
use roam_types::{
    ChannelMessage, Conduit, ConduitRx, ConduitTx, ConduitTxPermit, ConnectionAccept,
    ConnectionClose, ConnectionId, ConnectionOpen, ConnectionReject, ConnectionSettings, Hello,
    HelloYourself, Message, MessageFamily, MessagePayload, Metadata, Parity, ProtocolError,
    RequestMessage, SelfRef, SessionRole,
};
use tokio::sync::mpsc;

mod builders;
pub use builders::*;

// r[impl session.handshake]
/// Current roam session protocol version.
pub const PROTOCOL_VERSION: u32 = 7;

/// Session state machine,
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

    /// Sender for routing incoming request messages to the per-connection task.
    pub req_tx: mpsc::Sender<SelfRef<RequestMessage<'static>>>,
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

struct ConnectionHandle {
    connection_id: ConnectionId,
    sess_core: Arc<SessionCore>,
    rx: mpsc::Receiver<SelfRef<ConnectionMessage<'static>>>,
}

enum ConnectionMessage<'payload> {
    Request(RequestMessage<'payload>),
    Channel(ChannelMessage<'payload>),
}

impl ConnectionHandle {
    async fn send<'a>(&self, msg: ConnectionMessage<'a>) -> Result<(), ()> {
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

    async fn recv(&mut self) -> Option<SelfRef<ConnectionMessage<'static>>> {
        self.rx.recv().await
    }
}

impl<C> Session<C>
where
    C: Conduit<Msg = MessageFamily>,
{
    async fn establish(conduit: C) -> () {}
}

struct SessionCore {
    conns: BTreeMap<ConnectionId, ConnectionSlot>,
}

impl SessionCore {
    async fn send<'a>(&self, msg: Message<'a>) -> Result<(), ProtocolError> {
        todo!()
    }
}

#[cfg(test)]
mod tests;
