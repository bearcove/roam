//! Schema types for cross-language code generation.
//!
//! These types are produced by the `#[roam::service]` macro's
//! `*_service_detail()` functions, carrying enough information for
//! code generators (TypeScript, Swift, etc.) to emit typed clients
//! and servers without access to Rust source.

use facet_core::Shape;

/// Full description of a service, including all methods.
pub struct ServiceDetail {
    /// Service name (e.g., "Calculator").
    pub name: String,
    /// All methods in this service.
    pub methods: Vec<MethodDetail>,
    /// Documentation string, if any.
    pub doc: Option<String>,
}

/// Description of a single RPC method.
pub struct MethodDetail {
    /// Service name (e.g., "Calculator").
    pub service_name: String,
    /// Method name (e.g., "add").
    pub method_name: String,
    /// Arguments in declaration order.
    pub args: Vec<ArgDetail>,
    /// Return type shape.
    pub return_type: &'static Shape,
    /// Documentation string, if any.
    pub doc: Option<String>,
}

/// Description of a single method argument.
pub struct ArgDetail {
    /// Argument name (e.g., "user_id").
    pub name: String,
    /// Argument type shape.
    pub ty: &'static Shape,
}
