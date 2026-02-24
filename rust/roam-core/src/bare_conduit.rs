use std::marker::PhantomData;

use facet::Facet;
use facet_core::{PtrConst, Shape};
use facet_reflect::Peek;

use roam_types::{
    Conduit, ConduitRx, ConduitTx, ConduitTxPermit, Link, LinkTx, LinkTxPermit, RpcPlan, SelfRef,
    WriteSlot,
};

/// Wraps a [`Link`] with postcard serialization. No reconnect, no reliability.
///
/// If the link dies, the conduit is dead. For localhost, SHM, or any
/// transport where reconnect isn't needed.
///
/// `T` is the message type family. The send path accepts `T` with any
/// lifetime (borrowed data serialized in place via `Peek`). The receive
/// path yields `SelfRef<T<'static>>` (owned).
// r[impl conduit.bare]
// r[impl conduit.typeplan]
pub struct BareConduit<T: 'static, L: Link> {
    link: L,
    shape: &'static Shape,
    _phantom: PhantomData<fn(T) -> T>,
}

impl<T: Facet<'static> + 'static, L: Link> BareConduit<T, L> {
    /// Create a new BareConduit.
    ///
    /// Panics if the plan's shape doesn't match `T::SHAPE`.
    pub fn new(link: L, plan: &'static RpcPlan) -> Self {
        assert!(
            plan.shape == T::SHAPE,
            "RpcPlan shape mismatch: plan is for {}, expected {}",
            plan.shape,
            T::SHAPE,
        );
        Self {
            link,
            shape: T::SHAPE,
            _phantom: PhantomData,
        }
    }
}

impl<T: Facet<'static> + 'static, L: Link> Conduit for BareConduit<T, L>
where
    L::Tx: Send + 'static,
    L::Rx: Send + 'static,
{
    type Msg<'a> = T;
    type Tx = BareConduitTx<T, L::Tx>;
    type Rx = BareConduitRx<T, L::Rx>;

    fn split(self) -> (Self::Tx, Self::Rx) {
        let (tx, rx) = self.link.split();
        (
            BareConduitTx {
                link_tx: tx,
                shape: self.shape,
                _phantom: PhantomData,
            },
            BareConduitRx {
                link_rx: rx,
                recv_shape: self.shape,
                _phantom: PhantomData,
            },
        )
    }
}

// ---------------------------------------------------------------------------
// Tx
// ---------------------------------------------------------------------------

pub struct BareConduitTx<T, LTx: LinkTx> {
    link_tx: LTx,
    shape: &'static Shape,
    _phantom: PhantomData<fn(T)>,
}

impl<T, LTx: LinkTx + Send + 'static> ConduitTx for BareConduitTx<T, LTx> {
    type Msg<'a> = T;
    type Permit<'a>
        = BareConduitPermit<'a, T, LTx>
    where
        Self: 'a;

    async fn reserve(&self) -> std::io::Result<Self::Permit<'_>> {
        let permit = self.link_tx.reserve().await?;
        Ok(BareConduitPermit {
            permit,
            shape: self.shape,
            _phantom: PhantomData,
        })
    }

    async fn close(self) -> std::io::Result<()> {
        self.link_tx.close().await
    }
}

// ---------------------------------------------------------------------------
// Permit
// ---------------------------------------------------------------------------

pub struct BareConduitPermit<'a, T, LTx: LinkTx> {
    permit: LTx::Permit,
    shape: &'static Shape,
    _phantom: PhantomData<fn(T, &'a ())>,
}

impl<T, LTx: LinkTx> ConduitTxPermit for BareConduitPermit<'_, T, LTx> {
    type Msg<'a> = T;
    type Error = BareConduitError;

    fn send(self, item: T) -> Result<(), Self::Error> {
        // SAFETY: send_shape was set from T::SHAPE at construction time.
        // The item is a valid instance of T, so (ptr, shape) is consistent.
        #[allow(unsafe_code)]
        let peek = unsafe {
            Peek::unchecked_new(PtrConst::new((&raw const item).cast::<u8>()), self.shape)
        };
        let encoded = facet_postcard::peek_to_vec(peek).map_err(BareConduitError::Encode)?;

        let mut slot = self
            .permit
            .alloc(encoded.len())
            .map_err(BareConduitError::Io)?;

        slot.as_mut_slice().copy_from_slice(&encoded);
        slot.commit();
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Rx
// ---------------------------------------------------------------------------

pub struct BareConduitRx<T: 'static, LRx> {
    link_rx: LRx,
    recv_shape: &'static Shape,
    _phantom: PhantomData<fn() -> T>,
}

impl<T: Facet<'static> + 'static, LRx> ConduitRx for BareConduitRx<T, LRx>
where
    LRx: roam_types::LinkRx + Send + 'static,
{
    type Msg<'a> = T;
    type Error = BareConduitError;

    async fn recv(&mut self) -> Result<Option<SelfRef<T>>, Self::Error> {
        let backing = match self
            .link_rx
            .recv()
            .await
            .map_err(|_| BareConduitError::LinkDead)?
        {
            Some(b) => b,
            None => return Ok(None),
        };

        let shape = self.recv_shape;
        SelfRef::try_new(backing, |bytes| deserialize_with_shape::<T>(shape, bytes)).map(Some)
    }
}

/// Deserialize bytes into T using a shape (plan is cached transparently).
///
/// Uses `Partial::from_raw_with_shape` + `FormatDeserializer::deserialize_into`
/// for stack-allocated, plan-cached deserialization.
fn deserialize_with_shape<T: 'static>(
    shape: &'static Shape,
    bytes: &[u8],
) -> Result<T, BareConduitError> {
    use facet_format::{FormatDeserializer, MetaSource};
    use facet_postcard::PostcardParser;
    use facet_reflect::Partial;

    let mut value = std::mem::MaybeUninit::<T>::uninit();
    let ptr = facet_core::PtrUninit::new(value.as_mut_ptr().cast::<u8>());

    // SAFETY: ptr points to valid, aligned, properly-sized memory for T.
    // shape comes from T::SHAPE, set at BareConduit construction time.
    #[allow(unsafe_code)]
    let partial: Partial<'_, false> = unsafe { Partial::from_raw_with_shape(ptr, shape) }
        .map_err(|e| BareConduitError::Decode(e.into()))?;

    let mut parser = PostcardParser::new(bytes);
    let mut deserializer = FormatDeserializer::new_owned(&mut parser);
    let partial = deserializer
        .deserialize_into(partial, MetaSource::FromEvents)
        .map_err(BareConduitError::Decode)?;

    partial
        .finish_in_place()
        .map_err(|e| BareConduitError::Decode(e.into()))?;

    // SAFETY: finish_in_place succeeded, so value is fully initialized.
    #[allow(unsafe_code)]
    Ok(unsafe { value.assume_init() })
}

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

#[derive(Debug)]
pub enum BareConduitError {
    Encode(facet_postcard::SerializeError),
    Decode(facet_format::DeserializeError),
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
