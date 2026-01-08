//! Procedural macros for roam RPC service definitions.
//!
//! # Architecture: Why This Macro Is Intentionally Thin
//!
//! **This macro does NOT generate clients, servers, or protocol implementations.**
//!
//! Instead, it serves as a **metadata emitter** — it parses your trait definition and
//! produces reflection information (service names, method names, argument types, return types)
//! that can be consumed later by [`roam-codegen`] running in a **build script**.
//!
//! ## The Two-Phase Design
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────────────────┐
//! │  PHASE 1: Compile Time (this crate)                                     │
//! │  ─────────────────────────────────────────────────────────────────────  │
//! │  #[service] macro runs as a proc macro                                  │
//! │  → Parses trait definition using unsynn grammar                         │
//! │  → Emits metadata functions: service_detail(), method_ids()             │
//! │  → Emits minimal dispatch/client stubs for Rust-to-Rust calls           │
//! └─────────────────────────────────────────────────────────────────────────┘
//!                                    │
//!                                    ▼
//! ┌─────────────────────────────────────────────────────────────────────────┐
//! │  PHASE 2: Build Script (roam-codegen)                                   │
//! │  ─────────────────────────────────────────────────────────────────────  │
//! │  build.rs calls service_detail() to get ServiceDetail                   │
//! │  → Uses facet::Shape for full type introspection                        │
//! │  → Generates code for ANY language: Rust, TypeScript, Go, Swift, etc.   │
//! │  → Handles serialization formats, protocol details, optimizations       │
//! └─────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Why This Separation?
//!
//! 1. **Multi-language support**: The same service definition produces bindings for
//!    Rust, TypeScript, Go, Java, Swift, Python — all from `roam-codegen`, not here.
//!
//! 2. **Build-time flexibility**: Code generation in build scripts can read config files,
//!    query the environment, and make decisions that proc macros cannot.
//!
//! 3. **Simpler proc macro**: Proc macros are notoriously hard to debug. By keeping this
//!    one minimal, we reduce complexity and improve compile times.
//!
//! 4. **Type introspection via facet**: The heavy lifting of type analysis happens through
//!    `facet::Shape`, which provides complete type information at runtime. We just emit
//!    the glue to access it.
//!
//! ## The Fundamental Limitation: Proc Macros Can't See Types
//!
//! This is the **technical reason** the separation exists, not just an architectural preference.
//!
//! Proc macros operate on **tokens**, not types. When you write:
//!
//! ```ignore
//! async fn send(&self, data: Tx<String>) -> Result<(), Error>;
//! ```
//!
//! The proc macro sees the tokens `Tx`, `<`, `String`, `>` — but it has **no idea**:
//!
//! - Is `Tx` the roam streaming type, or a user-defined type that happens to be named `Tx`?
//! - What's inside `Result`? What's inside `Vec<Option<Tx<T>>>`?
//! - What traits does `Error` implement? Is it `Send`? `Sync`?
//!
//! The macro cannot see into the type system. It cannot see what's in scope. It cannot
//! resolve type aliases or follow generic parameters.
//!
//! **But `facet::Shape` can.**
//!
//! At runtime (in a build script), `facet` provides complete type introspection:
//!
//! ```ignore
//! // facet can tell us this is roam::Tx, not some other Tx
//! let shape = <Tx<String> as facet::Facet>::SHAPE;
//!
//! // facet can see inside nested types
//! let shape = <Result<Vec<Tx<String>>, MyError> as facet::Facet>::SHAPE;
//! // → We can traverse this and find the Tx<String> buried inside
//! ```
//!
//! This is why validation of streaming types (`Tx`/`Rx`), error type constraints, and
//! other type-aware checks happen in `roam-codegen`, not here.
//!
//! ## What This Macro Actually Emits
//!
//! For a trait like:
//!
//! ```ignore
//! #[roam_service_macros::service]
//! trait Calculator {
//!     /// Add two numbers.
//!     async fn add(&self, a: i32, b: i32) -> i32;
//! }
//! ```
//!
//! The macro generates:
//!
//! - **The original trait** (unchanged)
//! - **`calculator_service_detail()`** — Returns `ServiceDetail` with method signatures
//!   and type information via `facet::Shape`
//! - **`calculator_method_ids()`** — Lazily-computed method ID hashes for dispatch
//! - **`calculator_dispatch_unary()`** — Minimal dispatch helper (Rust-to-Rust only)
//! - **`CalculatorClient<C>`** — Basic client stub (Rust-to-Rust only)
//!
//! The dispatch and client code is intentionally minimal — just enough for Rust-to-Rust
//! communication. For production use with other languages, use `roam-codegen`.
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
use parser::{MethodParam, ServiceTrait, ToTokens, Type, method_ok_and_err_types};

