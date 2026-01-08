//! Rust code generation for roam services.
//!
//! Generates caller traits and handler dispatchers from ServiceDetail.
//! Intended for use in build.rs scripts.

use codegen::{Block, Impl, Scope};
use facet_core::{ScalarType, Shape};
use heck::{ToSnakeCase, ToUpperCamelCase};
use roam_schema::{
    EnumInfo, MethodDetail, ServiceDetail, ShapeKind, StructInfo, VariantKind, classify_shape,
    classify_variant, is_bytes, is_rx, is_tx,
};

/// Extract the Ok and Err types from a Result<T, E> shape.
/// Returns (ok_type, Some(err_type)) for Result, or (shape, None) for non-Result types.
fn extract_result_types(shape: &'static Shape) -> (&'static Shape, Option<&'static Shape>) {
    if let ShapeKind::Enum(EnumInfo {
        name: None,
        variants,
    }) = classify_shape(shape)
        && variants.len() == 2
    {
        let ok_variant = variants.iter().find(|v| v.name == "Ok");
        let err_variant = variants.iter().find(|v| v.name == "Err");

        if let (Some(ok_v), Some(err_v)) = (ok_variant, err_variant)
            && let (VariantKind::Newtype { inner: ok_ty }, VariantKind::Newtype { inner: err_ty }) =
                (classify_variant(ok_v), classify_variant(err_v))
        {
            return (ok_ty, Some(err_ty));
        }
    }
    (shape, None)
}

/// Format the return type for service trait methods.
/// Returns Result<T, RoamError<E>> where E is the user error type (or Never for infallible methods).
fn format_handler_return_type(return_shape: &'static Shape) -> String {
    let (ok_ty, err_ty) = extract_result_types(return_shape);
    let ok_type_str = rust_type_server_return(ok_ty);
    let err_type_str = err_ty
        .map(rust_type_base)
        .unwrap_or_else(|| "Never".to_string());
    format!("Result<{ok_type_str}, RoamError<{err_type_str}>>")
}

use crate::render::hex_u64;

/// Options for Rust code generation.
#[derive(Debug, Clone, Default)]
pub struct RustCodegenOptions {
    /// Generate tracing spans and events for RPC calls.
    ///
    /// When enabled, each method dispatch will be wrapped in a `tracing::debug_span!`
    /// with the service and method name. Events are emitted for request/response sizes.
    ///
    /// Requires the `tracing` crate in the consuming crate.
    pub tracing: bool,
}

/// Generator for Rust code from service definitions.
struct RustGenerator<'a> {
    service: &'a ServiceDetail,
    options: &'a RustCodegenOptions,
    scope: Scope,
}

impl<'a> RustGenerator<'a> {
    fn new(service: &'a ServiceDetail, options: &'a RustCodegenOptions) -> Self {
        Self {
            service,
            options,
            scope: Scope::new(),
        }
    }

