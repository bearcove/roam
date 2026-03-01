#![cfg(all(unix, target_os = "macos"))]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::{Duration, Instant};

use facet_postcard::{from_slice_borrowed, to_vec};
use roam_shm::framing::{MmapRef, OwnedFrame, read_frame, write_inline, write_mmap_ref};
use roam_shm::segment::{Segment, SegmentConfig};
use roam_shm::varslot::SizeClassConfig;
use roam_types::{
    ChannelBody, ChannelClose, ChannelGrantCredit, ChannelId, ChannelMessage, ConnectionId,
    Message, MessagePayload, Metadata, MetadataEntry, MetadataFlags, MetadataValue, Payload,
    RequestBody, RequestId, RequestMessage, RequestResponse,
};
use shm_primitives::{
    FileCleanup, MmapAttachMessage, MmapRegion, clear_cloexec, create_mmap_control_pair,
};

fn swift_runtime_package_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../swift/roam-runtime")
        .canonicalize()
        .expect("swift runtime package path")
}

fn swift_shm_guest_client_path() -> PathBuf {
    let pkg = swift_runtime_package_path();
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

    panic!("shm-guest-client binary not found; build swift/roam-runtime target first")
}

fn read_guest_payloads(
    segment: &Segment,
    peer_id: shm_primitives::PeerId,
    expected_count: usize,
    deadline: Instant,
) -> Vec<Vec<u8>> {
    let g2h = segment.g2h_bipbuf(peer_id);
    let (_tx, mut rx) = g2h.split();
    let mut payloads = Vec::new();

    while Instant::now() < deadline && payloads.len() < expected_count {
        if let Some(frame) = read_frame(&mut rx) {
            match frame {
                OwnedFrame::Inline(bytes) => payloads.push(bytes),
                OwnedFrame::SlotRef(slot_ref) => {
                    let raw = unsafe { segment.var_pool().slot_data(&slot_ref) };
                    let len = u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]) as usize;
                    payloads.push(raw[4..4 + len].to_vec());
                }
                OwnedFrame::MmapRef(_) => panic!("unexpected mmap-ref frame"),
            }
        } else {
            thread::sleep(Duration::from_millis(10));
        }
    }

    payloads
}

fn send_host_ack(segment: &Segment, peer_id: shm_primitives::PeerId, payload: &[u8]) {
    let h2g = segment.h2g_bipbuf(peer_id);
    let (mut tx, _rx) = h2g.split();
    write_inline(&mut tx, payload).expect("write ack frame");
}

fn send_host_message(segment: &Segment, peer_id: shm_primitives::PeerId, message: &Message<'_>) {
    let h2g = segment.h2g_bipbuf(peer_id);
    let (mut tx, _rx) = h2g.split();
    let payload = to_vec(message).expect("encode host message");
    write_inline(&mut tx, &payload).expect("write host message");
}

fn sample_metadata<'a>() -> Metadata<'a> {
    vec![MetadataEntry {
        key: "trace-id",
        value: MetadataValue::String("rust-trace"),
        flags: MetadataFlags::NONE,
    }]
}

fn make_socketpair() -> (i32, i32) {
    let mut fds = [0i32; 2];
    let rc = unsafe { libc::socketpair(libc::AF_UNIX, libc::SOCK_STREAM, 0, fds.as_mut_ptr()) };
    assert_eq!(
        rc,
        0,
        "socketpair failed: {}",
        std::io::Error::last_os_error()
    );
    (fds[0], fds[1])
}

fn ring_doorbell(fd: i32) {
    let byte = [1u8];
    let rc = unsafe { libc::send(fd, byte.as_ptr().cast(), 1, libc::MSG_DONTWAIT) };
    assert!(
        rc >= 0,
        "doorbell send failed: {}",
        std::io::Error::last_os_error()
    );
}

