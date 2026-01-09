use std::collections::HashSet;

use facet_core::{ScalarType, Shape, StructKind};
use heck::{ToLowerCamelCase, ToUpperCamelCase};
use roam_schema::{
    EnumInfo, MethodDetail, ServiceDetail, ShapeKind, StructInfo, VariantKind, classify_shape,
    classify_variant, is_bytes, is_rx, is_tx,
};

use crate::render::{fq_name, hex_u64};

/// Generate TypeScript field access expression.
/// Uses bracket notation for numeric field names (tuple fields), dot notation otherwise.
fn ts_field_access(expr: &str, field_name: &str) -> String {
    if field_name
        .chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit())
    {
        format!("{expr}[{field_name}]")
    } else {
        format!("{expr}.{field_name}")
    }
}

/// Collect all named types (structs and enums with a name) from a service.
/// Returns a set of (name, Shape) pairs.
fn collect_named_types(service: &ServiceDetail) -> Vec<(String, &'static Shape)> {
    let mut seen = HashSet::new();
    let mut types = Vec::new();

    fn visit(
        shape: &'static Shape,
        seen: &mut HashSet<String>,
        types: &mut Vec<(String, &'static Shape)>,
    ) {
        match classify_shape(shape) {
            ShapeKind::Struct(StructInfo {
                name: Some(name),
                fields,
                ..
            }) => {
                if !seen.contains(name) {
                    seen.insert(name.to_string());
                    // Visit nested types first (dependencies before dependents)
                    for field in fields {
                        visit(field.shape(), seen, types);
                    }
                    types.push((name.to_string(), shape));
                }
            }
            ShapeKind::Enum(EnumInfo {
                name: Some(name),
                variants,
            }) => {
                if !seen.contains(name) {
                    seen.insert(name.to_string());
                    // Visit nested types in variants
                    for variant in variants {
                        match classify_variant(variant) {
                            VariantKind::Newtype { inner } => visit(inner, seen, types),
                            VariantKind::Struct { fields } | VariantKind::Tuple { fields } => {
                                for field in fields {
                                    visit(field.shape(), seen, types);
                                }
                            }
                            VariantKind::Unit => {}
                        }
                    }
                    types.push((name.to_string(), shape));
                }
            }
            ShapeKind::List { element } => visit(element, seen, types),
            ShapeKind::Option { inner } => visit(inner, seen, types),
            ShapeKind::Array { element, .. } => visit(element, seen, types),
            ShapeKind::Map { key, value } => {
                visit(key, seen, types);
                visit(value, seen, types);
            }
            ShapeKind::Set { element } => visit(element, seen, types),
            ShapeKind::Tuple { elements } => {
                for param in elements {
                    visit(param.shape, seen, types);
                }
            }
            ShapeKind::Tx { inner } | ShapeKind::Rx { inner } => visit(inner, seen, types),
            ShapeKind::Pointer { pointee } => visit(pointee, seen, types),
            // Scalars, slices, opaque - no named types to collect
            _ => {}
        }
    }

    for method in &service.methods {
        for arg in &method.args {
            visit(arg.ty, &mut seen, &mut types);
        }
        visit(method.return_type, &mut seen, &mut types);
    }

    types
}

/// Generate TypeScript type definitions for all named types.
fn generate_named_types(named_types: &[(String, &'static Shape)]) -> String {
    let mut out = String::new();

    if named_types.is_empty() {
        return out;
    }

    out.push_str("// Named type definitions\n");

    for (name, shape) in named_types {
        match classify_shape(shape) {
            ShapeKind::Struct(StructInfo { fields, .. }) => {
                out.push_str(&format!("export interface {} {{\n", name));
                for field in fields {
                    out.push_str(&format!(
                        "  {}: {};\n",
                        field.name,
                        ts_type_base_named(field.shape())
                    ));
                }
                out.push_str("}\n\n");
            }
            ShapeKind::Enum(EnumInfo { variants, .. }) => {
                out.push_str(&format!("export type {} =\n", name));
                for (i, variant) in variants.iter().enumerate() {
                    let variant_type = match classify_variant(variant) {
                        VariantKind::Unit => format!("{{ tag: '{}' }}", variant.name),
                        VariantKind::Newtype { inner } => {
                            format!(
                                "{{ tag: '{}'; value: {} }}",
                                variant.name,
                                ts_type_base_named(inner)
                            )
                        }
                        VariantKind::Tuple { fields } | VariantKind::Struct { fields } => {
                            let field_strs = fields
                                .iter()
                                .map(|f| format!("{}: {}", f.name, ts_type_base_named(f.shape())))
                                .collect::<Vec<_>>()
                                .join("; ");
                            format!("{{ tag: '{}'; {} }}", variant.name, field_strs)
                        }
                    };
                    let sep = if i < variants.len() - 1 { "" } else { ";" };
                    out.push_str(&format!("  | {}{}\n", variant_type, sep));
                }
                out.push('\n');
            }
            _ => {}
        }
    }

    out
}

/// Convert Shape to TypeScript type string, using named types when available.
/// This handles container types recursively, using named types at every level.
fn ts_type_base_named(shape: &'static Shape) -> String {
    match classify_shape(shape) {
        // Named types - use the name directly
        ShapeKind::Struct(StructInfo {
            name: Some(name), ..
        }) => name.to_string(),
        ShapeKind::Enum(EnumInfo {
            name: Some(name), ..
        }) => name.to_string(),

        // Container types - recurse with ts_type_base_named
        ShapeKind::List { element } => {
            // Check for bytes first
            if is_bytes(shape) {
                return "Uint8Array".into();
            }
            // Wrap in parens if inner is an anonymous enum to avoid precedence issues
            if matches!(
                classify_shape(element),
                ShapeKind::Enum(EnumInfo { name: None, .. })
            ) {
                format!("({})[]", ts_type_base_named(element))
            } else {
                format!("{}[]", ts_type_base_named(element))
            }
        }
        ShapeKind::Option { inner } => format!("{} | null", ts_type_base_named(inner)),
        ShapeKind::Array { element, len } => format!("[{}; {}]", ts_type_base_named(element), len),
        ShapeKind::Map { key, value } => {
            format!(
                "Map<{}, {}>",
                ts_type_base_named(key),
                ts_type_base_named(value)
            )
        }
        ShapeKind::Set { element } => format!("Set<{}>", ts_type_base_named(element)),
        ShapeKind::Tuple { elements } => {
            let inner = elements
                .iter()
                .map(|p| ts_type_base_named(p.shape))
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{inner}]")
        }
        ShapeKind::Tx { inner } => format!("Tx<{}>", ts_type_base_named(inner)),
        ShapeKind::Rx { inner } => format!("Rx<{}>", ts_type_base_named(inner)),

        // Anonymous structs - inline as object type
        ShapeKind::Struct(StructInfo {
            name: None, fields, ..
        }) => {
            let inner = fields
                .iter()
                .map(|f| format!("{}: {}", f.name, ts_type_base_named(f.shape())))
                .collect::<Vec<_>>()
                .join("; ");
            format!("{{ {inner} }}")
        }

        // Anonymous enums - inline as union type
        ShapeKind::Enum(EnumInfo {
            name: None,
            variants,
        }) => variants
            .iter()
            .map(|v| match classify_variant(v) {
                VariantKind::Unit => format!("{{ tag: '{}' }}", v.name),
                VariantKind::Newtype { inner } => {
                    format!(
                        "{{ tag: '{}'; value: {} }}",
                        v.name,
                        ts_type_base_named(inner)
                    )
                }
                VariantKind::Tuple { fields } | VariantKind::Struct { fields } => {
                    let field_strs = fields
                        .iter()
                        .map(|f| format!("{}: {}", f.name, ts_type_base_named(f.shape())))
                        .collect::<Vec<_>>()
                        .join("; ");
                    format!("{{ tag: '{}'; {} }}", v.name, field_strs)
                }
            })
            .collect::<Vec<_>>()
            .join(" | "),

        // Scalars and other types
        ShapeKind::Scalar(scalar) => ts_scalar_type(scalar),
        ShapeKind::Slice { element } => format!("{}[]", ts_type_base_named(element)),
        ShapeKind::Pointer { pointee } => ts_type_base_named(pointee),
        ShapeKind::Result { ok, err } => {
            format!(
                "{{ ok: true; value: {} }} | {{ ok: false; error: {} }}",
                ts_type_base_named(ok),
                ts_type_base_named(err)
            )
        }
        ShapeKind::Opaque => "unknown".into(),
    }
}

/// Convert ScalarType to TypeScript type string.
fn ts_scalar_type(scalar: ScalarType) -> String {
    match scalar {
        ScalarType::Bool => "boolean".into(),
        ScalarType::U8
        | ScalarType::U16
        | ScalarType::U32
        | ScalarType::I8
        | ScalarType::I16
        | ScalarType::I32
        | ScalarType::F32
        | ScalarType::F64 => "number".into(),
        ScalarType::U64
        | ScalarType::U128
        | ScalarType::I64
        | ScalarType::I128
        | ScalarType::USize
        | ScalarType::ISize => "bigint".into(),
        ScalarType::Char | ScalarType::Str | ScalarType::String | ScalarType::CowStr => {
            "string".into()
        }
        ScalarType::Unit => "void".into(),
        _ => "unknown".into(),
    }
}

pub fn generate_method_ids(methods: &[MethodDetail]) -> String {
    let mut items = methods
        .iter()
        .map(|m| (fq_name(m), crate::method_id(m)))
        .collect::<Vec<_>>();
    items.sort_by(|a, b| a.0.cmp(&b.0));

    let mut out = String::new();
    out.push_str("// @generated by roam-codegen\n");
    out.push_str("// This file defines canonical roam method IDs.\n\n");
    out.push_str("export const METHOD_ID: Record<string, bigint> = {\n");
    for (name, id) in items {
        out.push_str(&format!("  \"{name}\": {}n,\n", hex_u64(id)));
    }
    out.push_str("} as const;\n");
    out
}

/// Generate a complete TypeScript module for a service.
///
/// r[impl codegen.typescript.service] - Generate client, server, and method IDs.
pub fn generate_service(service: &ServiceDetail) -> String {
    let mut out = String::new();
    out.push_str("// @generated by roam-codegen\n");
    out.push_str("// DO NOT EDIT - regenerate with `cargo xtask codegen --typescript`\n\n");

    // Import runtime primitives from @bearcove/roam-core
    out.push_str("import type { MethodHandler, Connection, MessageTransport, DecodeResult } from \"@bearcove/roam-core\";\n");
    out.push_str("import {\n");
    out.push_str("  encodeResultOk, encodeResultErr, encodeInvalidPayload,\n");
    out.push_str("  concat, encodeVarint, decodeVarintNumber, decodeRpcResult,\n");
    out.push_str("  encodeBool, decodeBool,\n");
    out.push_str("  encodeU8, decodeU8, encodeI8, decodeI8,\n");
    out.push_str("  encodeU16, decodeU16, encodeI16, decodeI16,\n");
    out.push_str("  encodeU32, decodeU32, encodeI32, decodeI32,\n");
    out.push_str("  encodeU64, decodeU64, encodeI64, decodeI64,\n");
    out.push_str("  encodeF32, decodeF32, encodeF64, decodeF64,\n");
    out.push_str("  encodeString, decodeString,\n");
    out.push_str("  encodeBytes, decodeBytes,\n");
    out.push_str("  encodeOption, decodeOption,\n");
    out.push_str("  encodeVec, decodeVec,\n");
    out.push_str("  encodeTuple2, decodeTuple2, encodeTuple3, decodeTuple3,\n");
    out.push_str("  encodeEnumVariant, decodeEnumVariant,\n");
    out.push_str("} from \"@bearcove/roam-core\";\n");

    // Check if any method uses streaming
    let has_streaming = service.methods.iter().any(|m| {
        m.args.iter().any(|a| is_tx(a.ty) || is_rx(a.ty))
            || is_tx(m.return_type)
            || is_rx(m.return_type)
    });

    if has_streaming {
        out.push_str("import { Tx, Rx } from \"@bearcove/roam-core\";\n");
        out.push_str("import type { StreamId } from \"@bearcove/roam-core\";\n");
    }
    out.push('\n');

    // Generate method IDs
    out.push_str("export const METHOD_ID = {\n");
    for method in &service.methods {
        let id = crate::method_id(method);
        let method_name = method.method_name.to_lower_camel_case();
        out.push_str(&format!("  {method_name}: {}n,\n", hex_u64(id)));
    }
    out.push_str("} as const;\n\n");

    // Collect and generate named types (structs and enums)
    let named_types = collect_named_types(service);
    out.push_str(&generate_named_types(&named_types));

    // Generate type aliases for request/response types
    out.push_str(&generate_types(service));

    // Generate caller interface (for making calls)
    out.push_str(&generate_caller_interface(service));

    // Generate client implementation (for making calls)
    out.push_str(&generate_client_impl(service));

    // Generate handler interface (for handling calls)
    out.push_str(&generate_handler_interface(service));

    out
}

fn generate_types(service: &ServiceDetail) -> String {
    let mut out = String::new();
    out.push_str("// Type definitions\n");

    for method in &service.methods {
        let method_name = method.method_name.to_upper_camel_case();

        // Request type (tuple of args)
        if method.args.is_empty() {
            out.push_str(&format!("export type {method_name}Request = [];\n"));
        } else if method.args.len() == 1 {
            let ty = ts_type(method.args[0].ty);
            out.push_str(&format!("export type {method_name}Request = [{ty}];\n"));
        } else {
            out.push_str(&format!("export type {method_name}Request = [\n"));
            for arg in &method.args {
                let ty = ts_type(arg.ty);
                out.push_str(&format!("  {ty}, // {}\n", arg.name));
            }
            out.push_str("];\n");
        }

        // Response type
        let ret_ty = ts_type(method.return_type);
        out.push_str(&format!(
            "export type {method_name}Response = {ret_ty};\n\n"
        ));
    }

    out
}

/// Generate caller interface (for making calls to the service).
///
/// r[impl streaming.caller-pov] - Caller uses Tx for args, Rx for returns.
fn generate_caller_interface(service: &ServiceDetail) -> String {
    let mut out = String::new();
    let service_name = service.name.to_upper_camel_case();

    out.push_str(&format!("// Caller interface for {service_name}\n"));
    out.push_str(&format!("export interface {service_name}Caller {{\n"));

    for method in &service.methods {
        let method_name = method.method_name.to_lower_camel_case();
        // Caller args: Tx stays Tx, Rx stays Rx
        let args = method
            .args
            .iter()
            .map(|a| {
                format!(
                    "{}: {}",
                    a.name.to_lower_camel_case(),
                    ts_type_client_arg(a.ty)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        // Caller returns
        let ret_ty = ts_type_client_return(method.return_type);

        if let Some(doc) = &method.doc {
            out.push_str(&format!("  /** {} */\n", doc));
        }
        out.push_str(&format!("  {method_name}({args}): Promise<{ret_ty}>;\n"));
    }

    out.push_str("}\n\n");
    out
}

/// Generate client implementation (for making calls to the service).
///
/// r[impl codegen.typescript.client] - Generate client class with method implementations.
fn generate_client_impl(service: &ServiceDetail) -> String {
    let mut out = String::new();
    let service_name = service.name.to_upper_camel_case();

    out.push_str(&format!("// Client implementation for {service_name}\n"));
    out.push_str(&format!(
        "export class {service_name}Client<T extends MessageTransport = MessageTransport> implements {service_name}Caller {{\n"
    ));
    out.push_str("  private conn: Connection<T>;\n\n");
    out.push_str("  constructor(conn: Connection<T>) {\n");
    out.push_str("    this.conn = conn;\n");
    out.push_str("  }\n\n");

    for method in &service.methods {
        let method_name = method.method_name.to_lower_camel_case();
        let id = crate::method_id(method);

        // Build args list
        let args = method
            .args
            .iter()
            .map(|a| {
                format!(
                    "{}: {}",
                    a.name.to_lower_camel_case(),
                    ts_type_client_arg(a.ty)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");

        // Return type
        let ret_ty = ts_type_client_return(method.return_type);

        // Check if we can generate encoding/decoding for this method
        let can_encode_args = method.args.iter().all(|a| is_fully_supported(a.ty));
        let can_decode_return = is_fully_supported(method.return_type);

        if let Some(doc) = &method.doc {
            out.push_str(&format!("  /** {} */\n", doc));
        }
        out.push_str(&format!(
            "  async {method_name}({args}): Promise<{ret_ty}> {{\n"
        ));

        if can_encode_args && can_decode_return {
            // Generate payload encoding
            if method.args.is_empty() {
                out.push_str("    const payload = new Uint8Array(0);\n");
            } else if method.args.len() == 1 {
                let arg_name = method.args[0].name.to_lower_camel_case();
                let encode_expr = generate_encode_expr(method.args[0].ty, &arg_name);
                out.push_str(&format!("    const payload = {encode_expr};\n"));
            } else {
                // Multiple args - concat their encodings
                let parts: Vec<_> = method
                    .args
                    .iter()
                    .map(|a| {
                        let arg_name = a.name.to_lower_camel_case();
                        generate_encode_expr(a.ty, &arg_name)
                    })
                    .collect();
                out.push_str(&format!(
                    "    const payload = concat({});\n",
                    parts.join(", ")
                ));
            }

            // Call the server
            out.push_str(&format!(
                "    const response = await this.conn.call({}n, payload);\n",
                hex_u64(id)
            ));

            // Parse the result (CallResult<T, RoamError>) - throws RpcError on failure
            out.push_str("    const buf = response;\n");
            out.push_str("    let offset = decodeRpcResult(buf, 0);\n");
            // For client returns, use client-aware decode
            let decode_stmt = generate_decode_stmt_client(method.return_type, "result", "offset");
            out.push_str(&format!("    {decode_stmt}\n"));
            out.push_str("    return result;\n");
        } else {
            // Streaming or unsupported - throw error
            out.push_str(
                "    throw new Error(\"Not yet implemented: encoding/decoding for this method\");\n",
            );
        }

        out.push_str("  }\n\n");
    }

    out.push_str("}\n\n");
    out
}

/// Generate handler interface (for handling incoming calls).
///
/// r[impl streaming.caller-pov] - Handler uses Rx for args, Tx for returns.
fn generate_handler_interface(service: &ServiceDetail) -> String {
    let mut out = String::new();
    let service_name = service.name.to_upper_camel_case();

    out.push_str(&format!("// Handler interface for {service_name}\n"));
    out.push_str(&format!("export interface {service_name}Handler {{\n"));

    for method in &service.methods {
        let method_name = method.method_name.to_lower_camel_case();
        // Handler args: Tx becomes Rx (receives), Rx becomes Tx (sends)
        let args = method
            .args
            .iter()
            .map(|a| {
                format!(
                    "{}: {}",
                    a.name.to_lower_camel_case(),
                    ts_type_server_arg(a.ty)
                )
            })
            .collect::<Vec<_>>()
            .join(", ");
        // Handler returns
        let ret_ty = ts_type_server_return(method.return_type);

        out.push_str(&format!(
            "  {method_name}({args}): Promise<{ret_ty}> | {ret_ty};\n"
        ));
    }

    out.push_str("}\n\n");

    // Generate method handlers map
    out.push_str(&format!("// Method handlers for {service_name}\n"));
    out.push_str(&format!("export const {}_methodHandlers = new Map<bigint, MethodHandler<{service_name}Handler>>([\n", service_name.to_lower_camel_case()));

    for method in &service.methods {
        let method_name = method.method_name.to_lower_camel_case();
        let id = crate::method_id(method);

        out.push_str(&format!(
            "  [{}n, async (handler, payload) => {{\n",
            hex_u64(id)
        ));
        out.push_str("    try {\n");

        // Check if we can fully implement this method
        let can_decode_args = method.args.iter().all(|a| is_fully_supported(a.ty));
        let can_encode_return = is_fully_supported(method.return_type);

        if can_decode_args && can_encode_return {
            // Decode all arguments
            out.push_str("      const buf = payload;\n");
            out.push_str("      let offset = 0;\n");
            for arg in &method.args {
                let arg_name = arg.name.to_lower_camel_case();
                // Use server-side decode
                let decode_stmt = generate_decode_stmt_server(arg.ty, &arg_name, "offset");
                out.push_str(&format!("      {decode_stmt}\n"));
            }
            out.push_str(
                "      if (offset !== buf.length) throw new Error(\"args: trailing bytes\");\n",
            );

            // Call handler
            let arg_names = method
                .args
                .iter()
                .map(|a| a.name.to_lower_camel_case())
                .collect::<Vec<_>>()
                .join(", ");
            out.push_str(&format!(
                "      const result = await handler.{method_name}({arg_names});\n"
            ));

            // Encode response
            let encode_expr = generate_encode_expr(method.return_type, "result");
            out.push_str(&format!("      return encodeResultOk({encode_expr});\n"));
        } else {
            // Streaming or unsupported - return error
            out.push_str("      // TODO: implement encoding/decoding for streaming types\n");
            out.push_str("      return encodeResultErr(encodeInvalidPayload());\n");
        }

        out.push_str("    } catch (e) {\n");
        out.push_str("      return encodeResultErr(encodeInvalidPayload());\n");
        out.push_str("    }\n");
        out.push_str("  }],\n");
    }

    out.push_str("]);\n");

    out
}

/// Convert Shape to TypeScript type string for client arguments.
/// Schema is from server's perspective - no inversion needed.
/// Client passes the same types that server receives.
fn ts_type_client_arg(shape: &'static Shape) -> String {
    match classify_shape(shape) {
        ShapeKind::Tx { inner } => format!("Tx<{}>", ts_type_client_arg(inner)),
        ShapeKind::Rx { inner } => format!("Rx<{}>", ts_type_client_arg(inner)),
        _ => ts_type_base_named(shape),
    }
}

/// Convert Shape to TypeScript type string for client returns.
/// Schema is from server's perspective - no inversion needed.
fn ts_type_client_return(shape: &'static Shape) -> String {
    match classify_shape(shape) {
        ShapeKind::Tx { inner } => format!("Tx<{}>", ts_type_client_return(inner)),
        ShapeKind::Rx { inner } => format!("Rx<{}>", ts_type_client_return(inner)),
        _ => ts_type_base_named(shape),
    }
}

/// Convert Shape to TypeScript type string for server/handler arguments.
/// Schema is from server's perspective - no inversion needed.
/// Rx means server receives, Tx means server sends.
fn ts_type_server_arg(shape: &'static Shape) -> String {
    match classify_shape(shape) {
        ShapeKind::Tx { inner } => format!("Tx<{}>", ts_type_server_arg(inner)),
        ShapeKind::Rx { inner } => format!("Rx<{}>", ts_type_server_arg(inner)),
        _ => ts_type_base_named(shape),
    }
}

/// Schema is from server's perspective - no inversion needed.
fn ts_type_server_return(shape: &'static Shape) -> String {
    match classify_shape(shape) {
        ShapeKind::Tx { inner } => format!("Tx<{}>", ts_type_server_return(inner)),
        ShapeKind::Rx { inner } => format!("Rx<{}>", ts_type_server_return(inner)),
        _ => ts_type_base_named(shape),
    }
}

/// TypeScript type for user-facing type definitions.
/// Uses named types when available.
fn ts_type(shape: &'static Shape) -> String {
    ts_type_base_named(shape)
}

// ============================================================================
// Encoding/Decoding code generation
// ============================================================================

/// Generate a TypeScript expression that encodes a value of the given type.
/// `expr` is the JavaScript expression to encode.
fn generate_encode_expr(shape: &'static Shape, expr: &str) -> String {
    // Check for bytes first (Vec<u8>)
    if is_bytes(shape) {
        return format!("encodeBytes({expr})");
    }

    match classify_shape(shape) {
        ShapeKind::Scalar(scalar) => encode_scalar_expr(scalar, expr),
        ShapeKind::List { element } => {
            let item_encode = generate_encode_expr(element, "item");
            format!("encodeVec({expr}, (item) => {item_encode})")
        }
        ShapeKind::Option { inner } => {
            let inner_encode = generate_encode_expr(inner, "v");
            format!("encodeOption({expr}, (v) => {inner_encode})")
        }
        ShapeKind::Array { element, .. } => {
            // Encode as vec for now
            let item_encode = generate_encode_expr(element, "item");
            format!("encodeVec({expr}, (item) => {item_encode})")
        }
        ShapeKind::Slice { element } => {
            let item_encode = generate_encode_expr(element, "item");
            format!("encodeVec({expr}, (item) => {item_encode})")
        }
        ShapeKind::Map { key, value } => {
            // Encode as vec of tuples
            let k_enc = generate_encode_expr(key, "k");
            let v_enc = generate_encode_expr(value, "v");
            format!("encodeVec(Array.from({expr}.entries()), ([k, v]) => concat({k_enc}, {v_enc}))")
        }
        ShapeKind::Set { element } => {
            let item_encode = generate_encode_expr(element, "item");
            format!("encodeVec(Array.from({expr}), (item) => {item_encode})")
        }
        ShapeKind::Tuple { elements } => {
            if elements.len() == 2 {
                let a_enc = generate_encode_expr(elements[0].shape, &format!("{expr}[0]"));
                let b_enc = generate_encode_expr(elements[1].shape, &format!("{expr}[1]"));
                format!("concat({a_enc}, {b_enc})")
            } else if elements.len() == 3 {
                let a_enc = generate_encode_expr(elements[0].shape, &format!("{expr}[0]"));
                let b_enc = generate_encode_expr(elements[1].shape, &format!("{expr}[1]"));
                let c_enc = generate_encode_expr(elements[2].shape, &format!("{expr}[2]"));
                format!("concat({a_enc}, {b_enc}, {c_enc})")
            } else if elements.is_empty() {
                "new Uint8Array(0)".into()
            } else {
                // Fallback: concat all
                let parts: Vec<_> = elements
                    .iter()
                    .enumerate()
                    .map(|(i, p)| generate_encode_expr(p.shape, &format!("{expr}[{i}]")))
                    .collect();
                format!("concat({})", parts.join(", "))
            }
        }
        ShapeKind::Struct(StructInfo { fields, kind, .. }) => {
            if fields.is_empty() || kind == StructKind::Unit {
                "new Uint8Array(0)".into()
            } else {
                let parts: Vec<_> = fields
                    .iter()
                    .map(|f| generate_encode_expr(f.shape(), &ts_field_access(expr, f.name)))
                    .collect();
                format!("concat({})", parts.join(", "))
            }
        }
        ShapeKind::Enum(EnumInfo { variants, .. }) => {
            // Generate switch on tag
            let mut cases = String::new();
            for (i, v) in variants.iter().enumerate() {
                cases.push_str(&format!("      case '{}': ", v.name));
                match classify_variant(v) {
                    VariantKind::Unit => {
                        cases.push_str(&format!("return encodeEnumVariant({i});\n"));
                    }
                    VariantKind::Newtype { inner } => {
                        let inner_enc = generate_encode_expr(inner, &format!("{expr}.value"));
                        cases.push_str(&format!(
                            "return concat(encodeEnumVariant({i}), {inner_enc});\n"
                        ));
                    }
                    VariantKind::Tuple { fields } | VariantKind::Struct { fields } => {
                        let field_encs: Vec<_> = fields
                            .iter()
                            .map(|f| {
                                generate_encode_expr(f.shape(), &ts_field_access(expr, f.name))
                            })
                            .collect();
                        cases.push_str(&format!(
                            "return concat(encodeEnumVariant({i}), {});\n",
                            field_encs.join(", ")
                        ));
                    }
                }
            }
            format!(
                "(() => {{ switch ({expr}.tag) {{\n{cases}      default: throw new Error('unknown enum variant'); }} }})()"
            )
        }
        ShapeKind::Tx { .. } | ShapeKind::Rx { .. } => {
            // Streaming types encode as u64 stream ID (varint)
            // r[impl streaming.type] - Tx/Rx serialize as channel_id on wire.
            format!("encodeU64({expr}.streamId)")
        }
        ShapeKind::Pointer { pointee } => generate_encode_expr(pointee, expr),
        ShapeKind::Result { .. } => {
            "/* Result type encoding not yet implemented */ new Uint8Array(0)".to_string()
        }
        ShapeKind::Opaque => "/* unsupported type */ new Uint8Array(0)".to_string(),
    }
}

/// Generate encode expression for scalar types.
fn encode_scalar_expr(scalar: ScalarType, expr: &str) -> String {
    match scalar {
        ScalarType::Bool => format!("encodeBool({expr})"),
        ScalarType::U8 => format!("encodeU8({expr})"),
        ScalarType::I8 => format!("encodeI8({expr})"),
        ScalarType::U16 => format!("encodeU16({expr})"),
        ScalarType::I16 => format!("encodeI16({expr})"),
        ScalarType::U32 => format!("encodeU32({expr})"),
        ScalarType::I32 => format!("encodeI32({expr})"),
        ScalarType::U64 | ScalarType::USize => format!("encodeU64({expr})"),
        ScalarType::I64 | ScalarType::ISize => format!("encodeI64({expr})"),
        ScalarType::U128 => format!("encodeU64({expr})"), // Use u64 for now
        ScalarType::I128 => format!("encodeI64({expr})"), // Use i64 for now
        ScalarType::F32 => format!("encodeF32({expr})"),
        ScalarType::F64 => format!("encodeF64({expr})"),
        ScalarType::Char | ScalarType::Str | ScalarType::String | ScalarType::CowStr => {
            format!("encodeString({expr})")
        }
        ScalarType::Unit => "new Uint8Array(0)".into(),
        _ => "/* unsupported scalar */ new Uint8Array(0)".to_string(),
    }
}

/// Generate TypeScript code that decodes a value from a buffer for CLIENT context.
/// Schema is from server's perspective - types match on both sides.
fn generate_decode_stmt_client(shape: &'static Shape, var_name: &str, offset_var: &str) -> String {
    match classify_shape(shape) {
        ShapeKind::Tx { inner } => {
            // Caller's Tx (caller sends) - decode channel_id and create Tx handle
            // r[impl streaming.type] - Channel types decode as channel_id on wire.
            // TODO: Need Connection access to create proper Tx handle
            let inner_type = ts_type_client_return(inner);
            format!(
                "const _{var_name}_r = decodeU64(buf, {offset_var}); const {var_name} = {{ streamId: _{var_name}_r.value }} as Tx<{inner_type}>; {offset_var} = _{var_name}_r.next; /* TODO: create real Tx handle */"
            )
        }
        ShapeKind::Rx { inner } => {
            // Caller's Rx (caller receives) - decode channel_id and create Rx handle
            // r[impl streaming.type] - Channel types decode as channel_id on wire.
            // TODO: Need Connection access to create proper Rx handle
            let inner_type = ts_type_client_return(inner);
            format!(
                "const _{var_name}_r = decodeU64(buf, {offset_var}); const {var_name} = {{ streamId: _{var_name}_r.value }} as Rx<{inner_type}>; {offset_var} = _{var_name}_r.next; /* TODO: create real Rx handle */"
            )
        }
        // For non-streaming types, use the regular decode
        _ => generate_decode_stmt(shape, var_name, offset_var),
    }
}

/// Generate TypeScript code that decodes a value from a buffer for SERVER context.
/// Schema is from server's perspective - no inversion needed.
/// - Schema Tx → server sends via Tx
/// - Schema Rx → server receives via Rx
fn generate_decode_stmt_server(shape: &'static Shape, var_name: &str, offset_var: &str) -> String {
    match classify_shape(shape) {
        ShapeKind::Tx { inner } => {
            // Schema Tx → server sends via Tx
            // r[impl streaming.type] - Channel types decode as channel_id on wire.
            let inner_type = ts_type_server_arg(inner);
            format!(
                "const _{var_name}_r = decodeU64(buf, {offset_var}); const {var_name} = {{ streamId: _{var_name}_r.value }} as Tx<{inner_type}>; {offset_var} = _{var_name}_r.next; /* TODO: create real Tx handle */"
            )
        }
        ShapeKind::Rx { inner } => {
            // Schema Rx → server receives via Rx
            // r[impl streaming.type] - Channel types decode as channel_id on wire.
            let inner_type = ts_type_server_arg(inner);
            format!(
                "const _{var_name}_r = decodeU64(buf, {offset_var}); const {var_name} = {{ streamId: _{var_name}_r.value }} as Rx<{inner_type}>; {offset_var} = _{var_name}_r.next; /* TODO: create real Rx handle */"
            )
        }
        // For non-streaming types, use the regular decode
        _ => generate_decode_stmt(shape, var_name, offset_var),
    }
}

/// Generate TypeScript code that decodes a value from a buffer.
/// Returns the decoded value in a variable and updates offset.
/// `var_name` is the variable to assign the result to.
/// `offset_var` is the variable holding the current offset.
fn generate_decode_stmt(shape: &'static Shape, var_name: &str, offset_var: &str) -> String {
    // Check for bytes first
    if is_bytes(shape) {
        return format!(
            "const _{var_name}_r = decodeBytes(buf, {offset_var}); const {var_name} = _{var_name}_r.value; {offset_var} = _{var_name}_r.next;"
        );
    }

    match classify_shape(shape) {
        ShapeKind::Scalar(scalar) => decode_scalar_stmt(scalar, var_name, offset_var),
        ShapeKind::List { element } => {
            let decode_fn = generate_decode_fn(element, "item");
            format!(
                "const _{var_name}_r = decodeVec(buf, {offset_var}, {decode_fn}); const {var_name} = _{var_name}_r.value; {offset_var} = _{var_name}_r.next;"
            )
        }
        ShapeKind::Option { inner } => {
            let decode_fn = generate_decode_fn(inner, "inner");
            format!(
                "const _{var_name}_r = decodeOption(buf, {offset_var}, {decode_fn}); const {var_name} = _{var_name}_r.value; {offset_var} = _{var_name}_r.next;"
            )
        }
        ShapeKind::Array { element, .. } | ShapeKind::Slice { element } => {
            let decode_fn = generate_decode_fn(element, "item");
            format!(
                "const _{var_name}_r = decodeVec(buf, {offset_var}, {decode_fn}); const {var_name} = _{var_name}_r.value; {offset_var} = _{var_name}_r.next;"
            )
        }
        ShapeKind::Tuple { elements } if elements.len() == 2 => {
            let decode_a = generate_decode_fn(elements[0].shape, "a");
            let decode_b = generate_decode_fn(elements[1].shape, "b");
            format!(
                "const _{var_name}_r = decodeTuple2(buf, {offset_var}, {decode_a}, {decode_b}); const {var_name} = _{var_name}_r.value; {offset_var} = _{var_name}_r.next;"
            )
        }
        ShapeKind::Tuple { elements } if elements.len() == 3 => {
            let decode_a = generate_decode_fn(elements[0].shape, "a");
            let decode_b = generate_decode_fn(elements[1].shape, "b");
            let decode_c = generate_decode_fn(elements[2].shape, "c");
            format!(
                "const _{var_name}_r = decodeTuple3(buf, {offset_var}, {decode_a}, {decode_b}, {decode_c}); const {var_name} = _{var_name}_r.value; {offset_var} = _{var_name}_r.next;"
            )
        }
        ShapeKind::Tuple { elements } => {
            if elements.is_empty() {
                return format!("const {var_name} = undefined;");
            }
            // Generic tuple decoding
            let mut code = format!("const {var_name}: [");
            code.push_str(
                &elements
                    .iter()
                    .map(|p| ts_type_base_named(p.shape))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            code.push_str("] = [] as any;\n");
            for (i, param) in elements.iter().enumerate() {
                let item_var = format!("{var_name}_{i}");
                code.push_str(&generate_decode_stmt(param.shape, &item_var, offset_var));
                code.push_str(&format!(" {var_name}[{i}] = {item_var};\n"));
            }
            code
        }
        ShapeKind::Struct(StructInfo { fields, kind, .. }) => {
            if fields.is_empty() || kind == StructKind::Unit {
                return format!("const {var_name} = undefined;");
            }
            let mut code = String::new();
            for (i, field) in fields.iter().enumerate() {
                let field_var = format!("{var_name}_f{i}");
                code.push_str(&generate_decode_stmt(field.shape(), &field_var, offset_var));
                code.push('\n');
            }
            code.push_str(&format!("const {var_name} = {{ "));
            for (i, field) in fields.iter().enumerate() {
                let field_var = format!("{var_name}_f{i}");
                if i > 0 {
                    code.push_str(", ");
                }
                code.push_str(&format!("{}: {field_var}", field.name));
            }
            code.push_str(" };");
            code
        }
        ShapeKind::Enum(EnumInfo { variants, .. }) => {
            let mut code = format!(
                "const _{var_name}_disc = decodeEnumVariant(buf, {offset_var}); {offset_var} = _{var_name}_disc.next;\n"
            );
            code.push_str(&format!("let {var_name}: {};\n", ts_type_base_named(shape)));
            code.push_str(&format!("switch (_{var_name}_disc.value) {{\n"));
            for (i, v) in variants.iter().enumerate() {
                code.push_str(&format!("  case {i}: {{\n"));
                match classify_variant(v) {
                    VariantKind::Unit => {
                        code.push_str(&format!("    {var_name} = {{ tag: '{}' }};\n", v.name));
                    }
                    VariantKind::Newtype { inner } => {
                        let inner_var = format!("{var_name}_inner");
                        code.push_str(&format!(
                            "    {}\n",
                            generate_decode_stmt(inner, &inner_var, offset_var)
                        ));
                        code.push_str(&format!(
                            "    {var_name} = {{ tag: '{}', value: {inner_var} }};\n",
                            v.name
                        ));
                    }
                    VariantKind::Tuple { fields } | VariantKind::Struct { fields } => {
                        for (fi, field) in fields.iter().enumerate() {
                            let field_var = format!("{var_name}_f{fi}");
                            code.push_str(&format!(
                                "    {}\n",
                                generate_decode_stmt(field.shape(), &field_var, offset_var)
                            ));
                        }
                        code.push_str(&format!("    {var_name} = {{ tag: '{}'", v.name));
                        for (fi, field) in fields.iter().enumerate() {
                            let field_var = format!("{var_name}_f{fi}");
                            code.push_str(&format!(", {}: {field_var}", field.name));
                        }
                        code.push_str(" };\n");
                    }
                }
                code.push_str("    break;\n  }\n");
            }
            code.push_str(&format!(
                "  default: throw new Error(`unknown enum variant ${{_{var_name}_disc.value}}`);\n}}"
            ));
            code
        }
        ShapeKind::Map { key, value } => {
            let decode_k = generate_decode_fn(key, "k");
            let decode_v = generate_decode_fn(value, "v");
            format!(
                "const _{var_name}_r = decodeVec(buf, {offset_var}, (buf, off) => {{ \
                const kr = ({decode_k})(buf, off); \
                const vr = ({decode_v})(buf, kr.next); \
                return {{ value: [kr.value, vr.value] as [any, any], next: vr.next }}; \
                }}); const {var_name} = new Map(_{var_name}_r.value); {offset_var} = _{var_name}_r.next;"
            )
        }
        ShapeKind::Set { element } => {
            let decode_fn = generate_decode_fn(element, "item");
            format!(
                "const _{var_name}_r = decodeVec(buf, {offset_var}, {decode_fn}); const {var_name} = new Set(_{var_name}_r.value); {offset_var} = _{var_name}_r.next;"
            )
        }
        ShapeKind::Tx { inner } => {
            let inner_type = ts_type_base_named(inner);
            format!(
                "const _{var_name}_r = decodeU64(buf, {offset_var}); const {var_name} = {{ streamId: _{var_name}_r.value }} as Tx<{inner_type}>; {offset_var} = _{var_name}_r.next;"
            )
        }
        ShapeKind::Rx { inner } => {
            let inner_type = ts_type_base_named(inner);
            format!(
                "const _{var_name}_r = decodeU64(buf, {offset_var}); const {var_name} = {{ streamId: _{var_name}_r.value }} as Rx<{inner_type}>; {offset_var} = _{var_name}_r.next;"
            )
        }
        ShapeKind::Pointer { pointee } => generate_decode_stmt(pointee, var_name, offset_var),
        ShapeKind::Result { .. } => {
            format!("const {var_name} = undefined; /* Result type decoding not yet implemented */")
        }
        ShapeKind::Opaque => format!("const {var_name} = undefined; /* unsupported type */"),
    }
}

/// Generate decode statement for scalar types.
fn decode_scalar_stmt(scalar: ScalarType, var_name: &str, offset_var: &str) -> String {
    match scalar {
        ScalarType::Bool => format!(
            "const _{var_name}_r = decodeBool(buf, {offset_var}); const {var_name} = _{var_name}_r.value; {offset_var} = _{var_name}_r.next;"
        ),
        ScalarType::U8 => format!(
            "const _{var_name}_r = decodeU8(buf, {offset_var}); const {var_name} = _{var_name}_r.value; {offset_var} = _{var_name}_r.next;"
        ),
        ScalarType::I8 => format!(
            "const _{var_name}_r = decodeI8(buf, {offset_var}); const {var_name} = _{var_name}_r.value; {offset_var} = _{var_name}_r.next;"
        ),
        ScalarType::U16 => format!(
            "const _{var_name}_r = decodeU16(buf, {offset_var}); const {var_name} = _{var_name}_r.value; {offset_var} = _{var_name}_r.next;"
        ),
        ScalarType::I16 => format!(
            "const _{var_name}_r = decodeI16(buf, {offset_var}); const {var_name} = _{var_name}_r.value; {offset_var} = _{var_name}_r.next;"
        ),
        ScalarType::U32 => format!(
            "const _{var_name}_r = decodeU32(buf, {offset_var}); const {var_name} = _{var_name}_r.value; {offset_var} = _{var_name}_r.next;"
        ),
        ScalarType::I32 => format!(
            "const _{var_name}_r = decodeI32(buf, {offset_var}); const {var_name} = _{var_name}_r.value; {offset_var} = _{var_name}_r.next;"
        ),
        ScalarType::U64 | ScalarType::USize => format!(
            "const _{var_name}_r = decodeU64(buf, {offset_var}); const {var_name} = _{var_name}_r.value; {offset_var} = _{var_name}_r.next;"
        ),
        ScalarType::I64 | ScalarType::ISize => format!(
            "const _{var_name}_r = decodeI64(buf, {offset_var}); const {var_name} = _{var_name}_r.value; {offset_var} = _{var_name}_r.next;"
        ),
        ScalarType::U128 => format!(
            "const _{var_name}_r = decodeU64(buf, {offset_var}); const {var_name} = _{var_name}_r.value; {offset_var} = _{var_name}_r.next;"
        ),
        ScalarType::I128 => format!(
            "const _{var_name}_r = decodeI64(buf, {offset_var}); const {var_name} = _{var_name}_r.value; {offset_var} = _{var_name}_r.next;"
        ),
        ScalarType::F32 => format!(
            "const _{var_name}_r = decodeF32(buf, {offset_var}); const {var_name} = _{var_name}_r.value; {offset_var} = _{var_name}_r.next;"
        ),
        ScalarType::F64 => format!(
            "const _{var_name}_r = decodeF64(buf, {offset_var}); const {var_name} = _{var_name}_r.value; {offset_var} = _{var_name}_r.next;"
        ),
        ScalarType::Char | ScalarType::Str | ScalarType::String | ScalarType::CowStr => format!(
            "const _{var_name}_r = decodeString(buf, {offset_var}); const {var_name} = _{var_name}_r.value; {offset_var} = _{var_name}_r.next;"
        ),
        ScalarType::Unit => format!("const {var_name} = undefined;"),
        _ => format!("const {var_name} = undefined; /* unsupported scalar */"),
    }
}

/// Generate a decode function expression for use with decodeVec, decodeOption, etc.
fn generate_decode_fn(shape: &'static Shape, _var_hint: &str) -> String {
    // Check for bytes first
    if is_bytes(shape) {
        return "(buf, off) => decodeBytes(buf, off)".into();
    }

    match classify_shape(shape) {
        ShapeKind::Scalar(scalar) => decode_scalar_fn(scalar),
        ShapeKind::List { element } => {
            let inner_fn = generate_decode_fn(element, "item");
            format!("(buf, off) => decodeVec(buf, off, {inner_fn})")
        }
        ShapeKind::Option { inner } => {
            let inner_fn = generate_decode_fn(inner, "inner");
            format!("(buf, off) => decodeOption(buf, off, {inner_fn})")
        }
        ShapeKind::Array { element, .. } | ShapeKind::Slice { element } => {
            let inner_fn = generate_decode_fn(element, "item");
            format!("(buf, off) => decodeVec(buf, off, {inner_fn})")
        }
        ShapeKind::Tuple { elements } if elements.len() == 2 => {
            let a_fn = generate_decode_fn(elements[0].shape, "a");
            let b_fn = generate_decode_fn(elements[1].shape, "b");
            format!("(buf, off) => decodeTuple2(buf, off, {a_fn}, {b_fn})")
        }
        ShapeKind::Tuple { elements } if elements.len() == 3 => {
            let a_fn = generate_decode_fn(elements[0].shape, "a");
            let b_fn = generate_decode_fn(elements[1].shape, "b");
            let c_fn = generate_decode_fn(elements[2].shape, "c");
            format!("(buf, off) => decodeTuple3(buf, off, {a_fn}, {b_fn}, {c_fn})")
        }
        ShapeKind::Tuple { elements: [] } => {
            "(buf, off) => ({ value: undefined, next: off })".into()
        }
        ShapeKind::Struct(StructInfo { fields, kind, .. }) => {
            if fields.is_empty() || kind == StructKind::Unit {
                return "(buf, off) => ({ value: undefined, next: off })".into();
            }
            // Generate inline struct decoder
            let mut code = "(buf: Uint8Array, off: number) => { let o = off;\n".to_string();
            for (i, f) in fields.iter().enumerate() {
                code.push_str(&format!(
                    "  {}\n",
                    generate_decode_stmt(f.shape(), &format!("f{i}"), "o")
                ));
            }
            code.push_str("  return { value: { ");
            for (i, f) in fields.iter().enumerate() {
                if i > 0 {
                    code.push_str(", ");
                }
                code.push_str(&format!("{}: f{i}", f.name));
            }
            code.push_str(" }, next: o };\n}");
            code
        }
        ShapeKind::Enum(EnumInfo { variants, .. }) => {
            // Generate inline enum decoder
            let mut code =
                "(buf: Uint8Array, off: number): DecodeResult<any> => { let o = off;\n".to_string();
            code.push_str("  const disc = decodeEnumVariant(buf, o); o = disc.next;\n");
            code.push_str("  switch (disc.value) {\n");
            for (i, v) in variants.iter().enumerate() {
                code.push_str(&format!("    case {i}: "));
                match classify_variant(v) {
                    VariantKind::Unit => {
                        code.push_str(&format!(
                            "return {{ value: {{ tag: '{}' }}, next: o }};\n",
                            v.name
                        ));
                    }
                    VariantKind::Newtype { inner } => {
                        code.push_str("{\n");
                        code.push_str(&format!(
                            "      {}\n",
                            generate_decode_stmt(inner, "val", "o")
                        ));
                        code.push_str(&format!(
                            "      return {{ value: {{ tag: '{}', value: val }}, next: o }};\n",
                            v.name
                        ));
                        code.push_str("    }\n");
                    }
                    VariantKind::Tuple { fields } | VariantKind::Struct { fields } => {
                        code.push_str("{\n");
                        for (j, f) in fields.iter().enumerate() {
                            code.push_str(&format!(
                                "      {}\n",
                                generate_decode_stmt(f.shape(), &format!("f{j}"), "o")
                            ));
                        }
                        code.push_str(&format!("      return {{ value: {{ tag: '{}', ", v.name));
                        for (j, f) in fields.iter().enumerate() {
                            if j > 0 {
                                code.push_str(", ");
                            }
                            code.push_str(&format!("{}: f{j}", f.name));
                        }
                        code.push_str(" }, next: o };\n    }\n");
                    }
                }
            }
            code.push_str(
                "    default: throw new Error(`unknown enum variant: ${disc.value}`);\n  }\n}",
            );
            code
        }
        ShapeKind::Pointer { pointee } => generate_decode_fn(pointee, _var_hint),
        _ => "(buf, off) => { throw new Error('unsupported type'); }".into(),
    }
}

/// Generate decode function for scalar types.
fn decode_scalar_fn(scalar: ScalarType) -> String {
    match scalar {
        ScalarType::Bool => "(buf, off) => decodeBool(buf, off)".into(),
        ScalarType::U8 => "(buf, off) => decodeU8(buf, off)".into(),
        ScalarType::I8 => "(buf, off) => decodeI8(buf, off)".into(),
        ScalarType::U16 => "(buf, off) => decodeU16(buf, off)".into(),
        ScalarType::I16 => "(buf, off) => decodeI16(buf, off)".into(),
        ScalarType::U32 => "(buf, off) => decodeU32(buf, off)".into(),
        ScalarType::I32 => "(buf, off) => decodeI32(buf, off)".into(),
        ScalarType::U64 | ScalarType::USize => "(buf, off) => decodeU64(buf, off)".into(),
        ScalarType::I64 | ScalarType::ISize => "(buf, off) => decodeI64(buf, off)".into(),
        ScalarType::U128 => "(buf, off) => decodeU64(buf, off)".into(),
        ScalarType::I128 => "(buf, off) => decodeI64(buf, off)".into(),
        ScalarType::F32 => "(buf, off) => decodeF32(buf, off)".into(),
        ScalarType::F64 => "(buf, off) => decodeF64(buf, off)".into(),
        ScalarType::Char | ScalarType::Str | ScalarType::String | ScalarType::CowStr => {
            "(buf, off) => decodeString(buf, off)".into()
        }
        ScalarType::Unit => "(buf, off) => ({ value: undefined, next: off })".into(),
        _ => "(buf, off) => { throw new Error('unsupported scalar'); }".into(),
    }
}

/// Check if a type can be fully encoded/decoded.
/// Streaming types (Tx/Rx) are supported - they encode as stream IDs.
fn is_fully_supported(shape: &'static Shape) -> bool {
    match classify_shape(shape) {
        // Streaming types are supported - they encode/decode as stream IDs
        ShapeKind::Tx { inner } | ShapeKind::Rx { inner } => is_fully_supported(inner),
        ShapeKind::List { element }
        | ShapeKind::Option { inner: element }
        | ShapeKind::Set { element }
        | ShapeKind::Array { element, .. }
        | ShapeKind::Slice { element } => is_fully_supported(element),
        ShapeKind::Map { key, value } => is_fully_supported(key) && is_fully_supported(value),
        ShapeKind::Tuple { elements } => elements.iter().all(|p| is_fully_supported(p.shape)),
        ShapeKind::Struct(StructInfo { fields, .. }) => {
            fields.iter().all(|f| is_fully_supported(f.shape()))
        }
        ShapeKind::Enum(EnumInfo { variants, .. }) => {
            variants.iter().all(|v| match classify_variant(v) {
                VariantKind::Unit => true,
                VariantKind::Newtype { inner } => is_fully_supported(inner),
                VariantKind::Tuple { fields } | VariantKind::Struct { fields } => {
                    fields.iter().all(|f| is_fully_supported(f.shape()))
                }
            })
        }
        ShapeKind::Pointer { pointee } => is_fully_supported(pointee),
        ShapeKind::Scalar(_) => true,
        ShapeKind::Result { .. } => false,
        ShapeKind::Opaque => false,
    }
}
