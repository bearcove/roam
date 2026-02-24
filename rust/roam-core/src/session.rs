use std::collections::BTreeMap;

use roam_types::{
    Conduit, ConduitRx, ConduitTx, ConduitTxPermit, ConnectionAccept, ConnectionClose,
    ConnectionId, ConnectionOpen, ConnectionReject, ConnectionSettings, Hello, HelloYourself,
    Message, MessageFamily, MessagePayload, Metadata, Parity, ProtocolError, SelfRef, SessionRole,
};

// r[impl session.handshake]
/// Current roam session protocol version.
pub const PROTOCOL_VERSION: u32 = 7;

// r[impl connection]
/// Static data for one active connection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionState {
    pub id: ConnectionId,
    pub local_parity: Parity,
    pub peer_parity: Parity,
    pub local_settings: ConnectionSettings,
    pub peer_settings: ConnectionSettings,
}

pub enum SessionEvent {
    IncomingConnectionOpen {
        conn_id: ConnectionId,
        peer_parity: Parity,
        peer_settings: ConnectionSettings,
        metadata: Metadata,
    },
    OutgoingConnectionAccepted {
        conn_id: ConnectionId,
    },
    OutgoingConnectionRejected {
        conn_id: ConnectionId,
        metadata: Metadata,
    },
    ConnectionClosed {
        conn_id: ConnectionId,
        metadata: Metadata,
    },
    IncomingMessage(SelfRef<Message<'static>>),
}

#[derive(Debug)]
pub enum SessionError {
    Transport(String),
    ProtocolViolation(String),
    RemoteProtocolError(String),
    UnexpectedEof(&'static str),
    InvalidState(String),
}

impl std::fmt::Display for SessionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Transport(msg) => write!(f, "transport error: {msg}"),
            Self::ProtocolViolation(msg) => write!(f, "protocol violation: {msg}"),
            Self::RemoteProtocolError(msg) => write!(f, "remote protocol error: {msg}"),
            Self::UnexpectedEof(ctx) => write!(f, "unexpected eof while {ctx}"),
            Self::InvalidState(msg) => write!(f, "invalid state: {msg}"),
        }
    }
}

impl std::error::Error for SessionError {}

#[derive(Debug, Clone)]
struct PendingInboundOpen {
    peer_parity: Parity,
    peer_settings: ConnectionSettings,
}

#[derive(Debug, Clone)]
struct PendingOutboundOpen {
    local_parity: Parity,
    local_settings: ConnectionSettings,
}

#[derive(Debug, Clone)]
enum ConnectionSlot {
    Active(ConnectionState),
    PendingInbound(PendingInboundOpen),
    PendingOutbound(PendingOutboundOpen),
}

// r[impl session]
pub struct Session<C: Conduit> {
    tx: Option<C::Tx>,
    rx: C::Rx,
    // r[impl session.role]
    role: SessionRole,
    // r[impl session.parity]
    local_session_parity: Parity,
    slots: BTreeMap<ConnectionId, ConnectionSlot>,
}

