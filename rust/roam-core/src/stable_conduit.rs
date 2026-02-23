use facet::Facet;

// ---------------------------------------------------------------------------
// Frame — internal to StableConduit
// ---------------------------------------------------------------------------

#[derive(Facet, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
#[facet(transparent)]
struct PacketSeq(u32);

#[derive(Facet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct PacketAck {
    max_delivered: PacketSeq,
}

/// Opaque session identifier for reliability-layer resume.
///
/// Assigned by the server on first connection. Sent by the client on
/// reconnect so the server can route the new raw link to the correct
/// StableConduit instance.
#[derive(Facet, Debug, Clone, PartialEq, Eq, Hash)]
struct ResumeKey(Vec<u8>);

/// Client's opening handshake, sent as the first message on a new connection.
///
/// The server reads this to determine whether the connection is new or a
/// reconnect, and routes accordingly.
#[derive(Facet, Debug, Clone)]
struct ClientHello {
    /// `None` = new session (server assigns a key).
    /// `Some` = reconnect to existing session.
    resume_key: Option<ResumeKey>,

    /// Last contiguous seq delivered to this peer's upper layer.
    /// The other side replays everything after this.
    last_received: Option<PacketSeq>,
}

/// Server's handshake response.
///
/// Always carries a resume key (assigned for new sessions, confirmed for
/// reconnects).
#[derive(Facet, Debug, Clone)]
struct ServerHello {
    /// The session's resume key. Always present.
    resume_key: ResumeKey,

    /// Last contiguous seq delivered to this peer's upper layer.
    /// The other side replays everything after this.
    last_received: Option<PacketSeq>,
}

/// Sequenced data frame, serialized by StableConduit over a raw Link.
///
/// Handshake messages (ClientHello / ServerHello) are exchanged before
/// data flow begins — they're separate types, not part of this frame.
/// After the handshake, all traffic is `Frame<T>`.
#[derive(Facet, Debug, Clone)]
struct Frame<T> {
    seq: PacketSeq,
    ack: Option<PacketAck>,
    item: T,
}

// ---------------------------------------------------------------------------
// Attachment / LinkSource — for StableConduit reconnect
// ---------------------------------------------------------------------------

/// A raw link bundled with the peer's Hello (if already consumed).
///
/// - **Client** (`client_hello = None`): Dialer connected, no Hello yet.
/// - **Server** (`client_hello = Some`): acceptor already read client's Hello.
struct Attachment<L> {
    link: L,
    client_hello: Option<ClientHello>,
}

/// Source of replacement links for StableConduit reconnect.
///
/// - **Client (pull)**: Dialer that connects and returns a new link.
/// - **Server (push)**: channel receiver from the acceptor.
trait LinkSource: Send + 'static {
    type Link: roam_types::Link;

    async fn next_link(&mut self) -> std::io::Result<Attachment<Self::Link>>;
}
