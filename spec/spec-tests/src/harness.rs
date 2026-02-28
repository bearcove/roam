use std::future::Future;
use std::time::Duration;

use roam_core::{BareConduit, Driver, DriverCaller, DriverReplySink, acceptor};
use roam_stream::StreamLink;
use roam_types::{MessageFamily, Parity, RequestCall, SelfRef};
use spec_proto::TestbedClient;
use std::process::Stdio;
use tokio::net::TcpListener;
use tokio::process::{Child, Command};

type TcpLink = StreamLink<tokio::net::tcp::OwnedReadHalf, tokio::net::tcp::OwnedWriteHalf>;

struct NoopHandler;

impl roam_types::Handler<DriverReplySink> for NoopHandler {
    async fn handle(&self, _call: SelfRef<RequestCall<'static>>, _reply: DriverReplySink) {}
}

pub fn workspace_root() -> &'static std::path::Path {
    // `spec/spec-tests` → `spec` → workspace root
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root")
}

pub fn subject_cmd() -> String {
    match std::env::var("SUBJECT_CMD") {
        Ok(s) if !s.trim().is_empty() => s,
        _ => "./target/release/subject-rust".to_string(),
    }
}

pub fn run_async<T>(f: impl Future<Output = T>) -> T {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    rt.block_on(f)
}

/// Spawn the subject binary, telling it to connect to `peer_addr`.
pub async fn spawn_subject(peer_addr: &str) -> Result<Child, String> {
    let cmd = subject_cmd();

    let mut child = Command::new("sh")
        .current_dir(workspace_root())
        .arg("-lc")
        .arg(cmd)
        .env("PEER_ADDR", peer_addr)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .spawn()
        .map_err(|e| format!("failed to spawn subject: {e}"))?;

    // If it exits immediately, surface that early.
    tokio::time::sleep(Duration::from_millis(10)).await;
    if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
        return Err(format!("subject exited immediately with {status}"));
    }

    Ok(child)
}

/// Listen on a random TCP port, spawn the subject (which connects to us),
/// complete the roam handshake as acceptor, and return a ready `TestbedClient`.
pub async fn accept_subject() -> Result<(TestbedClient<DriverCaller>, Child), String> {
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .map_err(|e| format!("bind: {e}"))?;
    let addr = listener
        .local_addr()
        .map_err(|e| format!("local_addr: {e}"))?;

    let child = spawn_subject(&addr.to_string()).await?;

    let (stream, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
        .await
        .map_err(|_| "subject did not connect within 5s".to_string())?
        .map_err(|e| format!("accept: {e}"))?;
    stream.set_nodelay(true).unwrap();

    let conduit: BareConduit<MessageFamily, TcpLink> = BareConduit::new(StreamLink::tcp(stream));

    let (mut session, handle, _sh) = acceptor(conduit)
        .establish()
        .await
        .map_err(|e| format!("handshake: {e}"))?;

    let mut driver = Driver::new(handle, NoopHandler, Parity::Even);
    let caller = driver.caller();

    moire::task::spawn(async move { session.run().await });
    moire::task::spawn(async move { driver.run().await });

    Ok((TestbedClient::new(caller), child))
}
