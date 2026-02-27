//! Generate v7 wire golden vectors from canonical Rust `roam-types::Message`.

use std::{fs, path::PathBuf};

use roam_types::{
    ChannelBody, ChannelClose, ChannelGrantCredit, ChannelId, ChannelItem, ChannelMessage,
    ChannelReset, ConnectionAccept, ConnectionClose, ConnectionId, ConnectionOpen,
    ConnectionReject, ConnectionSettings, Hello, HelloYourself, Message, MessagePayload, Metadata,
    MetadataEntry, MetadataFlags, MetadataValue, MethodId, Parity, Payload, ProtocolError,
    RequestBody, RequestCall, RequestCancel, RequestId, RequestMessage, RequestResponse,
};

fn fixture_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("test-fixtures")
        .join("golden-vectors")
        .join("wire-v7")
}

fn write_fixture(name: &str, bytes: &[u8]) {
    let path = fixture_dir().join(format!("{name}.bin"));
    fs::write(&path, bytes).expect("failed to write fixture");
    eprintln!("wrote {} ({} bytes)", path.display(), bytes.len());
}

fn encode_message(message: &Message<'_>) -> Vec<u8> {
    facet_postcard::to_vec(message).expect("serialize message fixture")
}

fn sample_metadata() -> Metadata<'static> {
    vec![
        MetadataEntry {
            key: "trace-id",
            value: MetadataValue::String("abc123"),
            flags: MetadataFlags::NONE,
        },
        MetadataEntry {
            key: "auth",
            value: MetadataValue::Bytes(&[0xDE, 0xAD, 0xBE, 0xEF]),
            flags: MetadataFlags::SENSITIVE | MetadataFlags::NO_PROPAGATE,
        },
        MetadataEntry {
            key: "attempt",
            value: MetadataValue::U64(2),
            flags: MetadataFlags::NONE,
        },
    ]
}

fn main() {
    fs::create_dir_all(fixture_dir()).expect("failed to create fixture directory");

    let conn_settings = ConnectionSettings {
        parity: Parity::Odd,
        max_concurrent_requests: 64,
    };
    let meta = sample_metadata();

    let hello = Message {
        connection_id: ConnectionId::ROOT,
        payload: MessagePayload::Hello(Hello {
            version: 7,
            connection_settings: conn_settings.clone(),
            metadata: meta.clone(),
        }),
    };
    write_fixture("message_hello", &encode_message(&hello));

    let hello_yourself = Message {
        connection_id: ConnectionId::ROOT,
        payload: MessagePayload::HelloYourself(HelloYourself {
            connection_settings: ConnectionSettings {
                parity: Parity::Even,
                max_concurrent_requests: 32,
            },
            metadata: meta.clone(),
        }),
    };
    write_fixture("message_hello_yourself", &encode_message(&hello_yourself));

    let protocol_error = Message {
        connection_id: ConnectionId::ROOT,
        payload: MessagePayload::ProtocolError(ProtocolError {
            description: "bad frame sequence",
        }),
    };
    write_fixture("message_protocol_error", &encode_message(&protocol_error));

    let connection_open = Message {
        connection_id: ConnectionId(2),
        payload: MessagePayload::ConnectionOpen(ConnectionOpen {
            connection_settings: conn_settings.clone(),
            metadata: meta.clone(),
        }),
    };
    write_fixture("message_connection_open", &encode_message(&connection_open));

    let connection_accept = Message {
        connection_id: ConnectionId(2),
        payload: MessagePayload::ConnectionAccept(ConnectionAccept {
            connection_settings: ConnectionSettings {
                parity: Parity::Even,
                max_concurrent_requests: 96,
            },
            metadata: meta.clone(),
        }),
    };
    write_fixture(
        "message_connection_accept",
        &encode_message(&connection_accept),
    );

    let connection_reject = Message {
        connection_id: ConnectionId(4),
        payload: MessagePayload::ConnectionReject(ConnectionReject {
            metadata: meta.clone(),
        }),
    };
    write_fixture(
        "message_connection_reject",
        &encode_message(&connection_reject),
    );

    let connection_close = Message {
        connection_id: ConnectionId(2),
        payload: MessagePayload::ConnectionClose(ConnectionClose {
            metadata: meta.clone(),
        }),
    };
    write_fixture(
        "message_connection_close",
        &encode_message(&connection_close),
    );

    let args_call: u32 = 0x1234_5678;
    let request_call = Message {
        connection_id: ConnectionId(2),
        payload: MessagePayload::RequestMessage(RequestMessage {
            id: RequestId(11),
            body: RequestBody::Call(RequestCall {
                method_id: MethodId(0xE5A1_D6B2_C390_F001),
                args: Payload::outgoing(&args_call),
                channels: vec![ChannelId(3), ChannelId(5)],
                metadata: meta.clone(),
            }),
        }),
    };
    write_fixture("message_request_call", &encode_message(&request_call));

    let ret_response: u64 = 0xFACE_B00C;
    let request_response = Message {
        connection_id: ConnectionId(2),
        payload: MessagePayload::RequestMessage(RequestMessage {
            id: RequestId(11),
            body: RequestBody::Response(RequestResponse {
                ret: Payload::outgoing(&ret_response),
                channels: vec![ChannelId(7)],
                metadata: meta.clone(),
            }),
        }),
    };
    write_fixture(
        "message_request_response",
        &encode_message(&request_response),
    );

    let request_cancel = Message {
        connection_id: ConnectionId(2),
        payload: MessagePayload::RequestMessage(RequestMessage {
            id: RequestId(11),
            body: RequestBody::Cancel(RequestCancel {
                metadata: meta.clone(),
            }),
        }),
    };
    write_fixture("message_request_cancel", &encode_message(&request_cancel));

    let channel_item_value: u16 = 77;
    let channel_item = Message {
        connection_id: ConnectionId(2),
        payload: MessagePayload::ChannelMessage(ChannelMessage {
            id: ChannelId(3),
            body: ChannelBody::Item(ChannelItem {
                item: Payload::outgoing(&channel_item_value),
            }),
        }),
    };
    write_fixture("message_channel_item", &encode_message(&channel_item));

    let channel_close = Message {
        connection_id: ConnectionId(2),
        payload: MessagePayload::ChannelMessage(ChannelMessage {
            id: ChannelId(3),
            body: ChannelBody::Close(ChannelClose {
                metadata: meta.clone(),
            }),
        }),
    };
    write_fixture("message_channel_close", &encode_message(&channel_close));

    let channel_reset = Message {
        connection_id: ConnectionId(2),
        payload: MessagePayload::ChannelMessage(ChannelMessage {
            id: ChannelId(3),
            body: ChannelBody::Reset(ChannelReset {
                metadata: meta.clone(),
            }),
        }),
    };
    write_fixture("message_channel_reset", &encode_message(&channel_reset));

    let channel_credit = Message {
        connection_id: ConnectionId(2),
        payload: MessagePayload::ChannelMessage(ChannelMessage {
            id: ChannelId(3),
            body: ChannelBody::GrantCredit(ChannelGrantCredit { additional: 1024 }),
        }),
    };
    write_fixture(
        "message_channel_grant_credit",
        &encode_message(&channel_credit),
    );
}
