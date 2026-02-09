#![cfg(all(unix, target_os = "macos"))]

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use roam_shm::bootstrap::{SessionId, SessionPaths, unix};
use roam_shm::layout::SegmentConfig;
use roam_shm::msg::ShmMsg;
use roam_shm::peer::PeerId;
use roam_shm::{AddPeerOptions, ShmHost, msg_type};
use shm_primitives::Doorbell;

fn swift_package_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../swift/roam-runtime")
        .canonicalize()
        .expect("swift package path")
}

fn swift_bootstrap_client_path() -> PathBuf {
    let pkg = swift_package_path();
    let candidates = [
        pkg.join(".build/debug/shm-bootstrap-client"),
        pkg.join(".build/arm64-apple-macosx/debug/shm-bootstrap-client"),
        pkg.join(".build/x86_64-apple-macosx/debug/shm-bootstrap-client"),
    ];

    for candidate in candidates {
        if candidate.exists() {
            return candidate;
        }
    }

    panic!("shm-bootstrap-client binary not found; ensure nextest setup built swift target");
}

fn swift_shm_guest_client_path() -> PathBuf {
    let pkg = swift_package_path();
    let candidates = [
        pkg.join(".build/debug/shm-guest-client"),
        pkg.join(".build/arm64-apple-macosx/debug/shm-guest-client"),
        pkg.join(".build/x86_64-apple-macosx/debug/shm-guest-client"),
    ];

    for candidate in candidates {
        if candidate.exists() {
            return candidate;
        }
    }

    panic!("shm-guest-client binary not found; ensure nextest setup built swift target");
}

#[tokio::test]
async fn rust_host_bootstrap_to_swift_client() {
    let tmp = tempfile::Builder::new()
        .prefix("rshm-xlang-")
        .tempdir_in("/tmp")
        .unwrap();
    let container_root = tmp.path().join("app-group");
    std::fs::create_dir_all(&container_root).unwrap();

    let sid = SessionId::parse("123e4567-e89b-12d3-a456-426614174000").unwrap();
    let paths = SessionPaths::new(&container_root, sid.clone()).unwrap();
    let listener = unix::bind_control_socket(&paths).unwrap();

    let (_host_doorbell, guest_handle) = Doorbell::create_pair().unwrap();
    let peer_id = PeerId::new(1).unwrap();
    let hub_path = paths.shm_path();
    let hub_path_for_host = hub_path.clone();

    let host_task = tokio::spawn(async move {
        unix::accept_and_send_ticket(
            &listener,
            &sid,
            peer_id,
            &hub_path_for_host,
            guest_handle.as_raw_fd(),
        )
        .await
    });

    let client_bin = swift_bootstrap_client_path();
    let control_sock = paths.control_sock_path();
    let sid_arg = "123e4567-e89b-12d3-a456-426614174000";

    let output = tokio::task::spawn_blocking(move || {
        Command::new(client_bin)
            .args([control_sock.to_str().unwrap(), sid_arg])
            .output()
            .expect("run swift bootstrap client")
    })
    .await
    .unwrap();

    if !output.status.success() {
        panic!(
            "swift client failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("peer_id=1"), "missing peer_id in: {stdout}");
    assert!(
        stdout.contains(&format!("hub_path={}", hub_path.display())),
        "missing hub_path in: {stdout}"
    );

    host_task.await.unwrap().unwrap();
}

#[tokio::test]
async fn rust_host_shm_to_swift_guest_data_path() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("xlang-shm-data.shm");

    let mut host = ShmHost::create(&path, SegmentConfig::default()).unwrap();
    let ticket = host.add_peer(AddPeerOptions::default()).unwrap();
    let peer_id = ticket.peer_id;
    let args = ticket.to_args();

    let client_bin = swift_shm_guest_client_path();
    let child = Command::new(client_bin)
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn swift shm guest client");

    // Parent closes its copy of guest doorbell fd; child keeps inherited one.
    drop(ticket);

    let mut received = Vec::new();
    for _ in 0..100 {
        let result = host.poll();
        received.extend(result.messages);
        if received.len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert_eq!(received.len(), 2, "expected 2 messages from Swift guest");
    assert!(received.iter().all(|(pid, _)| *pid == peer_id));

    let inline = received
        .iter()
        .find(|(_, msg)| msg.id == 1)
        .expect("missing inline message");
    assert_eq!(inline.1.payload_bytes(), b"swift-inline");

    let slot_ref = received
        .iter()
        .find(|(_, msg)| msg.id == 2)
        .expect("missing slot-ref message");
    assert_eq!(slot_ref.1.payload_bytes().len(), 2048);
    for (i, b) in slot_ref.1.payload_bytes().iter().enumerate() {
        assert_eq!(*b, i as u8, "slot payload mismatch at byte {i}");
    }

    host.send(
        peer_id,
        &ShmMsg::new(msg_type::DATA, 101, 0, b"ack-inline".to_vec()),
    )
    .unwrap();
    host.send(
        peer_id,
        &ShmMsg::new(msg_type::DATA, 102, 0, b"ack-slot".to_vec()),
    )
    .unwrap();

    let output = tokio::task::spawn_blocking(move || child.wait_with_output())
        .await
        .expect("join wait_with_output task")
        .expect("wait for swift guest client");
    if !output.status.success() {
        panic!(
            "swift shm guest client failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
