//! Spec-level wire types.
//!
//! Canonical definitions live in `docs/content/spec/_index.md` and `docs/content/shm-spec/_index.md`.

use std::marker::PhantomData;

use crate::{ChannelId, ConnectionId, Metadata, MethodId, RequestId};
use facet::{Facet, PtrConst, Shape};

/// Protocol message.
// r[impl session]
// r[impl session.message]
// r[impl session.message.connection-id]
// r[impl session.peer]
// r[impl session.symmetry]
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Message<'payload> {
    /// Connection ID: 0 for control messages (Hello, HelloYourself)
    connection_id: ConnectionId,

    /// Message payload
    payload: MessagePayload<'payload>,
}

/// Whether a peer will use odd or even IDs for requests and channels
/// on a given connection.
// r[impl session.parity]
// r[impl session.role]
// r[impl connection.parity]
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
#[repr(u8)]
pub enum Parity {
    Odd,
    Even,
}

impl Parity {
    /// Returns the opposite parity.
    pub fn other(&self) -> Self {
        match self {
            Parity::Odd => Parity::Even,
            Parity::Even => Parity::Odd,
        }
    }
}

structstruck::strike! {
    #[repr(u8)]
    // r[impl session.message.payloads]
    #[structstruck::each[derive(Debug, Clone, PartialEq, Eq, Facet)]]
    pub enum MessagePayload<'payload> {
        // ========================================================================
        // Control (conn 0 only)
        // ========================================================================

        /// Sent by initiator to acceptor as the first message
        // r[impl session.handshake]
        Hello(pub struct Hello {
            /// Must be equal to 7
            pub version: u32,

            /// Parity claimed by the initiator — acceptor will take the other
            pub parity: Parity,

            /// Metadata associated with the connection.
            pub metadata: Metadata,
        }),

        /// Sent by acceptor back to initiator. Poetic on purpose, I'm not changing the name.
        HelloYourself(pub struct HelloYourself {
            /// You can _also_ have metadata if you want.
            pub metadata: Metadata,
        }),

        /// Sent by either peer when the counterpart has violated the protocol.
        /// The sender closes the transport immediately after sending this message.
        /// No reply is expected or valid.
        // r[impl session.protocol-error]
        ProtocolError(pub struct ProtocolError {
            /// Human-readable description of the protocol violation.
            pub description: String,
        }),

        // ========================================================================
        // Connection control
        // ========================================================================

        /// Request a new virtual connection. This is sent on the desired connection
        /// ID, even though it doesn't exist yet.
        // r[impl connection.open]
        // r[impl connection.virtual]
        OpenConnection(pub struct OpenConnection {
            /// Metadata associated with the connection.
            pub metadata: Metadata,
        }),

        /// Accept a virtual connection request — sent on the connection ID requested.
        AcceptConnection(pub struct AcceptConnection {
            /// Metadata associated with the connection.
            pub metadata: Metadata,
        }),

        /// Reject a virtual connection request — sent on the connection ID requested.
        RejectConnection(pub struct RejectConnection {
            /// Metadata associated with the rejection.
            pub metadata: Metadata,
        }),

        /// Close a virtual connection. Trying to close conn 0 is a protocol error.
        CloseConnection(pub struct CloseConnection {
            /// Metadata associated with the close.
            pub metadata: Metadata,
        }),


        // ========================================================================
        // RPC
        // ========================================================================

        /// Perform a request (or a "call")
        Request(pub struct Request<'payload> {
            /// Unique (connection-wide) request identifier, caller-allocated (as per parity)
            pub request_id: RequestId,

            /// Unique method identifier, hash of fully qualified name + args etc.
            pub method_id: MethodId,

            /// Argument tuple
            pub args: Payload<'payload>,

            /// Channel identifiers, allocated by the caller, that are passed as part
            /// of the arguments.
            pub channels: Vec<ChannelId>,

            /// Metadata associated with this call
            pub metadata: Metadata,
        }),

        /// Respond to a request
        Response(struct Response<'payload> {
            /// Request ID of the request being responded to.
            pub request_id: RequestId,

            /// Channel IDs for streams in the response, in return type declaration order.
            pub channels: Vec<ChannelId>,

            /// Return value
            pub payload: Payload<'payload>,

            /// Arbitrary response metadata
            pub metadata: Metadata,
        }),

        /// Cancel processing of a request.
        CancelRequest(struct CancelRequest {
            /// Request ID of the request being canceled.
            pub request_id: RequestId,

            /// Arbitrary cancel metadata
            pub metadata: Metadata,
        }),

        // ========================================================================
        // Channels
        // ========================================================================

        /// Send an item on a channel. Channels are not "opened", they are created
        /// implicitly by calls.
        ChannelItem(struct ChannelItem<'payload> {
            /// Channel ID (unique per-connection) for the channel to send data on.
            pub channel_id: ChannelId,

            /// Payload to send on the channel.
            pub payload: Payload<'payload>,
        }),

        /// Close a channel — sent by the sender of the channel when they're gracefully done
        /// with a channel.
        CloseChannel(struct CloseChannel {
            /// Channel ID (unique per-connection) for the channel to close.
            pub channel_id: ChannelId,

            /// Metadata associated with closing the channel.
            pub metadata: Metadata,
        }),

        /// Reset a channel — sent by the receiver of a channel when they would like the sender
        /// to please, stop sending items through.
        ResetChannel(struct ResetChannel {
            /// Channel ID (unique per-connection) for the channel to reset.
            pub channel_id: ChannelId,

            /// Metadata associated with resetting the channel.
            pub metadata: Metadata,
        }),

    }
}

/// A payload — arguments for a request, or return type for a response
#[derive(Debug, PartialEq, Eq, Facet, Clone)]
#[repr(u8)]
#[facet(opaque)]
pub enum Payload<'payload> {
    // Borrowed from somewhere, type-erased, still enough to serialize
    Borrowed {
        ptr: PtrConst,
        shape: &'static Shape,
        _phantom2: PhantomData<&'payload ()>,
    },
}
