//! Roam — Rust-native RPC where traits are the schema.
//!
//! This is the facade crate. It re-exports everything needed by both
//! hand-written code and `#[roam::service]` macro-generated code.

// Re-export the proc macro
pub use roam_service_macros::service;

// Re-export facet (generated code uses `roam::facet::Facet`)
pub use facet;

// Re-export facet-postcard (generated code uses `roam::facet_postcard::from_slice_borrowed`)
pub use facet_postcard;

// Re-export roam-hash (generated code uses `roam::hash::method_descriptor`)
pub use roam_hash as hash;

// Re-export roam-types items used by generated code
pub use roam_types::{
    // Traits
    Call,
    Caller,
    Handler,
    MethodDescriptor,
    // Descriptors
    MethodId,
    // Types
    Payload,
    ReplySink,
    RequestCall,
    RequestResponse,
    ResponseParts,
    RoamError,
    RpcPlan,
    Rx,
    SelfRef,
    ServiceDescriptor,
    SinkCall,
    // Channels
    Tx,
    // Channel binding (used by generated dispatcher/client code)
    bind_channels_client,
    bind_channels_server,
    channel,
};

// Re-export the session module (generated code uses `roam::session::ServiceDescriptor`)
pub mod session {
    pub use roam_types::{MethodDescriptor, ServiceDescriptor};
}
