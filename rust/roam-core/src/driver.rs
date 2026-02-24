use roam_types::{Conduit, MessageFamily};

use crate::Session;

// purpose: drive a Session. For each conn, keep track of in-flight calls (request/response)
// and channels. deals with flow control
pub struct Driver<C: Conduit> {
    session: Session<C>,
}

impl<C> Driver<C>
where
    C: Conduit<Msg = MessageFamily>,
{
    async fn work(self) {
        loop {
            let ev = self.session.recv_event().await;
            match ev {}
        }
    }
}