    /// Generate the body of dispatch_streaming for ServiceDispatcher impl.
    ///
    /// This generates match arms for each streaming method that:
    /// 1. Decode stream IDs from payload
    /// 2. Register streams with the registry
    /// 3. Create Tx/Rx handles
    /// 4. Clone handler into a 'static future
    /// 5. Call the handler method
    fn generate_dispatch_streaming_body(&self) -> String {
        let mut arms = Vec::new();

        for method in &self.service.methods {
            let method_name = method.method_name.to_snake_case();
            let const_name = method_name.to_uppercase();
            let is_streaming =
                method.args.iter().any(|a| is_stream(a.ty)) || is_stream(method.return_type);

            if !is_streaming {
                continue;
            }

            // Build wire tuple type (streams as u64)
            let wire_arg_types: Vec<String> = method
                .args
                .iter()
                .map(|arg| {
                    if is_tx(arg.ty) || is_rx(arg.ty) {
                        "u64".to_string()
                    } else {
                        rust_type_server_arg(arg.ty)
                    }
                })
                .collect();

            let arg_names: Vec<String> =
                method.args.iter().map(|a| a.name.to_snake_case()).collect();

            let mut arm_body = String::new();
            arm_body.push_str(&format!("            method_id::{const_name} => {{\n"));

            // Decode wire format
            if method.args.is_empty() {
                arm_body.push_str("                // No arguments to decode\n");
            } else {
                let tuple_pattern = arg_names.join(", ");
                let tuple_type = wire_arg_types.join(", ");
                arm_body.push_str(&format!(
                    "                let ({tuple_pattern}) = match facet_postcard::from_slice::<({tuple_type})>(&payload) {{\n"
                ));
                arm_body.push_str("                    Ok(args) => args,\n");
                arm_body.push_str("                    Err(_) => return Box::pin(async {{ Ok(vec![1, 2]) }}), // InvalidPayload\n");
                arm_body.push_str("                };\n");
            }

            // Create channels for streaming args and register with registry
            // No inversion: handler signature matches schema exactly
            // - Rx<T>: server receives from client
            // - Tx<T>: server sends to client
            for arg in &method.args {
                let arg_name = arg.name.to_snake_case();
                if is_rx(arg.ty) {
                    // Rx<T> = server receives from client
                    // Create channel, register tx for incoming data routing, give handler Rx
                    let inner_type = if let Some(inner) = arg.ty.type_params.first() {
                        rust_type_base(inner.shape)
                    } else {
                        "()".to_string()
                    };
                    arm_body.push_str(&format!(
                        "                let ({arg_name}_tx, {arg_name}_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);\n"
                    ));
                    arm_body.push_str(&format!(
                        "                registry.register_incoming({arg_name}, {arg_name}_tx);\n"
                    ));
                    arm_body.push_str(&format!(
                        "                let {arg_name} = Rx::<{inner_type}>::new({arg_name}, {arg_name}_rx);\n"
                    ));
                } else if is_tx(arg.ty) {
                    // Tx<T> = server sends to client
                    // Create channel, give handler Tx, need to drain rx and send as Data
                    // TODO: The rx needs to be returned to driver for FuturesUnordered polling
                    let inner_type = if let Some(inner) = arg.ty.type_params.first() {
                        rust_type_base(inner.shape)
                    } else {
                        "()".to_string()
                    };
                    arm_body.push_str(&format!(
                        "                let ({arg_name}_tx, _{arg_name}_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);\n"
                    ));
                    arm_body.push_str(&format!(
                        "                registry.register_outgoing_credit({arg_name});\n"
                    ));
                    arm_body.push_str(&format!(
                        "                let {arg_name} = Tx::<{inner_type}>::new({arg_name}, {arg_name}_tx);\n"
                    ));
                    // TODO: Need to return _{arg_name}_rx to the driver for polling
                }
            }

            // Clone handler for 'static future
            arm_body.push_str("                let handler = self.handler.clone();\n");

            // Build handler call
            let args_str = arg_names.join(", ");
            arm_body.push_str("                Box::pin(async move {\n");
            arm_body.push_str(&format!(
                "                    match handler.{method_name}({args_str}).await {{\n"
            ));
            arm_body.push_str("                        Ok(result) => {\n");
            arm_body.push_str("                            let mut out = vec![0u8];\n");
            arm_body.push_str(
                "                            out.extend(facet_postcard::to_vec(&result).map_err(|e| e.to_string())?);\n",
            );
            arm_body.push_str("                            Ok(out)\n");
            arm_body.push_str("                        }\n");
            arm_body.push_str("                        Err(_e) => Ok(vec![1, 1]),\n");
            arm_body.push_str("                    }\n");
            arm_body.push_str("                })\n");
            arm_body.push_str("            }");

            arms.push(arm_body);
        }

        // Add fallback for unknown streaming methods
        arms.push(
            "            _ => Box::pin(async move { Err(format!(\"unknown streaming method {}\", method_id)) })"
                .to_string(),
        );

        format!(
            "            match method_id {{\n{}\n            }}",
            arms.join("\n")
        )
    }

    fn generate(mut self) -> String {
        // Header comment - only once per file (caller should only include once)
        self.scope.raw("// @generated by roam-codegen");
        self.scope
            .raw("// DO NOT EDIT - regenerate with build.rs\n");

        // Wrap in module named after the service to avoid conflicts
        // Use outer attributes to suppress clippy/unused warnings on generated code
        let mod_name = self.service.name.to_snake_case();
        self.scope.raw(format!(
            "#[allow(clippy::all, unused)]\npub mod {mod_name} {{"
        ));

        // Re-export common types for convenience
        self.scope
            .raw("    pub use ::roam::session::{Tx, Rx, StreamId, RoamError, CallResult, Never};");

        // Internal imports for codegen (with allow to suppress warnings when not all are used)
        self.scope
            .raw("    #[allow(unused_imports)]\n    use ::roam::__private::facet_postcard;");

        // Generate method IDs
        self.generate_method_ids();

        // Generate service trait and dispatcher
        self.generate_service_trait();
        self.generate_dispatcher();

        // Close module
        self.scope.raw("}");

        self.scope.to_string()
    }

    fn generate_method_ids(&mut self) {
        // method_id module is now scoped within the service module, so no prefix needed
        self.scope
            .raw("    /// Method IDs for this service.\n    pub mod method_id {");

        for method in &self.service.methods {
            let id = crate::method_id(method);
            let const_name = method.method_name.to_snake_case().to_uppercase();
            self.scope.raw(format!(
                "        pub const {const_name}: u64 = {};",
                hex_u64(id)
            ));
        }

        self.scope.raw("    }");
    }

    fn generate_service_trait(&mut self) {
        // Just use the service name as the trait name
        let trait_name = self.service.name.to_upper_camel_case();
        let trait_def = self.scope.new_trait(&trait_name);
        trait_def.vis("pub");
        trait_def.bound("Self", "Send + Sync");

        if let Some(doc) = &self.service.doc {
            trait_def.doc(doc);
        }

        for method in &self.service.methods {
            let method_name = method.method_name.to_snake_case();

            // Return type
            let full_return = format_handler_return_type(method.return_type);

            let fn_def = trait_def.new_fn(&method_name);
            fn_def.arg_ref_self();
            for arg in &method.args {
                let arg_name = arg.name.to_snake_case();
                // No inversion - types are used as-is
                let arg_type = rust_type_server_arg(arg.ty);
                fn_def.arg(&arg_name, &arg_type);
            }
            fn_def.ret(format!(
                "impl std::future::Future<Output = {full_return}> + Send"
            ));

            if let Some(doc) = &method.doc {
                fn_def.doc(doc.as_ref());
            }
        }
    }

