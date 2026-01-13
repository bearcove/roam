//! Generic bridge service implementation.
//!
//! This module provides `GenericBridgeService`, which wraps a roam `ConnectionHandle`
//! and `ServiceDetail` to implement `BridgeService` using runtime transcoding.

use std::collections::HashMap;

use facet_core::Shape;
use roam_schema::{MethodDetail, ServiceDetail, contains_stream};
use roam_session::ConnectionHandle;

use crate::{
    BoxFuture, BridgeError, BridgeMetadata, BridgeResponse, BridgeService, ProtocolErrorKind,
    transcode::{json_to_postcard, postcard_to_json_with_shape},
};

/// A generic bridge service that wraps a roam connection.
///
/// This uses runtime transcoding via `facet_value::Value` - no per-service
/// code generation required.
pub struct GenericBridgeService {
    /// The roam connection handle for making calls.
    handle: ConnectionHandle,
    /// Service metadata (name, methods, types).
    detail: &'static ServiceDetail,
    /// Precomputed method info for fast lookup.
    methods: HashMap<String, MethodInfo>,
}

/// Cached method information for fast lookup.
struct MethodInfo {
    method_id: u64,
    has_channels: bool,
    /// The return type shape (for decoding responses).
    return_shape: &'static Shape,
    /// The error type shape (for decoding user errors), if any.
    /// For methods returning Result<T, E>, this is E's shape.
    #[allow(dead_code)]
    error_shape: Option<&'static Shape>,
}

impl GenericBridgeService {
    /// Create a new bridge service wrapping a connection.
    ///
    /// # Arguments
    /// * `handle` - The roam connection handle for making RPC calls
    /// * `detail` - Static service metadata (from generated code)
    pub fn new(handle: ConnectionHandle, detail: &'static ServiceDetail) -> Self {
        let mut methods = HashMap::new();

        for method in &detail.methods {
            let method_id = roam_hash::method_id_from_detail(method);
            let has_channels = method.args.iter().any(|a| contains_stream(a.ty))
                || contains_stream(method.return_type);

            // Extract return type and error type from the method signature
            let (return_shape, error_shape) = extract_result_types(method);

            methods.insert(
                method.method_name.to_string(),
                MethodInfo {
                    method_id,
                    has_channels,
                    return_shape,
                    error_shape,
                },
            );
        }

        Self {
            handle,
            detail,
            methods,
        }
    }
}

/// Extract the success and error types from a method's return type.
///
/// roam methods return `Result<T, RoamError<E>>` where:
/// - T is the success type
/// - E is the user error type (or Never for infallible methods)
fn extract_result_types(method: &MethodDetail) -> (&'static Shape, Option<&'static Shape>) {
    let return_shape = method.return_type;

    // The return type should be Result<T, RoamError<E>>
    // We need to extract T for success responses and E for user errors
    if let facet_core::Def::Result(result_def) = return_shape.def {
        let success_shape = result_def.t();

        // The error type is RoamError<E>, extract E from its type params
        let roam_error_shape = result_def.e();
        let user_error_shape = roam_error_shape.type_params.first().map(|e| e.shape);

        return (success_shape, user_error_shape);
    }

    // Fallback: use the return type as-is (shouldn't happen for roam methods)
    (return_shape, None)
}

impl BridgeService for GenericBridgeService {
    fn service_detail(&self) -> &'static ServiceDetail {
        self.detail
    }

    fn call_json<'a>(
        &'a self,
        method_name: &'a str,
        json_body: &'a [u8],
        metadata: BridgeMetadata,
    ) -> BoxFuture<'a, Result<BridgeResponse, BridgeError>> {
        Box::pin(async move {
            // Look up method
            let method_info = self.methods.get(method_name).ok_or_else(|| {
                // r[bridge.response.protocol-error]
                BridgeError::new(
                    http::StatusCode::OK,
                    format!("Unknown method: {}", method_name),
                )
            })?;

            // r[bridge.json.channels-forbidden]
            if method_info.has_channels {
                return Err(BridgeError::bad_request(
                    "Channel methods require WebSocket",
                ));
            }

            // r[bridge.json.facet]
            // Transcode JSON → postcard
            let postcard_payload = json_to_postcard(json_body)?;

            // Convert metadata to wire format
            let wire_metadata = metadata.to_wire_metadata();

            // Make the roam call
            let response_bytes = self
                .handle
                .call_raw_with_metadata(method_info.method_id, postcard_payload, wire_metadata)
                .await
                .map_err(|e| BridgeError::backend_unavailable(format!("Call failed: {e}")))?;

            // Parse the response envelope
            // The response is a postcard-encoded Result<T, RoamError<E>>
            // Postcard encodes Result as: 0x00 + value_bytes for Ok, 0x01 + error_bytes for Err
            // RoamError<E> variants: User(E)=0, UnknownMethod=1, InvalidPayload=2, Cancelled=3
            if response_bytes.is_empty() {
                return Err(BridgeError::internal("Empty response from backend"));
            }

            match response_bytes[0] {
                0x00 => {
                    // Result::Ok(value) - transcode the value part using the return shape
                    let value_bytes = &response_bytes[1..];
                    let json_bytes =
                        postcard_to_json_with_shape(value_bytes, method_info.return_shape)?;
                    Ok(BridgeResponse::Success(json_bytes))
                }
                0x01 => {
                    // Result::Err(RoamError<E>) - decode which error variant
                    if response_bytes.len() < 2 {
                        return Err(BridgeError::internal("Truncated error response"));
                    }
                    match response_bytes[1] {
                        0x00 => {
                            // RoamError::User(E) - transcode the error value
                            let error_bytes = &response_bytes[2..];
                            // Use error shape if available, otherwise return raw
                            if let Some(error_shape) = method_info.error_shape {
                                let json_bytes =
                                    postcard_to_json_with_shape(error_bytes, error_shape)?;
                                Ok(BridgeResponse::UserError(json_bytes))
                            } else {
                                // No error type (Never) - shouldn't have user errors
                                Ok(BridgeResponse::UserError(b"null".to_vec()))
                            }
                        }
                        0x01 => {
                            // RoamError::UnknownMethod
                            Ok(BridgeResponse::ProtocolError(
                                ProtocolErrorKind::UnknownMethod,
                            ))
                        }
                        0x02 => {
                            // RoamError::InvalidPayload
                            Ok(BridgeResponse::ProtocolError(
                                ProtocolErrorKind::InvalidPayload,
                            ))
                        }
                        0x03 => {
                            // RoamError::Cancelled
                            Ok(BridgeResponse::ProtocolError(ProtocolErrorKind::Cancelled))
                        }
                        tag => Err(BridgeError::internal(format!(
                            "Unknown RoamError variant: {tag}"
                        ))),
                    }
                }
                tag => Err(BridgeError::internal(format!(
                    "Unknown Result variant: {tag}"
                ))),
            }
        })
    }
}
