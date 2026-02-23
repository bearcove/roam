pub mod bipbuf;
pub mod region;
pub mod slot;
pub mod sync;

pub use bipbuf::{
    BIPBUF_HEADER_SIZE, BipBuf, BipBufConsumer, BipBufFull, BipBufHeader, BipBufProducer, BipBufRaw,
};
pub use region::HeapRegion;
pub use region::Region;
pub use slot::{SlotState, VarSlotMeta};

// OS-level primitives for SHM
#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::*;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::*;
