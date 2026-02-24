use moire::task::FutureExt;
use roam_types::{Conduit, ConduitTx, ConnectionSettings, MessageFamily, Metadata, Parity};

use super::{ConnectionHandle, Session, SessionError};

// r[impl session.role]
pub fn initiator<C>(conduit: C) -> SessionInitiatorBuilder<'static, C> {
    SessionInitiatorBuilder::new(conduit)
}

// r[impl session.role]
pub fn acceptor<C>(conduit: C) -> SessionAcceptorBuilder<'static, C> {
    SessionAcceptorBuilder::new(conduit)
}

pub struct SessionInitiatorBuilder<'a, C> {
    conduit: C,
    root_settings: ConnectionSettings,
    metadata: Metadata<'a>,
}

impl<'a, C> SessionInitiatorBuilder<'a, C> {
    fn new(conduit: C) -> Self {
        Self {
            conduit,
            root_settings: ConnectionSettings {
                parity: Parity::Odd,
                max_concurrent_requests: 64,
            },
            metadata: &[],
        }
    }

    pub fn parity(mut self, parity: Parity) -> Self {
        self.root_settings.parity = parity;
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

    pub fn metadata(mut self, metadata: Metadata<'a>) -> Self {
        self.metadata = metadata;
        self
    }

    pub async fn establish(self) -> Result<(Session<C>, ConnectionHandle), SessionError>
    where
        C: Conduit<Msg = MessageFamily>,
        C::Tx: Send + Sync + 'static,
        for<'p> <C::Tx as ConduitTx>::Permit<'p>: Send,
        C::Rx: Send,
    {
        let (tx, rx) = self.conduit.split();
        let mut session = Session::pre_handshake(tx, rx);
        let handle = session
            .establish_as_initiator(self.root_settings, self.metadata)
            .await?;
        Ok((session, handle))
    }
}

pub struct SessionAcceptorBuilder<'a, C> {
    conduit: C,
    root_settings: ConnectionSettings,
    metadata: Metadata<'a>,
}

impl<'a, C> SessionAcceptorBuilder<'a, C> {
    fn new(conduit: C) -> Self {
        Self {
            conduit,
            root_settings: ConnectionSettings {
                parity: Parity::Even,
                max_concurrent_requests: 64,
            },
            metadata: &[],
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

    pub fn metadata(mut self, metadata: Metadata<'a>) -> Self {
        self.metadata = metadata;
        self
    }

    pub async fn establish(self) -> Result<(Session<C>, ConnectionHandle), SessionError>
    where
        C: Conduit<Msg = MessageFamily>,
        C::Tx: Send + Sync + 'static,
        for<'p> <C::Tx as ConduitTx>::Permit<'p>: Send,
        C::Rx: Send,
    {
        let (tx, rx) = self.conduit.split();
        let mut session = Session::pre_handshake(tx, rx);
        let handle = session
            .establish_as_acceptor(self.root_settings, self.metadata)
            .await?;
        Ok((session, handle))
    }
}
