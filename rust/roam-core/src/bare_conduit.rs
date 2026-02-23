use std::marker::PhantomData;
use std::sync::Arc;

use facet::Facet;
use facet_reflect::TypePlanCore;

use roam_types::{
    Conduit, ConduitRx, ConduitTx, ConduitTxPermit, Link, LinkTx, LinkTxPermit, RpcPlan, SelfRef,
    WriteSlot,
};

/// Wraps a [`Link`] with postcard serialization. No reconnect, no reliability.
///
/// If the link dies, the conduit is dead. For localhost, SHM, or any
/// transport where reconnect isn't needed.
pub struct BareConduit<T: 'static, L: Link> {
    link: L,
    plan: Arc<TypePlanCore>,
    _phantom: PhantomData<fn() -> T>,
}

impl<T: Facet<'static> + 'static, L: Link> BareConduit<T, L> {
    /// Panics if `plan` was not built for type `T`.
    pub fn new(link: L, plan: &'static RpcPlan) -> Self {
        assert!(
            plan.shape == T::SHAPE,
            "RpcPlan shape mismatch: plan is for {}, expected {}",
            plan.shape,
            T::SHAPE,
        );
        Self {
            link,
            plan: Arc::clone(&plan.type_plan),
            _phantom: PhantomData,
        }
    }
}

impl<T: Facet<'static> + 'static, L: Link> Conduit<T> for BareConduit<T, L>
where
    L::Tx: Send + 'static,
    L::Rx: Send + 'static,
{
    type Tx = BareConduitTx<T, L::Tx>;
    type Rx = BareConduitRx<T, L::Rx>;

    fn split(self) -> (Self::Tx, Self::Rx) {
        let (tx, rx) = self.link.split();
        (
            BareConduitTx {
                link_tx: tx,
                _phantom: PhantomData,
            },
            BareConduitRx {
                link_rx: rx,
                plan: self.plan,
                _phantom: PhantomData,
            },
        )
    }
}

// ---------------------------------------------------------------------------
// Tx
// ---------------------------------------------------------------------------

pub struct BareConduitTx<T: 'static, LTx: LinkTx> {
    link_tx: LTx,
    _phantom: PhantomData<fn() -> T>,
}

impl<T: Facet<'static> + 'static, LTx: LinkTx + Send + 'static> ConduitTx<T>
    for BareConduitTx<T, LTx>
{
    type Permit<'a>
        = BareConduitPermit<'a, T, LTx>
    where
        Self: 'a;

    async fn reserve(&self) -> std::io::Result<Self::Permit<'_>> {
        let permit = self.link_tx.reserve().await?;
        Ok(BareConduitPermit {
            permit,
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

pub struct BareConduitPermit<'a, T: 'static, LTx: LinkTx> {
    permit: LTx::Permit,
    _phantom: PhantomData<fn() -> (&'a (), T)>,
}

impl<T: Facet<'static> + 'static, LTx: LinkTx> ConduitTxPermit<T>
    for BareConduitPermit<'_, T, LTx>
{
    type Error = BareConduitError;

    fn send(self, item: &T) -> Result<(), Self::Error> {
        let encoded = facet_postcard::to_vec(item).map_err(BareConduitError::Encode)?;

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
    plan: Arc<TypePlanCore>,
    _phantom: PhantomData<fn() -> T>,
}

impl<T: Facet<'static> + 'static, LRx> ConduitRx<T> for BareConduitRx<T, LRx>
where
    LRx: roam_types::LinkRx + Send + 'static,
{
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

        let plan = Arc::clone(&self.plan);
        SelfRef::try_new(backing, |bytes| deserialize_with_plan::<T>(&plan, bytes)).map(Some)
    }
}

/// Deserialize bytes into T using a precomputed TypePlanCore.
///
/// Uses `Partial::from_raw` + `FormatDeserializer::deserialize_into` for
/// fast plan-driven deserialization (no plan rebuild per call).
fn deserialize_with_plan<T: 'static>(
    plan: &Arc<TypePlanCore>,
    bytes: &[u8],
) -> Result<T, BareConduitError> {
    use facet_format::{FormatDeserializer, MetaSource};
    use facet_postcard::PostcardParser;
    use facet_reflect::Partial;

    let mut value = std::mem::MaybeUninit::<T>::uninit();
    let ptr = facet_core::PtrUninit::new(value.as_mut_ptr().cast::<u8>());

    let root_id = plan.root_id();

    // SAFETY: ptr points to valid, aligned, properly-sized memory for T.
    // The plan was built from RpcPlan::for_type::<T, _, _>() so root_id matches T's shape.
    #[allow(unsafe_code)]
    let partial: Partial<'_, false> = unsafe { Partial::from_raw(ptr, Arc::clone(plan), root_id) }
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
