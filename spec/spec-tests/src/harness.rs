use std::future::Future;
use std::os::fd::AsRawFd;
use std::os::unix::ffi::OsStrExt;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use roam_core::{BareConduit, Driver, DriverCaller, DriverReplySink, acceptor};
use roam_shm::HostHub;
use roam_shm::bootstrap::decode_request;
use roam_shm::segment::{Segment, SegmentConfig};
use roam_stream::StreamLink;
use roam_types::{MessageFamily, Parity, RequestCall, SelfRef};
use shm_primitives::FileCleanup;
use shm_primitives::SizeClassConfig;
use spec_proto::TestbedClient;
use std::process::Stdio;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;
use tokio::process::{Child, Command};
use tokio::sync::oneshot;
/// Spawn a task that catches panics and makes them loud.
///
/// If the spawned future panics, the panic message is printed to stderr
/// immediately and then re-raised. This prevents the silent-task-panic
/// problem where tokio tasks panic and nobody notices, causing mysterious
/// timeouts in tests.
pub fn spawn_loud<F>(fut: F) -> moire::task::JoinHandle<F::Output>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    moire::task::spawn(async move {
        // Inner spawn so we can catch the panic via JoinError
        let inner = tokio::task::spawn(fut);
        match inner.await {
            Ok(v) => v,
            Err(e) if e.is_panic() => {
                let panic = e.into_panic();
                let msg = panic
                    .downcast_ref::<&str>()
                    .map(|s| s.to_string())
                    .or_else(|| panic.downcast_ref::<String>().cloned())
                    .unwrap_or_else(|| format!("{panic:?}"));
                eprintln!("\n\n!!! SPAWNED TASK PANICKED !!!\n{msg}\n");
                std::panic::resume_unwind(panic);
            }
            Err(e) => {
                panic!("spawned task failed: {e}");
            }
        }
    })
}

type TcpLink = StreamLink<tokio::net::tcp::OwnedReadHalf, tokio::net::tcp::OwnedWriteHalf>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SubjectTransport {
    Tcp,
    Shm,
}

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

fn subject_transport() -> SubjectTransport {
    match std::env::var("SPEC_TRANSPORT")
        .ok()
        .unwrap_or_else(|| "tcp".to_string())
        .to_ascii_lowercase()
        .as_str()
    {
        "shm" => SubjectTransport::Shm,
        _ => SubjectTransport::Tcp,
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
    spawn_subject_with_env(peer_addr, &[]).await
}

async fn spawn_subject_with_env(
    peer_addr: &str,
    extra_env: &[(&str, &str)],
) -> Result<Child, String> {
    let cmd = subject_cmd();

    let mut command = Command::new("sh");
    command
        .current_dir(workspace_root())
        .arg("-lc")
        .arg(cmd)
        .env("PEER_ADDR", peer_addr)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    for (k, v) in extra_env {
        command.env(k, v);
    }

    let mut child = command
        .spawn()
        .map_err(|e| format!("failed to spawn subject: {e}"))?;

    // If it exits immediately, surface that early.
    tokio::time::sleep(Duration::from_millis(10)).await;
    if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
        return Err(format!("subject exited immediately with {status}"));
    }

    Ok(child)
}

fn sid_hex_32() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock before epoch")
        .as_nanos();
    format!("{nanos:032x}")
}

fn leaked_dirs() -> &'static Mutex<Vec<tempfile::TempDir>> {
    static DIRS: OnceLock<Mutex<Vec<tempfile::TempDir>>> = OnceLock::new();
    DIRS.get_or_init(|| Mutex::new(Vec::new()))
}

fn keep_tempdir_alive(dir: tempfile::TempDir) {
    leaked_dirs().lock().expect("tempdir mutex").push(dir);
}

/// Listen on a random TCP port, spawn the subject (which connects to us),
/// complete the roam handshake as acceptor, and return a ready `TestbedClient`.
pub async fn accept_subject() -> Result<(TestbedClient<DriverCaller>, Child), String> {
    match subject_transport() {
        SubjectTransport::Tcp => accept_subject_tcp().await,
        SubjectTransport::Shm => accept_subject_shm().await,
    }
}

