use std::marker::PhantomData;

use roam_types::{
    Attachment, Codec, EncodedPart, Link, LinkRx, LinkSource, LinkTx, LinkTxPermit, Packet,
    PacketSeq, ReliableHello, ResumeKey, SplitLink,
};

use crate::{
    MemoryLink, MemoryLinkRx, MemoryLinkTx, ReliableAcceptor, ReliableLink, ReliableLinkError,
    memory_link_pair,
};

// ---------------------------------------------------------------------------
// Test infrastructure
// ---------------------------------------------------------------------------

/// Codec that never encodes/decodes. MemoryLink passes values directly
/// through mpsc channels, so these methods are never called.
struct NullCodec;

#[derive(Debug)]
struct NullCodecError;

impl std::fmt::Display for NullCodecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "NullCodec should never encode/decode")
    }
}

impl std::error::Error for NullCodecError {}

impl Codec for NullCodec {
    type EncodeError = NullCodecError;
    type DecodeError = NullCodecError;

    fn encode_scatter<'a>(
        &self,
        _value: facet_reflect::Peek<'a, 'a>,
        _parts: &mut Vec<EncodedPart<'a>>,
    ) -> Result<usize, Self::EncodeError> {
        unreachable!("NullCodec::encode_scatter called")
    }

    unsafe fn decode_into(
        &self,
        _plan: &facet::TypePlanCore,
        _bytes: &[u8],
        _out: facet_core::PtrUninit,
    ) -> Result<(), Self::DecodeError> {
        unreachable!("NullCodec::decode_into called")
    }
}

/// A [`LinkSource`] that never provides a replacement link.
/// Reconnect is not tested here — if triggered, the test hangs.
struct NeverReconnect<L>(PhantomData<L>);

impl<L> NeverReconnect<L> {
    fn new() -> Self {
        Self(PhantomData)
    }
}

impl<T: Send + 'static, C: Codec, L: Link<Packet<T>, C> + Send + 'static> LinkSource<T, C>
    for NeverReconnect<L>
where
    L::Tx: Send + 'static,
    L::Rx: Send + 'static,
{
    type Link = L;

    async fn next_link(&mut self) -> std::io::Result<Attachment<Self::Link>> {
        std::future::pending().await
    }
}

// Type aliases
type TestCodec = NullCodec;
type ClientInner = MemoryLink<Packet<String>, TestCodec>;
type ServerInner =
    SplitLink<MemoryLinkTx<Packet<String>, TestCodec>, MemoryLinkRx<Packet<String>, TestCodec>>;
type ClientReliable = ReliableLink<String, TestCodec, NeverReconnect<ClientInner>>;
type ServerReliable = ReliableLink<String, TestCodec, NeverReconnect<ServerInner>>;

