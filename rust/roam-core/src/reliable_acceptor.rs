use std::collections::HashMap;

use roam_types::{Attachment, Codec, Link, LinkRx, LinkSource, Packet, ResumeKey, SplitLink};
use tokio::sync::mpsc;

use crate::ReliableLink;

// ---------------------------------------------------------------------------
// ChannelLinkSource
// ---------------------------------------------------------------------------

/// A [`LinkSource`] backed by an mpsc channel.
///
/// Used on the server side: the [`ReliableAcceptor`] pushes replacement
/// links into the sender half; the `ReliableLink` awaits on this receiver.
pub struct ChannelLinkSource<Tx, Rx> {
    rx: mpsc::Receiver<Attachment<SplitLink<Tx, Rx>>>,
}

impl<T, C, Tx, Rx> LinkSource<T, C> for ChannelLinkSource<Tx, Rx>
where
    T: Send + 'static,
    C: Codec,
    Tx: roam_types::LinkTx<Packet<T>, C> + Send + 'static,
    Rx: LinkRx<Packet<T>, C> + Send + 'static,
{
    type Link = SplitLink<Tx, Rx>;

    async fn next_link(&mut self) -> std::io::Result<Attachment<Self::Link>> {
        self.rx.recv().await.ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "acceptor dropped — no more reconnects",
            )
        })
    }
}

// ---------------------------------------------------------------------------
// ReliableAcceptor
// ---------------------------------------------------------------------------

/// Server-side router for the reliability layer.
///
/// Owns the mapping from [`ResumeKey`] to existing [`ReliableLink`] instances.
/// The server's accept loop hands each new raw connection to
/// [`ingest`](Self::ingest); the acceptor reads the first [`Packet::Hello`],
/// routes reconnects to existing sessions, and yields new sessions via
/// [`accept`](Self::accept).
pub struct ReliableAcceptor<T, C, L>
where
    T: Send + 'static,
    C: Codec,
    L: Link<Packet<T>, C>,
{
    /// Existing sessions keyed by resume key.
    /// Each sender pushes a replacement link attachment into the
    /// corresponding ReliableLink's ChannelLinkSource.
    sessions: HashMap<ResumeKey, mpsc::Sender<Attachment<SplitLink<L::Tx, L::Rx>>>>,

    /// New sessions ready to be yielded to user code.
    new_sessions_tx: mpsc::Sender<NewSession<T, C, L>>,
    new_sessions_rx: mpsc::Receiver<NewSession<T, C, L>>,

    /// Counter for generating unique resume keys.
    next_key_id: u64,
}

/// A newly established session, returned from [`ReliableAcceptor::accept`].
pub struct NewSession<T, C, L>
where
    T: Send + 'static,
    C: Codec,
    L: Link<Packet<T>, C>,
{
    pub resume_key: ResumeKey,
    pub reliable_link: ReliableLink<T, C, ChannelLinkSource<L::Tx, L::Rx>>,
}

