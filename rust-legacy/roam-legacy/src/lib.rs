//! roam - High-performance RPC framework
//!
//! This crate provides a unified API for the roam RPC protocol.
//! Users should depend on this crate rather than the individual component crates.

#![deny(unsafe_code)]

// Macro hygiene: Allow `::roam::` paths to work both externally and internally.
// When used in tests within this workspace, `::roam::` would normally
// fail because it would look for a `roam` module within `roam`. This
// self-referential module makes `::roam::session::...` etc. work everywhere.
#[doc(hidden)]
pub mod roam {
    pub use crate::*;
}

// Re-export the service macro
pub use roam_service_macros::service;

// Re-export session types for macro-generated code.
// This is a carefully-curated surface: most of the core implementation lives in `roam-core`,
// while the descriptor/schema types live in `roam-types`.
#[doc(hidden)]
pub mod session {
    pub use roam_core::{
        CURRENT_EXTENSIONS, CallError, CallFuture, Caller, ChannelError, ChannelIdAllocator,
        ChannelRegistry, ConnectionHandle, Context, DISPATCH_CONTEXT, DispatchContext,
        DriverMessage, IncomingChannelMessage, MethodOutcome, Middleware, PrepareError, Rejection,
        RejectionCode, ResponseData, Role, RoutedDispatcher, Rx, RxError, SendPeek, SendPtr,
        ServiceDispatcher, TransportError, Tx, TxError, dispatch_unknown_method, prepare_sync,
        rpc_plan, run_post_middleware, run_pre_middleware, send_error_response, send_ok_response,
        send_prepare_error,
    };

    pub use roam_types::{
        ArgDescriptor, ChannelId, ChannelKind, ConnectionId, Metadata, MethodDescriptor, MethodId,
        Payload, RequestId, RpcPlan, ServiceDescriptor,
    };
}

// Re-export Context at top level for convenience
pub use roam_core::Context;

// Re-export streaming types for user-facing API
pub use roam_core::{
    ChannelError, ChannelIdAllocator, ChannelRegistry, DriverMessage, Role, Rx, RxError, Tx,
    TxError, channel,
};

// Re-export a few wire-level IDs used in public signatures.
pub use roam_types::{ChannelId, ConnectionId, MethodId, RequestId};

// Re-export tunnel types for byte stream bridging
pub use roam_core::{Tunnel, tunnel_pair};

#[cfg(not(target_arch = "wasm32"))]
pub use roam_core::{DEFAULT_TUNNEL_CHUNK_SIZE, pump_read_to_tx, pump_rx_to_write, tunnel_stream};

// Re-export schema types
pub use roam_schema as schema;

// Re-export hash utilities
pub use roam_hash as hash;

// Re-export wire types for macro-generated code
pub use roam_types as types;

// Re-export facet for derive macros in service types
pub use facet;

// Re-export facet_core for macro-generated code that needs PtrConst
pub use facet_core;

// Re-export facet-pretty for macro-generated logging
pub use facet_pretty;
pub use facet_pretty::PrettyPrinter;

// Re-export tracing for macro-generated logging
pub use tracing;

/// Private module for proc-macro re-exports. Not part of the public API.
#[doc(hidden)]
pub mod __private {
    pub use facet_postcard;
}

/// Prelude module for convenient imports.
///
/// ```ignore
/// use roam::prelude::*;
/// ```
pub mod prelude {
    pub use crate::service;
    pub use facet::Facet;
}
