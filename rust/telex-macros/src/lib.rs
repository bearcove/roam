//! Procedural macros for telex RPC service definitions.
//!
//! The `#[service]` macro generates everything needed for a telex RPC service.
//! All generation logic lives in `telex-macros-core` for testability.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;

// r[service-macro.is-source-of-truth]
/// Marks a trait as a telex RPC service and generates all service code.
///
/// # Generated Items
///
/// For a trait named `Calculator`, this generates:
/// - `mod calculator` containing:
///   - `pub use` of common types (Tx, Rx, TelexError, etc.)
///   - `mod method_id` with lazy method ID functions
///   - `trait Calculator` - the service trait
///   - `struct CalculatorDispatcher<H>` - server-side dispatcher
///   - `struct CalculatorClient` - client for making calls
///
/// # Example
///
/// ```ignore
/// #[telex::service]
/// trait Calculator {
///     async fn add(&self, a: i32, b: i32) -> i32;
/// }
/// ```
#[proc_macro_attribute]
pub fn service(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = TokenStream2::from(item);

    let parsed = match telex_macros_core::parse(&input) {
        Ok(p) => p,
        Err(e) => return e.to_compile_error().into(),
    };

    match telex_macros_core::generate_service(&parsed, &telex_macros_core::telex_crate()) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}