/// Returns the token stream for accessing the `roam` crate.
///
/// This handles the case where the user has renamed the crate in their Cargo.toml.
fn roam_crate() -> TokenStream2 {
    match crate_name::crate_name("roam") {
        Ok(FoundCrate::Itself) => quote! { crate },
        Ok(FoundCrate::Name(name)) => {
            let ident = format_ident!("{}", name);
            quote! { ::#ident }
        }
        Err(_) => {
            // Fallback to the canonical name
            quote! { ::roam }
        }
    }
}

/// Marks a trait as a roam RPC service and generates codegen helpers.
///
/// # Generated Items
///
/// For a trait named `Calculator`:
/// - The original trait definition (unchanged)
/// - `calculator_service_detail()` - Returns `ServiceDetail` for codegen
/// - `calculator_method_ids()` - Returns a lazily-computed set of method IDs
/// - `calculator_dispatch_unary()` - Decodes arguments, calls the service, encodes response payload
/// - `CalculatorClient<C>` - Client stub operating over a `UnaryCaller`
#[proc_macro_attribute]
pub fn service(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = TokenStream2::from(item);

    let parsed = match parser::parse(&input) {
        Ok(p) => p,
        Err(e) => return e.to_compile_error().into(),
    };

    match generate_service(&parsed, input) {
        Ok(tokens) => tokens.into(),
        Err(e) => e.to_compile_error().into(),
    }
}

fn generate_service(
    parsed: &ServiceTrait,
    original: TokenStream2,
) -> Result<TokenStream2, parser::Error> {
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
    let trait_ident = format_ident!("{}", trait_name);
    let trait_snake = trait_name.to_snake_case();

    // Get the path to the roam crate (handles crate renames)
    let roam = roam_crate();

    let service_detail_fn_name = format_ident!("{}_service_detail", trait_snake);
    let method_ids_struct_name = format_ident!("{}MethodIds", trait_name);
    let method_ids_fn_name = format_ident!("{}_method_ids", trait_snake);
    let dispatch_fn_name = format_ident!("{}_dispatch_unary", trait_snake);
    let client_struct_name = format_ident!("{}Client", trait_name);
    let method_details = generate_method_details(parsed, &roam);
    let method_id_fields = generate_method_id_fields(parsed);
    let method_ids_init = generate_method_ids_init(parsed, &method_ids_struct_name, &roam);
    let dispatch_arms = generate_dispatch_arms(parsed, &roam);
    let client_methods = generate_client_methods(parsed, &method_ids_fn_name, &roam);

    let service_doc = parsed
        .doc()
        .map(|d| quote! { Some(#d.into()) })
        .unwrap_or_else(|| quote! { None });

    Ok(quote! {
        #[allow(async_fn_in_trait)]
        // Emit the original trait unchanged
        #original

        /// Returns the service detail for codegen.
        pub fn #service_detail_fn_name() -> #roam::schema::ServiceDetail {
            #roam::schema::ServiceDetail {
                name: #trait_name.into(),
                methods: vec![#(#method_details),*],
                doc: #service_doc,
            }
        }

        /// Method IDs for `#trait_ident` (computed from the canonical signature hash).
        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        pub struct #method_ids_struct_name {
            #(#method_id_fields),*
        }

        /// Lazily compute method IDs for this service from its `ServiceDetail`.
        pub fn #method_ids_fn_name() -> &'static #method_ids_struct_name {
            static IDS: ::std::sync::LazyLock<#method_ids_struct_name> = ::std::sync::LazyLock::new(|| {
                #method_ids_init
            });
            &IDS
        }

        /// Dispatch a unary request payload to the service implementation.
        ///
        /// r[impl unary.request.payload-encoding] - decode POSTCARD tuple of args
        /// This returns the *response payload bytes* (POSTCARD-encoded `Result<T, RoamError<E>>`).
        pub async fn #dispatch_fn_name<S: #trait_ident + ?Sized>(
            service: &S,
            method_id: u64,
            payload: &[u8],
        ) -> ::core::result::Result<::std::vec::Vec<u8>, #roam::session::DispatchError> {
            let ids = #method_ids_fn_name();
            match method_id {
                #(#dispatch_arms)*
                _ => {
                    let result: #roam::session::CallResult<(), #roam::session::Never> =
                        ::core::result::Result::Err(#roam::session::RoamError::UnknownMethod);
                    #roam::__private::facet_postcard::to_vec(&result).map_err(#roam::session::DispatchError::Encode)
                }
            }
        }

        /// Client stub for `#trait_ident` operating over a `roam::session::UnaryCaller`.
        pub struct #client_struct_name<C> {
            caller: C,
        }

        impl<C> #client_struct_name<C> {
            pub fn new(caller: C) -> Self {
                Self { caller }
            }

            pub fn into_inner(self) -> C {
                self.caller
            }

            pub fn caller(&self) -> &C {
                &self.caller
            }

            pub fn caller_mut(&mut self) -> &mut C {
                &mut self.caller
            }
        }

        impl<C> #client_struct_name<C>
        where
            C: #roam::session::UnaryCaller,
        {
            #(#client_methods)*
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
                    let type_detail = type_detail_expr(
                        &arg.ty,
                        &format!("{}.{} argument `{}`", service_name, method_name, arg_name),
                        roam,
                    );
                    quote! {
                        #roam::schema::ArgDetail {
                            name: #arg_name.into(),
                            type_info: #type_detail,
                        }
                    }
                })
                .collect();

            let return_type = m.return_type();
            let return_type_detail = type_detail_expr(
                &return_type,
                &format!("{}.{} return type", service_name, method_name),
                roam,
            );

            quote! {
                #roam::schema::MethodDetail {
                    service_name: #service_name.into(),
                    method_name: #method_name.into(),
                    args: vec![#(#arg_exprs),*],
                    return_type: #return_type_detail,
                    doc: #method_doc,
                }
            }
        })
        .collect()
}

