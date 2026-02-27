use roam_shm::ShmLink;
use roam_shm::varslot::SizeClassConfig;

use crate::BareConduit;

type MessageConduit = BareConduit<roam_types::MessageFamily, ShmLink>;

fn message_conduit_pair() -> (MessageConduit, MessageConduit) {
    let classes = [SizeClassConfig {
        slot_size: 4096,
        slot_count: 16,
    }];
    let (a, b) = ShmLink::heap_pair(1 << 16, 1 << 20, 256, &classes);
    (BareConduit::new(a), BareConduit::new(b))
}

#[tokio::test]
async fn adder_service_macro_end_to_end_over_shm() {
    super::service_macro_shared::run_adder_end_to_end(message_conduit_pair).await;
}
