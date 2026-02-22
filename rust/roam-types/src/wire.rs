//! Spec-level wire types.
//!
//! Canonical definitions live in `docs/content/spec/_index.md` and `docs/content/shm-spec/_index.md`.

use crate::{ChannelId, ConnectionId, MethodId, Payload, RequestId};
use facet::Facet;

/// Hello message for handshake.
// r[impl message.hello.structure]
#[repr(u8)]
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub enum Hello {
    /// Spec v1 Hello - deprecated, will be rejected.
    V1 {
        max_payload_size: u32,
        initial_channel_credit: u32,
    } = 0,

    /// Spec v2 Hello - deprecated, will be rejected.
    V2 {
        max_payload_size: u32,
        initial_channel_credit: u32,
    } = 1,

    /// Spec v3 Hello - deprecated.
    V3 {
        max_payload_size: u32,
        initial_channel_credit: u32,
    } = 2,

    /// Spec v4 Hello - length-prefixed byte-stream framing.
    V4 {
        max_payload_size: u32,
        initial_channel_credit: u32,
    } = 3,

    /// Spec v5 Hello - adds request/response concurrency negotiation.
    V5 {
        max_payload_size: u32,
        initial_channel_credit: u32,
        max_concurrent_requests: u32,
    } = 4,

    /// Spec v6 Hello - adds arbitrary metadata (peer identity, etc.).
    V6 {
        max_payload_size: u32,
        initial_channel_credit: u32,
        max_concurrent_requests: u32,
        metadata: Metadata,
    } = 5,
}

/// Metadata value.
// r[impl call.metadata.type]
#[repr(u8)]
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub enum MetadataValue {
    String(String) = 0,
    Bytes(Vec<u8>) = 1,
    U64(u64) = 2,
}

impl MetadataValue {
    /// Get the byte length of this value.
    pub fn byte_len(&self) -> usize {
        match self {
            MetadataValue::String(s) => s.len(),
            MetadataValue::Bytes(b) => b.len(),
            MetadataValue::U64(_) => 8,
        }
    }
}

/// Metadata entry flags.
///
/// r[impl call.metadata.flags] - Flags control metadata handling behavior.
pub mod metadata_flags {
    /// No special handling.
    pub const NONE: u64 = 0;

    /// Value MUST NOT be logged, traced, or included in error messages.
    pub const SENSITIVE: u64 = 1 << 0;

    /// Value MUST NOT be forwarded to downstream calls.
    pub const NO_PROPAGATE: u64 = 1 << 1;
}

/// Metadata validation limits.
///
/// r[impl call.metadata.limits] - Metadata has size limits.
pub mod metadata_limits {
    /// Maximum number of metadata entries.
    pub const MAX_ENTRIES: usize = 128;
    /// Maximum key size in bytes.
    pub const MAX_KEY_SIZE: usize = 256;
    /// Maximum value size in bytes (16 KB).
    pub const MAX_VALUE_SIZE: usize = 16 * 1024;
    /// Maximum total metadata size in bytes (64 KB).
    pub const MAX_TOTAL_SIZE: usize = 64 * 1024;
}

/// Validate metadata against protocol limits.
///
/// r[impl call.metadata.limits] - Validate all metadata constraints.
/// r[impl call.metadata.keys] - Keys at most 256 bytes.
/// r[impl call.metadata.order] - Order is preserved (Vec maintains order).
/// r[impl call.metadata.duplicates] - Duplicate keys are allowed.
pub fn validate_metadata(metadata: &[(String, MetadataValue, u64)]) -> Result<(), &'static str> {
    use metadata_limits::*;

    // Check entry count
    if metadata.len() > MAX_ENTRIES {
        return Err("call.metadata.limits");
    }

    let mut total_size = 0usize;

    for (key, value, _flags) in metadata {
        // Check key size
        if key.len() > MAX_KEY_SIZE {
            return Err("call.metadata.limits");
        }

        // Check value size
        let value_len = value.byte_len();
        if value_len > MAX_VALUE_SIZE {
            return Err("call.metadata.limits");
        }

        // Accumulate total size (flags are varint-encoded, typically 1 byte)
        total_size += key.len() + value_len;
    }

    // Check total size
    if total_size > MAX_TOTAL_SIZE {
        return Err("call.metadata.limits");
    }

    Ok(())
}