fn type_detail_expr(ty: &Type, context: &str, roam: &TokenStream2) -> TokenStream2 {
    let ty_tokens = ty.to_token_stream();
    let ty_s = ty_tokens.to_string();
    quote! {
        #roam::reflect::type_detail::<#ty_tokens>().unwrap_or_else(|e| {
            panic!(
                "failed to compute roam TypeDetail for {} (type: `{}`): {}",
                #context,
                #ty_s,
                e,
            )
        })
    }
}

fn generate_method_id_fields(parsed: &ServiceTrait) -> Vec<TokenStream2> {
    parsed
        .methods()
        .map(|m| {
            let name = format_ident!("{}", m.name().to_snake_case());
            quote! { pub #name: u64 }
        })
        .collect()
}

fn generate_method_ids_init(
    parsed: &ServiceTrait,
    method_ids_struct_name: &proc_macro2::Ident,
    roam: &TokenStream2,
) -> TokenStream2 {
    let trait_name = parsed.name();
    let trait_snake = trait_name.to_snake_case();
    let service_detail_fn_name = format_ident!("{}_service_detail", trait_snake);

    let methods: Vec<_> = parsed.methods().collect();

    let vars: Vec<_> = methods
        .iter()
        .map(|m| format_ident!("id_{}", m.name().to_snake_case()))
        .collect();

    let mut init_arms = Vec::new();
    for (method, var) in methods.iter().zip(vars.iter()) {
        let method_name = method.name();
        init_arms.push(quote! { #method_name => { #var = Some(id); } });
    }

    let field_inits: Vec<_> = methods
        .iter()
        .zip(vars.iter())
        .map(|(m, var)| {
            let field = format_ident!("{}", m.name().to_snake_case());
            let msg = format!("service method id missing: {}.{}", trait_name, m.name());
            quote! { #field: #var.expect(#msg) }
        })
        .collect();

    quote! {
        let svc = #service_detail_fn_name();
        #(let mut #vars: ::core::option::Option<u64> = None;)*
        for m in &svc.methods {
            let id = #roam::hash::method_id_from_detail(m);
            match m.method_name.as_str() {
                #(#init_arms)*
                _ => {}
            }
        }
        #method_ids_struct_name { #(#field_inits),* }
    }
}

fn generate_dispatch_arms(parsed: &ServiceTrait, roam: &TokenStream2) -> Vec<TokenStream2> {
    parsed
        .methods()
        .map(|m| {
            let method_ident = format_ident!("{}", m.name());
            let method_id_field = format_ident!("{}", m.name().to_snake_case());
            let return_type = m.return_type();
            let (ok_ty, user_err_ty) = method_ok_and_err_types(&return_type);
            let ok_ty_tokens = ok_ty.to_token_stream();
            let user_err_ty_tokens = user_err_ty.map(|t| t.to_token_stream());
            let args: Vec<_> = m.args().collect();
            let args_tuple_ty = args_tuple_type(&args);
            let args_pat = args_tuple_pattern(&args);
            let arg_idents: Vec<_> = args.iter().map(|a| format_ident!("{}", a.name())).collect();

            let call_and_wrap = if let Some(user_err_ty) = user_err_ty_tokens.as_ref() {
                quote! {
                    let out: ::core::result::Result<#ok_ty_tokens, #user_err_ty> =
                        service.#method_ident(#(#arg_idents),*).await;
                    let result: #roam::session::CallResult<#ok_ty_tokens, #user_err_ty> =
                        out.map_err(#roam::session::RoamError::User);
                    #roam::__private::facet_postcard::to_vec(&result).map_err(#roam::session::DispatchError::Encode)
                }
            } else {
                quote! {
                    let out: #ok_ty_tokens = service.#method_ident(#(#arg_idents),*).await;
                    let result: #roam::session::CallResult<#ok_ty_tokens, #roam::session::Never> =
                        ::core::result::Result::Ok(out);
                    #roam::__private::facet_postcard::to_vec(&result).map_err(#roam::session::DispatchError::Encode)
                }
            };

            let invalid_payload = if let Some(user_err_ty) = user_err_ty_tokens.as_ref() {
                quote! {
                    let result: #roam::session::CallResult<#ok_ty_tokens, #user_err_ty> =
                        ::core::result::Result::Err(#roam::session::RoamError::InvalidPayload);
                    return #roam::__private::facet_postcard::to_vec(&result).map_err(#roam::session::DispatchError::Encode);
                }
            } else {
                quote! {
                    let result: #roam::session::CallResult<#ok_ty_tokens, #roam::session::Never> =
                        ::core::result::Result::Err(#roam::session::RoamError::InvalidPayload);
                    return #roam::__private::facet_postcard::to_vec(&result).map_err(#roam::session::DispatchError::Encode);
                }
            };

            quote! {
                id if id == ids.#method_id_field => {
                    let decoded: #args_tuple_ty = match #roam::__private::facet_postcard::from_slice(payload) {
                        Ok(v) => v,
                        Err(_) => { #invalid_payload }
                    };
                    let #args_pat = decoded;
                    #call_and_wrap
                }
            }
        })
        .collect()
}

fn generate_client_methods(
    parsed: &ServiceTrait,
    method_ids_fn_name: &proc_macro2::Ident,
    roam: &TokenStream2,
) -> Vec<TokenStream2> {
    parsed
        .methods()
        .map(|m| {
            let method_ident = format_ident!("{}", m.name());
            let method_id_field = format_ident!("{}", m.name().to_snake_case());
            let args: Vec<_> = m.args().collect();
            let fn_args = args.iter().map(|arg| {
                let name = format_ident!("{}", arg.name());
                let ty = arg.ty.to_token_stream();
                quote! { #name: #ty }
            });
            let arg_idents: Vec<_> = args.iter().map(|a| format_ident!("{}", a.name())).collect();

            let return_type = m.return_type();
            let (ok_ty, user_err_ty) = method_ok_and_err_types(&return_type);
            let ok_ty_tokens = ok_ty.to_token_stream();
            let user_err_ty_tokens = user_err_ty.map(|t| t.to_token_stream());
            let (result_ty, decode_expr) = if needs_borrowed_call_result(ok_ty, user_err_ty)
            {
                let err_ty_tokens = user_err_ty_tokens.unwrap_or_else(|| quote! { #roam::session::Never });
                (
                    quote! { #roam::session::BorrowedCallResult<#ok_ty_tokens, #err_ty_tokens> },
                    quote! {
                        let owned: #roam::session::BorrowedCallResult<#ok_ty_tokens, #err_ty_tokens> =
                            #roam::session::OwnedMessage::try_new(frame, |payload| {
                                #roam::__private::facet_postcard::from_slice_borrowed(payload)
                            })
                            .map_err(#roam::session::ClientError::Decode)?;
                        Ok(owned)
                    },
                )
            } else {
                let err_ty_tokens = user_err_ty_tokens.unwrap_or_else(|| quote! { #roam::session::Never });
                (
                    quote! { #roam::session::CallResult<#ok_ty_tokens, #err_ty_tokens> },
                    quote! {
                        let decoded: #roam::session::CallResult<#ok_ty_tokens, #err_ty_tokens> =
                            #roam::__private::facet_postcard::from_slice(frame.payload_bytes())
                                .map_err(#roam::session::ClientError::Decode)?;
                        Ok(decoded)
                    },
                )
            };

            let encode_args = args_encode_expr(&arg_idents, roam);

            quote! {
                pub async fn #method_ident(
                    &mut self,
                    #(#fn_args),*
                ) -> ::core::result::Result<
                    #result_ty,
                    #roam::session::ClientError<<C as #roam::session::UnaryCaller>::Error>,
                > {
                    let ids = #method_ids_fn_name();
                    let request_payload = #encode_args.map_err(#roam::session::ClientError::Encode)?;
                    let frame = self
                        .caller
                        .call_unary(ids.#method_id_field, request_payload)
                        .await
                        .map_err(#roam::session::ClientError::Transport)?;
                    #decode_expr
                }
            }
        })
        .collect()
}

fn args_tuple_type(args: &[&MethodParam]) -> TokenStream2 {
    let tys: Vec<_> = args.iter().map(|a| a.ty.to_token_stream()).collect();
    match tys.len() {
        0 => quote! { () },
        1 => {
            let t0 = &tys[0];
            quote! { (#t0,) }
        }
        _ => quote! { ( #(#tys),* ) },
    }
}

fn args_tuple_pattern(args: &[&MethodParam]) -> TokenStream2 {
    let idents: Vec<_> = args.iter().map(|a| format_ident!("{}", a.name())).collect();
    match idents.len() {
        0 => quote! { () },
        1 => {
            let a0 = &idents[0];
            quote! { (#a0,) }
        }
        _ => quote! { ( #(#idents),* ) },
    }
}

fn args_encode_expr(arg_idents: &[proc_macro2::Ident], roam: &TokenStream2) -> TokenStream2 {
    match arg_idents.len() {
        0 => quote! { #roam::__private::facet_postcard::to_vec(&()) },
        1 => {
            let a0 = &arg_idents[0];
            quote! { #roam::__private::facet_postcard::to_vec(&(#a0,)) }
        }
        _ => quote! { #roam::__private::facet_postcard::to_vec(&(#(#arg_idents),*)) },
    }
}

fn needs_borrowed_call_result(ok_ty: &Type, err_ty: Option<&Type>) -> bool {
    ok_ty.has_lifetime() || err_ty.is_some_and(|t| t.has_lifetime())
}
