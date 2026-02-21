use facet::Facet;

declare_u64_id!(
    /// Connection ID identifying a virtual connection on a link.
    ///
    /// Connection 0 is the root connection, established implicitly when the link is created.
    /// Additional connections are opened via Connect/Accept messages.
    ConnectionId
);

impl ConnectionId {
    /// The root connection (always exists on a link).
    pub const ROOT: Self = Self(0);

    /// Check if this is the root connection.
    pub const fn is_root(self) -> bool {
        self.0 == Self::ROOT.0
    }
}

declare_u64_id!(
    /// Request ID identifying an in-flight RPC request.
    ///
    /// Request IDs are unique within a connection and monotonically increasing.
    /// r[impl call.request-id.uniqueness]
    RequestId
);

declare_u64_id!(
    /// ID of a channel between two peers.
    ChannelId
);

/// Opaque payload (arguments, response, etc.) encoded as postcard
#[derive(Facet, Clone, Debug, PartialEq, Eq, Hash)]
#[facet(transparent)]
#[repr(transparent)]
pub struct Payload(pub Vec<u8>);
