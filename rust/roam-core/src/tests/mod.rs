use facet_core::ConstParamKind;

use std::sync::OnceLock;

use crate::{Rx, Tx};
use roam_types::{Conduit, ConduitRx, ConduitTx, ConduitTxPermit, RpcPlan};

use crate::{BareConduit, memory_link_pair};

fn string_plan() -> &'static RpcPlan {
    static PLAN: OnceLock<RpcPlan> = OnceLock::new();
    PLAN.get_or_init(|| RpcPlan::for_type::<String, (), ()>())
}

/// Create a connected pair of BareConduits over MemoryLink for String messages.
fn conduit_pair() -> (impl Conduit<String, String>, impl Conduit<String, String>) {
    let (a, b) = memory_link_pair(16);
    let plan = string_plan();
    (
        BareConduit::new_symmetric(a, plan),
        BareConduit::new_symmetric(b, plan),
    )
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

/// Proves that BareConduit accepts borrowed send types (no `'static` required).
/// The send side uses a struct borrowing from the calling scope; the receive
/// side deserializes into the owned equivalent.
#[tokio::test]
async fn send_borrowed_recv_owned() {
    use facet::Facet;

    #[derive(Facet)]
    struct BorrowedMsg<'a> {
        name: &'a str,
        data: &'a [u8],
    }

    #[derive(Facet, Debug, PartialEq)]
    struct OwnedMsg {
        name: String,
        data: Vec<u8>,
    }

    fn owned_plan() -> &'static RpcPlan {
        static PLAN: OnceLock<RpcPlan> = OnceLock::new();
        PLAN.get_or_init(|| RpcPlan::for_type::<OwnedMsg, (), ()>())
    }

    let (a, b) = memory_link_pair(16);
    let plan = owned_plan();

    let sender: BareConduit<BorrowedMsg<'_>, OwnedMsg, _> =
        BareConduit::new(a, BorrowedMsg::SHAPE, plan);
    let receiver: BareConduit<BorrowedMsg<'_>, OwnedMsg, _> =
        BareConduit::new(b, BorrowedMsg::SHAPE, plan);

    let (tx, _) = sender.split();
    let (_, mut rx) = receiver.split();

    // Data borrowed from the calling scope — not 'static.
    let name = String::from("hello");
    let data = vec![1u8, 2, 3];

    let permit = tx.reserve().await.unwrap();
    permit
        .send(BorrowedMsg {
            name: &name,
            data: &data,
        })
        .unwrap();

    let received = rx.recv().await.unwrap().unwrap();
    assert_eq!(
        &*received,
        &OwnedMsg {
            name: "hello".into(),
            data: vec![1, 2, 3],
        }
    );
}

#[test]
fn tx_rx_const_param_n() {
    use facet::Facet;

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