/// Create a connected pair of ReliableLinks over MemoryLink.
///
/// The client side does the full Hello exchange (send + receive).
/// The server side reads the client's Hello manually, then creates
/// its ReliableLink with `peer_hello = Some(...)`.
async fn linked_pair() -> (ClientReliable, ServerReliable) {
    let (client_inner, server_inner) = memory_link_pair::<Packet<String>, TestCodec>(16);

    let client_attachment = Attachment {
        link: client_inner,
        peer_hello: None,
    };

    // Spawn client — it sends Hello then blocks reading the server's Hello.
    let client_handle = tokio::spawn(async move {
        ReliableLink::new(NeverReconnect::new(), client_attachment, None)
            .await
            .unwrap()
    });

    // Server side: split inner link, read client's Hello.
    let (server_tx, mut server_rx) = server_inner.split();

    let hello_packet = server_rx.recv().await.unwrap().unwrap();
    let peer_hello = match &*hello_packet {
        Packet::Hello(h) => h.clone(),
        _ => panic!("expected Hello from client, got Data"),
    };
    drop(hello_packet);

    let server_attachment = Attachment {
        link: SplitLink {
            tx: server_tx,
            rx: server_rx,
        },
        peer_hello: Some(peer_hello),
    };

    let server_reliable = ReliableLink::new(
        NeverReconnect::new(),
        server_attachment,
        Some(ResumeKey(vec![42])),
    )
    .await
    .unwrap();

    let client_reliable = client_handle.await.unwrap();

    (client_reliable, server_reliable)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn hello_exchange() {
    let (_client, _server) = linked_pair().await;
}

#[tokio::test]
async fn send_recv_single() {
    let (client, server) = linked_pair().await;

    let (client_tx, _client_rx) = client.split();
    let (_server_tx, mut server_rx) = server.split();

    let permit = client_tx.reserve().await.unwrap();
    permit.send("hello".to_string()).unwrap();

    let received = server_rx.recv().await.unwrap().unwrap();
    assert_eq!(&*received, "hello");
}

#[tokio::test]
async fn send_recv_multiple_in_order() {
    let (client, server) = linked_pair().await;

    let (client_tx, _client_rx) = client.split();
    let (_server_tx, mut server_rx) = server.split();

    for i in 0..10 {
        let permit = client_tx.reserve().await.unwrap();
        permit.send(format!("msg-{i}")).unwrap();
    }

    for i in 0..10 {
        let received = server_rx.recv().await.unwrap().unwrap();
        assert_eq!(&*received, &format!("msg-{i}"));
    }
}

#[tokio::test]
async fn bidirectional() {
    let (client, server) = linked_pair().await;

    let (client_tx, mut client_rx) = client.split();
    let (server_tx, mut server_rx) = server.split();

    // Client → Server
    let permit = client_tx.reserve().await.unwrap();
    permit.send("from-client".to_string()).unwrap();

    let received = server_rx.recv().await.unwrap().unwrap();
    assert_eq!(&*received, "from-client");

    // Server → Client
    let permit = server_tx.reserve().await.unwrap();
    permit.send("from-server".to_string()).unwrap();

    let received = client_rx.recv().await.unwrap().unwrap();
    assert_eq!(&*received, "from-server");
}

#[tokio::test]
async fn close_signals_end() {
    let (client, server) = linked_pair().await;

    let (client_tx, _client_rx) = client.split();
    let (_server_tx, mut server_rx) = server.split();

    let permit = client_tx.reserve().await.unwrap();
    permit.send("last".to_string()).unwrap();
    client_tx.close().await.unwrap();

    let received = server_rx.recv().await.unwrap().unwrap();
    assert_eq!(&*received, "last");

    let end = server_rx.recv().await.unwrap();
    assert!(end.is_none());
}

#[tokio::test]
async fn interleaved_send_recv() {
    let (client, server) = linked_pair().await;

    let (client_tx, mut client_rx) = client.split();
    let (server_tx, mut server_rx) = server.split();

    // Interleave: client sends, server reads, server sends, client reads
    for i in 0..5 {
        let permit = client_tx.reserve().await.unwrap();
        permit.send(format!("c2s-{i}")).unwrap();

        let received = server_rx.recv().await.unwrap().unwrap();
        assert_eq!(&*received, &format!("c2s-{i}"));

        let permit = server_tx.reserve().await.unwrap();
        permit.send(format!("s2c-{i}")).unwrap();

        let received = client_rx.recv().await.unwrap().unwrap();
        assert_eq!(&*received, &format!("s2c-{i}"));
    }
}

// ---------------------------------------------------------------------------
// ReliableAcceptor tests
// ---------------------------------------------------------------------------

type AcceptorLink = MemoryLink<Packet<String>, TestCodec>;

/// Helper: create a MemoryLink pair where the "client" side sends a Hello
/// and returns the client's tx/rx halves for further interaction.
async fn connect_to_acceptor(
    acceptor: &mut ReliableAcceptor<String, TestCodec, AcceptorLink>,
) -> (
    MemoryLinkTx<Packet<String>, TestCodec>,
    MemoryLinkRx<Packet<String>, TestCodec>,
) {
    let (client_inner, server_inner) = memory_link_pair::<Packet<String>, TestCodec>(16);

    let (client_tx, client_rx) = client_inner.split();

    // Client sends Hello (no resume key = new session).
    let permit = client_tx.reserve().await.unwrap();
    permit
        .send(Packet::Hello(ReliableHello {
            resume_key: None,
            last_received: None,
        }))
        .unwrap();

    // Acceptor ingests the server-side link.
    acceptor.ingest(server_inner).await.unwrap();

    (client_tx, client_rx)
}

#[tokio::test]
async fn acceptor_new_session() {
    let mut acceptor: ReliableAcceptor<String, TestCodec, AcceptorLink> = ReliableAcceptor::new();

    let (_client_tx, mut client_rx) = connect_to_acceptor(&mut acceptor).await;

    // Client should receive the server's Hello response.
    let response = client_rx.recv().await.unwrap().unwrap();
    assert!(
        matches!(&*response, Packet::Hello(_)),
        "expected Hello response from server"
    );
    drop(response);

    // Accept the new session.
    let session = acceptor.accept().await.unwrap();
    assert!(!session.resume_key.0.is_empty());

    // Send data through the session's ReliableLink.
    let (_server_tx, _server_rx) = session.reliable_link.split();
}

#[tokio::test]
async fn acceptor_send_recv_through_session() {
    let mut acceptor: ReliableAcceptor<String, TestCodec, AcceptorLink> = ReliableAcceptor::new();

    let (client_tx, mut client_rx) = connect_to_acceptor(&mut acceptor).await;

    // Read server's Hello.
    let _hello = client_rx.recv().await.unwrap().unwrap();

    let session = acceptor.accept().await.unwrap();
    let (server_tx, mut server_rx) = session.reliable_link.split();

    // Client sends a Data packet directly on the raw link.
    let permit = client_tx.reserve().await.unwrap();
    permit
        .send(Packet::Data {
            seq: PacketSeq(0),
            ack: None,
            item: "from-client".to_string(),
        })
        .unwrap();

    // Server's ReliableLink receives the unwrapped item.
    let received = server_rx.recv().await.unwrap().unwrap();
    assert_eq!(&*received, "from-client");

    // Server sends through ReliableLink, client receives raw Packet.
    let permit = server_tx.reserve().await.unwrap();
    permit.send("from-server".to_string()).unwrap();

    let raw = client_rx.recv().await.unwrap().unwrap();
    match &*raw {
        Packet::Data { seq, item, .. } => {
            assert_eq!(seq.0, 0);
            assert_eq!(item, "from-server");
        }
        _ => panic!("expected Data packet"),
    }
}

#[tokio::test]
async fn acceptor_multiple_sessions() {
    let mut acceptor: ReliableAcceptor<String, TestCodec, AcceptorLink> = ReliableAcceptor::new();

    // Connect three clients.
    let _c1 = connect_to_acceptor(&mut acceptor).await;
    let _c2 = connect_to_acceptor(&mut acceptor).await;
    let _c3 = connect_to_acceptor(&mut acceptor).await;

    // Accept all three — each gets a unique resume key.
    let s1 = acceptor.accept().await.unwrap();
    let s2 = acceptor.accept().await.unwrap();
    let s3 = acceptor.accept().await.unwrap();

    assert_ne!(s1.resume_key, s2.resume_key);
    assert_ne!(s2.resume_key, s3.resume_key);
    assert_ne!(s1.resume_key, s3.resume_key);
}

#[tokio::test]
async fn acceptor_remove_session() {
    let mut acceptor: ReliableAcceptor<String, TestCodec, AcceptorLink> = ReliableAcceptor::new();

    let _c1 = connect_to_acceptor(&mut acceptor).await;
    let session = acceptor.accept().await.unwrap();
    let key = session.resume_key.clone();

    // Session exists.
    acceptor.remove_session(&key);
    // Removing again is a no-op.
    acceptor.remove_session(&key);
}

#[tokio::test]
async fn acceptor_ingest_data_first_is_error() {
    let mut acceptor: ReliableAcceptor<String, TestCodec, AcceptorLink> = ReliableAcceptor::new();

    let (client_inner, server_inner) = memory_link_pair::<Packet<String>, TestCodec>(16);
    let (client_tx, _client_rx) = client_inner.split();

    // Send Data instead of Hello.
    let permit = client_tx.reserve().await.unwrap();
    permit
        .send(Packet::Data {
            seq: PacketSeq(0),
            ack: None,
            item: "oops".to_string(),
        })
        .unwrap();

    let result = acceptor.ingest(server_inner).await;
    assert!(matches!(result, Err(crate::IngestError::ExpectedHello)));
}

#[tokio::test]
async fn acceptor_ingest_closed_is_error() {
    let mut acceptor: ReliableAcceptor<String, TestCodec, AcceptorLink> = ReliableAcceptor::new();

    let (client_inner, server_inner) = memory_link_pair::<Packet<String>, TestCodec>(16);
    // Drop client immediately — server sees closed channel.
    drop(client_inner);

    let result = acceptor.ingest(server_inner).await;
    assert!(matches!(result, Err(crate::IngestError::ConnectionClosed)));
}

#[tokio::test]
async fn acceptor_unknown_resume_key() {
    let mut acceptor: ReliableAcceptor<String, TestCodec, AcceptorLink> = ReliableAcceptor::new();

    let (client_inner, server_inner) = memory_link_pair::<Packet<String>, TestCodec>(16);
    let (client_tx, _client_rx) = client_inner.split();

    // Send Hello with a resume key that doesn't exist.
    let permit = client_tx.reserve().await.unwrap();
    permit
        .send(Packet::Hello(ReliableHello {
            resume_key: Some(ResumeKey(vec![99, 99, 99])),
            last_received: None,
        }))
        .unwrap();

    let result = acceptor.ingest(server_inner).await;
    assert!(matches!(result, Err(crate::IngestError::UnknownResumeKey)));
}

// ---------------------------------------------------------------------------
// ReliableLinkError display coverage
// ---------------------------------------------------------------------------

#[tokio::test]
async fn error_display() {
    let err: ReliableLinkError<std::io::Error> = ReliableLinkError::SequenceGap {
        expected: PacketSeq(5),
        got: PacketSeq(8),
    };
    let msg = format!("{err}");
    assert!(msg.contains("5"));
    assert!(msg.contains("8"));

    let err: ReliableLinkError<std::io::Error> = ReliableLinkError::Transport(std::io::Error::new(
        std::io::ErrorKind::BrokenPipe,
        "broken",
    ));
    let msg = format!("{err}");
    assert!(msg.contains("broken"));

    // Debug coverage
    let _ = format!("{err:?}");
}

// ---------------------------------------------------------------------------
// IngestError display coverage
// ---------------------------------------------------------------------------

#[tokio::test]
async fn ingest_error_display() {
    use crate::IngestError;
    let variants = [
        IngestError::ReadFailed,
        IngestError::ConnectionClosed,
        IngestError::ExpectedHello,
        IngestError::UnknownResumeKey,
        IngestError::SessionDead,
        IngestError::HelloFailed,
        IngestError::AcceptorFull,
    ];
    for v in &variants {
        let msg = format!("{v}");
        assert!(!msg.is_empty());
        let _ = format!("{v:?}");
    }
    // Error trait
    let err: &dyn std::error::Error = &IngestError::ReadFailed;
    let _ = format!("{err}");
}
