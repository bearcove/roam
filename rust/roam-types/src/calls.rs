use crate::{RequestCall, RequestResponse, RoamError, SelfRef, TxError};

// As a recap, a service defined like so:
//
// #[roam::service]
// trait Hash {
//   async fn hash(&self, payload: &[u8]) -> Result<&[u8], E>;
// }
//
// Would expand to the following caller:
//
// impl HashClient {
//   async fn hash(&self, payload: &[u8]) -> Result<&[u8], RoamError<E>>;
// }
//
// Would expand to the following handler:
//
// trait HashServer {
//   async fn hash(&self, call: Call<Result<&[u8], E>>, payload: &[u8]);
// }
//
// Why? So that the HashServer can reply with something _borrowed_.
//
// For example, a HashServer implementation could compute the hash result
// into a stack-allocated buffer and reply with a borrow of it:
//
// impl HashServer for MyHasher {
//   async fn hash(&self, call: Call<Result<&[u8], E>>, payload: &[u8]) {
//     let result: [u8; 16] = compute_hash(payload);
//     call.ok(&result).await;
//   }
// }
//
// Call's public API is:
//
// trait Call<T, E> {
//   async fn reply(self, result: Result<T, E>);
//   async fn ok(self, value: T) { self.reply(Ok(value)).await }
//   async fn err(self, error: E) { self.reply(Err(error)).await }
// }
//
// If a Call is dropped before reply/ok/err is called, the caller will
// receive a RoamError::Cancelled error. This is to ensure that the caller
// is always notified, even if the handler panics or otherwise fails to
// reply.

/// Represents an in-progress call from a client that must be replied to.
///
/// A `Call` is handed to a [`Handler`] implementation and provides the
/// mechanism for sending a response back to the caller. The response can
/// be sent via [`Call::reply`], [`Call::ok`], or [`Call::err`].
///
/// # Cancellation
///
/// If a `Call` is dropped without a reply being sent, the caller will
/// automatically receive a [`RoamError::Cancelled`] error. This guarantees
/// that the caller is always notified, even if the handler panics or
/// otherwise fails to produce a reply.
///
/// # Type Parameters
///
/// - `T`: The success value type of the response.
/// - `E`: The error value type of the response.
pub trait Call<T, E> {
    /// Send a [`Result`] back to the caller, consuming this `Call`.
    #[allow(async_fn_in_trait)]
    async fn reply(self, result: Result<T, E>);

    /// Send a successful response back to the caller, consuming this `Call`.
    ///
    /// Equivalent to `self.reply(Ok(value)).await`.
    #[allow(async_fn_in_trait)]
    async fn ok(self, value: T)
    where
        Self: Sized,
    {
        self.reply(Ok(value)).await
    }

    /// Send an error response back to the caller, consuming this `Call`.
    ///
    /// Equivalent to `self.reply(Err(error)).await`.
    #[allow(async_fn_in_trait)]
    async fn err(self, error: E)
    where
        Self: Sized,
    {
        self.reply(Err(error)).await
    }
}

/// Sink for sending a reply back to the caller.
///
/// Implemented by the session driver. Provides backpressure: `send_reply`
/// awaits until the transport can accept the response before serializing it.
///
/// # Cancellation
///
/// If the `ReplySink` is dropped without `send_reply` being called, the caller
/// will automatically receive a [`crate::RoamError::Cancelled`] error.
pub trait ReplySink: Send + Sync + 'static {
    /// Send the response, awaiting until the transport is ready.
    ///
    /// Successful return means the response has been serialized and committed
    /// to the link's write buffer.
    #[allow(async_fn_in_trait)]
    async fn send_reply<'a>(&self, response: RequestResponse<'a>) -> Result<(), TxError>;
}

/// Type-erased handler for incoming service calls.
///
/// Implemented (by the macro-generated dispatch code) for server-side types.
/// Takes a fully decoded [`RequestCall`](crate::RequestCall) — already parsed
/// from the wire — and a [`ReplySink`] through which the response is sent.
///
/// The dispatch impl decodes the args, routes by [`crate::MethodId`], and
/// invokes the appropriate typed [`Call`]-based method on the concrete server type.
/// A cloneable handle to a connection, handed out by the session driver.
///
/// Generated clients hold a `C: Caller` and use it to send calls. The caller
/// serializes the outgoing [`RequestCall`] (with borrowed args), registers a
/// pending response slot, and awaits the response from the peer.
pub trait Caller: Clone + Send + Sync + 'static {
    /// Send a call and wait for the response.
    #[allow(async_fn_in_trait)]
    async fn call<'a>(
        &self,
        call: RequestCall<'a>,
    ) -> Result<SelfRef<RequestResponse<'static>>, RoamError>;
}

pub trait Handler<R: ReplySink>: Send + Sync + 'static {
    /// Dispatch an incoming call to the appropriate method implementation.
    #[allow(async_fn_in_trait)]
    async fn handle(&self, call: SelfRef<crate::RequestCall<'static>>, reply: R);
}
