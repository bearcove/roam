use crate::{
    BareConduit, ConnectionState, MemoryLink, PROTOCOL_VERSION, SessionError, SessionEvent,
    establish_acceptor, establish_initiator, memory_link_pair,
};
use facet::Facet;
use roam_types::{
    CancelRequest, ChannelId, CloseChannel, Conduit, ConduitRx, ConduitTx, ConduitTxPermit,
    ConnectionId, ConnectionSettings, Hello, Message, MessageFamily, MessagePayload, MethodId,
    Parity, Payload, Request, RequestId,
};

type MessageConduit = BareConduit<MessageFamily, MemoryLink>;

fn conduit_pair() -> (MessageConduit, MessageConduit) {
    let (a, b) = memory_link_pair(16);
    (BareConduit::new(a), BareConduit::new(b))
}

fn default_settings(max_concurrent_requests: u32) -> ConnectionSettings {
    ConnectionSettings {
        max_concurrent_requests,
    }
}

#[tokio::test]
async fn handshake_establishes_root_connection() {
    let (initiator_conduit, acceptor_conduit) = conduit_pair();

    let initiator_fut =
        establish_initiator(initiator_conduit, Parity::Odd, default_settings(17), vec![]);
    let acceptor_fut = establish_acceptor(acceptor_conduit, default_settings(33), vec![]);

    let (initiator, acceptor) = tokio::join!(initiator_fut, acceptor_fut);
    let initiator = initiator.expect("initiator handshake should succeed");
    let acceptor = acceptor.expect("acceptor handshake should succeed");

    assert_eq!(initiator.local_session_parity(), Parity::Odd);
    assert_eq!(initiator.peer_session_parity(), Parity::Even);
    assert_eq!(acceptor.local_session_parity(), Parity::Even);
    assert_eq!(acceptor.peer_session_parity(), Parity::Odd);

    let initiator_root = initiator
        .connection(ConnectionId::ROOT)
        .expect("root exists");
    assert_eq!(initiator_root.local_settings.max_concurrent_requests, 17);
    assert_eq!(initiator_root.peer_settings.max_concurrent_requests, 33);
}

#[tokio::test]
async fn open_accept_and_close_virtual_connection() {
    let (initiator_conduit, acceptor_conduit) = conduit_pair();
    let (initiator, acceptor) = tokio::join!(
        establish_initiator(initiator_conduit, Parity::Odd, default_settings(10), vec![]),
        establish_acceptor(acceptor_conduit, default_settings(20), vec![])
    );
    let mut initiator = initiator.expect("initiator handshake");
    let mut acceptor = acceptor.expect("acceptor handshake");

    let conn_id = ConnectionId(1);
    initiator
        .open_connection(conn_id, Parity::Even, default_settings(111), vec![])
        .await
        .expect("open_connection should send OpenConnection");

    let incoming = acceptor
        .recv_event()
        .await
        .expect("acceptor should get event");
    let SessionEvent::IncomingConnectionOpen {
        conn_id: opened,
        peer_parity,
        peer_settings,
        ..
    } = incoming
    else {
        panic!("expected IncomingConnectionOpen");
    };
    assert_eq!(opened, conn_id);
    assert_eq!(peer_parity, Parity::Even);
    assert_eq!(peer_settings.max_concurrent_requests, 111);

    acceptor
        .accept_connection(conn_id, default_settings(222), vec![])
        .await
        .expect("accept_connection should send AcceptConnection");

    let accepted = initiator
        .recv_event()
        .await
        .expect("initiator should receive accept");
    let SessionEvent::OutgoingConnectionAccepted {
        conn_id: accepted_conn_id,
    } = accepted
    else {
        panic!("expected OutgoingConnectionAccepted");
    };
    assert_eq!(accepted_conn_id, conn_id);

    let ConnectionState {
        local_parity,
        peer_parity,
        local_settings,
        peer_settings,
        ..
    } = initiator
        .connection(conn_id)
        .expect("initiator connection should be active");
    assert_eq!(local_parity, &Parity::Even);
    assert_eq!(peer_parity, &Parity::Odd);
    assert_eq!(local_settings.max_concurrent_requests, 111);
    assert_eq!(peer_settings.max_concurrent_requests, 222);

    initiator
        .close_connection(conn_id, vec![])
        .await
        .expect("close_connection should send CloseConnection");
    assert!(initiator.connection(conn_id).is_none());

    let closed = acceptor
        .recv_event()
        .await
        .expect("acceptor should see close");
    let SessionEvent::ConnectionClosed {
        conn_id: closed_conn_id,
        metadata,
    } = closed
    else {
        panic!("expected ConnectionClosed");
    };
    assert_eq!(closed_conn_id, conn_id);
    assert_eq!(metadata, vec![]);
    assert!(acceptor.connection(conn_id).is_none());
}

#[tokio::test]
async fn acceptor_rejects_invalid_hello_version_with_protocol_error() {
    let (initiator_conduit, acceptor_conduit) = conduit_pair();
    let (initiator_tx, mut initiator_rx) = initiator_conduit.split();

    let initiator_fut = async move {
        let hello = Message::new(
            ConnectionId::ROOT,
            MessagePayload::Hello(Hello {
                version: PROTOCOL_VERSION + 1,
                parity: Parity::Odd,
                connection_settings: default_settings(5),
                metadata: vec![],
            }),
        );
        let permit = initiator_tx.reserve().await.expect("reserve");
        permit.send(hello).expect("send hello");

        initiator_rx
            .recv()
            .await
            .expect("recv transport")
            .expect("protocol error message")
    };

    let (protocol_response, acceptor_result) = tokio::join!(
        initiator_fut,
        establish_acceptor(acceptor_conduit, default_settings(1), vec![])
    );
    match protocol_response.payload() {
        MessagePayload::ProtocolError(err) => {
            assert!(err.description.contains("unsupported Hello version"));
        }
        other => panic!("expected ProtocolError, got {other:?}"),
    }
    match acceptor_result {
        Err(SessionError::ProtocolViolation(msg)) => {
            assert!(msg.contains("unsupported Hello version"));
        }
        Ok(_) => panic!("expected ProtocolViolation error, got success"),
        Err(other) => panic!("expected ProtocolViolation error, got {other}"),
    }
}

