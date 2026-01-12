//! JSON ↔ postcard transcoding using facet_value::Value.
//!
//! r[bridge.json.facet]
//! The bridge uses facet-json and facet-postcard for transcoding between
//! HTTP/JSON and roam wire format (postcard). The `Value` type acts as
//! the interchange format, enabling runtime transcoding without per-service
//! code generation.

use crate::BridgeError;
use facet_value::Value;

/// Transcode JSON bytes to postcard bytes.
///
/// r[bridge.json.facet]
/// JSON → Value → postcard
pub fn json_to_postcard(json: &[u8]) -> Result<Vec<u8>, BridgeError> {
    // Parse JSON into Value
    let value: Value = facet_json::from_slice(json)
        .map_err(|e| BridgeError::bad_request(format!("Invalid JSON: {e}")))?;

    // Serialize Value to postcard
    facet_postcard::to_vec(&value)
        .map_err(|e| BridgeError::internal(format!("Postcard serialization failed: {e}")))
}

/// Transcode postcard bytes to JSON bytes.
///
/// r[bridge.json.facet]
/// postcard → Value → JSON
pub fn postcard_to_json(postcard: &[u8]) -> Result<Vec<u8>, BridgeError> {
    // Parse postcard into Value
    let value: Value = facet_postcard::from_slice(postcard)
        .map_err(|e| BridgeError::internal(format!("Invalid postcard response: {e}")))?;

    // Serialize Value to JSON
    facet_json::to_vec(&value)
        .map_err(|e| BridgeError::internal(format!("JSON serialization failed: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_roundtrip_simple() {
        let json = br#"{"x": 10, "y": 20}"#;
        let postcard = json_to_postcard(json).unwrap();
        let back = postcard_to_json(&postcard).unwrap();
        // Parse both to Value to compare (JSON formatting may differ)
        let orig: Value = facet_json::from_slice(json).unwrap();
        let roundtrip: Value = facet_json::from_slice(&back).unwrap();
        assert_eq!(format!("{orig:?}"), format!("{roundtrip:?}"));
    }

    #[test]
    fn test_roundtrip_array() {
        let json = br#"[1, 2, 3]"#;
        let postcard = json_to_postcard(json).unwrap();
        let back = postcard_to_json(&postcard).unwrap();
        let orig: Value = facet_json::from_slice(json).unwrap();
        let roundtrip: Value = facet_json::from_slice(&back).unwrap();
        assert_eq!(format!("{orig:?}"), format!("{roundtrip:?}"));
    }

    #[test]
    fn test_invalid_json() {
        let result = json_to_postcard(b"not valid json");
        assert!(result.is_err());
    }
}
