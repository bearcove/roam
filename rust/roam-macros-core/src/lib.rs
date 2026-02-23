//! Code generation core for roam RPC service macros.
//!
//! This crate contains all the code generation logic for the `#[service]` proc macro,
//! extracted into a regular library so it can be unit-tested with insta snapshots.

#![deny(unsafe_code)]

use ::quote::{format_ident, quote};
use heck::ToSnakeCase;
use proc_macro2::TokenStream as TokenStream2;

pub mod crate_name;

pub use roam_macros_parse::*;

use crate_name::FoundCrate;

/// Error type for validation/codegen errors.
#[derive(Debug, Clone)]
pub struct Error {
    pub span: proc_macro2::Span,
    pub message: String,
}

impl Error {
    pub fn new(span: proc_macro2::Span, message: impl Into<String>) -> Self {
        Self {
            span,
            message: message.into(),
        }
    }

    pub fn to_compile_error(&self) -> TokenStream2 {
        let msg = &self.message;
        let span = self.span;
        quote::quote_spanned! {span=> compile_error!(#msg); }
    }
}

impl From<ParseError> for Error {
    fn from(err: ParseError) -> Self {
        Self::new(proc_macro2::Span::call_site(), err.to_string())
    }
}

/// Parse a trait definition from a token stream, returning a codegen-friendly error.
pub fn parse(tokens: &TokenStream2) -> Result<ServiceTrait, Error> {
    parse_trait(tokens).map_err(Error::from)
}

/// Returns the token stream for accessing the `roam` crate.
pub fn roam_crate() -> TokenStream2 {
    match crate_name::crate_name("roam") {
        Ok(FoundCrate::Itself) => quote! { crate },
        Ok(FoundCrate::Name(name)) => {
            let ident = format_ident!("{}", name);
            quote! { ::#ident }
        }
        Err(_) => quote! { ::roam },
    }
}

// r[service-macro.is-source-of-truth]
/// Generate all service code for a parsed trait.
///
/// Takes a `roam` token stream (the path to the roam crate) so that this function
/// can be called from tests with a fixed path like `::roam`.
pub fn generate_service(parsed: &ServiceTrait, roam: &TokenStream2) -> Result<TokenStream2, Error> {
    // Validate: no channels in error types
    for method in parsed.methods() {
        let return_type = method.return_type();
        if let Some((_, err_ty)) = return_type.as_result()
            && err_ty.contains_channel()
        {
            return Err(Error::new(
                proc_macro2::Span::call_site(),
                format!(
                    "method `{}` has Channel (Tx/Rx) in error type - channels are not allowed in error types",
                    method.name()
                ),
            ));
        }
    }

    let service_descriptor_fn = generate_service_descriptor_fn(parsed, roam);
    let method_id_mod = generate_method_id_module(parsed, roam);
    let service_trait = generate_service_trait(parsed, roam);
    let dispatcher = generate_dispatcher(parsed, roam);
    let client = generate_client(parsed, roam);
    let service_detail_fn = generate_service_detail_fn(parsed, roam);

    // Generate items directly in the current module scope - no wrapper module.
    // This avoids type resolution issues since all types are already in scope.
    // Note: We use fully qualified paths for RoamError and Never instead of
    // importing them, to allow multiple services in the same module.
    Ok(quote! {
        #service_descriptor_fn
        #method_id_mod
        #service_trait
        #dispatcher
        #client
        #service_detail_fn
    })
}

// ============================================================================
// Service Descriptor Generation
// ============================================================================

fn generate_service_descriptor_fn(parsed: &ServiceTrait, roam: &TokenStream2) -> TokenStream2 {
    let service_name = parsed.name();
    let descriptor_fn_name = format_ident!("{}_service_descriptor", service_name.to_snake_case());

    // Build method descriptor expressions
    let method_descriptors: Vec<TokenStream2> = parsed
        .methods()
        .map(|m| {
            let method_name_str = m.name();

            // Build ArgDescriptor array
            let arg_descriptors: Vec<TokenStream2> = m
                .args()
                .map(|arg| {
                    let name_str = arg.name().to_string();
                    let ty = arg.ty.to_token_stream();
                    quote! {
                        #roam::session::ArgDescriptor {
                            name: #name_str,
                            shape: <#ty as #roam::facet::Facet>::SHAPE,
                        }
                    }
                })
                .collect();

            let args_expr = if arg_descriptors.is_empty() {
                quote! { &[] }
            } else {
                quote! { &[#(#arg_descriptors),*] }
            };

            // Build arg shapes for method ID hashing
            let arg_shapes_for_id: Vec<TokenStream2> = m
                .args()
                .map(|arg| {
                    let ty = arg.ty.to_token_stream();
                    quote! { <#ty as #roam::facet::Facet>::SHAPE }
                })
                .collect();

            let args_shapes_for_id = if arg_shapes_for_id.is_empty() {
                quote! { &[] }
            } else {
                quote! { &[#(#arg_shapes_for_id),*] }
            };

            // Build arg types tuple for args_plan
            let arg_types: Vec<TokenStream2> =
                m.args().map(|arg| arg.ty.to_token_stream()).collect();
            let tuple_type = if arg_types.is_empty() {
                quote! { () }
            } else if arg_types.len() == 1 {
                let t = &arg_types[0];
                quote! { (#t,) }
            } else {
                quote! { (#(#arg_types),*) }
            };

            // Return type
            let return_type = m.return_type();
            let return_ty_tokens = return_type.to_token_stream();

            // ret_plan type: Result<T, RoamError<E>> or Result<T, RoamError> for infallible
            let ret_plan_ty = if let Some((ok_ty, err_ty)) = return_type.as_result() {
                let ok = ok_ty.to_token_stream();
                let err = err_ty.to_token_stream();
                quote! { Result<#ok, #roam::RoamError<#err>> }
            } else {
                let ty = return_type.to_token_stream();
                quote! { Result<#ty, #roam::RoamError> }
            };

            let method_doc_expr = match m.doc() {
                Some(d) => quote! { Some(#d) },
                None => quote! { None },
            };

            quote! {
                Box::leak(Box::new(#roam::session::MethodDescriptor {
                    id: #roam::hash::method_id_from_shapes(
                        #service_name,
                        #method_name_str,
                        #args_shapes_for_id,
                        <#return_ty_tokens as #roam::facet::Facet>::SHAPE,
                    ),
                    service_name: #service_name,
                    method_name: #method_name_str,
                    args: #args_expr,
                    return_shape: <#return_ty_tokens as #roam::facet::Facet>::SHAPE,
                    args_plan: #roam::rpc_plan::<#tuple_type>(),
                    ret_plan: #roam::rpc_plan::<#ret_plan_ty>(),
                    doc: #method_doc_expr,
                }))
            }
        })
        .collect();

    let service_doc_expr = match parsed.doc() {
        Some(d) => quote! { Some(#d) },
        None => quote! { None },
    };

    quote! {
        #[allow(non_snake_case, clippy::all)]
        pub fn #descriptor_fn_name() -> &'static #roam::session::ServiceDescriptor {
            static DESCRIPTOR: std::sync::OnceLock<&'static #roam::session::ServiceDescriptor> = std::sync::OnceLock::new();
            DESCRIPTOR.get_or_init(|| {
                let methods: Vec<&'static #roam::session::MethodDescriptor> = vec![
                    #(#method_descriptors),*
                ];
                Box::leak(Box::new(#roam::session::ServiceDescriptor {
                    service_name: #service_name,
                    methods: Box::leak(methods.into_boxed_slice()),
                    doc: #service_doc_expr,
                }))
            })
        }
    }
}

// ============================================================================
// Method ID Generation
// ============================================================================

fn generate_method_id_module(parsed: &ServiceTrait, roam: &TokenStream2) -> TokenStream2 {
    let service_name = parsed.name();
    let mod_name = format_ident!("{}_method_id", service_name.to_snake_case());
    let descriptor_fn_name = format_ident!("{}_service_descriptor", service_name.to_snake_case());

    let method_fns: Vec<TokenStream2> = parsed
        .methods()
        .enumerate()
        .map(|(i, m)| {
            let fn_name = format_ident!("{}", m.name().to_snake_case());
            let idx = i;
            quote! {
                pub fn #fn_name() -> #roam::session::MethodId {
                    super::#descriptor_fn_name().methods[#idx].id
                }
            }
        })
        .collect();

    quote! {
        /// Method IDs for the service (indexes into the service descriptor).
        #[allow(non_snake_case, clippy::all, unused)]
        pub mod #mod_name {
            use super::*;

            #(#method_fns)*
        }
    }
}

// ============================================================================
// Service Trait Generation
// ============================================================================

fn generate_service_trait(parsed: &ServiceTrait, roam: &TokenStream2) -> TokenStream2 {
    let trait_name = format_ident!("{}", parsed.name());

    let trait_doc = parsed.doc().map(|d| quote! { #[doc = #d] });

    let methods: Vec<TokenStream2> = parsed
        .methods()
        .map(|m| generate_trait_method(m, roam))
        .collect();

    quote! {
        #trait_doc
        pub trait #trait_name
        where
            Self: Send + Sync,
        {
            #(#methods)*
        }
    }
}

fn generate_trait_method(method: &ServiceMethod, roam: &TokenStream2) -> TokenStream2 {
    let method_name = format_ident!("{}", method.name().to_snake_case());
    let method_doc = method.doc().map(|d| quote! { #[doc = #d] });

    // Parameters - cx: &Context comes first
    let params: Vec<TokenStream2> = method
        .args()
        .map(|arg| {
            let name = format_ident!("{}", arg.name().to_snake_case());
            let ty = arg.ty.to_token_stream();
            quote! { #name: #ty }
        })
        .collect();
    // Return type - wrap in Result<T, RoamError<E>> or Result<T, RoamError<Never>>
    let return_type = method.return_type();
    let full_return = format_handler_return_type(&return_type, roam);

    quote! {
        #method_doc
        fn #method_name(&self, cx: &#roam::Context, #(#params),*) -> impl std::future::Future<Output = #full_return> + Send;
    }
}

/// Format the return type for handler trait - uses original type as-is.
fn format_handler_return_type(return_type: &Type, _roam: &TokenStream2) -> TokenStream2 {
    return_type.to_token_stream()
}

// ============================================================================
// Dispatcher Generation
// ============================================================================

fn generate_dispatcher(parsed: &ServiceTrait, roam: &TokenStream2) -> TokenStream2 {
    let trait_name = format_ident!("{}", parsed.name());
    let dispatcher_name = format_ident!("{}Dispatcher", parsed.name());
    let method_id_mod = format_ident!("{}_method_id", parsed.name().to_snake_case());
    let descriptor_fn_name = format_ident!("{}_service_descriptor", parsed.name().to_snake_case());

    // Generate dispatch methods
    let dispatch_methods: Vec<TokenStream2> = parsed
        .methods()
        .enumerate()
        .map(|(i, m)| generate_dispatch_method(m, i, roam, &descriptor_fn_name))
        .collect();

    // Generate the if-else chain for ServiceDispatcher impl
    let dispatch_arms: Vec<TokenStream2> = parsed
        .methods()
        .enumerate()
        .map(|(i, m)| {
            let method_name = format_ident!("{}", m.name().to_snake_case());
            let dispatch_name = format_ident!("dispatch_{}", m.name().to_snake_case());
            let keyword = if i == 0 {
                quote! { if }
            } else {
                quote! { else if }
            };
            quote! {
                #keyword method_id == #method_id_mod::#method_name() {
                    self.#dispatch_name(cx, payload, registry)
                }
            }
        })
        .collect();

    quote! {
        /// Dispatcher for this service.
        ///
        /// Supports middleware that can inspect deserialized args before the handler runs.
        /// Middleware is configured via [`with_middleware`](Self::with_middleware).
        #[derive(Clone)]
        pub struct #dispatcher_name<H> {
            handler: H,
            middleware: Vec<::std::sync::Arc<dyn #roam::session::Middleware>>,
        }

        impl<H> #dispatcher_name<H>
        where
            H: #trait_name + Clone + 'static,
        {
            /// Create a new dispatcher with no middleware.
            pub fn new(handler: H) -> Self {
                Self {
                    handler,
                    middleware: Vec::new(),
                }
            }

            /// Add middleware to this dispatcher.
            ///
            /// Middleware runs after deserialization but before the handler.
            /// It can inspect args via reflection and reject requests.
            /// Middleware runs in the order it was added.
            pub fn with_middleware<M: #roam::session::Middleware + 'static>(mut self, mw: M) -> Self {
                self.middleware.push(std::sync::Arc::new(mw));
                self
            }

            /// Add already-Arc'd middleware to this dispatcher.
            pub fn with_middleware_arc(mut self, mw: std::sync::Arc<dyn #roam::session::Middleware>) -> Self {
                self.middleware.push(mw);
                self
            }

            #(#dispatch_methods)*
        }

        impl<H> #roam::session::ServiceDispatcher for #dispatcher_name<H>
        where
            H: #trait_name + Clone + 'static,
        {
            fn service_descriptor(&self) -> &'static #roam::session::ServiceDescriptor {
                #descriptor_fn_name()
            }

            fn dispatch(
                &self,
                cx: #roam::Context,
                payload: Vec<u8>,
                registry: &mut #roam::session::ChannelRegistry,
            ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
                let method_id = cx.method_id();
                #(#dispatch_arms)*
                else {
                    #roam::session::dispatch_unknown_method(&cx, registry)
                }
            }
        }
    }
}

fn generate_dispatch_method(
    method: &ServiceMethod,
    method_index: usize,
    roam: &TokenStream2,
    descriptor_fn_name: &proc_macro2::Ident,
) -> TokenStream2 {
    let dispatch_name = format_ident!("dispatch_{}", method.name().to_snake_case());
    let idx = method_index;
    let _ = (roam, descriptor_fn_name, idx);

    quote! {
        fn #dispatch_name(
            &self,
            cx: #roam::Context,
            payload: Vec<u8>,
            registry: &mut #roam::session::ChannelRegistry,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'static>> {
            todo!()
        }
    }
}

// ============================================================================
// Client Generation
// ============================================================================

fn generate_client(parsed: &ServiceTrait, roam: &TokenStream2) -> TokenStream2 {
    let client_name = format_ident!("{}Client", parsed.name());
    let descriptor_fn_name = format_ident!("{}_service_descriptor", parsed.name().to_snake_case());

    let client_doc = format!(
        "Client for {} service.\n\n\
        Generic over any [`Caller`]({roam}::session::Caller) implementation, \
        allowing use with both [`ConnectionHandle`]({roam}::session::ConnectionHandle) \
        and reconnecting clients.",
        parsed.name()
    );

    let client_methods: Vec<TokenStream2> = parsed
        .methods()
        .enumerate()
        .map(|(i, m)| generate_client_method(m, i, &descriptor_fn_name, roam))
        .collect();

    quote! {
        #[doc = #client_doc]
        #[derive(Clone)]
        pub struct #client_name<C: #roam::session::Caller = #roam::session::ConnectionHandle> {
            caller: C,
        }

        impl<C: #roam::session::Caller> #client_name<C> {
            /// Create a new client wrapping the given caller.
            pub fn new(caller: C) -> Self {
                Self { caller }
            }

            #(#client_methods)*
        }
    }
}

fn generate_client_method(
    method: &ServiceMethod,
    method_index: usize,
    descriptor_fn_name: &proc_macro2::Ident,
    roam: &TokenStream2,
) -> TokenStream2 {
    let method_name = format_ident!("{}", method.name().to_snake_case());
    let method_doc = method.doc().map(|d| quote! { #[doc = #d] });
    let idx = method_index;

    // Parameters
    let params: Vec<TokenStream2> = method
        .args()
        .map(|arg| {
            let name = format_ident!("{}", arg.name().to_snake_case());
            let ty = arg.ty.to_token_stream();
            quote! { #name: #ty }
        })
        .collect();
    let arg_names: Vec<proc_macro2::Ident> = method
        .args()
        .map(|arg| format_ident!("{}", arg.name().to_snake_case()))
        .collect();

    // Build args tuple type for CallFuture
    let args_tuple_type = if arg_names.is_empty() {
        quote! { () }
    } else {
        let arg_types: Vec<TokenStream2> =
            method.args().map(|arg| arg.ty.to_token_stream()).collect();
        if arg_types.len() == 1 {
            let ty = &arg_types[0];
            quote! { (#ty,) }
        } else {
            quote! { (#(#arg_types),*) }
        }
    };

    // Build args tuple value
    let args_tuple = if arg_names.is_empty() {
        quote! { () }
    } else if arg_names.len() == 1 {
        let n = &arg_names[0];
        quote! { (#n,) }
    } else {
        quote! { (#(#arg_names),*) }
    };

    // Return type and error type depend on whether method is fallible
    let return_type = method.return_type();
    let (ok_ty, err_ty, _client_return) = format_client_return_type(&return_type, roam);

    // The CallFuture type
    let call_future_return = quote! {
        #roam::session::CallFuture<C, #args_tuple_type, #ok_ty, #err_ty>
    };

    quote! {
        #method_doc
        pub fn #method_name(
            &self,
            #(#params),*
        ) -> #call_future_return {
            let desc = #descriptor_fn_name().methods[#idx];

            #roam::session::CallFuture::new(
                self.caller.clone(),
                desc,
                #args_tuple,
            )
        }
    }
}

/// Format the return type as Result<T, CallError<E>> for client.
///
/// Returns (ok_type, err_type, full_return_type) for use in codegen.
fn format_client_return_type(
    return_type: &Type,
    roam: &TokenStream2,
) -> (TokenStream2, TokenStream2, TokenStream2) {
    if let Some((ok_ty, err_ty)) = return_type.as_result() {
        let ok_tokens = ok_ty.to_token_stream();
        let err_tokens = err_ty.to_token_stream();
        (
            ok_tokens.clone(),
            err_tokens.clone(),
            quote! { Result<#ok_tokens, #roam::session::CallError<#err_tokens>> },
        )
    } else {
        let ty_tokens = return_type.to_token_stream();
        (
            ty_tokens.clone(),
            quote! { ::core::convert::Infallible },
            quote! { Result<#ty_tokens, #roam::session::CallError<::core::convert::Infallible>> },
        )
    }
}

// ============================================================================
// Service Detail Function Generation (for codegen in other languages)
// ============================================================================

fn generate_service_detail_fn(parsed: &ServiceTrait, roam: &TokenStream2) -> TokenStream2 {
    let trait_name = parsed.name();
    let trait_snake = trait_name.to_snake_case();
    let fn_name = format_ident!("{}_service_detail", trait_snake);

    let method_details = generate_method_details(parsed, roam);

    let service_doc = parsed
        .doc()
        .map(|d| quote! { Some(#d.into()) })
        .unwrap_or_else(|| quote! { None });

    quote! {
        /// Returns the service detail for codegen.
        pub fn #fn_name() -> #roam::schema::ServiceDetail {
            #roam::schema::ServiceDetail {
                name: #trait_name.into(),
                methods: vec![#(#method_details),*],
                doc: #service_doc,
            }
        }
    }
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
                    let ty = arg.ty.to_token_stream();
                    quote! {
                        #roam::schema::ArgDetail {
                            name: #arg_name.into(),
                            ty: <#ty as #roam::facet::Facet>::SHAPE,
                        }
                    }
                })
                .collect();

            let return_type = m.return_type();
            let return_ty_tokens = return_type.to_token_stream();

            quote! {
                #roam::schema::MethodDetail {
                    service_name: #service_name.into(),
                    method_name: #method_name.into(),
                    args: vec![#(#arg_exprs),*],
                    return_type: <#return_ty_tokens as #roam::facet::Facet>::SHAPE,
                    doc: #method_doc,
                }
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use insta::assert_snapshot;
    use quote::quote;

    fn prettyprint(ts: proc_macro2::TokenStream) -> String {
        use std::io::Write;
        use std::process::{Command, Stdio};

        let mut child = Command::new("rustfmt")
            .args(["--edition", "2024"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("failed to spawn rustfmt");

        child
            .stdin
            .take()
            .unwrap()
            .write_all(ts.to_string().as_bytes())
            .unwrap();

        let output = child.wait_with_output().expect("rustfmt failed");
        assert!(
            output.status.success(),
            "rustfmt exited with {}",
            output.status
        );
        String::from_utf8(output.stdout).expect("rustfmt output not UTF-8")
    }

    fn generate(input: proc_macro2::TokenStream) -> String {
        let parsed = roam_macros_parse::parse_trait(&input).unwrap();
        let roam = quote! { ::roam };
        let ts = crate::generate_service(&parsed, &roam).unwrap();
        prettyprint(ts)
    }

    #[test]
    fn adder_infallible() {
        assert_snapshot!(generate(quote! {
            pub trait Adder { async fn add(&self, a: i32, b: i32) -> i32; }
        }));
    }

    #[test]
    fn fallible() {
        assert_snapshot!(generate(quote! {
            trait Calc { async fn div(&self, a: f64, b: f64) -> Result<f64, DivError>; }
        }));
    }

    #[test]
    fn no_args() {
        assert_snapshot!(generate(quote! {
            trait Ping { async fn ping(&self) -> u64; }
        }));
    }

    #[test]
    fn unit_return() {
        assert_snapshot!(generate(quote! {
            trait Notifier { async fn notify(&self, msg: String); }
        }));
    }
}
