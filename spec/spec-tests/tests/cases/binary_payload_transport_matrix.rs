use spec_proto::Message;
use spec_tests::harness::{
    RustTransport, SubjectTestTransport, accept_rust_inproc, accept_subject_with_transport,
    run_async,
};

fn payload_sizes() -> &'static [usize] {
    &[
        0,
        1,
        247,
        248,
        249,
        31,
        32,
        63,
        64,
        127,
        128,
        255,
        256,
        257,
        511,
        512,
        1024,
        4091,
        4092,
        4093,
        4095,
        4096,
        4097,
        16 * 1024,
        64 * 1024,
        256 * 1024,
        900_000,
        1_000_000,
    ]
}

fn make_payload(size: usize) -> Vec<u8> {
    (0..size).map(|i| (i as u8).wrapping_mul(17)).collect()
}

async fn run_for_transport(transport: RustTransport) -> Result<(), String> {
    let client = accept_rust_inproc(transport).await?;
    for &size in payload_sizes() {
        let payload = make_payload(size);
        let resp = client
            .process_message(Message::Data(payload.clone()))
            .await
            .map_err(|e| format!("transport={transport:?} size={size}: {e:?}"))?;
        let actual = match &resp.ret {
            Message::Data(actual) => actual,
            _ => {
                return Err(format!(
                    "transport={transport:?} size={size}: expected Data response"
                ));
            }
        };
        let mut expected = payload;
        expected.reverse();
        if actual != &expected {
            return Err(format!(
                "transport={transport:?} size={size}: payload mismatch (len={})",
                actual.len()
            ));
        }
    }
    Ok(())
}

async fn run_for_subject_transport(transport: SubjectTestTransport) -> Result<(), String> {
    let (client, mut child) = accept_subject_with_transport(transport).await?;
    for &size in payload_sizes() {
        let payload = make_payload(size);
        let resp = client
            .process_message(Message::Data(payload.clone()))
            .await
            .map_err(|e| format!("subject transport={transport:?} size={size}: {e:?}"))?;
        let actual = match &resp.ret {
            Message::Data(actual) => actual,
            _ => {
                child.kill().await.ok();
                return Err(format!(
                    "subject transport={transport:?} size={size}: expected Data response"
                ));
            }
        };
        let mut expected = payload;
        expected.reverse();
        if actual != &expected {
            child.kill().await.ok();
            return Err(format!(
                "subject transport={transport:?} size={size}: payload mismatch (len={})",
                actual.len()
            ));
        }
    }
    child.kill().await.ok();
    Ok(())
}

fn transport_matrix_enabled() -> bool {
    if std::env::var("RUN_RUST_TRANSPORT_MATRIX").as_deref() != Ok("1") {
        eprintln!("skipping rust transport matrix (set RUN_RUST_TRANSPORT_MATRIX=1 to enable)");
        return false;
    }
    true
}

// r[verify transport.message.binary]
pub fn run_rust_binary_payload_transport_matrix_mem() {
    if !transport_matrix_enabled() {
        return;
    }
    run_async(async {
        run_for_transport(RustTransport::Mem).await?;
        Ok::<_, String>(())
    })
    .unwrap();
}

// r[verify transport.message.binary]
pub fn run_rust_binary_payload_transport_matrix_subject_tcp() {
    if !transport_matrix_enabled() {
        return;
    }
    run_async(async {
        run_for_subject_transport(SubjectTestTransport::Tcp).await?;
        Ok::<_, String>(())
    })
    .unwrap();
}

// r[verify transport.message.binary]
pub fn run_rust_binary_payload_transport_matrix_subject_shm() {
    if !transport_matrix_enabled() {
        return;
    }
    run_async(async {
        run_for_subject_transport(SubjectTestTransport::Shm).await?;
        Ok::<_, String>(())
    })
    .unwrap();
}
