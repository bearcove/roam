//! Spec-level wire types.
//!
//! Canonical definitions live in `docs/content/spec/_index.md` and `docs/content/shm-spec/_index.md`.

use std::marker::PhantomData;

use crate::{ChannelId, ConnectionId, Metadata, MethodId, RequestId};
use facet::{Facet, FacetOpaqueAdapter, OpaqueDeserialize, OpaqueSerialize, PtrConst, Shape};

/// Per-connection limits advertised by a peer.
// r[impl session.connection-settings]
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub struct ConnectionSettings {
    /// Maximum number of in-flight requests this peer is willing to accept on this connection.
    pub max_concurrent_requests: u32,
}

impl Default for ConnectionSettings {
    fn default() -> Self {
        Self {
            max_concurrent_requests: 1024,
        }
    }
}

/// Protocol message.
// r[impl session]
// r[impl session.message]
// r[impl session.message.connection-id]
// r[impl session.peer]
// r[impl session.symmetry]
#[derive(Debug, Facet)]
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
    #[structstruck::each[derive(Debug, Facet)]]
    pub enum MessagePayload<'payload> {
        // ========================================================================
        // Control (conn 0 only)
        // ========================================================================

        /// Sent by initiator to acceptor as the first message
        // r[impl session.handshake]
        // r[impl session.connection-settings.hello]
        Hello(pub struct Hello {
            /// Must be equal to 7
            pub version: u32,

            /// Parity claimed by the initiator — acceptor will take the other
            pub parity: Parity,

            /// Connection limits advertised by the initiator for the root connection.
            pub connection_settings: ConnectionSettings,

            /// Metadata associated with the connection.
            pub metadata: Metadata,
        }),

        /// Sent by acceptor back to initiator. Poetic on purpose, I'm not changing the name.
        // r[impl session.connection-settings.hello]
        HelloYourself(pub struct HelloYourself {
            /// Connection limits advertised by the acceptor for the root connection.
            pub connection_settings: ConnectionSettings,

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
        // r[impl session.connection-settings.open]
        OpenConnection(pub struct OpenConnection {
            /// Parity requested by the opener for this virtual connection.
            pub parity: Parity,

            /// Connection limits advertised by the opener.
            pub connection_settings: ConnectionSettings,

            /// Metadata associated with the connection.
            pub metadata: Metadata,
        }),

        /// Accept a virtual connection request — sent on the connection ID requested.
        // r[impl session.connection-settings.open]
        AcceptConnection(pub struct AcceptConnection {
            /// Connection limits advertised by the accepter.
            pub connection_settings: ConnectionSettings,

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

        /// Grant additional send credit to a channel sender.
        // r[impl rpc.flow-control.credit.grant]
        GrantCredit(struct GrantCredit {
            /// Channel ID to grant credit on.
            pub channel_id: ChannelId,

            /// Number of additional items the sender may send.
            pub additional: u32,
        }),

    }
}

/// A payload — arguments for a request, or return type for a response.
///
/// Uses `#[facet(opaque = PayloadAdapter)]` so that format crates handle
/// serialization/deserialization through the adapter contract:
/// - **Send path:** `serialize_map` extracts `(ptr, shape)` from `Borrowed`.
/// - **Recv path:** `deserialize_build` produces `RawBorrowed` or `RawOwned`.
// r[impl zerocopy.payload]
#[derive(Debug, Facet)]
#[repr(u8)]
#[facet(opaque = PayloadAdapter, traits(Debug))]
pub enum Payload<'payload> {
    // r[impl zerocopy.payload.borrowed]
    /// Outgoing: type-erased pointer to caller-owned memory + its Shape.
    Borrowed {
        ptr: PtrConst,
        shape: &'static Shape,
        _lt: PhantomData<&'payload ()>,
    },

    // r[impl zerocopy.payload.bytes]
    /// Incoming: raw bytes borrowed from the backing (zero-copy).
    RawBorrowed(&'payload [u8]),

    /// Incoming: owned bytes (when borrowing is unavailable).
    RawOwned(Vec<u8>),
}

/// Adapter that bridges [`Payload`] through the opaque field contract.
pub struct PayloadAdapter;

impl FacetOpaqueAdapter for PayloadAdapter {
    type Error = String;
    type SendValue<'a> = Payload<'a>;
    type RecvValue<'de> = Payload<'de>;

    fn serialize_map(value: &Self::SendValue<'_>) -> OpaqueSerialize {
        match value {
            Payload::Borrowed { ptr, shape, .. } => OpaqueSerialize { ptr: *ptr, shape },
            _ => unreachable!("serialize_map is only called on outgoing Payload::Borrowed"),
        }
    }

    fn deserialize_build<'de>(
        input: OpaqueDeserialize<'de>,
    ) -> Result<Self::RecvValue<'de>, Self::Error> {
        Ok(match input {
            OpaqueDeserialize::Borrowed(bytes) => Payload::RawBorrowed(bytes),
            OpaqueDeserialize::Owned(bytes) => Payload::RawOwned(bytes),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use facet::Facet;
    use facet_reflect::Peek;

    /// A wrapper struct that contains a Payload, mimicking how Request/Response use it.
    #[derive(Debug, Facet)]
    struct Envelope<'a> {
        method_id: u32,
        payload: Payload<'a>,
    }

    /// Construct a Payload::Borrowed from a concrete value.
    fn borrowed_payload<'a, T: Facet<'a>>(value: &'a T) -> Payload<'a> {
        Payload::Borrowed {
            ptr: PtrConst::new((value as *const T).cast::<u8>()),
            shape: T::SHAPE,
            _lt: PhantomData,
        }
    }

    /// Serialize a non-'static value via Peek.
    fn serialize_envelope(envelope: &Envelope<'_>) -> Vec<u8> {
        let peek = Peek::new(envelope);
        facet_postcard::peek_to_vec(peek).expect("serialize")
    }

    #[test]
    fn payload_roundtrip_owned() {
        let data: Vec<u8> = vec![0xDE, 0xAD, 0xBE, 0xEF];
        let envelope = Envelope {
            method_id: 42,
            payload: borrowed_payload(&data),
        };

        let bytes = serialize_envelope(&envelope);
        let decoded: Envelope<'static> =
            facet_postcard::from_slice(&bytes).expect("deserialize owned");

        assert_eq!(decoded.method_id, 42);
        match &decoded.payload {
            Payload::RawOwned(buf) => {
                let inner: Vec<u8> = facet_postcard::from_slice(buf).expect("decode inner payload");
                assert_eq!(inner, vec![0xDE, 0xAD, 0xBE, 0xEF]);
            }
            other => panic!("expected RawOwned, got {other:?}"),
        }
    }

    #[test]
    fn payload_roundtrip_borrowed() {
        let data: Vec<u8> = vec![0xCA, 0xFE];
        let envelope = Envelope {
            method_id: 7,
            payload: borrowed_payload(&data),
        };

        let bytes = serialize_envelope(&envelope);
        let decoded: Envelope<'_> =
            facet_postcard::from_slice_borrowed(&bytes).expect("deserialize borrowed");

        assert_eq!(decoded.method_id, 7);
        match &decoded.payload {
            Payload::RawBorrowed(slice) => {
                let inner: Vec<u8> =
                    facet_postcard::from_slice(slice).expect("decode inner payload");
                assert_eq!(inner, vec![0xCA, 0xFE]);
            }
            other => panic!("expected RawBorrowed, got {other:?}"),
        }
    }

    #[test]
    fn payload_borrowed_slice_points_into_input() {
        let data: &[u8] = &[0x01, 0x02, 0x03];
        let envelope = Envelope {
            method_id: 1,
            payload: borrowed_payload(&data),
        };

        let bytes = serialize_envelope(&envelope);
        let decoded: Envelope<'_> =
            facet_postcard::from_slice_borrowed(&bytes).expect("deserialize borrowed");

        match &decoded.payload {
            Payload::RawBorrowed(slice) => {
                // The borrowed slice should point into `bytes` (zero-copy)
                let bytes_range = bytes.as_ptr_range();
                let slice_start = slice.as_ptr();
                assert!(
                    bytes_range.contains(&slice_start),
                    "borrowed slice should point into the input buffer"
                );
            }
            other => panic!("expected RawBorrowed, got {other:?}"),
        }
    }

    #[test]
    fn payload_with_string_value() {
        let value = String::from("hello roam");
        let envelope = Envelope {
            method_id: 99,
            payload: borrowed_payload(&value),
        };

        let bytes = serialize_envelope(&envelope);
        let decoded: Envelope<'static> =
            facet_postcard::from_slice(&bytes).expect("deserialize owned");

        assert_eq!(decoded.method_id, 99);
        match &decoded.payload {
            Payload::RawOwned(buf) => {
                let inner: String = facet_postcard::from_slice(buf).expect("decode inner payload");
                assert_eq!(inner, "hello roam");
            }
            other => panic!("expected RawOwned, got {other:?}"),
        }
    }
}
