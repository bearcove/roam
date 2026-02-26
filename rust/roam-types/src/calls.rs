use crate::{Metadata, RequestCall, RequestResponse, RoamError, SelfRef, TxError};

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
//   async fn hash(&self, payload: &[u8]) -> Result<SelfRef<&[u8]>, RoamError<E>>;
// }
//
// Would expand to a handler trait (what users implement):
//
// trait HashServer {
//   async fn hash(&self, call: impl Call<&[u8], E>, payload: &[u8]);
// }
//
// And a HashDispatcher<S: HashServer> that implements Handler<R: ReplySink>:
// it deserializes args, constructs an ErasedCall<T, E> from the ReplySink,
// and routes to the appropriate HashServer method by method ID.
//
// HashDispatcher<S> implements Handler<R>, and can be stored as
// Box<dyn Handler<R>> to erase both S and the service type.
//
// Why impl Call in HashServer? So that the server can reply with something
// _borrowed_ from its own stack frame.
//
// For example:
//
// impl HashServer for MyHasher {
//   async fn hash(&self, call: impl Call<&[u8], E>, payload: &[u8]) {
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
pub trait Call<T, E>: Send {
    /// Send a [`Result`] back to the caller, consuming this `Call`.
    fn reply(self, result: Result<T, E>) -> impl std::future::Future<Output = ()> + Send;

    /// Send a successful response back to the caller, consuming this `Call`.
    ///
    /// Equivalent to `self.reply(Ok(value)).await`.
    fn ok(self, value: T) -> impl std::future::Future<Output = ()> + Send
    where
        Self: Sized,
        T: Send,
    {
        self.reply(Ok(value))
    }

    /// Send an error response back to the caller, consuming this `Call`.
    ///
    /// Equivalent to `self.reply(Err(error)).await`.
    fn err(self, error: E) -> impl std::future::Future<Output = ()> + Send
    where
        Self: Sized,
        E: Send,
    {
        self.reply(Err(error))
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
    /// Send the response, consuming the sink. Any error that happens during send_reply
    /// must set a flag in the driver for it to reply with an error.
    ///
    /// This cannot return a Result because we cannot trust callers to deal with it, and
    /// it's not like they can try sending a second reply anyway.
    ///
    /// Do not spawn a task to send the error because it too, might fail.
    fn send_reply(
        self,
        response: RequestResponse<'_>,
    ) -> impl std::future::Future<Output = ()> + Send;

    /// Send an error response back to the caller, consuming the sink.
    ///
    /// This is a convenience method used by generated dispatchers when
    /// deserialization fails or the method ID is unknown.
    fn send_error<E: for<'a> facet::Facet<'a> + Send>(
        self,
        error: RoamError<E>,
    ) -> impl std::future::Future<Output = ()> + Send
    where
        Self: Sized,
    {
        use crate::{Payload, RequestResponse};
        // Wire format is always Result<T, RoamError<E>>. We don't know T here,
        // but postcard encodes () as zero bytes, so Result<(), RoamError<E>>
        // produces the same Err variant encoding as any Result<T, RoamError<E>>.
        async move {
            let wire: Result<(), RoamError<E>> = Err(error);
            self.send_reply(RequestResponse {
                ret: Payload::outgoing(&wire),
                channels: vec![],
                metadata: Default::default(),
            })
            .await;
        }
    }
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
    fn handle(
        &self,
        call: SelfRef<crate::RequestCall<'static>>,
        reply: R,
    ) -> impl std::future::Future<Output = ()> + Send + '_;
}

/// A decoded response value paired with its response metadata.
///
/// Returned by generated client methods. `Deref`s to `T` so existing
/// field/method access works without changes; opt into metadata via
/// `.metadata`.
pub struct ResponseParts<'a, T> {
    /// The decoded return value.
    pub ret: T,
    /// Metadata attached to the response by the server.
    pub metadata: Metadata<'a>,
}

impl<'a, T> std::ops::Deref for ResponseParts<'a, T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.ret
    }
}

/// Concrete [`Call`] implementation backed by a [`ReplySink`].
///
/// Constructed by the dispatcher and handed to the server method.
/// When the server calls [`Call::reply`], the result is serialized and
/// sent through the sink.
pub struct SinkCall<R: ReplySink> {
    reply: R,
}

impl<R: ReplySink> SinkCall<R> {
    pub fn new(reply: R) -> Self {
        Self { reply }
    }
}

impl<T, E, R> Call<T, E> for SinkCall<R>
where
    T: for<'a> facet::Facet<'a> + Send,
    E: for<'a> facet::Facet<'a> + Send,
    R: ReplySink,
{
    async fn reply(self, result: Result<T, E>) {
        use crate::{Payload, RequestResponse};
        let wire: Result<T, crate::RoamError<E>> = result.map_err(crate::RoamError::User);
        let ret = Payload::outgoing(&wire);
        self.reply
            .send_reply(RequestResponse {
                ret,
                channels: vec![],
                metadata: Default::default(),
            })
            .await;
    }
}