    fn generate_dispatcher(&mut self) {
        let service_name = self.service.name.to_upper_camel_case();
        let dispatcher_name = format!("{service_name}Dispatcher");
        // The trait is just named after the service (e.g., "Testbed" not "TestbedHandler")
        let service_trait = service_name.clone();

        // Generate the dispatcher struct
        self.scope.raw(format!(
            "\n    /// Dispatcher for {service_name} service.\n    pub struct {dispatcher_name}<H> {{\n        handler: H,\n    }}"
        ));

        // Generate impl block
        let impl_block = self.scope.new_impl(&dispatcher_name);
        impl_block.generic("H");
        impl_block
            .target_generic("H")
            .bound("H", format!("{service_trait} + 'static"));

        // Constructor
        let new_fn = impl_block.new_fn("new");
        new_fn.vis("pub");
        new_fn.arg("handler", "H");
        new_fn.ret("Self");
        new_fn.line("Self { handler }");

        // Dispatch method (only for unary methods - streaming uses ServiceDispatcher trait)
        let dispatch_fn = impl_block.new_fn("dispatch");
        dispatch_fn.vis("pub");
        dispatch_fn.set_async(true);
        dispatch_fn.arg_ref_self();
        dispatch_fn.arg("method_id", "u64");
        dispatch_fn.arg("payload", "&[u8]");
        dispatch_fn.ret("::std::vec::Vec<u8>");

        let mut dispatch_body = Block::new("match method_id");
        for method in &self.service.methods {
            let _id = crate::method_id(method);
            let method_name = method.method_name.to_snake_case();
            let const_name = method_name.to_uppercase();
            let method_is_streaming =
                method.args.iter().any(|a| is_stream(a.ty)) || is_stream(method.return_type);

            if method_is_streaming {
                // Streaming methods must use ServiceDispatcher::dispatch_streaming
                dispatch_body.line(format!(
                    "method_id::{const_name} => vec![1, 1], // Streaming: use ServiceDispatcher"
                ));
            } else {
                dispatch_body.line(format!(
                    "method_id::{const_name} => self.dispatch_{method_name}(payload).await,"
                ));
            }
        }
        dispatch_body.line("_ => Self::unknown_method_response(method_id),");
        dispatch_fn.push_block(dispatch_body);

        // Unknown method response helper
        let unknown_fn = impl_block.new_fn("unknown_method_response");
        unknown_fn.arg("_method_id", "u64");
        unknown_fn.ret("::std::vec::Vec<u8>");
        unknown_fn.line("// Return error response for unknown method");
        unknown_fn.line("vec![1] // Error marker");

        // Generate dispatch_<method> for each non-streaming method
        // Streaming methods are handled by ServiceDispatcher::dispatch_streaming
        for method in &self.service.methods {
            let method_is_streaming =
                method.args.iter().any(|a| is_stream(a.ty)) || is_stream(method.return_type);
            if !method_is_streaming {
                generate_dispatch_method_fn(impl_block, method, self.options);
            }
        }

        // Generate ServiceDispatcher trait implementation
        self.generate_service_dispatcher_impl(&dispatcher_name, &service_trait);
    }

    fn generate_service_dispatcher_impl(&mut self, dispatcher_name: &str, service_trait: &str) {
        // Check if any method is streaming - if so, we need Clone bound
        let has_streaming = self
            .service
            .methods
            .iter()
            .any(|m| m.args.iter().any(|a| is_stream(a.ty)) || is_stream(m.return_type));

        // Generate is_streaming method body
        let mut is_streaming_arms = Vec::new();
        for method in &self.service.methods {
            let method_name = method.method_name.to_snake_case();
            let const_name = method_name.to_uppercase();
            let is_streaming =
                method.args.iter().any(|a| is_stream(a.ty)) || is_stream(method.return_type);
            is_streaming_arms.push(format!(
                "            method_id::{const_name} => {is_streaming},"
            ));
        }
        let is_streaming_match = is_streaming_arms.join("\n");

        // Generate dispatch_unary arms
        let mut dispatch_unary_arms = Vec::new();
        for method in &self.service.methods {
            let method_name = method.method_name.to_snake_case();
            let const_name = method_name.to_uppercase();
            let is_streaming =
                method.args.iter().any(|a| is_stream(a.ty)) || is_stream(method.return_type);
            if !is_streaming {
                dispatch_unary_arms.push(format!(
                    "                method_id::{const_name} => Ok(self.dispatch_{method_name}(&payload).await),"
                ));
            }
        }
        // Unknown method returns encoded Result::Err(RoamError::UnknownMethod)
        // RoamError::UnknownMethod = 1, so payload is [1, 1] (Result::Err variant, UnknownMethod variant)
        dispatch_unary_arms.push(
            "                _ => Ok(vec![1, 1]), // Result::Err(RoamError::UnknownMethod)"
                .to_string(),
        );
        let dispatch_unary_match = dispatch_unary_arms.join("\n");

        // Generate dispatch_streaming body
        let dispatch_streaming_body = self.generate_dispatch_streaming_body();

        // Generate the impl block as raw code (codegen crate doesn't support impl Trait for Type<G> well)
        // Add Clone bound if there are streaming methods (handler needs to be cloned into spawned future)
        let clone_bound = if has_streaming { " + Clone" } else { "" };
        self.scope.raw(format!(
            r#"
    impl<H> ::roam::session::ServiceDispatcher for {dispatcher_name}<H>
    where
        H: {service_trait} + 'static{clone_bound},
    {{
        fn is_streaming(&self, method_id: u64) -> bool {{
            match method_id {{
{is_streaming_match}
                _ => false,
            }}
        }}

        fn dispatch_unary(
            &self,
            method_id: u64,
            payload: &[u8],
        ) -> impl std::future::Future<Output = Result<Vec<u8>, String>> + Send {{
            let payload = payload.to_vec();
            async move {{
                match method_id {{
{dispatch_unary_match}
                }}
            }}
        }}

        fn dispatch_streaming(
            &self,
            method_id: u64,
            payload: Vec<u8>,
            registry: &mut ::roam::session::StreamRegistry,
        ) -> std::pin::Pin<
            Box<dyn std::future::Future<Output = Result<Vec<u8>, String>> + Send + 'static>,
        > {{
{dispatch_streaming_body}
        }}
    }}"#
        ));
    }
}

