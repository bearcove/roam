//! Spec-level wire types.
//!
//! Canonical definitions live in `docs/content/spec/_index.md` and `docs/content/shm-spec/_index.md`.

use crate::{ChannelId, ConnectionId, Metadata, MethodId, RequestId};
use facet::Facet;

/// Protocol message.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Message {
    /// Connection ID: 0 for control messages (Hello, HelloYourself)
    connection_id: ConnectionId,

    /// Message payload
    payload: MessagePayload,
}

#[repr(u8)]
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub enum MessagePayload {
    // ========================================================================
    // Handshake
    // ========================================================================
    /// Sent by initiator to acceptor as the first message
    Hello(Hello),

    /// Sent by acceptor back to initiator
    HelloYourself(HelloYourself),

    // ========================================================================
    // Virtual connection control
    // ========================================================================
    /// Request a new virtual connection.
    Connect {
        request_id: RequestId,
        metadata: Metadata,
    },

    /// Accept a virtual connection request.
    Accept {
        request_id: RequestId,
        conn_id: ConnectionId,
        metadata: Metadata,
    },

    /// Reject a virtual connection request.
    Reject {
        request_id: RequestId,
        reason: String,
        metadata: Metadata,
    },

    // ========================================================================
    // Connection control (conn_id scoped)
    // ========================================================================
    /// Close a virtual connection.
    /// Goodbye on conn 0 closes entire link.
    Goodbye {
        conn_id: ConnectionId,
        reason: String,
    },

    // ========================================================================
    // RPC (conn_id scoped)
    // ========================================================================
    /// Request carries metadata key-value pairs.
    /// Unknown keys are ignored.
    Request {
        conn_id: ConnectionId,
        request_id: RequestId,
        method_id: MethodId,
        metadata: Metadata,

        /// Channel IDs used by this call, in argument declaration order.
        channels: Vec<ChannelId>,

        payload: Payload,
    },

    /// r[impl core.metadata] - Response carries metadata key-value pairs.
    /// r[impl call.metadata.unknown] - Unknown keys are ignored.
    Response {
        conn_id: ConnectionId,
        request_id: RequestId,
        metadata: Metadata,
        /// Channel IDs for streams in the response, in return type declaration order.
        /// Client uses these to bind receivers for incoming Data messages.
        channels: Vec<ChannelId>,
        payload: Payload,
    },

    /// r[impl call.cancel.message] - Cancel message requests callee stop processing.
    /// r[impl call.cancel.no-response-required] - Caller should timeout, not wait indefinitely.
    Cancel {
        conn_id: ConnectionId,
        request_id: RequestId,
    },

    // ========================================================================
    // Channels (conn_id scoped)
    // ========================================================================
    Data {
        conn_id: ConnectionId,
        channel_id: ChannelId,
        payload: Payload,
    },

    Close {
        conn_id: ConnectionId,
        channel_id: ChannelId,
    },

    Reset {
        conn_id: ConnectionId,
        channel_id: ChannelId,
    },
}

/// Spec v7 Hello
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Hello {
    /// Must be equal to 7
    version: u32,

    /// Metadata associated with the connection.
    metadata: Metadata,
}

/// Sent as a response to Hello
pub struct HelloYourself {
    /// You can _also_ have metadata if you want.
    metadata: Metadata,
}
