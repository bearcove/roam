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

/// Transcode JSON bytes to postcard bytes.
///
/// r[bridge.json.facet]
/// JSON → Value → postcard
pub fn json_to_postcard(json: &[u8]) -> Result<Vec<u8>, BridgeError> {
    // Parse JSON into Value (JSON is self-describing)
    let value: Value = facet_json::from_slice(json)
        .map_err(|e| BridgeError::bad_request(format!("Invalid JSON: {e}")))?;

    // Serialize Value to postcard
    facet_postcard::to_vec(&value)
        .map_err(|e| BridgeError::internal(format!("Postcard serialization failed: {e}")))
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

#[cfg(test)]
mod tests {
    use super::*;
    use facet::Facet;

    #[derive(Debug, Facet, PartialEq)]
    struct Point {
        x: i32,
        y: i32,
    }

    #[test]
    fn test_json_to_postcard() {
        let json = br#"{"x": 10, "y": 20}"#;
        let postcard = json_to_postcard(json).unwrap();
        // Should produce valid postcard bytes
        assert!(!postcard.is_empty());
    }

    #[test]
    fn test_array_vs_tuple_encoding() {
        // JSON array: ["hello world"]
        let json = br#"["hello world"]"#;
        let value: Value = facet_json::from_slice(json).unwrap();
        let value_postcard = facet_postcard::to_vec(&value).unwrap();
        eprintln!("Value from JSON array: {:?}", value);
        eprintln!("Value postcard bytes: {:?}", value_postcard);

        // Typed tuple: ("hello world",)
        let typed_tuple: (String,) = ("hello world".to_string(),);
        let typed_postcard = facet_postcard::to_vec(&typed_tuple).unwrap();
        eprintln!("Typed tuple postcard bytes: {:?}", typed_postcard);

        // Plain string
        let typed_string = "hello world".to_string();
        let string_postcard = facet_postcard::to_vec(&typed_string).unwrap();
        eprintln!("Plain string postcard bytes: {:?}", string_postcard);

        // Are they the same?
        eprintln!(
            "Value bytes == Typed tuple bytes: {}",
            value_postcard == typed_postcard
        );
    }

    #[test]
    fn test_postcard_to_json_with_shape() {
        // Create a Point, serialize to postcard
        let point = Point { x: 42, y: 99 };
        let postcard_bytes = facet_postcard::to_vec(&point).unwrap();

        // Transcode back to JSON using shape
        let json = postcard_to_json_with_shape(&postcard_bytes, Point::SHAPE).unwrap();
        let json_str = String::from_utf8(json).unwrap();

        // Should contain the field values
        assert!(json_str.contains("42"));
        assert!(json_str.contains("99"));
    }

    #[test]
    fn test_roundtrip_typed() {
        // The realistic scenario: typed Rust value → postcard → JSON
        // This is what happens when roam returns a response
        let point = Point { x: 10, y: 20 };

        // Typed value → postcard (what roam does internally)
        let postcard_bytes = facet_postcard::to_vec(&point).unwrap();

        // postcard → JSON (what the bridge does)
        let json = postcard_to_json_with_shape(&postcard_bytes, Point::SHAPE).unwrap();
        let json_str = String::from_utf8(json).unwrap();

        // Verify the values are correct
        let value: Value = facet_json::from_slice(json_str.as_bytes()).unwrap();
        let obj = value.as_object().unwrap();
        assert_eq!(
            obj.get("x").unwrap().as_number().unwrap().to_i64(),
            Some(10)
        );
        assert_eq!(
            obj.get("y").unwrap().as_number().unwrap().to_i64(),
            Some(20)
        );
    }

    #[test]
    fn test_invalid_json() {
        let result = json_to_postcard(b"not valid json");
        assert!(result.is_err());
    }
}
