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

macro_rules! spec_matrix_tests {
    ($($(#[$meta:meta])* $test_name:ident => $runner:path;)+) => {
        $(
            $(#[$meta])*
            #[test]
            fn $test_name() {
                $runner();
            }
        )+
    };
}

spec_matrix_tests! {
    // r[verify call.initiate]
    // r[verify call.complete]
    // r[verify call.lifecycle.single-response]
    // r[verify call.lifecycle.ordering]
    lang_subject_to_harness_transport_env_selected_direction_harness_to_subject_rpc_echo_roundtrip => testbed::run_rpc_echo_roundtrip;

    // r[verify call.error.user]
    lang_subject_to_harness_transport_env_selected_direction_harness_to_subject_rpc_user_error_roundtrip => testbed::run_rpc_user_error_roundtrip;

    // r[verify call.pipelining.allowed]
    // r[verify call.pipelining.independence]
    // r[verify core.call]
    // r[verify core.call.request-id]
    lang_subject_to_harness_transport_env_selected_direction_harness_to_subject_rpc_pipelining_multiple_requests => testbed::run_rpc_pipelining_multiple_requests;

    // r[verify channeling.type]
    // r[verify channeling.data]
    // r[verify channeling.close]
    // r[verify channeling.caller-pov]
    // r[verify channeling.allocation.caller]
    lang_subject_to_harness_transport_env_selected_direction_subject_to_harness_channeling_sum_client_to_server => channeling::run_channeling_sum_client_to_server;

    // r[verify channeling.type]
    // r[verify channeling.data]
    // r[verify channeling.close]
    lang_subject_to_harness_transport_env_selected_direction_harness_to_subject_channeling_generate_server_to_client => channeling::run_channeling_generate_server_to_client;

    // r[verify channeling.type]
    // r[verify channeling.lifecycle.immediate-data]
    lang_subject_to_harness_transport_env_selected_direction_bidirectional_channeling_transform => channeling::run_channeling_transform_bidirectional;

    // r[verify transport.message.binary]
    lang_subject_to_harness_transport_env_selected_direction_harness_to_subject_binary_payload_sizes => binary_payloads::run_subject_process_message_binary_payload_sizes;

    // r[verify transport.message.binary]
    // r[verify shm.framing.threshold]
    lang_subject_to_harness_transport_shm_direction_harness_to_subject_binary_payload_cutover_boundaries => binary_payloads::run_subject_process_message_binary_payload_shm_cutover_boundaries;

    // r[verify transport.message.binary]
    lang_rust_to_rust_transport_mem_direction_bidirectional_binary_payload_transport_matrix => binary_payload_transport_matrix::run_rust_binary_payload_transport_matrix_mem;

    // r[verify transport.message.binary]
    lang_subject_to_rust_transport_tcp_direction_bidirectional_binary_payload_transport_matrix => binary_payload_transport_matrix::run_rust_binary_payload_transport_matrix_subject_tcp;

    // r[verify transport.message.binary]
    lang_subject_to_rust_transport_shm_direction_bidirectional_binary_payload_transport_matrix => binary_payload_transport_matrix::run_rust_binary_payload_transport_matrix_subject_shm;

    // r[verify transport.shm]
    // r[verify transport.interop]
    #[cfg(all(unix, target_os = "macos"))]
    lang_swift_to_rust_transport_shm_direction_guest_to_host_cross_language_data_path => cross_language_shm_guest_matrix::run_data_path_case;

    // r[verify transport.shm]
    // r[verify transport.interop]
    #[cfg(all(unix, target_os = "macos"))]
    lang_swift_to_rust_transport_shm_direction_guest_to_host_cross_language_message_v7 => cross_language_shm_guest_matrix::run_message_v7_case;

    // r[verify transport.shm]
    // r[verify transport.interop]
    #[cfg(all(unix, target_os = "macos"))]
    lang_rust_to_swift_transport_shm_direction_host_to_guest_cross_language_mmap_ref_receive => cross_language_shm_guest_matrix::run_mmap_ref_receive_case;
}
