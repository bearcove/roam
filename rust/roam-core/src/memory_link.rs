use std::marker::PhantomData;

use roam_types::{Backing, Codec, Link, LinkRx, LinkTx, LinkTxPermit, SelfRef};
use tokio::sync::mpsc;

/// In-process [`Link`] backed by tokio mpsc channels.
///
/// Each direction is an unbounded channel carrying `T` values directly —
/// no serialization, no IO. Useful for testing `ReliableLink`, `Session`,
/// and anything above the transport layer without real networking.
///
/// The `C` (Codec) parameter is carried but unused — `MemoryLink` never
/// encodes or decodes. This lets it slot into any generic context that
/// requires `Link<T, C>`.
pub struct MemoryLink<T, C> {
    tx: mpsc::Sender<T>,
    rx: mpsc::Receiver<T>,
    _codec: PhantomData<C>,
}

/// Create a pair of connected [`MemoryLink`]s.
///
/// Returns `(a, b)` where sending on `a` delivers to `b` and vice versa.
pub fn memory_link_pair<T: Send + 'static, C: Codec>(
    buffer: usize,
) -> (MemoryLink<T, C>, MemoryLink<T, C>) {
    let (tx_a, rx_b) = mpsc::channel(buffer);
    let (tx_b, rx_a) = mpsc::channel(buffer);

    let a = MemoryLink {
        tx: tx_a,
        rx: rx_a,
        _codec: PhantomData,
    };
    let b = MemoryLink {
        tx: tx_b,
        rx: rx_b,
        _codec: PhantomData,
    };

    (a, b)
}

impl<T: Send + 'static, C: Codec> Link<T, C> for MemoryLink<T, C> {
    type Tx = MemoryLinkTx<T, C>;
    type Rx = MemoryLinkRx<T, C>;

    fn split(self) -> (Self::Tx, Self::Rx) {
        (
            MemoryLinkTx {
                tx: self.tx,
                _codec: PhantomData,
            },
            MemoryLinkRx {
                rx: self.rx,
                _codec: PhantomData,
            },
        )
    }
}

// ---------------------------------------------------------------------------
// Tx
// ---------------------------------------------------------------------------

/// Sending half of a [`MemoryLink`].
pub struct MemoryLinkTx<T, C> {
    tx: mpsc::Sender<T>,
    _codec: PhantomData<C>,
}

/// Permit for sending one item through a [`MemoryLinkTx`].
pub struct MemoryLinkPermit<'a, T, C> {
    permit: mpsc::Permit<'a, T>,
    _codec: PhantomData<C>,
}

impl<T: Send + 'static, C: Codec> LinkTx<T, C> for MemoryLinkTx<T, C> {
    type Permit<'a>
        = MemoryLinkPermit<'a, T, C>
    where
        Self: 'a;

    async fn reserve(&self) -> std::io::Result<Self::Permit<'_>> {
        let permit = self.tx.reserve().await.map_err(|_| {
            std::io::Error::new(std::io::ErrorKind::ConnectionReset, "receiver dropped")
        })?;
        Ok(MemoryLinkPermit {
            permit,
            _codec: PhantomData,
        })
    }

    async fn close(self) -> std::io::Result<()> {
        // Dropping the sender closes the channel.
        drop(self.tx);
        Ok(())
    }
}

impl<T: Send + 'static, C: Codec> LinkTxPermit<T, C> for MemoryLinkPermit<'_, T, C> {
    fn send(self, item: T) -> Result<(), C::EncodeError> {
        self.permit.send(item);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Rx
// ---------------------------------------------------------------------------

/// Receiving half of a [`MemoryLink`].
pub struct MemoryLinkRx<T, C> {
    rx: mpsc::Receiver<T>,
    _codec: PhantomData<C>,
}

/// MemoryLink never fails on recv — the only "error" is channel closed (returns None).
#[derive(Debug)]
pub struct MemoryLinkRxError;

impl std::fmt::Display for MemoryLinkRxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "memory link rx error (unreachable)")
    }
}

impl std::error::Error for MemoryLinkRxError {}

impl<T: Send + 'static, C: Codec> LinkRx<T, C> for MemoryLinkRx<T, C> {
    type Error = MemoryLinkRxError;

    async fn recv(&mut self) -> Result<Option<SelfRef<T>>, Self::Error> {
        match self.rx.recv().await {
            Some(value) => {
                // No backing storage needed — value is already in memory.
                // We use a zero-length boxed slice as backing.
                let backing = Backing::Boxed(Box::new([]));
                let sr = SelfRef::owning(backing, value);
                Ok(Some(sr))
            }
            None => Ok(None),
        }
    }
}