impl<C> Session<C>
where
    C: Conduit<Msg = MessageFamily>,
{
    /// Create a pre-handshake session from a split conduit.
    /// Role and parity are placeholders until handshake completes.
    fn pre_handshake(tx: C::Tx, rx: C::Rx) -> Self {
        Self {
            tx: Some(tx),
            rx,
            role: SessionRole::Initiator,
            local_session_parity: Parity::Odd,
            slots: BTreeMap::new(),
        }
    }

    // r[impl session.handshake]
    // r[impl session.connection-settings.hello]
    // r[impl rpc.session-setup]
    async fn establish_as_initiator(
        &mut self,
        parity: Parity,
        root_settings: ConnectionSettings,
        metadata: Metadata,
    ) -> Result<(), SessionError> {
        let hello = Message::new(
            ConnectionId::ROOT,
            MessagePayload::Hello(Hello {
                version: PROTOCOL_VERSION,
                parity,
                connection_settings: root_settings.clone(),
                metadata,
            }),
        );
        self.send(hello).await?;

        let first = self
            .recv()
            .await?
            .ok_or(SessionError::UnexpectedEof("waiting for HelloYourself"))?;

        match first.payload() {
            MessagePayload::HelloYourself(reply) => {
                if !first.connection_id().is_root() {
                    return Err(self
                        .protocol_violation("HelloYourself must use connection ID 0")
                        .await);
                }
                self.finish_handshake(
                    SessionRole::Initiator,
                    parity,
                    root_settings,
                    reply.connection_settings.clone(),
                );
                Ok(())
            }
            MessagePayload::ProtocolError(_) => {
                Err(self.handle_incoming_protocol_error(&first).await)
            }
            _ => Err(self
                .protocol_violation("expected HelloYourself during handshake")
                .await),
        }
    }

    // r[impl session.handshake]
    // r[impl session.parity]
    // r[impl session.connection-settings.hello]
    // r[impl rpc.session-setup]
    async fn establish_as_acceptor(
        &mut self,
        root_settings: ConnectionSettings,
        metadata: Metadata,
    ) -> Result<(), SessionError> {
        let first = self
            .recv()
            .await?
            .ok_or(SessionError::UnexpectedEof("waiting for Hello"))?;

        let MessagePayload::Hello(hello) = first.payload() else {
            if matches!(first.payload(), MessagePayload::ProtocolError(_)) {
                return Err(self.handle_incoming_protocol_error(&first).await);
            }
            return Err(self
                .protocol_violation("expected Hello during handshake")
                .await);
        };

        if !first.connection_id().is_root() {
            return Err(self
                .protocol_violation("Hello must use connection ID 0")
                .await);
        }
        if hello.version != PROTOCOL_VERSION {
            return Err(self
                .protocol_violation(&format!(
                    "unsupported Hello version {} (expected {})",
                    hello.version, PROTOCOL_VERSION
                ))
                .await);
        }

        let local_parity = hello.parity.other();
        let peer_root_settings = hello.connection_settings.clone();

        let reply = Message::new(
            ConnectionId::ROOT,
            MessagePayload::HelloYourself(HelloYourself {
                connection_settings: root_settings.clone(),
                metadata,
            }),
        );
        self.send(reply).await?;

        self.finish_handshake(
            SessionRole::Acceptor,
            local_parity,
            root_settings,
            peer_root_settings,
        );
        Ok(())
    }

    // r[impl connection.root]
    fn finish_handshake(
        &mut self,
        role: SessionRole,
        local_session_parity: Parity,
        local_root_settings: ConnectionSettings,
        peer_root_settings: ConnectionSettings,
    ) {
        self.role = role;
        self.local_session_parity = local_session_parity;
        self.slots.insert(
            ConnectionId::ROOT,
            ConnectionSlot::Active(ConnectionState {
                id: ConnectionId::ROOT,
                local_parity: local_session_parity,
                peer_parity: local_session_parity.other(),
                local_settings: local_root_settings,
                peer_settings: peer_root_settings,
            }),
        );
    }

    /// Handle an incoming ProtocolError message, validating connection ID.
    // r[impl session.protocol-error]
    // r[impl session.message.connection-id]
    async fn handle_incoming_protocol_error(
        &mut self,
        msg: &SelfRef<Message<'static>>,
    ) -> SessionError {
        if !msg.connection_id().is_root() {
            return self
                .protocol_violation("ProtocolError must use connection ID 0")
                .await;
        }
        let MessagePayload::ProtocolError(err) = msg.payload() else {
            unreachable!("called handle_incoming_protocol_error on non-ProtocolError");
        };
        self.teardown_local().await;
        SessionError::RemoteProtocolError(err.description.clone())
    }

    pub(crate) fn role(&self) -> SessionRole {
        self.role
    }

    pub(crate) fn local_session_parity(&self) -> Parity {
        self.local_session_parity
    }

    pub(crate) fn peer_session_parity(&self) -> Parity {
        self.local_session_parity.other()
    }

    pub(crate) fn connection(&self, conn_id: ConnectionId) -> Option<&ConnectionState> {
        match self.slots.get(&conn_id) {
            Some(ConnectionSlot::Active(state)) => Some(state),
            _ => None,
        }
    }

    pub async fn close(mut self) -> Result<(), SessionError> {
        if let Some(tx) = self.tx.take() {
            tx.close()
                .await
                .map_err(|e| SessionError::Transport(format!("close failed: {e}")))?;
        }
        Ok(())
    }

    // r[impl connection.open]
    // r[impl connection.virtual]
    // r[impl connection.parity]
    // r[impl rpc.virtual-connection.open]
    pub(crate) async fn open_connection(
        &mut self,
        conn_id: ConnectionId,
        local_parity: Parity,
        local_settings: ConnectionSettings,
        metadata: Metadata,
    ) -> Result<(), SessionError> {
        if conn_id.is_root() {
            return Err(SessionError::InvalidState(
                "cannot open root connection".into(),
            ));
        }
        if !id_matches_parity(conn_id.0, &self.local_session_parity) {
            return Err(SessionError::InvalidState(format!(
                "connection id {} does not match local session parity {:?}",
                conn_id, self.local_session_parity
            )));
        }
        if self.slots.contains_key(&conn_id) {
            return Err(SessionError::InvalidState(format!(
                "connection id {} already in use",
                conn_id
            )));
        }

        let payload = MessagePayload::ConnectionOpen(ConnectionOpen {
            parity: local_parity,
            connection_settings: local_settings.clone(),
            metadata,
        });
        self.send(Message::new(conn_id, payload)).await?;

        self.slots.insert(
            conn_id,
            ConnectionSlot::PendingOutbound(PendingOutboundOpen {
                local_parity,
                local_settings,
            }),
        );
        Ok(())
    }

    // r[impl rpc.virtual-connection.accept]
    // r[impl session.connection-settings.open]
    pub(crate) async fn accept_connection(
        &mut self,
        conn_id: ConnectionId,
        local_settings: ConnectionSettings,
        metadata: Metadata,
    ) -> Result<(), SessionError> {
        let Some(ConnectionSlot::PendingInbound(pending)) = self.slots.get(&conn_id) else {
            return Err(SessionError::InvalidState(format!(
                "no pending inbound open for {}",
                conn_id.0
            )));
        };
        let local_parity = pending.peer_parity.other();
        let peer_parity = pending.peer_parity;
        let peer_settings = pending.peer_settings.clone();

        let payload = MessagePayload::ConnectionAccept(ConnectionAccept {
            connection_settings: local_settings.clone(),
            metadata,
        });
        self.send(Message::new(conn_id, payload)).await?;

        self.slots.insert(
            conn_id,
            ConnectionSlot::Active(ConnectionState {
                id: conn_id,
                local_parity,
                peer_parity,
                local_settings,
                peer_settings,
            }),
        );
        Ok(())
    }

    // r[impl connection.open.rejection]
    pub(crate) async fn reject_connection(
        &mut self,
        conn_id: ConnectionId,
        metadata: Metadata,
    ) -> Result<(), SessionError> {
        if !matches!(
            self.slots.get(&conn_id),
            Some(ConnectionSlot::PendingInbound(_))
        ) {
            return Err(SessionError::InvalidState(format!(
                "no pending inbound open for {}",
                conn_id.0
            )));
        }

        let payload = MessagePayload::ConnectionReject(ConnectionReject { metadata });
        self.send(Message::new(conn_id, payload)).await?;

        self.slots.remove(&conn_id);
        Ok(())
    }

    // r[impl connection.close]
    // r[impl connection.root]
    pub(crate) async fn close_connection(
        &mut self,
        conn_id: ConnectionId,
        metadata: Metadata,
    ) -> Result<(), SessionError> {
        if conn_id.is_root() {
            return Err(SessionError::InvalidState(
                "cannot close root connection".into(),
            ));
        }
        if !matches!(self.slots.get(&conn_id), Some(ConnectionSlot::Active(_))) {
            return Err(SessionError::InvalidState(format!(
                "connection {} is not active",
                conn_id.0
            )));
        }

        let payload = MessagePayload::ConnectionClose(ConnectionClose { metadata });
        self.send(Message::new(conn_id, payload)).await?;

        self.slots.remove(&conn_id);
        Ok(())
    }

    // r[impl session.message]
    pub(crate) async fn send_rpc_message<'msg>(
        &self,
        msg: Message<'msg>,
    ) -> Result<(), SessionError> {
        let conn_id = msg.connection_id();
        if !matches!(self.slots.get(&conn_id), Some(ConnectionSlot::Active(_))) {
            return Err(SessionError::InvalidState(format!(
                "connection {} is not active",
                conn_id.0
            )));
        }

        if !is_rpc_or_channel_payload(msg.payload()) {
            return Err(SessionError::InvalidState(
                "send_rpc_message only accepts rpc/channel payloads".into(),
            ));
        }

        self.send(msg).await
    }

    pub(crate) async fn recv_event(&mut self) -> Result<SessionEvent, SessionError> {
        let msg = self
            .recv()
            .await?
            .ok_or(SessionError::UnexpectedEof("receiving session message"))?;
        self.handle_incoming(msg).await
    }

    // r[impl session.message.payloads]
    // r[impl session.message.connection-id]
    async fn handle_incoming(
        &mut self,
        msg: SelfRef<Message<'static>>,
    ) -> Result<SessionEvent, SessionError> {
        let conn_id = msg.connection_id();

        match msg.payload() {
            // r[impl session.handshake]
            MessagePayload::Hello(_) => Err(self
                .protocol_violation("unexpected Hello after handshake")
                .await),
            MessagePayload::HelloYourself(_) => Err(self
                .protocol_violation("unexpected HelloYourself after handshake")
                .await),
            // r[impl session.protocol-error]
            MessagePayload::ProtocolError(_) => {
                Err(self.handle_incoming_protocol_error(&msg).await)
            }
            // r[impl connection.open]
            MessagePayload::ConnectionOpen(open) => {
                if conn_id.is_root() {
                    return Err(self
                        .protocol_violation("OpenConnection cannot use connection 0")
                        .await);
                }
                if !id_matches_parity(conn_id.0, &self.peer_session_parity()) {
                    return Err(self
                        .protocol_violation("OpenConnection ID parity mismatch")
                        .await);
                }
                if self.slots.contains_key(&conn_id) {
                    return Err(self
                        .protocol_violation("OpenConnection for already-used ID")
                        .await);
                }

                self.slots.insert(
                    conn_id,
                    ConnectionSlot::PendingInbound(PendingInboundOpen {
                        peer_parity: open.parity,
                        peer_settings: open.connection_settings.clone(),
                    }),
                );

                Ok(SessionEvent::IncomingConnectionOpen {
                    conn_id,
                    peer_parity: open.parity,
                    peer_settings: open.connection_settings.clone(),
                    metadata: open.metadata.clone(),
                })
            }
            // r[impl connection.open]
            MessagePayload::ConnectionAccept(accept) => {
                if conn_id.is_root() {
                    return Err(self
                        .protocol_violation("AcceptConnection cannot use connection 0")
                        .await);
                }
                let Some(slot) = self.slots.remove(&conn_id) else {
                    return Err(self
                        .protocol_violation("AcceptConnection without matching outbound open")
                        .await);
                };
                let pending = match slot {
                    ConnectionSlot::PendingOutbound(pending) => pending,
                    ConnectionSlot::Active(_) | ConnectionSlot::PendingInbound(_) => {
                        self.slots.insert(conn_id, slot);
                        return Err(self
                            .protocol_violation("AcceptConnection without matching outbound open")
                            .await);
                    }
                };

                self.slots.insert(
                    conn_id,
                    ConnectionSlot::Active(ConnectionState {
                        id: conn_id,
                        local_parity: pending.local_parity,
                        peer_parity: pending.local_parity.other(),
                        local_settings: pending.local_settings,
                        peer_settings: accept.connection_settings.clone(),
                    }),
                );
                Ok(SessionEvent::OutgoingConnectionAccepted { conn_id })
            }
            // r[impl connection.open.rejection]
            MessagePayload::ConnectionReject(reject) => {
                if conn_id.is_root() {
                    return Err(self
                        .protocol_violation("RejectConnection cannot use connection 0")
                        .await);
                }
                let slot = self.slots.remove(&conn_id);
                let Some(ConnectionSlot::PendingOutbound(_)) = slot else {
                    if let Some(slot) = slot {
                        self.slots.insert(conn_id, slot);
                    }
                    return Err(self
                        .protocol_violation("RejectConnection without matching outbound open")
                        .await);
                };
                Ok(SessionEvent::OutgoingConnectionRejected {
                    conn_id,
                    metadata: reject.metadata.clone(),
                })
            }
            // r[impl connection.close]
            // r[impl connection.close.semantics]
            MessagePayload::ConnectionClose(close) => {
                if conn_id.is_root() {
                    return Err(self
                        .protocol_violation("CloseConnection cannot use connection 0")
                        .await);
                }
                let slot = self.slots.remove(&conn_id);
                let Some(ConnectionSlot::Active(_)) = slot else {
                    if let Some(slot) = slot {
                        self.slots.insert(conn_id, slot);
                    }
                    return Err(self
                        .protocol_violation("CloseConnection for unknown ID")
                        .await);
                };

                Ok(SessionEvent::ConnectionClosed {
                    conn_id,
                    metadata: close.metadata.clone(),
                })
            }
            MessagePayload::Request(_)
            | MessagePayload::Response(_)
            | MessagePayload::CancelRequest(_)
            | MessagePayload::ChannelItem(_)
            | MessagePayload::CloseChannel(_)
            | MessagePayload::ResetChannel(_)
            | MessagePayload::GrantCredit(_) => {
                if !matches!(self.slots.get(&conn_id), Some(ConnectionSlot::Active(_))) {
                    return Err(self
                        .protocol_violation("rpc/channel message for unknown connection")
                        .await);
                }
                Ok(SessionEvent::IncomingMessage(msg))
            }
        }
    }

    async fn send<'msg>(&self, msg: Message<'msg>) -> Result<(), SessionError> {
        let tx = self
            .tx
            .as_ref()
            .ok_or_else(|| SessionError::InvalidState("session transport already closed".into()))?;
        let permit = tx
            .reserve()
            .await
            .map_err(|e| SessionError::Transport(format!("reserve failed: {e}")))?;
        permit
            .send(msg)
            .map_err(|e| SessionError::Transport(format!("send failed: {e}")))?;
        Ok(())
    }

    async fn recv(&mut self) -> Result<Option<SelfRef<Message<'static>>>, SessionError> {
        self.rx
            .recv()
            .await
            .map_err(|e| SessionError::Transport(format!("recv failed: {e}")))
    }

    // r[impl session.protocol-error]
    async fn protocol_violation(&mut self, description: &str) -> SessionError {
        let error = SessionError::ProtocolViolation(description.to_string());
        if let Some(tx) = self.tx.take() {
            let protocol_error = Message::new(
                ConnectionId::ROOT,
                MessagePayload::ProtocolError(ProtocolError {
                    description: description.to_string(),
                }),
            );
            // Best-effort: ignore send/close failures during violation teardown
            if let Ok(permit) = tx.reserve().await {
                let _ = permit.send(protocol_error);
            }
            let _ = tx.close().await;
        }
        self.slots.clear();
        error
    }

    async fn teardown_local(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.close().await;
        }
        self.slots.clear();
    }
}

