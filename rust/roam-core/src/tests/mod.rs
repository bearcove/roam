use facet::Facet;
use facet_core::ConstParamKind;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::{ChannelSink, IncomingChannelMessage, Rx, Tx};
use roam_types::{
    Backing, ChannelId, Conduit, ConduitRx, ConduitTx, ConduitTxPermit, ConnectionId, Message,
    Metadata, MsgFamily, Payload, SelfRef,
};
use tokio::sync::{Notify, mpsc};

use crate::{BareConduit, MemoryLink, memory_link_pair};

mod session;

struct StringFamily;

impl MsgFamily for StringFamily {
    type Msg<'a> = String;

    fn shape() -> &'static facet_core::Shape {
        String::SHAPE
    }
}

type StringConduit = BareConduit<StringFamily, MemoryLink>;

/// Create a connected pair of BareConduits over MemoryLink for String messages.
fn conduit_pair() -> (StringConduit, StringConduit) {
    let (a, b) = memory_link_pair(16);
    (BareConduit::new(a), BareConduit::new(b))
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

struct TestSink {
    gate: Arc<Notify>,
    send_count: Arc<AtomicUsize>,
    close_count: Arc<AtomicUsize>,
}

impl TestSink {
    fn new() -> Self {
        Self {
            gate: Arc::new(Notify::new()),
            send_count: Arc::new(AtomicUsize::new(0)),
            close_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    fn open_gate(&self) {
        self.gate.notify_waiters();
    }
}

impl ChannelSink for TestSink {
    fn send_payload<'a>(
        &self,
        conn_id: ConnectionId,
        channel_id: ChannelId,
        payload: Payload<'a>,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), crate::TxError>> + 'a>> {
        let gate = self.gate.clone();
        let send_count = self.send_count.clone();
        Box::pin(async move {
            gate.notified().await;
            let _ = Message::channel_item(conn_id, channel_id, payload);
            send_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }

    fn close_channel(
        &self,
        conn_id: ConnectionId,
        channel_id: ChannelId,
        metadata: Metadata,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), crate::TxError>> + 'static>>
    {
        let gate = self.gate.clone();
        let close_count = self.close_count.clone();
        Box::pin(async move {
            gate.notified().await;
            let _ = Message::close_channel(conn_id, channel_id, metadata);
            close_count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        })
    }
}

#[tokio::test]
async fn tx_send_waits_for_sink_completion() {
    let mut tx = Tx::<String>::unbound();
    let sink = Arc::new(TestSink::new());
    tx.bind(ConnectionId(1), ChannelId(9), sink.clone());

    let payload = "hello".to_string();
    let fut = tx.send(&payload);
    tokio::pin!(fut);
    tokio::select! {
        res = &mut fut => panic!("send completed too early: {res:?}"),
        _ = tokio::time::sleep(std::time::Duration::from_millis(20)) => {}
    }

    sink.open_gate();
    fut.await.expect("send should complete once sink opens");
    assert_eq!(sink.send_count.load(Ordering::SeqCst), 1);
}

#[derive(Facet)]
struct BorrowedMsg<'a> {
    text: &'a str,
}

#[tokio::test]
async fn tx_send_accepts_borrowed_payloads() {
    type Schema = BorrowedMsg<'static>;

    let mut tx = Tx::<Schema>::unbound();
    let sink = Arc::new(TestSink::new());
    tx.bind(ConnectionId(3), ChannelId(5), sink.clone());

    let backing = String::from("borrowed");
    let msg = BorrowedMsg { text: &backing };
    let fut = tx.send(&msg);
    tokio::pin!(fut);
    tokio::select! {
        res = &mut fut => panic!("send completed too early: {res:?}"),
        _ = tokio::time::sleep(std::time::Duration::from_millis(20)) => {}
    }
    sink.open_gate();
    fut.await.expect("borrowed send should succeed");
}

#[tokio::test]
async fn rx_recv_decodes_channel_items() {
    let mut rx = Rx::<u32>::unbound();
    let (tx_items, rx_items) = mpsc::channel(4);
    rx.bind(ConnectionId(11), ChannelId(15), rx_items);

    let payload_bytes = facet_postcard::to_vec(&42_u32).expect("serialize channel item");
    let item_msg = Message::channel_item(
        ConnectionId(11),
        ChannelId(15),
        Payload::RawOwned(payload_bytes),
    );
    let item_ref = SelfRef::owning(Backing::Boxed(Box::<[u8]>::default()), item_msg);
    tx_items
        .send(IncomingChannelMessage::Data(item_ref))
        .await
        .expect("send data to rx");

    assert_eq!(rx.recv().await.expect("recv data"), Some(42));

    tx_items
        .send(IncomingChannelMessage::Close)
        .await
        .expect("send close to rx");
    assert_eq!(rx.recv().await.expect("recv close"), None);
}
