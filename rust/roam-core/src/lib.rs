#![deny(unsafe_code)]

//! Session/state machine and RPC-level utilities.
//!
//! Canonical definitions live in `docs/content/spec/_index.md`,
//! `docs/content/rust-spec/_index.md`, and `docs/content/shm-spec/_index.md`.

use roam_types::RpcPlan;

#[macro_use]
mod macros;

mod driver;
pub mod runtime;
mod transport;

pub use driver::{
    ConnectError, ConnectionError, Driver, FramedClient, HandshakeConfig, IncomingConnection,
    IncomingConnections, MessageConnector, Negotiated, NoDispatcher, RetryPolicy, accept_framed,
    connect_framed, connect_framed_with_policy, initiate_framed,
};
pub use transport::MessageTransport;

mod connection_handle;
pub use connection_handle::ConnectionHandle;

mod caller;
pub use caller::{CallFuture, Caller, SendPtr};

mod errors;
pub use errors::{
    BorrowedCallResult, CallError, CallResult, ChannelError, ClientError, DecodeError,
    DispatchError, RoamError, TransportError,
};

mod types;
pub use types::{
    ChannelIdAllocator, ChannelRegistry, DriverMessage, IncomingChannelMessage, ResponseData, Role,
};

mod channel;
pub use channel::{Rx, RxError, Tx, TxError, channel};

mod tunnel;
pub use tunnel::{
    DEFAULT_TUNNEL_CHUNK_SIZE, Tunnel, pump_read_to_tx, pump_rx_to_write, tunnel_pair,
    tunnel_stream,
};

mod flow_control;
pub use flow_control::{FlowControl, InfiniteCredit};

mod dispatch;
pub use dispatch::{
    CURRENT_EXTENSIONS, Context, DISPATCH_CONTEXT, DispatchContext, PrepareError, RoutedDispatcher,
    ServiceDispatcher, dispatch_unknown_method, prepare_sync, run_post_middleware,
    run_pre_middleware, send_error_response, send_ok_response, send_prepare_error,
};
// Re-export internal items needed by channel binding
pub(crate) use dispatch::get_dispatch_context;

mod forwarding;
pub use forwarding::{ForwardingDispatcher, LateBoundForwarder, LateBoundHandle};

mod extensions;
pub use extensions::Extensions;

mod middleware;
pub use middleware::{MethodOutcome, Middleware, Rejection, RejectionCode, SendPeek};

pub(crate) const CHANNEL_SIZE: usize = 1024;
pub(crate) const RX_STREAM_BUFFER_SIZE: usize = 1024;

/// Build an [`RpcPlan`] for type `T`, defaulting the `Tx` and `Rx` channel
/// type parameters to `Tx::<()>` / `Rx::<()>`.
pub fn rpc_plan<T: facet::Facet<'static>>() -> &'static RpcPlan {
    Box::leak(Box::new(RpcPlan::for_type::<T, Tx<()>, Rx<()>>()))
}

#[cfg(test)]
mod tests;
