use std::future::Future;
use std::io::Write as _;
use std::os::fd::{AsRawFd, IntoRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::UnixStream as StdUnixStream;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use roam::{Call, Rx, Tx};
use roam_core::{
    BareConduit, Driver, DriverCaller, DriverReplySink, acceptor, initiator, memory_link_pair,
};
use roam_shm::HostHub;
use roam_shm::ShmLink;
use roam_shm::bootstrap::{BootstrapStatus, decode_request, encode_request};
use roam_shm::guest_link_from_raw;
use roam_shm::segment::{Segment, SegmentConfig};
use roam_shm::varslot::SizeClassConfig as RoamShmSizeClassConfig;
use roam_stream::StreamLink;
use roam_types::{MessageFamily, Parity, RequestCall, SelfRef};
use shm_primitives::FileCleanup;
use shm_primitives::SizeClassConfig;
use spec_proto::{
    Canvas, Color, LookupError, MathError, Message, Person, Point, Rectangle, Shape, TestbedClient,
    TestbedDispatcher, TestbedServer,
};
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

fn shm_subject_mode() -> String {
    std::env::var("SPEC_SHM_SUBJECT_MODE")
        .ok()
        .unwrap_or_else(|| "shm-server".to_string())
        .to_ascii_lowercase()
}

pub fn run_async<T>(f: impl Future<Output = T>) -> T {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime");
    rt.block_on(f)
}

#[derive(Clone)]
struct TestbedService;

impl TestbedServer for TestbedService {
    async fn echo(&self, call: impl Call<String, std::convert::Infallible>, message: String) {
        call.ok(message).await;
    }

    async fn reverse(&self, call: impl Call<String, std::convert::Infallible>, message: String) {
        call.ok(message.chars().rev().collect()).await;
    }

    async fn divide(&self, call: impl Call<i64, MathError>, dividend: i64, divisor: i64) {
        if divisor == 0 {
            call.err(MathError::DivisionByZero).await;
        } else {
            call.ok(dividend / divisor).await;
        }
    }

    async fn lookup(&self, call: impl Call<Person, LookupError>, id: u32) {
        match id {
            1 => {
                call.ok(Person {
                    name: "Alice".to_string(),
                    age: 30,
                    email: Some("alice@example.com".to_string()),
                })
                .await
            }
            2 => {
                call.ok(Person {
                    name: "Bob".to_string(),
                    age: 25,
                    email: None,
                })
                .await
            }
            3 => {
                call.ok(Person {
                    name: "Charlie".to_string(),
                    age: 35,
                    email: Some("charlie@example.com".to_string()),
                })
                .await
            }
            _ => call.err(LookupError::NotFound).await,
        }
    }

    async fn sum(&self, call: impl Call<i64, std::convert::Infallible>, mut numbers: Rx<i32>) {
        let mut total: i64 = 0;
        while let Ok(Some(n)) = numbers.recv().await {
            total += *n as i64;
        }
        call.ok(total).await;
    }

    async fn generate(
        &self,
        call: impl Call<(), std::convert::Infallible>,
        count: u32,
        output: Tx<i32>,
    ) {
        for i in 0..count as i32 {
            if output.send(i).await.is_err() {
                break;
            }
        }
        output.close(Default::default()).await.ok();
        call.ok(()).await;
    }

    async fn transform(
        &self,
        call: impl Call<(), std::convert::Infallible>,
        mut input: Rx<String>,
        output: Tx<String>,
    ) {
        while let Ok(Some(s)) = input.recv().await {
            let _ = output.send(s.clone()).await;
        }
        output.close(Default::default()).await.ok();
        call.ok(()).await;
    }

    async fn echo_point(&self, call: impl Call<Point, std::convert::Infallible>, point: Point) {
        call.ok(point).await;
    }

    async fn create_person(
        &self,
        call: impl Call<Person, std::convert::Infallible>,
        name: String,
        age: u8,
        email: Option<String>,
    ) {
        call.ok(Person { name, age, email }).await;
    }

    async fn rectangle_area(
        &self,
        call: impl Call<f64, std::convert::Infallible>,
        rect: Rectangle,
    ) {
        let width = (rect.bottom_right.x - rect.top_left.x).abs() as f64;
        let height = (rect.bottom_right.y - rect.top_left.y).abs() as f64;
        call.ok(width * height).await;
    }

    async fn parse_color(
        &self,
        call: impl Call<Option<Color>, std::convert::Infallible>,
        name: String,
    ) {
        let color = match name.to_lowercase().as_str() {
            "red" => Some(Color::Red),
            "green" => Some(Color::Green),
            "blue" => Some(Color::Blue),
            _ => None,
        };
        call.ok(color).await;
    }

    async fn shape_area(&self, call: impl Call<f64, std::convert::Infallible>, shape: Shape) {
        let area = match shape {
            Shape::Circle { radius } => std::f64::consts::PI * radius * radius,
            Shape::Rectangle { width, height } => width * height,
            Shape::Point => 0.0,
        };
        call.ok(area).await;
    }

    async fn create_canvas(
        &self,
        call: impl Call<Canvas, std::convert::Infallible>,
        name: String,
        shapes: Vec<Shape>,
        background: Color,
    ) {
        call.ok(Canvas {
            name,
            shapes,
            background,
        })
        .await;
    }

    async fn process_message(
        &self,
        call: impl Call<Message, std::convert::Infallible>,
        msg: Message,
    ) {
        let response = match msg {
            Message::Text(s) => Message::Text(format!("processed: {s}")),
            Message::Number(n) => Message::Number(n * 2),
            Message::Data(d) => Message::Data(d.into_iter().rev().collect()),
        };
        call.ok(response).await;
    }

    async fn get_points(&self, call: impl Call<Vec<Point>, std::convert::Infallible>, count: u32) {
        let points = (0..count as i32)
            .map(|i| Point { x: i, y: i * 2 })
            .collect();
        call.ok(points).await;
    }

    async fn swap_pair(
        &self,
        call: impl Call<(String, i32), std::convert::Infallible>,
        pair: (i32, String),
    ) {
        call.ok((pair.1, pair.0)).await;
    }
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubjectTestTransport {
    Tcp,
    Shm,
}

pub async fn accept_subject_with_transport(
    transport: SubjectTestTransport,
) -> Result<(TestbedClient<DriverCaller>, Child), String> {
    match transport {
        SubjectTestTransport::Tcp => accept_subject_tcp().await,
        SubjectTestTransport::Shm => accept_subject_shm().await,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RustTransport {
    Mem,
    Tcp,
    Shm,
}

pub async fn accept_rust_inproc(
    transport: RustTransport,
) -> Result<TestbedClient<DriverCaller>, String> {
    match transport {
        RustTransport::Mem => {
            let (a, b) = memory_link_pair(64 * 1024);
            accept_rust_inproc_with_conduits(BareConduit::new(a), BareConduit::new(b)).await
        }
        RustTransport::Tcp => {
            let listener = TcpListener::bind("127.0.0.1:0")
                .await
                .map_err(|e| format!("bind: {e}"))?;
            let addr = listener
                .local_addr()
                .map_err(|e| format!("local_addr: {e}"))?;
            let connect_task =
                tokio::spawn(async move { tokio::net::TcpStream::connect(addr).await });
            let (server_stream, _) = listener
                .accept()
                .await
                .map_err(|e| format!("accept: {e}"))?;
            let client_stream = connect_task
                .await
                .map_err(|e| format!("connect task join: {e}"))?
                .map_err(|e| format!("connect: {e}"))?;
            server_stream.set_nodelay(true).unwrap();
            client_stream.set_nodelay(true).unwrap();
            accept_rust_inproc_with_conduits(
                BareConduit::new(StreamLink::tcp(client_stream)),
                BareConduit::new(StreamLink::tcp(server_stream)),
            )
            .await
        }
        RustTransport::Shm => {
            let classes = [RoamShmSizeClassConfig {
                slot_size: 4096,
                slot_count: 64,
            }];
            let (a, b) = ShmLink::heap_pair(1 << 16, 1 << 20, 256, &classes)
                .map_err(|e| format!("shm heap_pair: {e}"))?;
            accept_rust_inproc_with_conduits(BareConduit::new(a), BareConduit::new(b)).await
        }
    }
}

async fn accept_rust_inproc_with_conduits<L>(
    client_conduit: BareConduit<MessageFamily, L>,
    server_conduit: BareConduit<MessageFamily, L>,
) -> Result<TestbedClient<DriverCaller>, String>
where
    L: roam_types::Link + Send + 'static,
    L::Tx: Send + 'static,
    L::Rx: Send + 'static,
    <L::Rx as roam_types::LinkRx>::Error: std::error::Error + Send + Sync + 'static,
{
    let server_task = tokio::spawn(async move {
        let (mut server_session, server_handle, _sh) =
            acceptor(server_conduit)
                .establish()
                .await
                .map_err(|e| format!("server handshake: {e}"))?;
        let dispatcher = TestbedDispatcher::new(TestbedService);
        let mut server_driver = Driver::new(server_handle, dispatcher, Parity::Even);
        tokio::spawn(async move { server_session.run().await });
        tokio::spawn(async move { server_driver.run().await });
        Ok::<(), String>(())
    });

    let (mut client_session, client_handle, _sh) = initiator(client_conduit)
        .establish()
        .await
        .map_err(|e| format!("client handshake: {e}"))?;
    let mut client_driver = Driver::new(client_handle, NoopHandler, Parity::Odd);
    let caller = client_driver.caller();

    tokio::spawn(async move { client_session.run().await });
    tokio::spawn(async move { client_driver.run().await });

    server_task
        .await
        .map_err(|e| format!("server task join: {e}"))??;

    Ok(TestbedClient::new(caller))
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
    if shm_subject_mode() == "shm-host-server" {
        return accept_subject_shm_subject_is_host().await;
    }
    accept_subject_shm_subject_is_guest().await
}

async fn accept_subject_shm_subject_is_guest()
-> Result<(TestbedClient<DriverCaller>, Child), String> {
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

    let hub_path_bytes = shm_path.as_os_str().as_bytes().to_vec();
    let prepared = hub
        .prepare_bootstrap_success(&hub_path_bytes)
        .map_err(|e| format!("prepare bootstrap success: {e}"))?;
    let mmap_tx_fd_env = prepared.guest_ticket.mmap_tx_fd.to_string();

    let (peer_tx, peer_rx) = oneshot::channel();
    let sid_for_task = sid.clone();
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
            prepared
                .send_success_unix(stream.as_raw_fd(), &segment_for_task)
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
            ("SHM_MMAP_TX_FD", &mmap_tx_fd_env),
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

async fn accept_subject_shm_subject_is_host() -> Result<(TestbedClient<DriverCaller>, Child), String>
{
    let dir = tempfile::tempdir().map_err(|e| format!("tempdir: {e}"))?;
    let sid = sid_hex_32();
    let control_sock_path = dir.path().join("bootstrap.sock");
    let shm_path = dir.path().join("subject.shm");
    let control_sock = control_sock_path
        .to_str()
        .ok_or_else(|| format!("invalid socket path: {}", control_sock_path.display()))?
        .to_string();
    let shm_path_str = shm_path
        .to_str()
        .ok_or_else(|| format!("invalid shm path: {}", shm_path.display()))?
        .to_string();

    let mut child = spawn_subject_with_env(
        "",
        &[
            ("SUBJECT_MODE", "shm-host-server"),
            ("SHM_CONTROL_SOCK", &control_sock),
            ("SHM_SESSION_ID", &sid),
            ("SHM_HUB_PATH", &shm_path_str),
        ],
    )
    .await?;

    let setup_result: Result<TestbedClient<DriverCaller>, String> = async {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            match StdUnixStream::connect(&control_sock_path) {
                Ok(stream) => break stream,
                Err(e) => {
                    if tokio::time::Instant::now() >= deadline {
                        return Err(format!(
                            "connect bootstrap socket {}: {e}",
                            control_sock_path.display()
                        ));
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            }
        };

        let request = encode_request(sid.as_bytes()).map_err(|e| format!("encode request: {e}"))?;
        stream
            .write_all(&request)
            .map_err(|e| format!("send bootstrap request: {e}"))?;

        let received = shm_primitives::bootstrap::recv_response_unix(stream.as_raw_fd())
            .map_err(|e| format!("recv bootstrap response: {e}"))?;
        if received.response.status != BootstrapStatus::Success {
            return Err(format!(
                "bootstrap failed: status={:?}, payload={}",
                received.response.status,
                String::from_utf8_lossy(&received.response.payload)
            ));
        }

        let fds = received
            .fds
            .ok_or_else(|| "missing bootstrap success fds".to_string())?;
        let hub_path = std::str::from_utf8(&received.response.payload)
            .map_err(|e| format!("bootstrap payload is not utf-8 path: {e}"))?;
        let segment = Arc::new(
            Segment::attach(std::path::Path::new(hub_path))
                .map_err(|e| format!("attach segment at {hub_path}: {e}"))?,
        );
        let peer_id = shm_primitives::PeerId::new(received.response.peer_id as u8)
            .ok_or_else(|| format!("invalid peer id {}", received.response.peer_id))?;

        let doorbell_fd = fds.doorbell_fd.into_raw_fd();
        let mmap_rx_owned = fds.mmap_control_fd;
        let mmap_tx_owned = mmap_rx_owned
            .try_clone()
            .map_err(|e| format!("clone mmap control fd: {e}"))?;
        let mmap_rx_fd = mmap_rx_owned.into_raw_fd();
        let mmap_tx_fd = mmap_tx_owned.into_raw_fd();

        let link = unsafe {
            guest_link_from_raw(segment, peer_id, doorbell_fd, mmap_rx_fd, mmap_tx_fd, true)
        }
        .map_err(|e| format!("guest_link_from_raw: {e}"))?;
        let conduit: BareConduit<MessageFamily, roam_shm::ShmLink> = BareConduit::new(link);

        let (mut session, handle, _sh) = initiator(conduit)
            .establish()
            .await
            .map_err(|e| format!("handshake: {e}"))?;

        let mut driver = Driver::new(handle, NoopHandler, Parity::Odd);
        let caller = driver.caller();

        tokio::spawn(async move { session.run().await });
        tokio::spawn(async move { driver.run().await });

        Ok::<_, String>(TestbedClient::new(caller))
    }
    .await;

    match setup_result {
        Ok(client) => {
            keep_tempdir_alive(dir);
            Ok((client, child))
        }
        Err(e) => {
            child.kill().await.ok();
            Err(e)
        }
    }
}
