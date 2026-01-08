//! Procedural macros for roam RPC service definitions.
//!
//! # Architecture: Why This Macro Is Minimal
//!
//! This macro serves as a **metadata emitter** — it parses your trait definition and
//! produces a `ServiceDetail` that can be consumed by [`roam-codegen`] in a build script.
//!
//! ## The Two-Phase Design
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────┐
//! │  PHASE 1: Compile Time (this crate)                                     │
//! │  ─────────────────────────────────────────────────────────────────────  │
//! │  #[service] macro runs as a proc macro                                  │
//! │  → Parses trait definition using unsynn grammar                         │
//! │  → Emits ONE function: service_detail() -> ServiceDetail                │
//! └─────────────────────────────────────────────────────────────────────────┘
//!                                    │
//!                                    ▼
//! ┌─────────────────────────────────────────────────────────────────────────┐
//! │  PHASE 2: Build Script (roam-codegen)                                   │
//! │  ─────────────────────────────────────────────────────────────────────  │
//! │  build.rs calls service_detail() to get ServiceDetail                   │
//! │  → Uses facet::Shape for full type introspection                        │
//! │  → Generates Handler trait, Dispatcher, Client for Rust                 │
//! │  → Generates bindings for TypeScript, Go, Swift, etc.                   │
//! └─────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Why Not Generate More?
//!
//! The original trait is NOT emitted because nobody implements it directly:
//! - The generated `Handler` trait has different signatures (errors wrapped in `RoamError`)
//! - `Tx<T>`/`Rx<T>` are flipped between caller and callee perspectives
//! - All dispatch, client, and handler code comes from `roam-codegen`
//!
//! ## The Fundamental Limitation: Proc Macros Can't See Types
//!
//! Proc macros operate on **tokens**, not types. When you write:
//!
//! ```ignore
//! async fn send(&self, data: Tx<String>) -> Result<(), Error>;
//! ```
//!
//! The proc macro sees tokens `Tx`, `<`, `String`, `>` — but cannot determine:
//! - Is `Tx` the roam streaming type, or a user-defined type?
//! - What traits does `Error` implement?
//!
//! **But `facet::Shape` can.** At runtime (in a build script), `facet` provides
//! complete type introspection, which is why all the interesting work happens
//! in `roam-codegen`.
//!
//! [`roam-codegen`]: https://docs.rs/roam-codegen

#![deny(unsafe_code)]

use heck::ToSnakeCase;
use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};

mod crate_name;
mod parser;

use crate_name::FoundCrate;
use parser::{ServiceTrait, ToTokens, Type};

/// Returns the token stream for accessing the `roam` crate.
fn roam_crate() -> TokenStream2 {
    match crate_name::crate_name("roam") {
        Ok(FoundCrate::Itself) => quote! { crate },
        Ok(FoundCrate::Name(name)) => {
            let ident = format_ident!("{}", name);
            quote! { ::#ident }
        }
        Err(_) => quote! { ::roam },
    }
}

/// Marks a trait as a roam RPC service and generates a `ServiceDetail` function.
///
/// # Generated Items
///
/// For a trait named `Calculator`, this generates:
/// - `calculator_service_detail()` - Returns `ServiceDetail` for codegen
///
/// That's it. All other code (Handler trait, Dispatcher, Client) comes from `roam-codegen`.
#[proc_macro_attribute]
pub fn service(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = TokenStream2::from(item);

    let parsed = match parser::parse(&input) {
        Ok(p) => p,
        Err(e) => return e.to_compile_error().into(),
    };

    match generate_service(&parsed) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn generate_service(parsed: &ServiceTrait) -> Result<TokenStream2, parser::Error> {
    // rs[impl wire.stream.not-in-errors] - Compile-time validation that streams don't appear in error types
    for method in parsed.methods() {
        let return_type = method.return_type();
        if let Some((_, err_ty)) = return_type.as_result()
            && err_ty.contains_stream()
        {
            return Err(parser::Error::new(
                proc_macro2::Span::call_site(),
                format!(
                    "method `{}` has Stream (Tx/Rx) in error type - streams are not allowed in error types (see r[streaming.error-no-streams])",
                    method.name()
                ),
            ));
        }
    }

    let trait_name = parsed.name();
    let trait_snake = trait_name.to_snake_case();
    let roam = roam_crate();

    let service_detail_fn_name = format_ident!("{}_service_detail", trait_snake);
    let method_details = generate_method_details(parsed, &roam);

    let service_doc = parsed
        .doc()
        .map(|d| quote! { Some(#d.into()) })
        .unwrap_or_else(|| quote! { None });

    Ok(quote! {
        /// Returns the service detail for codegen.
        pub fn #service_detail_fn_name() -> #roam::schema::ServiceDetail {
            #roam::schema::ServiceDetail {
                name: #trait_name.into(),
                methods: vec![#(#method_details),*],
                doc: #service_doc,
            }
        }
    })
}

fn generate_method_details(parsed: &ServiceTrait, roam: &TokenStream2) -> Vec<TokenStream2> {
    let service_name = parsed.name();

    parsed
        .methods()
        .map(|m| {
            let method_name = m.name();
            let method_doc = m
                .doc()
                .map(|d| quote! { Some(#d.into()) })
                .unwrap_or_else(|| quote! { None });

            let arg_exprs: Vec<TokenStream2> = m
                .args()
                .map(|arg| {
                    let arg_name = arg.name();
                    let shape = shape_expr(&arg.ty, roam);
                    quote! {
                        #roam::schema::ArgDetail {
                            name: #arg_name.into(),
                            ty: #shape,
                        }
                    }
                })
                .collect();

            let return_type = m.return_type();
            let return_type_shape = shape_expr(&return_type, roam);

            quote! {
                #roam::schema::MethodDetail {
                    service_name: #service_name.into(),
                    method_name: #method_name.into(),
                    args: vec![#(#arg_exprs),*],
                    return_type: #return_type_shape,
                    doc: #method_doc,
                }
            }
        })
        .collect()
}

fn shape_expr(ty: &Type, roam: &TokenStream2) -> TokenStream2 {
    let ty_tokens = ty.to_token_stream();
    quote! {
        <#ty_tokens as #roam::facet::Facet>::SHAPE
    }
}