/// Generate a dispatch method for a single service method.
fn generate_dispatch_method_fn(
    impl_block: &mut Impl,
    method: &MethodDetail,
    options: &RustCodegenOptions,
) {
    let method_name = method.method_name.to_snake_case();
    let dispatch_name = format!("dispatch_{method_name}");

    let is_streaming = method.args.iter().any(|a| is_stream(a.ty)) || is_stream(method.return_type);

    let dispatch_fn = impl_block.new_fn(&dispatch_name);
    dispatch_fn.set_async(true);
    dispatch_fn.arg_ref_self();
    dispatch_fn.arg("payload", "&[u8]");
    dispatch_fn.ret("::std::vec::Vec<u8>");

    if is_streaming {
        generate_dispatch_streaming(dispatch_fn, method, options);
    } else {
        generate_dispatch_unary(dispatch_fn, method, options);
    }
}

/// Generate body for non-streaming method dispatch.
fn generate_dispatch_unary(
    dispatch_fn: &mut codegen::Function,
    method: &MethodDetail,
    options: &RustCodegenOptions,
) {
    let method_name = method.method_name.to_snake_case();

    // Decode arguments - payload is encoded as a tuple of all args
    if method.args.is_empty() {
        dispatch_fn.line("// No arguments to decode");
    } else {
        dispatch_fn.line("// Decode arguments (encoded as tuple)");
        let arg_names: Vec<String> = method.args.iter().map(|a| a.name.to_snake_case()).collect();
        let arg_types: Vec<String> = method
            .args
            .iter()
            .map(|a| rust_type_server_arg(a.ty))
            .collect();
        let tuple_pattern = arg_names.join(", ");
        let tuple_type = arg_types.join(", ");
        dispatch_fn.line(format!(
            "let ({tuple_pattern}) = match facet_postcard::from_slice::<({tuple_type})>(payload) {{"
        ));
        dispatch_fn.line("    Ok(args) => args,");
        dispatch_fn
            .line("    Err(_) => return vec![1, 2], // Result::Err, RoamError::InvalidPayload");
        dispatch_fn.line("};");
    }

    // Call handler
    let args: Vec<String> = method.args.iter().map(|a| a.name.to_snake_case()).collect();
    let args_str = args.join(", ");

    if options.tracing {
        dispatch_fn.line(format!(
            "let _span = tracing::debug_span!(\"dispatch\", method = \"{method_name}\").entered();"
        ));
    }

    dispatch_fn.line(format!(
        "match self.handler.{method_name}({args_str}).await {{"
    ));
    dispatch_fn.line("    Ok(result) => {");
    dispatch_fn.line("        let mut out = vec![0u8]; // Success marker");
    generate_encode_expr(dispatch_fn, "result", method.return_type, "        ");
    dispatch_fn.line("        out");
    dispatch_fn.line("    }");
    dispatch_fn.line("    Err(_e) => {");
    dispatch_fn.line("        // Encode RoamError - for now just return error marker");
    dispatch_fn.line("        vec![1, 1] // Result::Err, RoamError::UnknownMethod placeholder");
    dispatch_fn.line("    }");
    dispatch_fn.line("}");
}