impl<T, C, L> ReliableAcceptor<T, C, L>
where
    T: Send + Clone + 'static,
    C: Codec,
    L: Link<Packet<T>, C> + Send + 'static,
    L::Tx: Send + 'static,
    L::Rx: Send + 'static,
{
    /// Create a new acceptor.
    pub fn new() -> Self {
        let (new_sessions_tx, new_sessions_rx) = mpsc::channel(16);
        Self {
            sessions: HashMap::new(),
            new_sessions_tx,
            new_sessions_rx,
            next_key_id: 0,
        }
    }

    /// Ingest a raw connection from the transport accept loop.
    ///
    /// Splits the link, reads the first `Packet::Hello` from Rx to
    /// determine routing:
    ///
    /// - **Reconnect** (has resume key): wraps (tx, rx) in a [`SplitLink`],
    ///   bundles with the peer's Hello into an [`Attachment`], and pushes it
    ///   to the existing session's channel.
    /// - **New session**: assigns a resume key, creates a [`ReliableLink`]
    ///   with a channel-based [`ChannelLinkSource`], and queues the new
    ///   session for [`accept`](Self::accept).
    pub async fn ingest(&mut self, link: L) -> Result<(), IngestError> {
        let (tx, mut rx) = link.split();

        // Read the first packet — must be Hello.
        let hello_packet = rx
            .recv()
            .await
            .map_err(|_| IngestError::ReadFailed)?
            .ok_or(IngestError::ConnectionClosed)?;

        let peer_hello = match &*hello_packet {
            Packet::Hello(hello) => hello.clone(),
            Packet::Data { .. } => return Err(IngestError::ExpectedHello),
        };
        drop(hello_packet);

        let split_link = SplitLink { tx, rx };

        if let Some(resume_key) = &peer_hello.resume_key {
            // Reconnect: route to existing session.
            let sender = self
                .sessions
                .get(resume_key)
                .ok_or(IngestError::UnknownResumeKey)?;

            let attachment = Attachment {
                link: split_link,
                peer_hello: Some(peer_hello),
            };

            sender
                .send(attachment)
                .await
                .map_err(|_| IngestError::SessionDead)?;
        } else {
            // New session: assign a resume key, create ReliableLink.
            let resume_key = self.generate_resume_key();

            let (attach_tx, attach_rx) = mpsc::channel(1);
            self.sessions.insert(resume_key.clone(), attach_tx);

            let channel_source = ChannelLinkSource { rx: attach_rx };

            let attachment = Attachment {
                link: split_link,
                peer_hello: Some(peer_hello),
            };

            let reliable_link =
                ReliableLink::new(channel_source, attachment, Some(resume_key.clone()))
                    .await
                    .map_err(|_| IngestError::HelloFailed)?;

            let new_session = NewSession {
                resume_key,
                reliable_link,
            };

            self.new_sessions_tx
                .send(new_session)
                .await
                .map_err(|_| IngestError::AcceptorFull)?;
        }

        Ok(())
    }

    /// Wait for the next new session.
    ///
    /// Reconnects are handled internally and never surface here.
    pub async fn accept(&mut self) -> Option<NewSession<T, C, L>> {
        self.new_sessions_rx.recv().await
    }

    /// Remove expired session mappings (called when a session dies).
    pub fn remove_session(&mut self, key: &ResumeKey) {
        self.sessions.remove(key);
    }

    fn generate_resume_key(&mut self) -> ResumeKey {
        self.next_key_id += 1;
        // In production, use a cryptographically random key.
        // For now, a monotonic ID suffices to demonstrate the routing.
        ResumeKey(self.next_key_id.to_be_bytes().to_vec())
    }
}

/// Errors from [`ReliableAcceptor::ingest`].
#[derive(Debug)]
pub enum IngestError {
    /// Failed to read the first packet from the raw link.
    ReadFailed,
    /// Connection closed before Hello was received.
    ConnectionClosed,
    /// First packet was Data, not Hello.
    ExpectedHello,
    /// Resume key doesn't match any known session.
    UnknownResumeKey,
    /// The target session's ReliableLink has been dropped.
    SessionDead,
    /// Hello exchange failed on new session.
    HelloFailed,
    /// Internal channel full (new_sessions backlog).
    AcceptorFull,
}

impl std::fmt::Display for IngestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ReadFailed => write!(f, "failed to read Hello packet"),
            Self::ConnectionClosed => write!(f, "connection closed before Hello"),
            Self::ExpectedHello => write!(f, "first packet was Data, expected Hello"),
            Self::UnknownResumeKey => write!(f, "resume key not found"),
            Self::SessionDead => write!(f, "target session has been dropped"),
            Self::HelloFailed => write!(f, "Hello exchange failed on new session"),
            Self::AcceptorFull => write!(f, "new session backlog full"),
        }
    }
}

impl std::error::Error for IngestError {}
