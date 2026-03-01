use afl::fuzz;
use roam_shm::ShmLink;
use roam_shm::varslot::SizeClassConfig;
use roam_types::{Link, LinkRx, LinkTx, LinkTxPermit, WriteSlot};

fn main() {
    fuzz!(|data: &[u8]| {
        if data.is_empty() {
            return;
        }

        // Keep inputs bounded so each fuzz case stays fast/deterministic.
        let payload_len = usize::from(data[0]).min(192);
        let payload = &data[1..data.len().min(1 + payload_len)];

        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("create runtime");
        rt.block_on(async {
            let classes = [SizeClassConfig {
                slot_size: 512,
                slot_count: 4,
            }];
            let Ok((a, b)) = ShmLink::heap_pair(4096, 4096, 64, &classes) else {
                return;
            };
            let (a_tx, _a_rx) = a.split();
            let (_b_tx, mut b_rx) = b.split();

            let permit = match a_tx.reserve().await {
                Ok(p) => p,
                Err(_) => return,
            };
            let mut slot = match permit.alloc(payload.len()) {
                Ok(s) => s,
                Err(_) => return,
            };
            slot.as_mut_slice().copy_from_slice(payload);
            slot.commit();

            if let Ok(Some(backing)) = b_rx.recv().await {
                assert_eq!(backing.as_bytes(), payload);
            }
        });
    });
}