/// Generate body for streaming method dispatch.
///
/// For streaming methods, we need to:
/// 1. Decode stream IDs from the payload (streams serialize as u64)
/// 2. Decode non-stream arguments
/// 3. Register streams with the registry and create Tx/Rx handles
/// 4. Call the handler with properly bound streams
fn generate_dispatch_streaming(
    dispatch_fn: &mut codegen::Function,
    method: &MethodDetail,
    _options: &RustCodegenOptions,
) {
    let method_name = method.method_name.to_snake_case();

    // For streaming dispatch, we need to change the signature to accept registry
    // This function generates the internal dispatch method body
    // The actual streaming dispatch happens through ServiceDispatcher::dispatch_streaming

    // Build the tuple type for decoding - streams become u64 for wire format
    let wire_arg_types: Vec<String> = method
        .args
        .iter()
        .map(|arg| {
            if is_tx(arg.ty) || is_rx(arg.ty) {
                // On wire, streams are just u64 stream IDs
                "u64".to_string()
            } else {
                rust_type_server_arg(arg.ty)
            }
        })
        .collect();

    let arg_names: Vec<String> = method.args.iter().map(|a| a.name.to_snake_case()).collect();

    // Decode the wire format (streams as u64)
    if method.args.is_empty() {
        dispatch_fn.line("// No arguments to decode");
    } else {
        dispatch_fn.line("// Decode arguments (streams encoded as u64 stream IDs)");
        let tuple_pattern = arg_names.join(", ");
        let tuple_type = wire_arg_types.join(", ");
        dispatch_fn.line(format!(
            "let ({tuple_pattern}) = match facet_postcard::from_slice::<({tuple_type})>(payload) {{"
        ));
        dispatch_fn.line("    Ok(args) => args,");
        dispatch_fn
            .line("    Err(_) => return vec![1, 2], // Result::Err, RoamError::InvalidPayload");
        dispatch_fn.line("};");
    }

    // Create channels for streaming args and register with registry
    // No inversion: handler signature matches schema exactly
    // - Rx<T>: server receives from client
    // - Tx<T>: server sends to client
    for (i, arg) in method.args.iter().enumerate() {
        let arg_name = arg.name.to_snake_case();
        if is_rx(arg.ty) {
            // Rx<T> = server receives from client
            // Create channel, register tx for incoming data routing, give handler Rx
            let inner_type = if let Some(inner) = arg.ty.type_params.first() {
                rust_type_base(inner.shape)
            } else {
                "()".to_string()
            };
            dispatch_fn.line(format!(
                "let ({arg_name}_tx, {arg_name}_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);"
            ));
            dispatch_fn.line(format!(
                "registry.register_incoming({arg_name}, {arg_name}_tx);"
            ));
            dispatch_fn.line(format!(
                "let {arg_name} = Rx::<{inner_type}>::new({arg_name}, {arg_name}_rx);"
            ));
        } else if is_tx(arg.ty) {
            // Tx<T> = server sends to client
            // Create channel, give handler Tx, need to drain rx and send as Data
            // TODO: The rx needs to be returned to driver for FuturesUnordered polling
            let inner_type = if let Some(inner) = arg.ty.type_params.first() {
                rust_type_base(inner.shape)
            } else {
                "()".to_string()
            };
            dispatch_fn.line(format!(
                "let ({arg_name}_tx, _{arg_name}_rx) = tokio::sync::mpsc::channel::<Vec<u8>>(64);"
            ));
            dispatch_fn.line(format!("registry.register_outgoing_credit({arg_name});"));
            dispatch_fn.line(format!(
                "let {arg_name} = Tx::<{inner_type}>::new({arg_name}, {arg_name}_tx);"
            ));
            // TODO: Need to return _{arg_name}_rx to the driver for polling
        }
        // Non-stream args are already decoded with correct types
        let _ = i; // suppress unused warning
    }

    // Call handler
    let args_str = arg_names.join(", ");
    dispatch_fn.line(format!(
        "match self.handler.{method_name}({args_str}).await {{"
    ));
    dispatch_fn.line("    Ok(result) => {");
    dispatch_fn.line("        let mut out = vec![0u8]; // Success marker");
    generate_encode_expr(dispatch_fn, "result", method.return_type, "        ");
    dispatch_fn.line("        out");
    dispatch_fn.line("    }");
    dispatch_fn.line("    Err(_e) => {");
    dispatch_fn.line("        // Encode RoamError - for now just return error marker");
    dispatch_fn.line("        vec![1, 1] // Result::Err, RoamError::UnknownMethod placeholder");
    dispatch_fn.line("    }");
    dispatch_fn.line("}");
}

/// Generate encode expression for a type.
fn generate_encode_expr(
    func: &mut codegen::Function,
    expr: &str,
    _shape: &'static Shape,
    indent: &str,
) {
    // Use postcard for encoding
    func.line(format!(
        "{indent}out.extend(facet_postcard::to_vec(&{expr}).expect(\"encode\"));"
    ));
}

/// Generate service code with default options.
pub fn generate_service(service: &ServiceDetail) -> String {
    generate_service_with_options(service, &RustCodegenOptions::default())
}

/// Generate service code with custom options.
pub fn generate_service_with_options(
    service: &ServiceDetail,
    options: &RustCodegenOptions,
) -> String {
    RustGenerator::new(service, options).generate()
}

/// Check if a shape is a stream (Tx or Rx).
fn is_stream(shape: &'static Shape) -> bool {
    is_tx(shape) || is_rx(shape)
}

