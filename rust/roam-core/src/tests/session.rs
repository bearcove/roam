use crate::{
    BareConduit, ConnectionState, MemoryLink, PROTOCOL_VERSION, SessionError, SessionEvent,
    establish_acceptor, establish_initiator, memory_link_pair, rpc_plan,
};
use roam_types::{
    Conduit, ConduitRx, ConduitTx, ConduitTxPermit, ConnectionId, ConnectionSettings, Hello,
    Message, MessageFamily, MessagePayload, Parity,
};

type MessageConduit = BareConduit<MessageFamily, MemoryLink>;

fn conduit_pair() -> (MessageConduit, MessageConduit) {
    let (a, b) = memory_link_pair(16);
    let plan = rpc_plan::<Message<'static>>();
    (BareConduit::new(a, plan), BareConduit::new(b, plan))
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
    assert_eq!(
        accepted,
        SessionEvent::OutgoingConnectionAccepted { conn_id }
    );

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
    assert_eq!(
        closed,
        SessionEvent::ConnectionClosed {
            conn_id,
            metadata: vec![]
        }
    );
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
