//! Telex — Rust-native RPC where traits are the schema.
//!
//! This is the facade crate. It re-exports everything needed by both
//! hand-written code and `#[telex::service]` macro-generated code.

mod client_logging;
pub mod schema_deser;
mod server_logging;

// Re-export the proc macro
pub use client_logging::{ClientLogging, ClientLoggingOptions};
pub use telex_service_macros::service;
pub use server_logging::{ServerLogging, ServerLoggingOptions};

// Re-export facet (generated code uses `telex::facet::Facet`)
pub use facet;

// Re-export telex-postcard for generated code and downstream helpers.
pub use telex_postcard;

// Re-export method identity functions (generated code uses `telex::hash::method_descriptor`)
// TODO: generated code should be updated to use telex::method_descriptor directly
pub mod hash {
    pub use telex_types::{
        method_descriptor, method_descriptor_with_retry, method_id_name_only,
        shape_contains_channel,
    };
}

// Re-export telex-types items used by generated code
pub use telex_types::{
    Backing,
    BoxMiddlewareFuture,
    // Traits
    Call,
    Caller,
    // Descriptors
    ChannelId,
    ChannelRetryMode,
    ClientCallOutcome,
    ClientContext,
    ClientMiddleware,
    ClientRequest,
    Conduit,
    ConduitAcceptor,
    ConduitRx,
    ConduitTx,
    ConduitTxPermit,
    // Types
    ConnectionId,
    ConnectionSettings,
    ErasedCaller,
    Extensions,
    Handler,
    HandshakeResult,
    Link,
    LinkRx,
    LinkTx,
    LinkTxPermit,
    MaybeSend,
    MaybeSync,
    MessageFamily,
    Metadata,
    MetadataEntry,
    MetadataFlags,
    MetadataValue,
    MethodDescriptor,
    MethodId,
    MiddlewareCaller,
    MsgFamily,
    OPERATION_ID_METADATA_KEY,
    Parity,
    Payload,
    RETRY_SUPPORT_METADATA_KEY,
    RETRY_SUPPORT_VERSION,
    ReplySink,
    RequestCall,
    RequestContext,
    RequestResponse,
    ResponseParts,
    RetryPolicy,
    TelexError,
    Rx,
    RxError,
    SchemaRecvTracker,
    SelfRef,
    ServerCallOutcome,
    ServerMiddleware,
    ServiceDescriptor,
    SessionRole,
    SinkCall,
    TransportMode,
    // Channels
    Tx,
    TxError,
    WithTracker,
    WriteSlot,
    // Channels
    channel,
    ensure_channel_retry_mode,
    observe_reply,
};

// Re-export runtime/session primitives from `telex-core`.
// This keeps user-facing setup to `telex` + a transport crate.
#[cfg(feature = "runtime")]
pub use telex_core::*;

// Channel binding via thread-local binder during deserialization
pub use telex_types::channel::with_channel_binder;

// Re-export the session module (generated code uses `telex::session::ServiceDescriptor`)
pub mod session {
    pub use telex_types::{MethodDescriptor, RetryPolicy, ServiceDescriptor};
}