/// Convert Shape to Rust type string for service trait arguments.
/// No inversion needed - Tx/Rx describe what the holder does:
/// - Tx<T>: holder sends data
/// - Rx<T>: holder receives data
fn rust_type_server_arg(shape: &'static Shape) -> String {
    match classify_shape(shape) {
        ShapeKind::Tx { inner } => format!("Tx<{}>", rust_type_base(inner)),
        ShapeKind::Rx { inner } => format!("Rx<{}>", rust_type_base(inner)),
        _ => rust_type_base(shape),
    }
}

/// Convert Shape to Rust type string for service trait returns.
/// No inversion needed - same as args.
fn rust_type_server_return(shape: &'static Shape) -> String {
    match classify_shape(shape) {
        ShapeKind::Tx { inner } => format!("Tx<{}>", rust_type_base(inner)),
        ShapeKind::Rx { inner } => format!("Rx<{}>", rust_type_base(inner)),
        _ => rust_type_base(shape),
    }
}

/// Convert Shape to base Rust type (non-streaming).
fn rust_type_base(shape: &'static Shape) -> String {
    // Check for bytes first (Vec<u8>)
    if is_bytes(shape) {
        return "::std::vec::Vec<u8>".into();
    }

    match classify_shape(shape) {
        ShapeKind::Scalar(scalar) => rust_scalar_type(scalar),
        ShapeKind::List { element } => format!("::std::vec::Vec<{}>", rust_type_base(element)),
        ShapeKind::Slice { element } => format!("&[{}]", rust_type_base(element)),
        ShapeKind::Option { inner } => {
            format!("::std::option::Option<{}>", rust_type_base(inner))
        }
        ShapeKind::Array { element, len } => format!("[{}; {}]", rust_type_base(element), len),
        ShapeKind::Map { key, value } => {
            format!(
                "::std::collections::HashMap<{}, {}>",
                rust_type_base(key),
                rust_type_base(value)
            )
        }
        ShapeKind::Set { element } => {
            format!("::std::collections::HashSet<{}>", rust_type_base(element))
        }
        ShapeKind::Tuple { elements } => {
            let inner = elements
                .iter()
                .map(|p| rust_type_base(p.shape))
                .collect::<Vec<_>>()
                .join(", ");
            format!("({inner})")
        }
        ShapeKind::Tx { inner } => {
            // Should be handled by caller-specific functions, but fallback
            format!("Tx<{}>", rust_type_base(inner))
        }
        ShapeKind::Rx { inner } => {
            // Should be handled by caller-specific functions, but fallback
            format!("Rx<{}>", rust_type_base(inner))
        }
        ShapeKind::Struct(StructInfo {
            name: Some(name),
            fields: _,
            ..
        }) => {
            // Named struct - prefix with super:: to access from within service module
            format!("super::{name}")
        }
        ShapeKind::Struct(StructInfo {
            name: None, fields, ..
        }) => {
            // Anonymous struct - represent as tuple
            let inner = fields
                .iter()
                .map(|f| rust_type_base(f.shape()))
                .collect::<Vec<_>>()
                .join(", ");
            format!("({inner})")
        }
        ShapeKind::Enum(EnumInfo {
            name: Some(name), ..
        }) => {
            // Named enum - prefix with super:: to access from within service module
            format!("super::{name}")
        }
        ShapeKind::Result { ok, err } => {
            format!(
                "::std::result::Result<{}, {}>",
                rust_type_base(ok),
                rust_type_base(err)
            )
        }
        ShapeKind::Enum(EnumInfo {
            name: None,
            variants,
        }) => {
            // Check for Result pattern: two variants Ok(T) and Err(E)
            if variants.len() == 2 {
                let ok_variant = variants.iter().find(|v| v.name == "Ok");
                let err_variant = variants.iter().find(|v| v.name == "Err");

                if let (Some(ok_v), Some(err_v)) = (ok_variant, err_variant)
                    && let (
                        VariantKind::Newtype { inner: ok_ty },
                        VariantKind::Newtype { inner: err_ty },
                    ) = (classify_variant(ok_v), classify_variant(err_v))
                {
                    return format!(
                        "::std::result::Result<{}, {}>",
                        rust_type_base(ok_ty),
                        rust_type_base(err_ty)
                    );
                }
            }
            // Other anonymous enum - represent structure (shouldn't happen in practice)
            let variant_names: Vec<_> = variants.iter().map(|v| v.name).collect();
            format!("/* enum({}) */", variant_names.join("|"))
        }
        ShapeKind::Pointer { pointee } => rust_type_base(pointee),
        ShapeKind::Opaque => "/* opaque */".into(),
    }
}