// r[impl session.role]
pub fn initiator<C>(conduit: C) -> SessionInitiatorBuilder<C> {
    SessionInitiatorBuilder::new(conduit)
}

// r[impl session.role]
pub fn acceptor<C>(conduit: C) -> SessionAcceptorBuilder<C> {
    SessionAcceptorBuilder::new(conduit)
}

pub struct SessionInitiatorBuilder<C> {
    conduit: C,
    parity: Parity,
    root_settings: ConnectionSettings,
    metadata: Metadata,
}

impl<C> SessionInitiatorBuilder<C> {
    fn new(conduit: C) -> Self {
        Self {
            conduit,
            parity: Parity::Odd,
            root_settings: ConnectionSettings::default(),
            metadata: vec![],
        }
    }

    pub fn parity(mut self, parity: Parity) -> Self {
        self.parity = parity;
        self
    }

    pub fn root_settings(mut self, settings: ConnectionSettings) -> Self {
        self.root_settings = settings;
        self
    }

    pub fn max_concurrent_requests(mut self, max_concurrent_requests: u32) -> Self {
        self.root_settings.max_concurrent_requests = max_concurrent_requests;
        self
    }

    pub fn metadata(mut self, metadata: Metadata) -> Self {
        self.metadata = metadata;
        self
    }