async fn accept_subject_tcp() -> Result<(TestbedClient<DriverCaller>, Child), String> {
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

async fn accept_subject_shm() -> Result<(TestbedClient<DriverCaller>, Child), String> {
    let dir = tempfile::tempdir().map_err(|e| format!("tempdir: {e}"))?;
    let sid = sid_hex_32();
    let control_sock_path = dir.path().join("bootstrap.sock");
    let shm_path = dir.path().join("subject.shm");

    let size_classes = [SizeClassConfig {
        slot_size: 4096,
        slot_count: 8,
    }];
    let segment = Arc::new(
        Segment::create(
            &shm_path,
            SegmentConfig {
                max_guests: 1,
                bipbuf_capacity: 64 * 1024,
                max_payload_size: 1024 * 1024,
                inline_threshold: 256,
                heartbeat_interval: 0,
                size_classes: &size_classes,
            },
            FileCleanup::Manual,
        )
        .map_err(|e| format!("segment create: {e}"))?,
    );
    let hub = Arc::new(HostHub::new(Arc::clone(&segment)));

    let listener = tokio::net::UnixListener::bind(&control_sock_path)
        .map_err(|e| format!("unix bind {}: {e}", control_sock_path.display()))?;

    let (peer_tx, peer_rx) = oneshot::channel();
    let sid_for_task = sid.clone();
    let shm_path_for_task = shm_path.clone();
    let hub_for_task = Arc::clone(&hub);
    let segment_for_task = Arc::clone(&segment);
    tokio::spawn(async move {
        let result: Result<roam_shm::host::HostPeer, String> = async {
            let (mut stream, _) = listener
                .accept()
                .await
                .map_err(|e| format!("accept: {e}"))?;
            let mut request_buf = [0u8; 2048];
            let n = stream
                .read(&mut request_buf)
                .await
                .map_err(|e| format!("read bootstrap request: {e}"))?;
            if n == 0 {
                return Err("bootstrap request EOF".to_string());
            }
            let request = decode_request(&request_buf[..n])
                .map_err(|e| format!("decode bootstrap request: {e}"))?;
            let got_sid = String::from_utf8(request.sid.to_vec())
                .map_err(|e| format!("sid not utf-8: {e}"))?;
            if got_sid != sid_for_task {
                return Err(format!(
                    "sid mismatch: expected {sid_for_task}, got {got_sid}"
                ));
            }

            let hub_path_bytes = shm_path_for_task.as_os_str().as_bytes().to_vec();
            let prepared = hub_for_task
                .prepare_bootstrap_success(&hub_path_bytes)
                .map_err(|e| format!("prepare bootstrap success: {e}"))?;
            prepared
                .send_success_unix(stream.as_raw_fd(), &segment_for_task, true)
                .map_err(|e| format!("send bootstrap success: {e}"))?;
            Ok(prepared.host_peer)
        }
        .await;
        let _ = peer_tx.send(result);
    });

    let control_sock = control_sock_path
        .to_str()
        .ok_or_else(|| format!("invalid socket path: {}", control_sock_path.display()))?
        .to_string();

    let child = spawn_subject_with_env(
        "",
        &[
            ("SUBJECT_MODE", "shm-server"),
            ("SHM_CONTROL_SOCK", &control_sock),
            ("SHM_SESSION_ID", &sid),
        ],
    )
    .await?;

    let host_peer = tokio::time::timeout(Duration::from_secs(5), peer_rx)
        .await
        .map_err(|_| "timed out waiting for bootstrap request".to_string())?
        .map_err(|_| "bootstrap task dropped".to_string())??;

    let link = host_peer
        .into_link()
        .map_err(|e| format!("host peer to link: {e}"))?;
    let conduit: BareConduit<MessageFamily, roam_shm::ShmLink> = BareConduit::new(link);

    let (mut session, handle, _sh) = acceptor(conduit)
        .establish()
        .await
        .map_err(|e| format!("handshake: {e}"))?;

    let mut driver = Driver::new(handle, NoopHandler, Parity::Even);
    let caller = driver.caller();

    tokio::spawn(async move { session.run().await });
    tokio::spawn(async move { driver.run().await });

    keep_tempdir_alive(dir);
    Ok((TestbedClient::new(caller), child))
}
