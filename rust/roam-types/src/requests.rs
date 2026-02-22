use facet::Facet;

declare_id!(
    /// Connection ID identifying a virtual connection on a link.
    ///
    /// Connection 0 is the root connection, established implicitly when the link is created.
    /// Additional connections are opened via Connect/Accept messages.
    ConnectionId, u64
);

impl ConnectionId {
    /// The root connection (always exists on a link).
    pub const ROOT: Self = Self(0);

    /// Check if this is the root connection.
    pub const fn is_root(self) -> bool {
        self.0 == Self::ROOT.0
    }
}

declare_id!(
    /// Request ID identifying an in-flight RPC request.
    ///
    /// Request IDs are unique within a connection and monotonically increasing.
    /// r[impl call.request-id.uniqueness]
    RequestId, u32
);

declare_id!(
    /// ID of a channel between two peers.
    ChannelId, u32
);

impl ChannelId {
    /// Reserved channel ID (not usable).
    ///
    /// shm[impl shm.id.channel-id]
    pub const RESERVED: Self = Self(0);

    /// Create a new channel ID, returning None if the value is 0 (reserved).
    ///
    /// shm[impl shm.id.channel-id]
    #[inline]
    pub fn new(value: u32) -> Option<Self> {
        if value == 0 { None } else { Some(Self(value)) }
    }

    /// Check if this is a host-allocated channel ID (even).
    ///
    /// shm[impl shm.id.channel-parity]
    #[inline]
    pub fn is_host_allocated(self) -> bool {
        self.0.is_multiple_of(2)
    }

    /// Check if this is a guest-allocated channel ID (odd).
    ///
    /// shm[impl shm.id.channel-parity]
    #[inline]
    pub fn is_guest_allocated(self) -> bool {
        self.0 % 2 == 1
    }

    /// Get the table index for this channel ID.
    ///
    /// shm[impl shm.flow.channel-table-indexing]
    #[inline]
    pub fn table_index(self) -> usize {
        self.0 as usize
    }
}

/// Opaque payload (arguments, response, etc.) encoded as postcard
#[derive(Facet, Clone, Debug, PartialEq, Eq, Hash)]
#[facet(transparent)]
#[repr(transparent)]
pub struct Payload(pub Vec<u8>);