/// Convert ScalarType to Rust type string.
fn rust_scalar_type(scalar: ScalarType) -> String {
    match scalar {
        ScalarType::Bool => "bool".into(),
        ScalarType::U8 => "u8".into(),
        ScalarType::U16 => "u16".into(),
        ScalarType::U32 => "u32".into(),
        ScalarType::U64 => "u64".into(),
        ScalarType::U128 => "u128".into(),
        ScalarType::USize => "usize".into(),
        ScalarType::I8 => "i8".into(),
        ScalarType::I16 => "i16".into(),
        ScalarType::I32 => "i32".into(),
        ScalarType::I64 => "i64".into(),
        ScalarType::I128 => "i128".into(),
        ScalarType::ISize => "isize".into(),
        ScalarType::F32 => "f32".into(),
        ScalarType::F64 => "f64".into(),
        ScalarType::Char => "char".into(),
        ScalarType::Str | ScalarType::CowStr => "&str".into(),
        ScalarType::String => "::std::string::String".into(),
        ScalarType::Unit => "()".into(),
        _ => "/* unknown scalar */".into(),
    }
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use super::*;
    use facet::Facet;
    use roam_schema::{ArgDetail, MethodDetail, ServiceDetail};

    // ===========================================
    // rust_type_base tests for all type variants
    // ===========================================

    mod primitives {
        use super::*;

        #[test]
        fn bool_type() {
            assert_eq!(rust_type_base(<bool as Facet>::SHAPE), "bool");
        }

        #[test]
        fn unsigned_integers() {
            assert_eq!(rust_type_base(<u8 as Facet>::SHAPE), "u8");
            assert_eq!(rust_type_base(<u16 as Facet>::SHAPE), "u16");
            assert_eq!(rust_type_base(<u32 as Facet>::SHAPE), "u32");
            assert_eq!(rust_type_base(<u64 as Facet>::SHAPE), "u64");
            assert_eq!(rust_type_base(<u128 as Facet>::SHAPE), "u128");
        }

        #[test]
        fn signed_integers() {
            assert_eq!(rust_type_base(<i8 as Facet>::SHAPE), "i8");
            assert_eq!(rust_type_base(<i16 as Facet>::SHAPE), "i16");
            assert_eq!(rust_type_base(<i32 as Facet>::SHAPE), "i32");
            assert_eq!(rust_type_base(<i64 as Facet>::SHAPE), "i64");
            assert_eq!(rust_type_base(<i128 as Facet>::SHAPE), "i128");
        }

        #[test]
        fn floats() {
            assert_eq!(rust_type_base(<f32 as Facet>::SHAPE), "f32");
            assert_eq!(rust_type_base(<f64 as Facet>::SHAPE), "f64");
        }

        #[test]
        fn char_type() {
            assert_eq!(rust_type_base(<char as Facet>::SHAPE), "char");
        }

        #[test]
        fn string_type() {
            assert_eq!(
                rust_type_base(<String as Facet>::SHAPE),
                "::std::string::String"
            );
        }

        #[test]
        fn unit_type() {
            assert_eq!(rust_type_base(<() as Facet>::SHAPE), "()");
        }

        #[test]
        fn bytes_type() {
            assert_eq!(
                rust_type_base(<Vec<u8> as Facet>::SHAPE),
                "::std::vec::Vec<u8>"
            );
        }
    }

    mod containers {
        use super::*;

        #[test]
        fn list_of_primitives() {
            assert_eq!(
                rust_type_base(<Vec<i32> as Facet>::SHAPE),
                "::std::vec::Vec<i32>"
            );
        }

        #[test]
        fn list_of_strings() {
            assert_eq!(
                rust_type_base(<Vec<String> as Facet>::SHAPE),
                "::std::vec::Vec<::std::string::String>"
            );
        }

        #[test]
        fn option_of_primitive() {
            assert_eq!(
                rust_type_base(<Option<u64> as Facet>::SHAPE),
                "::std::option::Option<u64>"
            );
        }

        #[test]
        fn option_of_string() {
            assert_eq!(
                rust_type_base(<Option<String> as Facet>::SHAPE),
                "::std::option::Option<::std::string::String>"
            );
        }

        #[test]
        fn array_type() {
            assert_eq!(rust_type_base(<[u8; 32] as Facet>::SHAPE), "[u8; 32]");
        }

        #[test]
        fn map_type() {
            use std::collections::HashMap;
            assert_eq!(
                rust_type_base(<HashMap<String, i32> as Facet>::SHAPE),
                "::std::collections::HashMap<::std::string::String, i32>"
            );
        }

        #[test]
        fn set_type() {
            use std::collections::HashSet;
            assert_eq!(
                rust_type_base(<HashSet<String> as Facet>::SHAPE),
                "::std::collections::HashSet<::std::string::String>"
            );
        }

        #[test]
        fn tuple_type() {
            assert_eq!(
                rust_type_base(<(u32, String, bool) as Facet>::SHAPE),
                "(u32, ::std::string::String, bool)"
            );
        }

        #[test]
        fn empty_tuple() {
            assert_eq!(rust_type_base(<() as Facet>::SHAPE), "()");
        }

        #[test]
        fn single_element_tuple() {
            // Note: In Rust, (i32,) is a 1-tuple, but <(i32,) as Facet>::SHAPE
            // gives us a tuple type. The output format depends on facet's representation.
            assert_eq!(rust_type_base(<(i32,) as Facet>::SHAPE), "(i32)");
        }
    }

    mod composite_types {
        use super::*;

        #[derive(Facet)]
        struct MyStruct {
            x: i32,
            y: i32,
        }

        #[test]
        fn named_struct() {
            // Uses super:: because generated code is inside a module
            assert_eq!(
                rust_type_base(<MyStruct as Facet>::SHAPE),
                "super::MyStruct"
            );
        }

        #[derive(Facet)]
        #[repr(u8)]
        #[allow(dead_code)]
        enum MyEnum {
            A,
            B(i32),
        }

        #[test]
        fn named_enum() {
            // Uses super:: because generated code is inside a module
            assert_eq!(rust_type_base(<MyEnum as Facet>::SHAPE), "super::MyEnum");
        }

        #[test]
        fn result_pattern_recognized() {
            // Result<T, E> should be recognized
            assert_eq!(
                rust_type_base(<Result<String, i32> as Facet>::SHAPE),
                "::std::result::Result<::std::string::String, i32>"
            );
        }

        #[test]
        fn result_with_complex_types() {
            assert_eq!(
                rust_type_base(<Result<Vec<u8>, String> as Facet>::SHAPE),
                "::std::result::Result<::std::vec::Vec<u8>, ::std::string::String>"
            );
        }

        #[derive(Facet)]
        struct MyError {
            message: String,
        }

        #[test]
        fn result_with_named_error_type() {
            // MyError uses super:: because generated code is inside a module
            assert_eq!(
                rust_type_base(<Result<(), MyError> as Facet>::SHAPE),
                "::std::result::Result<(), super::MyError>"
            );
        }
    }

    mod nested_types {
        use super::*;

        #[test]
        fn vec_of_option() {
            assert_eq!(
                rust_type_base(<Vec<Option<i32>> as Facet>::SHAPE),
                "::std::vec::Vec<::std::option::Option<i32>>"
            );
        }

        #[test]
        fn option_of_vec() {
            assert_eq!(
                rust_type_base(<Option<Vec<String>> as Facet>::SHAPE),
                "::std::option::Option<::std::vec::Vec<::std::string::String>>"
            );
        }

        #[test]
        fn map_of_vec_to_option() {
            use std::collections::HashMap;
            assert_eq!(
                rust_type_base(<HashMap<String, Option<Vec<u8>>> as Facet>::SHAPE),
                "::std::collections::HashMap<::std::string::String, ::std::option::Option<::std::vec::Vec<u8>>>"
            );
        }

        #[derive(Facet)]
        struct Item {
            id: u32,
        }

        #[test]
        fn vec_of_named_struct() {
            assert_eq!(
                rust_type_base(<Vec<Item> as Facet>::SHAPE),
                "::std::vec::Vec<super::Item>"
            );
        }

        #[derive(Facet)]
        #[repr(u8)]
        enum Status {
            Active,
            Inactive,
        }

        #[test]
        fn option_of_named_enum() {
            assert_eq!(
                rust_type_base(<Option<Status> as Facet>::SHAPE),
                "::std::option::Option<super::Status>"
            );
        }

        #[test]
        fn tuple_of_mixed_types() {
            #[derive(Facet)]
            struct Point {
                x: i32,
                y: i32,
            }
            assert_eq!(
                rust_type_base(<(u32, Option<String>, Point) as Facet>::SHAPE),
                "(u32, ::std::option::Option<::std::string::String>, super::Point)"
            );
        }
    }

    // ===========================================
    // Service generation tests
    // ===========================================

    fn sample_service() -> ServiceDetail {
        ServiceDetail {
            name: Cow::Borrowed("Calculator"),
            doc: Some(Cow::Borrowed("A simple calculator service.")),
            methods: vec![MethodDetail {
                service_name: Cow::Borrowed("Calculator"),
                method_name: Cow::Borrowed("add"),
                args: vec![
                    ArgDetail {
                        name: Cow::Borrowed("a"),
                        ty: <i32 as Facet>::SHAPE,
                    },
                    ArgDetail {
                        name: Cow::Borrowed("b"),
                        ty: <i32 as Facet>::SHAPE,
                    },
                ],
                return_type: <i32 as Facet>::SHAPE,
                doc: Some(Cow::Borrowed("Add two numbers.")),
            }],
        }
    }

    #[test]
    fn test_generate_service() {
        let service = sample_service();
        let code = generate_service(&service);

        // Service trait is named after the service (not CallerCaller/Handler)
        assert!(code.contains("pub trait Calculator"));
        assert!(code.contains("CalculatorDispatcher"));
        assert!(code.contains("fn add("));
        assert!(code.contains("pub mod method_id"));
        assert!(code.contains("pub const ADD: u64"));
    }

    #[test]
    fn test_multiline_doc_comments() {
        let service = ServiceDetail {
            name: Cow::Borrowed("MultiDoc"),
            doc: Some(Cow::Borrowed("First line.\nSecond line.\nThird line.")),
            methods: vec![MethodDetail {
                service_name: Cow::Borrowed("MultiDoc"),
                method_name: Cow::Borrowed("test_method"),
                args: vec![],
                return_type: <() as Facet>::SHAPE,
                doc: Some(Cow::Borrowed("Method first line.\nMethod second line.")),
            }],
        };

        let code = generate_service(&service);

        // Verify doc comments are present (exact format depends on codegen library)
        assert!(code.contains("First line"), "Service doc should be present");
    }
}