/// Metadata entry: (key, value, flags).
///
/// r[impl call.metadata.type] - Metadata is a list of entries.
/// r[impl call.metadata.flags] - Each entry includes flags for handling behavior.
pub type Metadata = Vec<(String, MetadataValue, u64)>;

/// Extract a metadata value as a string.
pub fn metadata_string(metadata: &Metadata, key: &str) -> Option<String> {
    metadata
        .iter()
        .find(|(k, _, _)| k == key)
        .map(|(_, value, _)| match value {
            MetadataValue::String(s) => s.clone(),
            MetadataValue::U64(n) => n.to_string(),
            MetadataValue::Bytes(bytes) => {
                let mut out = String::with_capacity(bytes.len() * 2);
                for byte in bytes {
                    use std::fmt::Write as _;
                    let _ = write!(&mut out, "{byte:02x}");
                }
                out
            }
        })
}

/// Protocol message.
#[repr(u8)]
#[derive(Debug, Clone, PartialEq, Eq, Facet)]
pub enum Message {
    // ========================================================================
    // Link control (no conn_id - applies to entire link)
    // ========================================================================
    /// r[impl message.hello.timing] - Sent immediately after link establishment.
    Hello(Hello) = 0,

    // ========================================================================
    // Virtual connection control
    // ========================================================================
    /// r[impl message.connect.initiate] - Request a new virtual connection.
    Connect {
        request_id: RequestId,
        metadata: Metadata,
    } = 1,

    /// r[impl message.accept.response] - Accept a virtual connection request.
    Accept {
        request_id: RequestId,
        conn_id: ConnectionId,
        metadata: Metadata,
    } = 2,

    /// r[impl message.reject.response] - Reject a virtual connection request.
    Reject {
        request_id: RequestId,
        reason: String,
        metadata: Metadata,
    } = 3,

    // ========================================================================
    // Connection control (conn_id scoped)
    // ========================================================================
    /// r[impl message.goodbye.send] - Close a virtual connection.
    /// r[impl message.goodbye.connection-zero] - Goodbye on conn 0 closes entire link.
    Goodbye {
        conn_id: ConnectionId,
        reason: String,
    } = 4,

    // ========================================================================
    // RPC (conn_id scoped)
    // ========================================================================
    /// r[impl core.metadata] - Request carries metadata key-value pairs.
    /// r[impl call.metadata.unknown] - Unknown keys are ignored.
    Request {
        conn_id: ConnectionId,
        request_id: RequestId,
        method_id: MethodId,
        metadata: Metadata,
        /// Channel IDs used by this call, in argument declaration order.
        /// This is the authoritative source - servers MUST use these IDs,
        /// not any IDs that may be embedded in the payload.
        channels: Vec<ChannelId>,
        payload: Payload,
    } = 5,

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
    } = 6,

    /// r[impl call.cancel.message] - Cancel message requests callee stop processing.
    /// r[impl call.cancel.no-response-required] - Caller should timeout, not wait indefinitely.
    Cancel {
        conn_id: ConnectionId,
        request_id: RequestId,
    } = 7,

    // ========================================================================
    // Channels (conn_id scoped)
    // ========================================================================
    Data {
        conn_id: ConnectionId,
        channel_id: ChannelId,
        payload: Payload,
    } = 8,

    Close {
        conn_id: ConnectionId,
        channel_id: ChannelId,
    } = 9,

    Reset {
        conn_id: ConnectionId,
        channel_id: ChannelId,
    } = 10,

    Credit {
        conn_id: ConnectionId,
        channel_id: ChannelId,
        bytes: u32,
    } = 11,
}
