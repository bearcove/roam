//! WebSocket transport for telex.
//!
//! Implements [`Link`](telex_types::Link) over a WebSocket connection.
//! Each telex message maps 1:1 to a WebSocket binary frame.
//!
//! - **Native**: uses `tokio-tungstenite`
//! - **WASM**: uses `web_sys::WebSocket`

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::*;

#[cfg(target_arch = "wasm32")]
mod wasm;
#[cfg(target_arch = "wasm32")]
pub use wasm::*;
