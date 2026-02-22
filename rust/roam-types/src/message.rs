//! Spec-level wire types.
//!
//! Canonical definitions live in `docs/content/spec/_index.md` and `docs/content/shm-spec/_index.md`.

use std::marker::PhantomData;

use crate::{ChannelId, ConnectionId, Metadata, MethodId, RequestId};
use facet::{Facet, PtrConst, Shape};

/// Protocol message.
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct Message<'payload> {
    /// Connection ID: 0 for control messages (Hello, HelloYourself)
    connection_id: ConnectionId,

    /// Message payload
    payload: MessagePayload<'payload>,
}

/// Whether a peer will use odd or even IDs for requests and channels
/// on a given connection.
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
    #[structstruck::each[derive(Debug, Clone, PartialEq, Eq, Facet)]]
    pub enum MessagePayload<'payload> {
        // ========================================================================
        // Handshake
        // ========================================================================

        /// Sent by initiator to acceptor as the first message
        Hello(pub struct Hello {
            /// Must be equal to 7
            pub version: u32,

            /// Parity claimed by the initiator — acceptor will take the other
            pub parity: Parity,

            /// Metadata associated with the connection.
            pub metadata: Metadata,
        }),

        /// Sent by acceptor back to initiator
        HelloYourself(pub struct HelloYourself {
            /// You can _also_ have metadata if you want.
            pub metadata: Metadata,
        }),

        // ========================================================================
        // Connection control
        // ========================================================================

        /// Request a new virtual connection. This is sent on the desired connection
        /// ID, even though it doesn't exist yet.
        Connect(pub struct Connect {
            /// Metadata associated with the connection.
            pub metadata: Metadata,
        }),

        /// Accept a virtual connection request — sent on the connection ID requested.
        Accept(pub struct Accept {
            /// Metadata associated with the connection.
            pub metadata: Metadata,
        }),

        /// Reject a virtual connection request — sent on the connection ID requested.
        Reject(pub struct Reject {
            /// Why the connection was rejected.
            pub reason: String,
        }),

        /// Close a virtual connection.
        /// Goodbye on conn 0 closes entire link.
        Goodbye(pub struct Goodbye {
            /// Why the connection was closed.
            pub reason: String,
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
        Cancel(struct Cancel {
            /// Request ID of the request being canceled.
            pub request_id: RequestId,

            /// Reason for cancellation
            pub reason: String,
        }),

        // ========================================================================
        // Channels
        // ========================================================================

        /// Send an item on a channel. Channels are not "opened", they are created
        /// implicitly by calls.
        Data(struct Data<'payload> {
            /// Channel ID (unique per-connection) for the channel to send data on.
            pub channel_id: ChannelId,

            /// Payload to send on the channel.
            pub payload: Payload<'payload>,
        }),

        /// Close a channel — sent by the initiator when they're gracefully done
        /// with a channel.
        Close(struct Close {
            /// Channel ID (unique per-connection) for the channel to close.
            pub channel_id: ChannelId,

            /// Reason for closing the channel.
            pub reason: String,
        }),

        /// Reset a channel — sent by the acceptor when they would like the initiator
        /// to please, stop sending items through.
        Reset(struct Reset {
            /// Channel ID (unique per-connection) for the channel to reset.
            pub channel_id: ChannelId,

            /// Reason for resetting the channel.
            pub reason: String,
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
