use facet_reflect::TypePlan;
use roam_types::{Conduit, ConduitRx, ConduitTx, ConduitTxPermit};

use crate::{BareConduit, memory_link_pair};

/// Create a connected pair of BareConduits over MemoryLink for String messages.
fn conduit_pair() -> (impl Conduit<String>, impl Conduit<String>) {
    let (a, b) = memory_link_pair(16);
    (
        BareConduit::new(a, TypePlan::<String>::build().unwrap()),
        BareConduit::new(b, TypePlan::<String>::build().unwrap()),
    )
}

#[tokio::test]
async fn send_recv_single() {
    let (client, server) = conduit_pair();
    let (client_tx, _client_rx) = client.split();
    let (_server_tx, mut server_rx) = server.split();

    let permit = client_tx.reserve().await.unwrap();
    permit.send(&"hello".to_string()).await.unwrap();

    let received = server_rx.recv().await.unwrap().unwrap();
    assert_eq!(&*received, "hello");
}

#[tokio::test]
async fn send_recv_multiple_in_order() {
    let (client, server) = conduit_pair();
    let (client_tx, _client_rx) = client.split();
    let (_server_tx, mut server_rx) = server.split();

    for i in 0..10 {
        let permit = client_tx.reserve().await.unwrap();
        permit.send(&format!("msg-{i}")).await.unwrap();
    }

    for i in 0..10 {
        let received = server_rx.recv().await.unwrap().unwrap();
        assert_eq!(&*received, &format!("msg-{i}"));
    }
}

#[tokio::test]
async fn bidirectional() {
    let (client, server) = conduit_pair();
    let (client_tx, mut client_rx) = client.split();
    let (server_tx, mut server_rx) = server.split();

    let permit = client_tx.reserve().await.unwrap();
    permit.send(&"from-client".to_string()).await.unwrap();

    let received = server_rx.recv().await.unwrap().unwrap();
    assert_eq!(&*received, "from-client");

    let permit = server_tx.reserve().await.unwrap();
    permit.send(&"from-server".to_string()).await.unwrap();

    let received = client_rx.recv().await.unwrap().unwrap();
    assert_eq!(&*received, "from-server");
}

#[tokio::test]
async fn close_signals_end() {
    let (client, server) = conduit_pair();
    let (client_tx, _client_rx) = client.split();
    let (_server_tx, mut server_rx) = server.split();

    let permit = client_tx.reserve().await.unwrap();
    permit.send(&"last".to_string()).await.unwrap();
    client_tx.close().await.unwrap();

    let received = server_rx.recv().await.unwrap().unwrap();
    assert_eq!(&*received, "last");

    let end = server_rx.recv().await.unwrap();
    assert!(end.is_none());
}

#[tokio::test]
async fn interleaved_send_recv() {
    let (client, server) = conduit_pair();
    let (client_tx, mut client_rx) = client.split();
    let (server_tx, mut server_rx) = server.split();

    for i in 0..5 {
        let permit = client_tx.reserve().await.unwrap();
        permit.send(&format!("c2s-{i}")).await.unwrap();

        let received = server_rx.recv().await.unwrap().unwrap();
        assert_eq!(&*received, &format!("c2s-{i}"));

        let permit = server_tx.reserve().await.unwrap();
        permit.send(&format!("s2c-{i}")).await.unwrap();

        let received = client_rx.recv().await.unwrap().unwrap();
        assert_eq!(&*received, &format!("s2c-{i}"));
    }
}
