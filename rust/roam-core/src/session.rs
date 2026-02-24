use std::collections::BTreeMap;

use roam_types::{
    AcceptConnection, CloseConnection, Conduit, ConduitRx, ConduitTx, ConduitTxPermit,
    ConnectionId, ConnectionSettings, Hello, HelloYourself, Message, MessagePayload, Metadata,
    OpenConnection, Parity, ProtocolError, RejectConnection, SelfRef, SessionRole,
};

/// Current roam session protocol version.
pub const PROTOCOL_VERSION: u32 = 7;

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

pub struct Session<C: Conduit> {
    tx: Option<C::Tx>,
    rx: C::Rx,
    role: SessionRole,
    local_session_parity: Parity,
    peer_session_parity: Parity,
    connections: BTreeMap<ConnectionId, ConnectionState>,
    pending_inbound: BTreeMap<ConnectionId, PendingInboundOpen>,
    pending_outbound: BTreeMap<ConnectionId, PendingOutboundOpen>,
}

impl<C> Session<C>
where
    C: Conduit<Msg<'static> = Message<'static>>,
    C::Tx: for<'a> ConduitTx<Msg<'a> = Message<'a>>,
    C::Rx: ConduitRx<Msg<'static> = Message<'static>>,
{
    fn new(
        tx: C::Tx,
        rx: C::Rx,
        role: SessionRole,
        local_session_parity: Parity,
        peer_session_parity: Parity,
        local_root_settings: ConnectionSettings,
        peer_root_settings: ConnectionSettings,
    ) -> Self {
        let mut connections = BTreeMap::new();
        connections.insert(
            ConnectionId::ROOT,
            ConnectionState {
                id: ConnectionId::ROOT,
                local_parity: local_session_parity.clone(),
                peer_parity: peer_session_parity.clone(),
                local_settings: local_root_settings,
                peer_settings: peer_root_settings,
            },
        );

        Self {
            tx: Some(tx),
            rx,
            role,
            local_session_parity,
            peer_session_parity,
            connections,
            pending_inbound: BTreeMap::new(),
            pending_outbound: BTreeMap::new(),
        }
    }

    pub fn role(&self) -> SessionRole {
        self.role
    }

    pub fn local_session_parity(&self) -> Parity {
        self.local_session_parity
    }

    pub fn peer_session_parity(&self) -> Parity {
        self.peer_session_parity
    }

    pub fn connection(&self, conn_id: ConnectionId) -> Option<&ConnectionState> {
        self.connections.get(&conn_id)
    }

    pub async fn close(mut self) -> Result<(), SessionError> {
        if let Some(tx) = self.tx.take() {
            tx.close()
                .await
                .map_err(|e| SessionError::Transport(format!("close failed: {e}")))?;
        }
        Ok(())
    }

    pub async fn open_connection(
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
                conn_id.0, self.local_session_parity
            )));
        }
        if self.connections.contains_key(&conn_id)
            || self.pending_inbound.contains_key(&conn_id)
            || self.pending_outbound.contains_key(&conn_id)
        {
            return Err(SessionError::InvalidState(format!(
                "connection id {} already in use",
                conn_id.0
            )));
        }

        let payload = MessagePayload::OpenConnection(OpenConnection {
            parity: local_parity.clone(),
            connection_settings: local_settings.clone(),
            metadata,
        });
        self.send(Message::new(conn_id, payload)).await?;

        self.pending_outbound.insert(
            conn_id,
            PendingOutboundOpen {
                local_parity,
                local_settings,
            },
        );
        Ok(())
    }

    pub async fn accept_connection(
        &mut self,
        conn_id: ConnectionId,
        local_settings: ConnectionSettings,
        metadata: Metadata,
    ) -> Result<(), SessionError> {
        let pending = self.pending_inbound.remove(&conn_id).ok_or_else(|| {
            SessionError::InvalidState(format!("no pending inbound open for {}", conn_id.0))
        })?;

        let local_parity = pending.peer_parity.other();
        self.connections.insert(
            conn_id,
            ConnectionState {
                id: conn_id,
                local_parity,
                peer_parity: pending.peer_parity.clone(),
                local_settings: local_settings.clone(),
                peer_settings: pending.peer_settings,
            },
        );

        let payload = MessagePayload::AcceptConnection(AcceptConnection {
            connection_settings: local_settings,
            metadata,
        });
        self.send(Message::new(conn_id, payload)).await
    }

    pub async fn reject_connection(
        &mut self,
        conn_id: ConnectionId,
        metadata: Metadata,
    ) -> Result<(), SessionError> {
        if self.pending_inbound.remove(&conn_id).is_none() {
            return Err(SessionError::InvalidState(format!(
                "no pending inbound open for {}",
                conn_id.0
            )));
        }

        let payload = MessagePayload::RejectConnection(RejectConnection { metadata });
        self.send(Message::new(conn_id, payload)).await
    }

    pub async fn close_connection(
        &mut self,
        conn_id: ConnectionId,
        metadata: Metadata,
    ) -> Result<(), SessionError> {
        if conn_id.is_root() {
            return Err(SessionError::InvalidState(
                "cannot close root connection".into(),
            ));
        }
        if self.connections.remove(&conn_id).is_none() {
            return Err(SessionError::InvalidState(format!(
                "connection {} is not active",
                conn_id.0
            )));
        }

        let payload = MessagePayload::CloseConnection(CloseConnection { metadata });
        self.send(Message::new(conn_id, payload)).await
    }

    pub async fn send_rpc_message<'msg>(&self, msg: Message<'msg>) -> Result<(), SessionError> {
        let conn_id = msg.connection_id();
        if !self.connections.contains_key(&conn_id) {
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

    pub async fn recv_event(&mut self) -> Result<SessionEvent, SessionError> {
        let msg = recv_message(&mut self.rx)
            .await?
            .ok_or(SessionError::UnexpectedEof("receiving session message"))?;
        self.handle_incoming(msg).await
    }

    async fn handle_incoming(
        &mut self,
        msg: SelfRef<Message<'static>>,
    ) -> Result<SessionEvent, SessionError> {
        let conn_id = msg.connection_id();

        match msg.payload() {
            MessagePayload::Hello(_) => {
                let description = "unexpected Hello after handshake";
                Err(self.protocol_violation(description).await)
            }
            MessagePayload::HelloYourself(_) => {
                let description = "unexpected HelloYourself after handshake";
                Err(self.protocol_violation(description).await)
            }
            MessagePayload::ProtocolError(err) => {
                self.teardown_local().await;
                Err(SessionError::RemoteProtocolError(err.description.clone()))
            }
            MessagePayload::OpenConnection(open) => {
                if conn_id.is_root() {
                    return Err(self
                        .protocol_violation("OpenConnection cannot use connection 0")
                        .await);
                }
                if !id_matches_parity(conn_id.0, &self.peer_session_parity) {
                    return Err(self
                        .protocol_violation("OpenConnection ID parity mismatch")
                        .await);
                }
                if self.connections.contains_key(&conn_id)
                    || self.pending_inbound.contains_key(&conn_id)
                    || self.pending_outbound.contains_key(&conn_id)
                {
                    return Err(self
                        .protocol_violation("OpenConnection for already-used ID")
                        .await);
                }

                self.pending_inbound.insert(
                    conn_id,
                    PendingInboundOpen {
                        peer_parity: open.parity.clone(),
                        peer_settings: open.connection_settings.clone(),
                    },
                );

                Ok(SessionEvent::IncomingConnectionOpen {
                    conn_id,
                    peer_parity: open.parity.clone(),
                    peer_settings: open.connection_settings.clone(),
                    metadata: open.metadata.clone(),
                })
            }
            MessagePayload::AcceptConnection(accept) => {
                if conn_id.is_root() {
                    return Err(self
                        .protocol_violation("AcceptConnection cannot use connection 0")
                        .await);
                }
                let pending = self.pending_outbound.remove(&conn_id).ok_or_else(|| {
                    SessionError::ProtocolViolation(
                        "AcceptConnection without matching outbound open".into(),
                    )
                })?;

                self.connections.insert(
                    conn_id,
                    ConnectionState {
                        id: conn_id,
                        local_parity: pending.local_parity.clone(),
                        peer_parity: pending.local_parity.other(),
                        local_settings: pending.local_settings,
                        peer_settings: accept.connection_settings.clone(),
                    },
                );
                Ok(SessionEvent::OutgoingConnectionAccepted { conn_id })
            }
            MessagePayload::RejectConnection(reject) => {
                if conn_id.is_root() {
                    return Err(self
                        .protocol_violation("RejectConnection cannot use connection 0")
                        .await);
                }
                if self.pending_outbound.remove(&conn_id).is_none() {
                    return Err(self
                        .protocol_violation("RejectConnection without matching outbound open")
                        .await);
                }
                Ok(SessionEvent::OutgoingConnectionRejected {
                    conn_id,
                    metadata: reject.metadata.clone(),
                })
            }
            MessagePayload::CloseConnection(close) => {
                if conn_id.is_root() {
                    return Err(self
                        .protocol_violation("CloseConnection cannot use connection 0")
                        .await);
                }
                if self.connections.remove(&conn_id).is_none() {
                    return Err(self
                        .protocol_violation("CloseConnection for unknown ID")
                        .await);
                }
                self.pending_inbound.remove(&conn_id);
                self.pending_outbound.remove(&conn_id);

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
                if !self.connections.contains_key(&conn_id) {
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
        send_message(tx, msg).await
    }

    async fn protocol_violation(&mut self, description: &str) -> SessionError {
        let error = SessionError::ProtocolViolation(description.to_string());
        if let Some(tx) = self.tx.take() {
            let protocol_error = Message::new(
                ConnectionId::ROOT,
                MessagePayload::ProtocolError(ProtocolError {
                    description: description.to_string(),
                }),
            );
            let _ = send_message(&tx, protocol_error).await;
            let _ = tx.close().await;
        }
        self.connections.clear();
        self.pending_inbound.clear();
        self.pending_outbound.clear();
        error
    }

    async fn teardown_local(&mut self) {
        if let Some(tx) = self.tx.take() {
            let _ = tx.close().await;
        }
        self.connections.clear();
        self.pending_inbound.clear();
        self.pending_outbound.clear();
    }
}

pub fn initiator<C>(conduit: C) -> SessionInitiatorBuilder<C> {
    SessionInitiatorBuilder::new(conduit)
}

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
        C: Conduit<Msg<'static> = Message<'static>>,
        C::Tx: for<'a> ConduitTx<Msg<'a> = Message<'a>>,
        C::Rx: ConduitRx<Msg<'static> = Message<'static>>,
    {
        let (tx, mut rx) = self.conduit.split();

        let hello = Message::new(
            ConnectionId::ROOT,
            MessagePayload::Hello(Hello {
                version: PROTOCOL_VERSION,
                parity: self.parity.clone(),
                connection_settings: self.root_settings.clone(),
                metadata: self.metadata,
            }),
        );
        send_message(&tx, hello).await?;

        let first = recv_message(&mut rx)
            .await?
            .ok_or(SessionError::UnexpectedEof("waiting for HelloYourself"))?;

        match first.payload() {
            MessagePayload::HelloYourself(reply) => {
                if !first.connection_id().is_root() {
                    let _ =
                        send_protocol_error_and_close(tx, "HelloYourself must use connection ID 0")
                            .await;
                    return Err(SessionError::ProtocolViolation(
                        "HelloYourself used non-root connection ID".into(),
                    ));
                }

                Ok(Session::new(
                    tx,
                    rx,
                    SessionRole::Initiator,
                    self.parity.clone(),
                    self.parity.other(),
                    self.root_settings,
                    reply.connection_settings.clone(),
                ))
            }
            MessagePayload::ProtocolError(err) => {
                let _ = tx.close().await;
                Err(SessionError::RemoteProtocolError(err.description.clone()))
            }
            _ => {
                let _ = send_protocol_error_and_close(tx, "expected HelloYourself").await;
                Err(SessionError::ProtocolViolation(
                    "expected HelloYourself during handshake".into(),
                ))
            }
        }
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
        C: Conduit<Msg<'static> = Message<'static>>,
        C::Tx: for<'a> ConduitTx<Msg<'a> = Message<'a>>,
        C::Rx: ConduitRx<Msg<'static> = Message<'static>>,
    {
        let (tx, mut rx) = self.conduit.split();
        let first = recv_message(&mut rx)
            .await?
            .ok_or(SessionError::UnexpectedEof("waiting for Hello"))?;

        let MessagePayload::Hello(hello) = first.payload() else {
            if let MessagePayload::ProtocolError(err) = first.payload() {
                let _ = tx.close().await;
                return Err(SessionError::RemoteProtocolError(err.description.clone()));
            }
            let _ = send_protocol_error_and_close(tx, "expected Hello").await;
            return Err(SessionError::ProtocolViolation(
                "expected Hello during handshake".into(),
            ));
        };

        if !first.connection_id().is_root() {
            let _ = send_protocol_error_and_close(tx, "Hello must use connection ID 0").await;
            return Err(SessionError::ProtocolViolation(
                "Hello used non-root connection ID".into(),
            ));
        }
        if hello.version != PROTOCOL_VERSION {
            let _ = send_protocol_error_and_close(tx, "unsupported Hello version").await;
            return Err(SessionError::ProtocolViolation(format!(
                "unsupported Hello version {}",
                hello.version
            )));
        }

        let local_parity = hello.parity.other();
        let peer_parity = hello.parity.clone();
        let peer_root_settings = hello.connection_settings.clone();

        let reply = Message::new(
            ConnectionId::ROOT,
            MessagePayload::HelloYourself(HelloYourself {
                connection_settings: self.root_settings.clone(),
                metadata: self.metadata,
            }),
        );
        send_message(&tx, reply).await?;

        Ok(Session::new(
            tx,
            rx,
            SessionRole::Acceptor,
            local_parity,
            peer_parity,
            self.root_settings,
            peer_root_settings,
        ))
    }
}

pub async fn establish_initiator<C>(
    conduit: C,
    parity: Parity,
    local_root_settings: ConnectionSettings,
    metadata: Metadata,
) -> Result<Session<C>, SessionError>
where
    C: Conduit<Msg<'static> = Message<'static>>,
    C::Tx: for<'a> ConduitTx<Msg<'a> = Message<'a>>,
    C::Rx: ConduitRx<Msg<'static> = Message<'static>>,
{
    initiator(conduit)
        .parity(parity)
        .root_settings(local_root_settings)
        .metadata(metadata)
        .establish()
        .await
}

pub async fn establish_acceptor<C>(
    conduit: C,
    local_root_settings: ConnectionSettings,
    metadata: Metadata,
) -> Result<Session<C>, SessionError>
where
    C: Conduit<Msg<'static> = Message<'static>>,
    C::Tx: for<'a> ConduitTx<Msg<'a> = Message<'a>>,
    C::Rx: ConduitRx<Msg<'static> = Message<'static>>,
{
    acceptor(conduit)
        .root_settings(local_root_settings)
        .metadata(metadata)
        .establish()
        .await
}

async fn send_message<'msg, Tx>(tx: &Tx, msg: Message<'msg>) -> Result<(), SessionError>
where
    Tx: for<'a> ConduitTx<Msg<'a> = Message<'a>>,
{
    let permit = tx
        .reserve()
        .await
        .map_err(|e| SessionError::Transport(format!("reserve failed: {e}")))?;
    permit
        .send(msg)
        .map_err(|e| SessionError::Transport(format!("send failed: {e}")))?;
    Ok(())
}

async fn recv_message<Rx>(rx: &mut Rx) -> Result<Option<SelfRef<Message<'static>>>, SessionError>
where
    Rx: ConduitRx<Msg<'static> = Message<'static>>,
{
    rx.recv()
        .await
        .map_err(|e| SessionError::Transport(format!("recv failed: {e}")))
}

async fn send_protocol_error_and_close<Tx>(tx: Tx, description: &str) -> Result<(), SessionError>
where
    Tx: for<'a> ConduitTx<Msg<'a> = Message<'a>>,
{
    let protocol_error = Message::new(
        ConnectionId::ROOT,
        MessagePayload::ProtocolError(ProtocolError {
            description: description.to_string(),
        }),
    );
    let _ = send_message(&tx, protocol_error).await;
    tx.close()
        .await
        .map_err(|e| SessionError::Transport(format!("close failed: {e}")))?;
    Ok(())
}

fn id_matches_parity(id: u64, parity: &Parity) -> bool {
    match parity {
        Parity::Odd => id % 2 == 1,
        Parity::Even => id % 2 == 0,
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
