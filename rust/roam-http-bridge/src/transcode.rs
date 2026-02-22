//! JSON ↔ postcard transcoding using facet_value::Value.
//!
//! r[bridge.json.facet]
//! The bridge uses facet-json and facet-postcard for transcoding between
//! HTTP/JSON and roam wire format (postcard). The `Value` type acts as
//! the interchange format, enabling runtime transcoding without per-service
//! code generation.

use crate::BridgeError;
use facet_core::Shape;
use facet_value::Value;
use roam_schema::{is_rx, is_tx};

/// Check if a shape represents a channel type (Rx<T> or Tx<T>).
///
/// Channel types use `#[facet(proxy = u64)]` and serialize as just the channel ID.
fn is_channel_shape(shape: &'static Shape) -> bool {
    is_rx(shape) || is_tx(shape)
}

/// Transcode JSON array to postcard bytes using arg shapes.
///
/// r[bridge.json.facet]
/// JSON array → Value elements → postcard tuple (concatenated)
///
/// The JSON body is an array of arguments `[arg0, arg1, ...]`.
/// Each argument is serialized using its corresponding shape from the method signature.
/// The result is the concatenation of the serialized arguments, which is how
/// postcard encodes tuples.
pub fn json_args_to_postcard(
    json: &[u8],
    arg_shapes: &[&'static Shape],
) -> Result<Vec<u8>, BridgeError> {
    // Parse JSON into Value (JSON is self-describing)
    let value: Value = facet_json::from_slice(json)
        .map_err(|e| BridgeError::bad_request(format!("Invalid JSON: {e}")))?;

    // Must be an array
    let args = value.as_array().ok_or_else(|| {
        BridgeError::bad_request("Request body must be a JSON array of arguments")
    })?;

    // Check argument count matches
    if args.len() != arg_shapes.len() {
        return Err(BridgeError::bad_request(format!(
            "Expected {} arguments, got {}",
            arg_shapes.len(),
            args.len()
        )));
    }

    // Serialize each argument using its shape and concatenate
    // This produces the same bytes as serializing a typed tuple
    let mut result = Vec::new();
    for (arg, shape) in args.iter().zip(arg_shapes.iter()) {
        // Channel types (Rx<T>, Tx<T>) serialize as just u64 (the channel ID)
        // due to their #[facet(proxy = u64)] attribute
        let bytes = if is_channel_shape(shape) {
            // Serialize the number as u64 directly using u64's shape
            facet_postcard::to_vec_with_shape(arg, <u64 as facet::Facet>::SHAPE).map_err(|e| {
                BridgeError::bad_request(format!("Failed to encode channel ID: {e}"))
            })?
        } else {
            facet_postcard::to_vec_with_shape(arg, shape)
                .map_err(|e| BridgeError::bad_request(format!("Failed to encode argument: {e}")))?
        };
        result.extend(bytes);
    }

    Ok(result)
}

/// Transcode postcard bytes to JSON bytes using shape information.
///
/// r[bridge.json.facet]
/// postcard → Value (with shape hint) → JSON
///
/// Since postcard is not self-describing, we need the shape to decode it.
pub fn postcard_to_json_with_shape(
    postcard: &[u8],
    shape: &'static Shape,
) -> Result<Vec<u8>, BridgeError> {
    // Parse postcard into Value using shape information
    let value: Value = facet_postcard::from_slice_with_shape(postcard, shape)
        .map_err(|e| BridgeError::internal(format!("Invalid postcard response: {e}")))?;

    // Serialize Value to JSON
    facet_json::to_vec(&value)
        .map_err(|e| BridgeError::internal(format!("JSON serialization failed: {e}")))
}
