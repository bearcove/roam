use facet::Facet;
use facet_core::ConstParamKind;

use std::sync::OnceLock;

use crate::{Rx, Tx};
use roam_types::{Conduit, ConduitRx, ConduitTx, ConduitTxPermit, MsgFamily, RpcPlan};

use crate::{BareConduit, MemoryLink, memory_link_pair};

struct StringFamily;

impl MsgFamily for StringFamily {
    type Msg<'a> = String;

    fn shape() -> &'static facet_core::Shape {
        String::SHAPE
    }
}

type StringConduit = BareConduit<StringFamily, MemoryLink>;

fn string_plan() -> &'static RpcPlan {
    static PLAN: OnceLock<RpcPlan> = OnceLock::new();
    PLAN.get_or_init(|| RpcPlan::for_type::<String, (), ()>())
}

/// Create a connected pair of BareConduits over MemoryLink for String messages.
fn conduit_pair() -> (StringConduit, StringConduit) {
    let (a, b) = memory_link_pair(16);
    let plan = string_plan();
    (BareConduit::new(a, plan), BareConduit::new(b, plan))
}

#[tokio::test]
async fn send_recv_single() {
    let (client, server) = conduit_pair();
    let (client_tx, _client_rx) = client.split();
    let (_server_tx, mut server_rx) = server.split();

    let permit = client_tx.reserve().await.unwrap();
    permit.send("hello".to_string()).unwrap();

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
        permit.send(format!("msg-{i}")).unwrap();
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
    permit.send("from-client".to_string()).unwrap();

    let received = server_rx.recv().await.unwrap().unwrap();
    assert_eq!(&*received, "from-client");

    let permit = server_tx.reserve().await.unwrap();
    permit.send("from-server".to_string()).unwrap();

    let received = client_rx.recv().await.unwrap().unwrap();
    assert_eq!(&*received, "from-server");
}

#[tokio::test]
async fn close_signals_end() {
    let (client, server) = conduit_pair();
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
    let (client, server) = conduit_pair();
    let (client_tx, mut client_rx) = client.split();
    let (server_tx, mut server_rx) = server.split();

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

#[test]
fn tx_rx_const_param_n() {
    let tx_shape = Tx::<String, 32>::SHAPE;
    let cp = tx_shape.const_params;
    assert_eq!(cp.len(), 1);
    assert_eq!(cp[0].name, "N");
    assert_eq!(cp[0].kind, ConstParamKind::Usize);
    assert_eq!(cp[0].value, 32);

    let rx_shape = Rx::<String, 8>::SHAPE;
    let cp = rx_shape.const_params;
    assert_eq!(cp.len(), 1);
    assert_eq!(cp[0].name, "N");
    assert_eq!(cp[0].kind, ConstParamKind::Usize);
    assert_eq!(cp[0].value, 8);
}