#[test]
fn rust_segment_to_swift_guest_data_path() {
    let dir = tempfile::tempdir().unwrap();
    let shm_path = dir.path().join("xlang-shm-data-path.shm");
    let class = [SizeClassConfig {
        slot_size: 4096,
        slot_count: 2,
    }];
    let config = SegmentConfig {
        max_guests: 1,
        bipbuf_capacity: 64 * 1024,
        max_payload_size: 4096,
        inline_threshold: 64,
        heartbeat_interval: 0,
        size_classes: &class,
    };
    let segment = Segment::create(Path::new(&shm_path), config, FileCleanup::Manual).unwrap();

    let peer_id = segment.reserve_peer().expect("reserve peer slot");
    let (host_fd, guest_fd) = make_socketpair();
    clear_cloexec(guest_fd).expect("clear close-on-exec");

    let child = Command::new(swift_shm_guest_client_path())
        .arg(format!("--hub-path={}", shm_path.display()))
        .arg(format!("--peer-id={}", peer_id.get()))
        .arg(format!("--doorbell-fd={guest_fd}"))
        .arg("--size-class=4096:2")
        .arg("--scenario=data-path")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn swift shm guest client");

    let payloads = read_guest_payloads(
        &segment,
        peer_id,
        2,
        Instant::now() + Duration::from_secs(5),
    );
    if payloads.len() < 2 {
        let output = child
            .wait_with_output()
            .expect("wait for swift guest process");
        panic!(
            "expected two payload frames from Swift guest, got {}\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
            payloads.len(),
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert_eq!(
        payloads.len(),
        2,
        "expected two payload frames from Swift guest"
    );
    assert_eq!(payloads[0], b"swift-inline");
    assert_eq!(payloads[1].len(), 2048);
    for (idx, byte) in payloads[1].iter().enumerate() {
        assert_eq!(*byte, idx as u8, "slot payload mismatch at byte {idx}");
    }

    send_host_ack(&segment, peer_id, b"ack-inline");
    send_host_ack(&segment, peer_id, b"ack-slot");

    let output = child
        .wait_with_output()
        .expect("wait for swift guest process");
    unsafe {
        libc::close(host_fd);
        libc::close(guest_fd);
    }
    if !output.status.success() {
        panic!(
            "swift shm guest failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn rust_segment_to_swift_guest_message_v7_path() {
    let dir = tempfile::tempdir().unwrap();
    let shm_path = dir.path().join("xlang-shm-message-v7.shm");
    let class = [SizeClassConfig {
        slot_size: 4096,
        slot_count: 2,
    }];
    let config = SegmentConfig {
        max_guests: 1,
        bipbuf_capacity: 64 * 1024,
        max_payload_size: 4096,
        inline_threshold: 64,
        heartbeat_interval: 0,
        size_classes: &class,
    };
    let segment = Segment::create(Path::new(&shm_path), config, FileCleanup::Manual).unwrap();

    let peer_id = segment.reserve_peer().expect("reserve peer slot");
    let (host_fd, guest_fd) = make_socketpair();
    clear_cloexec(guest_fd).expect("clear close-on-exec");

    let child = Command::new(swift_shm_guest_client_path())
        .arg(format!("--hub-path={}", shm_path.display()))
        .arg(format!("--peer-id={}", peer_id.get()))
        .arg(format!("--doorbell-fd={guest_fd}"))
        .arg("--size-class=4096:2")
        .arg("--scenario=message-v7")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn swift shm guest client");

    let payloads = read_guest_payloads(
        &segment,
        peer_id,
        3,
        Instant::now() + Duration::from_secs(5),
    );
    if payloads.len() < 3 {
        let mut child = child;
        let _ = child.kill();
        let output = child
            .wait_with_output()
            .expect("wait for swift guest process");
        panic!(
            "expected three MessageV7 frames from Swift guest, got {}\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
            payloads.len(),
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }

    let req = from_slice_borrowed::<Message<'_>>(&payloads[0]).expect("decode request message");
    assert_eq!(req.connection_id, ConnectionId(2));
    match req.payload {
        MessagePayload::RequestMessage(RequestMessage {
            id: RequestId(11),
            body: RequestBody::Call(call),
        }) => {
            assert_eq!(call.channels, vec![ChannelId(3), ChannelId(5)]);
            let payload = match call.args {
                Payload::Incoming(bytes) => bytes,
                _ => panic!("expected incoming bytes payload"),
            };
            assert_eq!(payload, b"swift-request");
        }
        _ => panic!("unexpected first message payload"),
    }

    let close = from_slice_borrowed::<Message<'_>>(&payloads[1]).expect("decode channel close");
    assert_eq!(close.connection_id, ConnectionId(2));
    match close.payload {
        MessagePayload::ChannelMessage(ChannelMessage {
            id: ChannelId(3),
            body: ChannelBody::Close(ChannelClose { metadata }),
        }) => {
            assert_eq!(metadata.len(), 1);
            assert_eq!(metadata[0].key, "reason");
        }
        _ => panic!("unexpected second message payload"),
    }

    let proto = from_slice_borrowed::<Message<'_>>(&payloads[2]).expect("decode protocol error");
    match proto.payload {
        MessagePayload::ProtocolError(err) => {
            assert_eq!(err.description, "swift protocol violation")
        }
        _ => panic!("unexpected third message payload"),
    }

    let ret: u32 = 42;
    let response = Message {
        connection_id: ConnectionId(2),
        payload: MessagePayload::RequestMessage(RequestMessage {
            id: RequestId(11),
            body: RequestBody::Response(RequestResponse {
                ret: Payload::outgoing(&ret),
                channels: vec![ChannelId(7)],
                metadata: sample_metadata(),
            }),
        }),
    };
    send_host_message(&segment, peer_id, &response);

    let credit = Message {
        connection_id: ConnectionId(2),
        payload: MessagePayload::ChannelMessage(ChannelMessage {
            id: ChannelId(3),
            body: ChannelBody::GrantCredit(ChannelGrantCredit { additional: 4096 }),
        }),
    };
    send_host_message(&segment, peer_id, &credit);

    let output = child
        .wait_with_output()
        .expect("wait for swift guest process");
    unsafe {
        libc::close(host_fd);
        libc::close(guest_fd);
    }
    if !output.status.success() {
        panic!(
            "swift shm guest failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[test]
fn rust_segment_to_swift_guest_mmap_ref_receive_path() {
    let dir = tempfile::tempdir().unwrap();
    let shm_path = dir.path().join("xlang-shm-mmap-recv.shm");
    let class = [SizeClassConfig {
        slot_size: 64,
        slot_count: 2,
    }];
    let config = SegmentConfig {
        max_guests: 1,
        bipbuf_capacity: 64 * 1024,
        max_payload_size: 4096,
        inline_threshold: 64,
        heartbeat_interval: 0,
        size_classes: &class,
    };
    let segment = Segment::create(Path::new(&shm_path), config, FileCleanup::Manual).unwrap();

    let peer_id = segment.reserve_peer().expect("reserve peer slot");
    let (host_fd, guest_fd) = make_socketpair();
    clear_cloexec(guest_fd).expect("clear close-on-exec");

    let (mmap_tx, mmap_handle) = create_mmap_control_pair().expect("create mmap control pair");
    clear_cloexec(mmap_handle.as_raw_fd()).expect("clear close-on-exec for mmap control fd");

    let child = Command::new(swift_shm_guest_client_path())
        .arg(format!("--hub-path={}", shm_path.display()))
        .arg(format!("--peer-id={}", peer_id.get()))
        .arg(format!("--doorbell-fd={guest_fd}"))
        .arg(format!("--mmap-control-fd={}", mmap_handle.as_raw_fd()))
        .arg("--size-class=64:2")
        .arg("--scenario=mmap-recv")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn swift shm guest client");

    let mmap_path = dir.path().join("xlang-mmap-payload.shm");
    let mapping =
        MmapRegion::create(&mmap_path, 4096, FileCleanup::Manual).expect("create mmap payload");
    let payload: Vec<u8> = (0..512).map(|i| i as u8).collect();
    unsafe {
        let data = mapping.region().as_ptr().add(128);
        std::ptr::copy_nonoverlapping(payload.as_ptr(), data, payload.len());
    }
    let attach_msg = MmapAttachMessage {
        map_id: 7,
        map_generation: 1,
        mapping_length: mapping.len() as u64,
    };
    mmap_tx
        .send(mapping.as_raw_fd(), &attach_msg)
        .expect("send mmap attach message");

    {
        let h2g = segment.h2g_bipbuf(peer_id);
        let (mut tx, _rx) = h2g.split();
        let mmap_ref = MmapRef {
            map_id: 7,
            map_generation: 1,
            map_offset: 128,
            payload_len: payload.len() as u32,
        };
        write_mmap_ref(&mut tx, &mmap_ref).expect("write mmap-ref frame");
    }
    ring_doorbell(host_fd);

    let payloads = read_guest_payloads(
        &segment,
        peer_id,
        1,
        Instant::now() + Duration::from_secs(5),
    );
    if payloads.len() < 1 {
        let mut child = child;
        let _ = child.kill();
        let output = child
            .wait_with_output()
            .expect("wait for swift guest process");
        panic!(
            "expected mmap ack from Swift guest, got {}\nstatus: {}\nstdout:\n{}\nstderr:\n{}",
            payloads.len(),
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert_eq!(payloads[0], b"mmap-recv-ok");

    let output = child
        .wait_with_output()
        .expect("wait for swift guest process");
    unsafe {
        libc::close(host_fd);
        libc::close(guest_fd);
    }
    if !output.status.success() {
        panic!(
            "swift shm guest failed\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}
