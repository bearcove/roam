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

const SUBJECT_RUST_TCP: SubjectSpec = SubjectSpec::tcp(SubjectLanguage::Rust);
const SUBJECT_RUST_SHM_GUEST: SubjectSpec = SubjectSpec::shm_guest(SubjectLanguage::Rust);
const SUBJECT_TYPESCRIPT_TCP: SubjectSpec = SubjectSpec::tcp(SubjectLanguage::TypeScript);
const SUBJECT_SWIFT_TCP: SubjectSpec = SubjectSpec::tcp(SubjectLanguage::Swift);
const SUBJECT_SWIFT_SHM_GUEST: SubjectSpec = SubjectSpec::shm_guest(SubjectLanguage::Swift);
const SUBJECT_SWIFT_SHM_HOST: SubjectSpec = SubjectSpec::shm_host(SubjectLanguage::Swift);

macro_rules! emit_shm_harness_to_subject_tests {
    (tcp, $spec:expr, { $($(#[$meta:meta])* $name:ident => $call:path;)+ }) => {};
    (shm, $spec:expr, { $($(#[$meta:meta])* $name:ident => $call:path;)+ }) => {
        mod direction_harness_to_subject_shm_only {
            use super::*;
            $(
                $(#[$meta])*
                #[test]
                fn $name() {
                    $call($spec);
                }
            )+
        }
    };
}

macro_rules! subject_combo_matrix_one {
    (
        $combo_mod:ident,
        $spec:expr,
        $transport:ident,
        { $($(#[$hmeta:meta])* $hname:ident => $hcall:path;)+ },
        { $($(#[$smeta:meta])* $sname:ident => $scall:path;)+ },
        { $($(#[$bmeta:meta])* $bname:ident => $bcall:path;)+ },
        { $($(#[$shmmeta:meta])* $shmname:ident => $shmcall:path;)+ }
    ) => {
        mod $combo_mod {
            use super::*;
            const SPEC: SubjectSpec = $spec;

            mod direction_harness_to_subject {
                use super::*;
                $(
                    $(#[$hmeta])*
                    #[test]
                    fn $hname() {
                        $hcall(SPEC);
                    }
                )+
            }

            mod direction_subject_to_harness {
                use super::*;
                $(
                    $(#[$smeta])*
                    #[test]
                    fn $sname() {
                        $scall(SPEC);
                    }
                )+
            }

            mod direction_bidirectional {
                use super::*;
                $(
                    $(#[$bmeta])*
                    #[test]
                    fn $bname() {
                        $bcall(SPEC);
                    }
                )+
            }

            emit_shm_harness_to_subject_tests!(
                $transport,
                SPEC,
                {
                    $(
                        $(#[$shmmeta])*
                        $shmname => $shmcall;
                    )+
                }
            );
        }
    };
}

macro_rules! subject_combo_matrix_tests {
    (
        combos {
            $(
                $combo_mod:ident => {
                    spec: $spec:expr,
                    transport: $transport:ident
                }
            ),+ $(,)?
        }
        tests {
            harness_to_subject $harness_to_subject:tt
            subject_to_harness $subject_to_harness:tt
            bidirectional $bidirectional:tt
            shm_harness_to_subject $shm_harness_to_subject:tt
        }
    ) => {
        $(
            subject_combo_matrix_one!(
                $combo_mod,
                $spec,
                $transport,
                $harness_to_subject,
                $subject_to_harness,
                $bidirectional,
                $shm_harness_to_subject
            );
        )+
    };
}

subject_combo_matrix_tests! {
    combos {
        lang_rust_transport_tcp => {
            spec: SUBJECT_RUST_TCP,
            transport: tcp
        },
        lang_rust_transport_shm_guest_mode => {
            spec: SUBJECT_RUST_SHM_GUEST,
            transport: shm
        },
        lang_typescript_transport_tcp => {
            spec: SUBJECT_TYPESCRIPT_TCP,
            transport: tcp
        },
        lang_swift_transport_tcp => {
            spec: SUBJECT_SWIFT_TCP,
            transport: tcp
        },
        lang_swift_transport_shm_guest_mode => {
            spec: SUBJECT_SWIFT_SHM_GUEST,
            transport: shm
        },
        lang_swift_transport_shm_host_mode => {
            spec: SUBJECT_SWIFT_SHM_HOST,
            transport: shm
        }
    }
    tests {
        harness_to_subject {
            // r[verify call.initiate]
            // r[verify call.complete]
            // r[verify call.lifecycle.single-response]
            // r[verify call.lifecycle.ordering]
            rpc_echo_roundtrip => testbed::run_rpc_echo_roundtrip;

            // r[verify call.error.user]
            rpc_user_error_roundtrip => testbed::run_rpc_user_error_roundtrip;

            // r[verify call.pipelining.allowed]
            // r[verify call.pipelining.independence]
            // r[verify core.call]
            // r[verify core.call.request-id]
            rpc_pipelining_multiple_requests => testbed::run_rpc_pipelining_multiple_requests;

            // r[verify channeling.type]
            // r[verify channeling.data]
            // r[verify channeling.close]
            channeling_generate_server_to_client => channeling::run_channeling_generate_server_to_client;

            // r[verify transport.message.binary]
            binary_payload_sizes => binary_payloads::run_subject_process_message_binary_payload_sizes;
        }

        subject_to_harness {
            // r[verify channeling.type]
            // r[verify channeling.data]
            // r[verify channeling.close]
            // r[verify channeling.caller-pov]
            // r[verify channeling.allocation.caller]
            channeling_sum_client_to_server => channeling::run_channeling_sum_client_to_server;
        }

        bidirectional {
            // r[verify channeling.type]
            // r[verify channeling.lifecycle.immediate-data]
            channeling_transform => channeling::run_channeling_transform_bidirectional;
        }

        shm_harness_to_subject {
            // r[verify transport.message.binary]
            // r[verify shm.framing.threshold]
            binary_payload_cutover_boundaries => binary_payloads::run_subject_process_message_binary_payload_shm_cutover_boundaries;
        }
    }
}

// r[verify transport.message.binary]
#[test]
fn lang_rust_to_rust_transport_mem_direction_bidirectional_binary_payload_transport_matrix() {
    binary_payload_transport_matrix::run_rust_binary_payload_transport_matrix_mem();
}

// r[verify transport.message.binary]
#[test]
fn lang_rust_to_rust_transport_tcp_direction_bidirectional_binary_payload_transport_matrix() {
    binary_payload_transport_matrix::run_rust_binary_payload_transport_matrix_subject_tcp(
        SUBJECT_RUST_TCP,
    );
}

// r[verify transport.message.binary]
#[test]
fn lang_rust_to_rust_transport_shm_direction_bidirectional_binary_payload_transport_matrix() {
    binary_payload_transport_matrix::run_rust_binary_payload_transport_matrix_subject_shm(
        SUBJECT_RUST_SHM_GUEST,
    );
}

// r[verify transport.shm]
// r[verify transport.interop]
#[cfg(all(unix, target_os = "macos"))]
#[test]
fn lang_swift_to_rust_transport_shm_direction_guest_to_host_cross_language_data_path() {
    cross_language_shm_guest_matrix::run_data_path_case();
}

// r[verify transport.shm]
// r[verify transport.interop]
#[cfg(all(unix, target_os = "macos"))]
#[test]
fn lang_swift_to_rust_transport_shm_direction_guest_to_host_cross_language_message_v7() {
    cross_language_shm_guest_matrix::run_message_v7_case();
}

// r[verify transport.shm]
// r[verify transport.interop]
#[cfg(all(unix, target_os = "macos"))]
#[test]
fn lang_rust_to_swift_transport_shm_direction_host_to_guest_cross_language_mmap_ref_receive() {
    cross_language_shm_guest_matrix::run_mmap_ref_receive_case();
}