    pub async fn establish(self) -> Result<Session<C>, SessionError>
    where
        C: Conduit<Msg = MessageFamily>,
    {
        let (tx, rx) = self.conduit.split();
        let mut session = Session::pre_handshake(tx, rx);
        session
            .establish_as_initiator(self.parity, self.root_settings, self.metadata)
            .await?;
        Ok(session)
    }
}

pub struct SessionAcceptorBuilder<C> {
    conduit: C,
    root_settings: ConnectionSettings,
    metadata: Metadata,
}

impl<C> SessionAcceptorBuilder<C> {
    fn new(conduit: C) -> Self {
        Self {
            conduit,
            root_settings: ConnectionSettings::default(),
            metadata: vec![],
        }
    }

    pub fn root_settings(mut self, settings: ConnectionSettings) -> Self {
        self.root_settings = settings;
        self
    }

    pub fn max_concurrent_requests(mut self, max_concurrent_requests: u32) -> Self {
        self.root_settings.max_concurrent_requests = max_concurrent_requests;
        self
    }

    pub fn metadata(mut self, metadata: Metadata) -> Self {
        self.metadata = metadata;
        self
    }

    pub async fn establish(self) -> Result<Session<C>, SessionError>
    where
        C: Conduit<Msg = MessageFamily>,
    {
        let (tx, rx) = self.conduit.split();
        let mut session = Session::pre_handshake(tx, rx);
        session
            .establish_as_acceptor(self.root_settings, self.metadata)
            .await?;
        Ok(session)
    }
}

// r[impl session.parity]
fn id_matches_parity(id: u64, parity: &Parity) -> bool {
    match parity {
        Parity::Odd => !id.is_multiple_of(2),
        Parity::Even => id.is_multiple_of(2),
    }
}

fn is_rpc_or_channel_payload(payload: &MessagePayload<'_>) -> bool {
    matches!(
        payload,
        MessagePayload::Request(_)
            | MessagePayload::Response(_)
            | MessagePayload::CancelRequest(_)
            | MessagePayload::ChannelItem(_)
            | MessagePayload::CloseChannel(_)
            | MessagePayload::ResetChannel(_)
            | MessagePayload::GrantCredit(_)
    )
}