#[tokio::test]
async fn recv_event_surfaces_incoming_rpc_and_channel_messages() {
    let (initiator_conduit, acceptor_conduit) = conduit_pair();
    let (initiator, acceptor) = tokio::join!(
        establish_initiator(initiator_conduit, Parity::Odd, default_settings(10), vec![]),
        establish_acceptor(acceptor_conduit, default_settings(20), vec![])
    );
    let initiator = initiator.expect("initiator handshake");
    let mut acceptor = acceptor.expect("acceptor handshake");

    initiator
        .send_rpc_message(Message::new(
            ConnectionId::ROOT,
            MessagePayload::CancelRequest(CancelRequest {
                request_id: RequestId(7),
                metadata: vec![],
            }),
        ))
        .await
        .expect("send rpc cancel");

    let ev = acceptor.recv_event().await.expect("recv cancel");
    let SessionEvent::IncomingMessage(msg) = ev else {
        panic!("expected IncomingMessage event");
    };
    assert_eq!(msg.connection_id(), ConnectionId::ROOT);
    let MessagePayload::CancelRequest(cancel) = msg.payload() else {
        panic!("expected CancelRequest payload");
    };
    assert_eq!(cancel.request_id, RequestId(7));

    initiator
        .send_rpc_message(Message::new(
            ConnectionId::ROOT,
            MessagePayload::CloseChannel(CloseChannel {
                channel_id: ChannelId(9),
                metadata: vec![],
            }),
        ))
        .await
        .expect("send close channel");

    let ev = acceptor.recv_event().await.expect("recv close channel");
    let SessionEvent::IncomingMessage(msg) = ev else {
        panic!("expected IncomingMessage event");
    };
    let MessagePayload::CloseChannel(close) = msg.payload() else {
        panic!("expected CloseChannel payload");
    };
    assert_eq!(close.channel_id, ChannelId(9));
}

#[tokio::test]
async fn send_rpc_message_rejects_non_rpc_payloads_and_unknown_connections() {
    let (initiator_conduit, acceptor_conduit) = conduit_pair();
    let (initiator, _acceptor) = tokio::join!(
        establish_initiator(initiator_conduit, Parity::Odd, default_settings(10), vec![]),
        establish_acceptor(acceptor_conduit, default_settings(20), vec![])
    );
    let initiator = initiator.expect("initiator handshake");

    let err = initiator
        .send_rpc_message(Message::new(
            ConnectionId::ROOT,
            MessagePayload::OpenConnection(roam_types::OpenConnection {
                parity: Parity::Even,
                connection_settings: default_settings(1),
                metadata: vec![],
            }),
        ))
        .await
        .expect_err("OpenConnection should be rejected by send_rpc_message");
    match err {
        SessionError::InvalidState(msg) => {
            assert!(msg.contains("only accepts rpc/channel"));
        }
        other => panic!("expected InvalidState, got {other}"),
    }

    let err = initiator
        .send_rpc_message(Message::new(
            ConnectionId(11),
            MessagePayload::CancelRequest(CancelRequest {
                request_id: RequestId(3),
                metadata: vec![],
            }),
        ))
        .await
        .expect_err("unknown connection should be rejected");
    match err {
        SessionError::InvalidState(msg) => {
            assert!(msg.contains("is not active"));
        }
        other => panic!("expected InvalidState, got {other}"),
    }
}

#[derive(Facet)]
struct BorrowedArgs<'a> {
    label: &'a str,
}

#[derive(Facet)]
struct OwnedArgs {
    label: String,
}

#[tokio::test]
async fn send_rpc_message_supports_borrowed_payload_lifetimes() {
    let (initiator_conduit, acceptor_conduit) = conduit_pair();
    let (initiator, acceptor) = tokio::join!(
        establish_initiator(initiator_conduit, Parity::Odd, default_settings(10), vec![]),
        establish_acceptor(acceptor_conduit, default_settings(20), vec![])
    );
    let initiator = initiator.expect("initiator handshake");
    let mut acceptor = acceptor.expect("acceptor handshake");

    let backing = String::from("borrowed-through-session");
    let args = BorrowedArgs { label: &backing };
    initiator
        .send_rpc_message(Message::new(
            ConnectionId::ROOT,
            MessagePayload::Request(Request {
                request_id: RequestId(5),
                method_id: MethodId(99),
                args: Payload::borrowed(&args),
                channels: vec![],
                metadata: vec![],
            }),
        ))
        .await
        .expect("send borrowed request payload");

    let ev = acceptor.recv_event().await.expect("recv request");
    let SessionEvent::IncomingMessage(msg) = ev else {
        panic!("expected IncomingMessage event");
    };
    let MessagePayload::Request(request) = msg.payload() else {
        panic!("expected Request payload");
    };
    let bytes = request
        .args
        .as_incoming_bytes()
        .expect("incoming request payload bytes");
    let decoded: OwnedArgs = facet_postcard::from_slice(bytes).expect("decode request args");
    assert_eq!(decoded.label, "borrowed-through-session");
}
