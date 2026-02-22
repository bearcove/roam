use roam_types::connectivity2::{
    Conduit, ConduitRx, ConduitTx, ConduitTxPermit, Link, LinkTx, WriteSlot,
};
use roam_types::{Backing, SelfRef};

/// Wraps a [`Link`] with postcard serialization. No reconnect, no reliability.
///
/// If the link dies, the conduit is dead. For localhost, SHM, or any
/// transport where reconnect isn't needed.
pub struct BareConduit<L: Link> {
    link: L,
}

impl<L: Link> BareConduit<L> {
    pub fn new(link: L) -> Self {
        Self { link }
    }
}

impl<L: Link> Conduit for BareConduit<L>
where
    L::Tx: Send + 'static,
    L::Rx: Send + 'static,
{
    type Tx = BareConduitTx<L::Tx>;
    type Rx = BareConduitRx<L::Rx>;

    fn split(self) -> (Self::Tx, Self::Rx) {
        let (tx, rx) = self.link.split();
        (BareConduitTx { link_tx: tx }, BareConduitRx { link_rx: rx })
    }
}

// ---------------------------------------------------------------------------
// Tx
// ---------------------------------------------------------------------------

pub struct BareConduitTx<LTx: LinkTx> {
    link_tx: LTx,
}

impl<LTx: LinkTx + Send + 'static> ConduitTx for BareConduitTx<LTx> {
    type Permit<'a>
        = BareConduitPermit<'a, LTx>
    where
        Self: 'a;

    async fn reserve(&self) -> std::io::Result<Self::Permit<'_>> {
        Ok(BareConduitPermit { tx: self })
    }

    async fn close(self) -> std::io::Result<()> {
        self.link_tx.close().await
    }
}

// ---------------------------------------------------------------------------
// Permit
// ---------------------------------------------------------------------------

pub struct BareConduitPermit<'a, LTx: LinkTx> {
    tx: &'a BareConduitTx<LTx>,
}

impl<LTx: LinkTx> ConduitTxPermit for BareConduitPermit<'_, LTx> {
    type Error = BareConduitError;

    async fn send<'a, T: facet::Facet<'a>>(self, item: &'a T) -> Result<(), Self::Error> {
        // 1. Encode to vec (postcard determines the size)
        let encoded = facet_postcard::to_vec(item).map_err(BareConduitError::Encode)?;

        // 2. Allocate from link
        let mut slot = self
            .tx
            .link_tx
            .alloc(encoded.len())
            .await
            .map_err(BareConduitError::Io)?;

        // 3. Copy into slot and commit
        slot.as_mut_slice().copy_from_slice(&encoded);
        slot.commit();
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Rx
// ---------------------------------------------------------------------------

pub struct BareConduitRx<LRx> {
    link_rx: LRx,
}

impl<LRx> ConduitRx for BareConduitRx<LRx>
where
    LRx: roam_types::connectivity2::LinkRx + Send + 'static,
{
    type Error = BareConduitError;

    async fn recv<T: facet::Facet<'static> + 'static>(
        &mut self,
    ) -> Result<Option<SelfRef<T>>, Self::Error> {
        let backing = match self
            .link_rx
            .recv()
            .await
            .map_err(|_| BareConduitError::LinkDead)?
        {
            Some(b) => b,
            None => return Ok(None),
        };

        SelfRef::try_new(backing, |bytes| {
            facet_postcard::from_slice(bytes).map_err(BareConduitError::Decode)
        })
        .map(Some)
    }
}

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum BareConduitError {
    Encode(facet_postcard::SerializeError),
    Decode(facet_postcard::DeserializeError),
    Io(std::io::Error),
    LinkDead,
}

impl std::fmt::Display for BareConduitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Encode(e) => write!(f, "encode error: {e}"),
            Self::Decode(e) => write!(f, "decode error: {e}"),
            Self::Io(e) => write!(f, "io error: {e}"),
            Self::LinkDead => write!(f, "link dead"),
        }
    }
}

impl std::error::Error for BareConduitError {}
