#[path = "cases/binary_payload_transport_matrix.rs"]
mod binary_payload_transport_matrix;
#[path = "cases/binary_payloads.rs"]
mod binary_payloads;
#[path = "cases/channeling.rs"]
mod channeling;
#[path = "cases/testbed.rs"]
mod testbed;

#[cfg(all(unix, target_os = "macos"))]
#[path = "cases/cross_language_shm_guest_matrix.rs"]
mod cross_language_shm_guest_matrix;

use spec_tests::harness::{SubjectLanguage, SubjectSpec};

const RUST_TCP: SubjectSpec = SubjectSpec::tcp(SubjectLanguage::Rust);
const RUST_SHM_GUEST: SubjectSpec = SubjectSpec::shm_guest(SubjectLanguage::Rust);
const TYPESCRIPT_TCP: SubjectSpec = SubjectSpec::tcp(SubjectLanguage::TypeScript);
const SWIFT_TCP: SubjectSpec = SubjectSpec::tcp(SubjectLanguage::Swift);
const SWIFT_SHM_GUEST: SubjectSpec = SubjectSpec::shm_guest(SubjectLanguage::Swift);
const SWIFT_SHM_HOST: SubjectSpec = SubjectSpec::shm_host(SubjectLanguage::Swift);

macro_rules! spec_matrix_tests {
    ($($(#[$meta:meta])* $test_name:ident => $call:expr;)+) => {
        $(
            $(#[$meta])*
            #[test]
            fn $test_name() {
                $call;
            }
        )+
    };
}

spec_matrix_tests! {
    // r[verify call.initiate]
    // r[verify call.complete]
    // r[verify call.lifecycle.single-response]
    // r[verify call.lifecycle.ordering]
    lang_rust_transport_tcp_direction_harness_to_subject_rpc_echo_roundtrip => testbed::run_rpc_echo_roundtrip(RUST_TCP);

    // r[verify call.error.user]
    lang_rust_transport_tcp_direction_harness_to_subject_rpc_user_error_roundtrip => testbed::run_rpc_user_error_roundtrip(RUST_TCP);

    // r[verify call.pipelining.allowed]
    // r[verify call.pipelining.independence]
    // r[verify core.call]
    // r[verify core.call.request-id]
    lang_rust_transport_tcp_direction_harness_to_subject_rpc_pipelining_multiple_requests => testbed::run_rpc_pipelining_multiple_requests(RUST_TCP);

    // r[verify channeling.type]
    // r[verify channeling.data]
    // r[verify channeling.close]
    // r[verify channeling.caller-pov]
    // r[verify channeling.allocation.caller]
    lang_rust_transport_tcp_direction_subject_to_harness_channeling_sum_client_to_server => channeling::run_channeling_sum_client_to_server(RUST_TCP);

    // r[verify channeling.type]
    // r[verify channeling.data]
    // r[verify channeling.close]
    lang_rust_transport_tcp_direction_harness_to_subject_channeling_generate_server_to_client => channeling::run_channeling_generate_server_to_client(RUST_TCP);

    // r[verify channeling.type]
    // r[verify channeling.lifecycle.immediate-data]
    lang_rust_transport_tcp_direction_bidirectional_channeling_transform => channeling::run_channeling_transform_bidirectional(RUST_TCP);

    // r[verify transport.message.binary]
    lang_rust_transport_tcp_direction_harness_to_subject_binary_payload_sizes => binary_payloads::run_subject_process_message_binary_payload_sizes(RUST_TCP);

    // r[verify transport.message.binary]
    lang_typescript_transport_tcp_direction_harness_to_subject_rpc_echo_roundtrip => testbed::run_rpc_echo_roundtrip(TYPESCRIPT_TCP);

    // r[verify call.error.user]
    lang_typescript_transport_tcp_direction_harness_to_subject_rpc_user_error_roundtrip => testbed::run_rpc_user_error_roundtrip(TYPESCRIPT_TCP);

    // r[verify call.pipelining.allowed]
    // r[verify call.pipelining.independence]
    // r[verify core.call]
    // r[verify core.call.request-id]
    lang_typescript_transport_tcp_direction_harness_to_subject_rpc_pipelining_multiple_requests => testbed::run_rpc_pipelining_multiple_requests(TYPESCRIPT_TCP);

    // r[verify channeling.type]
    // r[verify channeling.data]
    // r[verify channeling.close]
    // r[verify channeling.caller-pov]
    // r[verify channeling.allocation.caller]
    lang_typescript_transport_tcp_direction_subject_to_harness_channeling_sum_client_to_server => channeling::run_channeling_sum_client_to_server(TYPESCRIPT_TCP);

    // r[verify channeling.type]
    // r[verify channeling.data]
    // r[verify channeling.close]
    lang_typescript_transport_tcp_direction_harness_to_subject_channeling_generate_server_to_client => channeling::run_channeling_generate_server_to_client(TYPESCRIPT_TCP);

    // r[verify channeling.type]
    // r[verify channeling.lifecycle.immediate-data]
    lang_typescript_transport_tcp_direction_bidirectional_channeling_transform => channeling::run_channeling_transform_bidirectional(TYPESCRIPT_TCP);

    // r[verify transport.message.binary]
    lang_typescript_transport_tcp_direction_harness_to_subject_binary_payload_sizes => binary_payloads::run_subject_process_message_binary_payload_sizes(TYPESCRIPT_TCP);

    // r[verify call.initiate]
    // r[verify call.complete]
    // r[verify call.lifecycle.single-response]
    // r[verify call.lifecycle.ordering]
    lang_swift_transport_tcp_direction_harness_to_subject_rpc_echo_roundtrip => testbed::run_rpc_echo_roundtrip(SWIFT_TCP);

    // r[verify call.error.user]
    lang_swift_transport_tcp_direction_harness_to_subject_rpc_user_error_roundtrip => testbed::run_rpc_user_error_roundtrip(SWIFT_TCP);

    // r[verify call.pipelining.allowed]
    // r[verify call.pipelining.independence]
    // r[verify core.call]
    // r[verify core.call.request-id]
    lang_swift_transport_tcp_direction_harness_to_subject_rpc_pipelining_multiple_requests => testbed::run_rpc_pipelining_multiple_requests(SWIFT_TCP);

    // r[verify channeling.type]
    // r[verify channeling.data]
    // r[verify channeling.close]
    // r[verify channeling.caller-pov]
    // r[verify channeling.allocation.caller]
    lang_swift_transport_tcp_direction_subject_to_harness_channeling_sum_client_to_server => channeling::run_channeling_sum_client_to_server(SWIFT_TCP);

    // r[verify channeling.type]
    // r[verify channeling.data]
    // r[verify channeling.close]
    lang_swift_transport_tcp_direction_harness_to_subject_channeling_generate_server_to_client => channeling::run_channeling_generate_server_to_client(SWIFT_TCP);

    // r[verify channeling.type]
    // r[verify channeling.lifecycle.immediate-data]
    lang_swift_transport_tcp_direction_bidirectional_channeling_transform => channeling::run_channeling_transform_bidirectional(SWIFT_TCP);

    // r[verify transport.message.binary]
    lang_swift_transport_tcp_direction_harness_to_subject_binary_payload_sizes => binary_payloads::run_subject_process_message_binary_payload_sizes(SWIFT_TCP);

    // r[verify transport.message.binary]
    lang_swift_transport_shm_direction_harness_to_subject_binary_payload_sizes_guest_mode => binary_payloads::run_subject_process_message_binary_payload_sizes(SWIFT_SHM_GUEST);

    // r[verify transport.message.binary]
    // r[verify shm.framing.threshold]
    lang_swift_transport_shm_direction_harness_to_subject_binary_payload_cutover_boundaries_guest_mode => binary_payloads::run_subject_process_message_binary_payload_shm_cutover_boundaries(SWIFT_SHM_GUEST);

    // r[verify transport.message.binary]
    lang_swift_transport_shm_direction_harness_to_subject_binary_payload_sizes_host_mode => binary_payloads::run_subject_process_message_binary_payload_sizes(SWIFT_SHM_HOST);

    // r[verify transport.message.binary]
    // r[verify shm.framing.threshold]
    lang_swift_transport_shm_direction_harness_to_subject_binary_payload_cutover_boundaries_host_mode => binary_payloads::run_subject_process_message_binary_payload_shm_cutover_boundaries(SWIFT_SHM_HOST);

    // r[verify transport.message.binary]
    lang_rust_to_rust_transport_mem_direction_bidirectional_binary_payload_transport_matrix => binary_payload_transport_matrix::run_rust_binary_payload_transport_matrix_mem();

    // r[verify transport.message.binary]
    lang_rust_to_rust_transport_tcp_direction_bidirectional_binary_payload_transport_matrix => binary_payload_transport_matrix::run_rust_binary_payload_transport_matrix_subject_tcp(RUST_TCP);

    // r[verify transport.message.binary]
    lang_rust_to_rust_transport_shm_direction_bidirectional_binary_payload_transport_matrix => binary_payload_transport_matrix::run_rust_binary_payload_transport_matrix_subject_shm(RUST_SHM_GUEST);

    // r[verify transport.shm]
    // r[verify transport.interop]
    #[cfg(all(unix, target_os = "macos"))]
    lang_swift_to_rust_transport_shm_direction_guest_to_host_cross_language_data_path => cross_language_shm_guest_matrix::run_data_path_case();

    // r[verify transport.shm]
    // r[verify transport.interop]
    #[cfg(all(unix, target_os = "macos"))]
    lang_swift_to_rust_transport_shm_direction_guest_to_host_cross_language_message_v7 => cross_language_shm_guest_matrix::run_message_v7_case();

    // r[verify transport.shm]
    // r[verify transport.interop]
    #[cfg(all(unix, target_os = "macos"))]
    lang_rust_to_swift_transport_shm_direction_host_to_guest_cross_language_mmap_ref_receive => cross_language_shm_guest_matrix::run_mmap_ref_receive_case();
}
