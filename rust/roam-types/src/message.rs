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

structstruck::strike! {
    #[repr(u8)]
    #[derive(Debug, Clone, PartialEq, Eq, Facet)]
    pub enum MessagePayload {
        // ========================================================================
        // Handshake
        // ========================================================================
        /// Sent by initiator to acceptor as the first message
        Hello(struct Hello {
            /// Must be equal to 7
            version: u32,

            /// Metadata associated with the connection.
            metadata: Metadata,
        }),

        /// Sent by acceptor back to initiator
        HelloYourself(struct HelloYourself {
            /// You can _also_ have metadata if you want.
            metadata: Metadata,
        }),

        // ========================================================================
        // Virtual connection control
        // ========================================================================
        /// Request a new virtual connection.
        Connect(struct ConnectPayload {
            request_id: RequestId,
            metadata: Metadata,
        }),

        /// Accept a virtual connection request.
        Accept(struct AcceptPayload {
            request_id: RequestId,
            conn_id: ConnectionId,
            metadata: Metadata,
        }),

        /// Reject a virtual connection request.
        Reject(struct RejectPayload {
            request_id: RequestId,
            reason: String,
            metadata: Metadata,
        }),

        // ========================================================================
        // Connection control (conn_id scoped)
        // ========================================================================
        /// Close a virtual connection.
        /// Goodbye on conn 0 closes entire link.
        Goodbye(struct GoodbyePayload {
            conn_id: ConnectionId,
            reason: String,
        }),

        // ========================================================================
        // RPC (conn_id scoped)
        // ========================================================================
        /// Request carries metadata key-value pairs.
        /// Unknown keys are ignored.
        Request(struct RequestPayload {
            conn_id: ConnectionId,
            request_id: RequestId,
            method_id: MethodId,
            metadata: Metadata,

            /// Channel IDs used by this call, in argument declaration order.
            channels: Vec<ChannelId>,

            payload: Payload,
        }),

        /// r[impl core.metadata] - Response carries metadata key-value pairs.
        /// r[impl call.metadata.unknown] - Unknown keys are ignored.
        Response(struct ResponsePayload {
            conn_id: ConnectionId,
            request_id: RequestId,
            metadata: Metadata,
            /// Channel IDs for streams in the response, in return type declaration order.
            /// Client uses these to bind receivers for incoming Data messages.
            channels: Vec<ChannelId>,
            payload: Payload,
        }),

        /// r[impl call.cancel.message] - Cancel message requests callee stop processing.
        /// r[impl call.cancel.no-response-required] - Caller should timeout, not wait indefinitely.
        Cancel(struct CancelPayload {
            conn_id: ConnectionId,
            request_id: RequestId,
        }),

        // ========================================================================
        // Channels (conn_id scoped)
        // ========================================================================
        Data(struct DataPayload {
            conn_id: ConnectionId,
            channel_id: ChannelId,
            payload: Payload,
        }),

        Close(struct ClosePayload {
            conn_id: ConnectionId,
            channel_id: ChannelId,
        }),

        Reset(struct ResetPayload {
            conn_id: ConnectionId,
            channel_id: ChannelId,
        }),
    }
}

/// Spec v7 Hello


/// Sent as a response to Hello
pub
