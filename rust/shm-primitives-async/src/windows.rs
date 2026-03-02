pub mod doorbell;

pub use doorbell::{
    Doorbell, DoorbellHandle, SignalResult, close_handle, set_handle_inheritable, validate_handle,
};

/// 16-byte metadata sent alongside each mmap region attachment.
///
/// Mirrors the Unix `MmapAttachMessage` so that in-process channels and
/// data structures compile on Windows without any Unix-specific socket code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(C)]
pub struct MmapAttachMessage {
    pub map_id: u32,
    pub map_generation: u32,
    pub mapping_length: u64,
}
