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
// r[impl rpc]
// r[impl rpc.service]
// r[impl rpc.service.methods]
/// Generate all service code for a parsed trait.
///
/// Takes a `roam` token stream (the path to the roam crate) so that this function
/// can be called from tests with a fixed path like `::roam`.
pub fn generate_service(parsed: &ServiceTrait, roam: &TokenStream2) -> Result<TokenStream2, Error> {
    // r[impl rpc.channel.placement]
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
    let service_trait = generate_service_trait(parsed, roam);
    let dispatcher = generate_dispatcher(parsed, roam);
    let client = generate_client(parsed, roam);
    let service_detail_fn = generate_service_detail_fn(parsed, roam);

    Ok(quote! {
        #service_descriptor_fn
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

            // Build args tuple type and return type
            let arg_types: Vec<TokenStream2> =
                m.args().map(|arg| arg.ty.to_token_stream()).collect();
            let args_tuple_ty = quote! { (#(#arg_types,)*) };
            let arg_name_strs: Vec<String> = m.args().map(|arg| arg.name().to_string()).collect();

            let return_type = m.return_type();
            let return_ty_tokens = return_type.to_token_stream();

            let method_doc_expr = match m.doc() {
                Some(d) => quote! { Some(#d) },
                None => quote! { None },
            };

            quote! {
                #roam::hash::method_descriptor::<#args_tuple_ty, #return_ty_tokens>(
                    #service_name,
                    #method_name_str,
                    &[#(#arg_name_strs),*],
                    #method_doc_expr,
                )
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
// Service Trait Generation
// ============================================================================

fn generate_service_trait(parsed: &ServiceTrait, roam: &TokenStream2) -> TokenStream2 {
    let trait_name = format_ident!("{}Server", parsed.name());

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

    let return_type = method.return_type();
    let (ok_ty, err_ty) = if let Some((ok, err)) = return_type.as_result() {
        (ok.to_token_stream(), err.to_token_stream())
    } else {
        (
            return_type.to_token_stream(),
            quote! { ::core::convert::Infallible },
        )
    };

    let params: Vec<TokenStream2> = method
        .args()
        .map(|arg| {
            let name = format_ident!("{}", arg.name().to_snake_case());
            let ty = arg.ty.to_token_stream();
            quote! { #name: #ty }
        })
        .collect();

    quote! {
        #method_doc
        fn #method_name(&self, call: impl #roam::Call<#ok_ty, #err_ty>, #(#params),*) -> impl std::future::Future<Output = ()> + Send;
    }
}

// ============================================================================
// Dispatcher Generation
// ============================================================================

fn generate_dispatcher(parsed: &ServiceTrait, roam: &TokenStream2) -> TokenStream2 {
    let trait_name = format_ident!("{}Server", parsed.name());
    let dispatcher_name = format_ident!("{}Dispatcher", parsed.name());
    let descriptor_fn_name = format_ident!("{}_service_descriptor", parsed.name().to_snake_case());

    // Generate the if-else dispatch arms inside handle()
    let dispatch_arms: Vec<TokenStream2> = parsed
        .methods()
        .enumerate()
        .map(|(i, m)| generate_dispatch_arm(m, i, roam, &descriptor_fn_name))
        .collect();

    let no_methods = dispatch_arms.is_empty();

    let dispatch_body = if no_methods {
        quote! {
            let _ = (call, reply);
        }
    } else {
        // r[impl rpc.unknown-method]
        quote! {
            let method_id = call.method_id;
            let args_bytes = match &call.args {
                #roam::Payload::Incoming(bytes) => bytes,
                _ => {
                    reply.send_error(#roam::RoamError::<::core::convert::Infallible>::InvalidPayload).await;
                    return;
                }
            };
            #(#dispatch_arms)*
            reply.send_error(#roam::RoamError::<::core::convert::Infallible>::UnknownMethod).await;
        }
    };

    quote! {
        /// Dispatcher for this service.
        ///
        /// Wraps a handler and implements [`#roam::Handler`] by routing incoming
        /// calls to the appropriate trait method by method ID.
        #[derive(Clone)]
        pub struct #dispatcher_name<H> {
            handler: H,
        }

        impl<H> #dispatcher_name<H>
        where
            H: #trait_name + Clone + Send + Sync + 'static,
        {
            /// Create a new dispatcher wrapping the given handler.
            pub fn new(handler: H) -> Self {
                Self { handler }
            }
        }

        impl<H, R> #roam::Handler<R> for #dispatcher_name<H>
        where
            H: #trait_name + Clone + Send + Sync + 'static,
            R: #roam::ReplySink,
        {
            async fn handle(&self, call: #roam::SelfRef<#roam::RequestCall<'static>>, reply: R) {
                #dispatch_body
            }
        }
    }
}

fn generate_dispatch_arm(
    method: &ServiceMethod,
    method_index: usize,
    roam: &TokenStream2,
    descriptor_fn_name: &proc_macro2::Ident,
) -> TokenStream2 {
    let method_fn = format_ident!("{}", method.name().to_snake_case());
    let idx = method_index;

    // Build args tuple type for deserialization
    let arg_types: Vec<TokenStream2> = method.args().map(|a| a.ty.to_token_stream()).collect();
    let args_tuple_type = match arg_types.len() {
        0 => quote! { () },
        1 => {
            let t = &arg_types[0];
            quote! { (#t,) }
        }
        _ => quote! { (#(#arg_types),*) },
    };

    // Destructure args tuple into named bindings
    let arg_names: Vec<proc_macro2::Ident> = method
        .args()
        .map(|a| format_ident!("{}", a.name().to_snake_case()))
        .collect();
    let destructure = match arg_names.len() {
        0 => quote! { let () = args; },
        1 => {
            let n = &arg_names[0];
            quote! { let (#n,) = args; }
        }
        _ => quote! { let (#(#arg_names),*) = args; },
    };

    let _ = idx;

    let has_channels = method.args().any(|a| a.ty.contains_channel());

    let channel_binding = if has_channels {
        quote! {
            if let Some(binder) = reply.channel_binder() {
                let plan = #roam::RpcPlan::for_type::<#args_tuple_type>();
                if !plan.channel_locations.is_empty() {
                    // SAFETY: args is a valid, initialized value of type #args_tuple_type
                    // and we have exclusive access to it via &mut.
                    #[allow(unsafe_code)]
                    unsafe {
                        #roam::bind_channels_server(
                            &mut args as *mut #args_tuple_type as *mut u8,
                            plan,
                            &call.channels,
                            binder,
                        );
                    }
                }
            }
        }
    } else {
        quote! {}
    };

    // When there are channels, args must be mut for binding
    let args_let = if has_channels {
        quote! { let mut args: #args_tuple_type }
    } else {
        quote! { let args: #args_tuple_type }
    };

    quote! {
        if method_id == #descriptor_fn_name().methods[#idx].id {
            #args_let = match #roam::facet_postcard::from_slice_borrowed(args_bytes) {
                Ok(v) => v,
                Err(_) => {
                    reply.send_error(#roam::RoamError::<::core::convert::Infallible>::InvalidPayload).await;
                    return;
                }
            };
            #channel_binding
            #destructure
            let sink_call = #roam::SinkCall::new(reply);
            self.handler.#method_fn(sink_call, #(#arg_names),*).await;
            return;
        }
    }
}

// ============================================================================
// Client Generation
// ============================================================================

// r[impl rpc.caller]
fn generate_client(parsed: &ServiceTrait, roam: &TokenStream2) -> TokenStream2 {
    let client_name = format_ident!("{}Client", parsed.name());
    let descriptor_fn_name = format_ident!("{}_service_descriptor", parsed.name().to_snake_case());
    let service_name = parsed.name();

    let client_doc = format!(
        "Client for the `{service_name}` service.\n\n\
        Generic over any [`Caller`]({roam}::Caller) implementation.",
    );

    let client_methods: Vec<TokenStream2> = parsed
        .methods()
        .enumerate()
        .map(|(i, m)| generate_client_method(m, i, &descriptor_fn_name, roam))
        .collect();

    quote! {
        #[doc = #client_doc]
        #[derive(Clone)]
        pub struct #client_name<C: #roam::Caller> {
            caller: C,
        }

        impl<C: #roam::Caller> #client_name<C> {
            /// Create a new client wrapping the given caller.
            pub fn new(caller: C) -> Self {
                Self { caller }
            }

            #(#client_methods)*
        }
    }
}

// r[impl zerocopy.send.borrowed]
// r[impl zerocopy.send.borrowed-in-struct]
// r[impl zerocopy.send.lifetime]
fn generate_client_method(
    method: &ServiceMethod,
    method_index: usize,
    descriptor_fn_name: &proc_macro2::Ident,
    roam: &TokenStream2,
) -> TokenStream2 {
    let method_name = format_ident!("{}", method.name().to_snake_case());
    let method_doc = method.doc().map(|d| quote! { #[doc = #d] });
    let idx = method_index;

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

    // Args tuple type (for RpcPlan::for_type)
    let arg_types: Vec<TokenStream2> = method.args().map(|a| a.ty.to_token_stream()).collect();
    let args_tuple_type = match arg_types.len() {
        0 => quote! { () },
        1 => {
            let t = &arg_types[0];
            quote! { (#t,) }
        }
        _ => quote! { (#(#arg_types),*) },
    };

    // Args tuple value (for serialization)
    let args_tuple = match arg_names.len() {
        0 => quote! { () },
        1 => {
            let n = &arg_names[0];
            quote! { (#n,) }
        }
        _ => quote! { (#(#arg_names),*) },
    };

    // r[impl rpc.fallible]
    // r[impl rpc.fallible.caller-signature]
    let return_type = method.return_type();
    let (ok_ty, err_ty, client_return) = if let Some((ok, err)) = return_type.as_result() {
        let ok_t = ok.to_token_stream();
        let err_t = err.to_token_stream();
        (
            ok_t.clone(),
            err_t.clone(),
            quote! { Result<#roam::SelfRef<#roam::ResponseParts<'static, #ok_t>>, #roam::RoamError<#err_t>> },
        )
    } else {
        let t = return_type.to_token_stream();
        (
            t.clone(),
            quote! { ::core::convert::Infallible },
            quote! { Result<#roam::SelfRef<#roam::ResponseParts<'static, #t>>, #roam::RoamError> },
        )
    };

    let has_channels = method.args().any(|a| a.ty.contains_channel());

    let (args_binding, channel_binding) = if has_channels {
        (
            quote! { let mut args = #args_tuple; },
            quote! {
                let channels = if let Some(binder) = self.caller.channel_binder() {
                    let plan = #roam::RpcPlan::for_type::<#args_tuple_type>();
                    // SAFETY: args is a valid, initialized value of the args tuple type
                    // and we have exclusive access to it via &mut.
                    #[allow(unsafe_code)]
                    unsafe {
                        #roam::bind_channels_client(
                            &mut args as *mut #args_tuple_type as *mut u8,
                            plan,
                            binder,
                        )
                    }
                } else {
                    vec![]
                };
            },
        )
    } else {
        (
            quote! { let args = #args_tuple; },
            quote! { let channels = vec![]; },
        )
    };

    quote! {
        #method_doc
        pub async fn #method_name(&self, #(#params),*) -> #client_return {
            let method_id = #descriptor_fn_name().methods[#idx].id;
            #args_binding
            #channel_binding
            let req = #roam::RequestCall {
                method_id,
                args: #roam::Payload::outgoing(&args),
                channels,
                metadata: Default::default(),
            };
            let response = self.caller.call(req).await?;
            response.try_repack(|resp, _bytes| {
                let ret_bytes = match &resp.ret {
                    #roam::Payload::Incoming(bytes) => bytes,
                    _ => return Err(#roam::RoamError::InvalidPayload),
                };
                let result: Result<#ok_ty, #roam::RoamError<#err_ty>> =
                    #roam::facet_postcard::from_slice_borrowed(ret_bytes)
                        .map_err(|_| #roam::RoamError::InvalidPayload)?;
                let ret = result?;
                Ok(#roam::ResponseParts { ret, metadata: resp.metadata })
            })
        }
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

    #[test]
    fn streaming_tx() {
        assert_snapshot!(generate(quote! {
            trait Streamer { async fn count_up(&self, start: i32, output: Tx<i32>) -> i32; }
        }));
    }
}
