use crate::RoamError;

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

pub trait CallHandle {}

pub trait Handler {
    /// Handle an incoming call, reply with a
    fn handle(&self, reply: Reply) -> Result<(), RoamError<E>>;
}
