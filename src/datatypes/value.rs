//! JSON value conversion at the public datatype boundary.
//!
//! Datatypes that store JSON payloads (e.g., `Variable`) exchange values with user code
//! through these helpers: a value is encoded once into compact JSON bytes on write, and
//! stored JSON bytes are decoded once into the caller's requested type on read. The
//! encoded bytes are exactly one valid UTF-8 JSON value and are preserved end-to-end as
//! the language-neutral storage representation.
//!
//! Failures surface as [`DatatypeError::ValueConversion`], returned directly to the API
//! caller without changing the datatype state or reaching the event loop.

use std::fmt::Display;

use serde::{Serialize, de::DeserializeOwned};

use crate::errors::datatypes::DatatypeError;

/// Encodes a user value into compact JSON bytes.
pub(crate) fn encode_json_value<T: Serialize + ?Sized>(
    value: &T,
) -> Result<Box<[u8]>, DatatypeError> {
    serde_json::to_vec(value)
        .map(Vec::into_boxed_slice)
        .map_err(|error| value_conversion_error("encode", error))
}

/// Decodes stored JSON bytes into the caller's requested type.
///
/// Rejects input that is not exactly one JSON value. JSON `null` decodes only into
/// nullable destinations such as `Option<T>` or `serde_json::Value`.
pub(crate) fn decode_json_value<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, DatatypeError> {
    serde_json::from_slice(bytes).map_err(|error| value_conversion_error("decode", error))
}

fn value_conversion_error(direction: &str, error: impl Display) -> DatatypeError {
    DatatypeError::ValueConversion(format!("{direction}: {error}"))
}

#[cfg(test)]
mod tests {
    use serde::{Deserialize, Serialize};
    use serde_json::{Value, json};

    use super::*;

    #[derive(Debug, PartialEq, Serialize, Deserialize)]
    struct Profile {
        name: String,
        age: u32,
        tags: Vec<String>,
    }

    fn sample_profile() -> Profile {
        Profile {
            name: "qortoo".to_string(),
            age: 3,
            tags: vec!["crdt".to_string(), "lww".to_string()],
        }
    }

    #[test]
    fn can_round_trip_struct_values() {
        let encoded = encode_json_value(&sample_profile()).unwrap();
        let decoded: Profile = decode_json_value(&encoded).unwrap();
        assert_eq!(decoded, sample_profile());
    }

    #[test]
    fn can_round_trip_scalar_array_and_null_values() {
        let encoded = encode_json_value(&42_i64).unwrap();
        assert_eq!(decode_json_value::<i64>(&encoded).unwrap(), 42);

        let encoded = encode_json_value("text").unwrap();
        assert_eq!(decode_json_value::<String>(&encoded).unwrap(), "text");

        let encoded = encode_json_value(&[true, false]).unwrap();
        assert_eq!(
            decode_json_value::<Vec<bool>>(&encoded).unwrap(),
            vec![true, false]
        );

        let encoded = encode_json_value(&Value::Null).unwrap();
        assert_eq!(&*encoded, b"null");
        assert_eq!(decode_json_value::<Value>(&encoded).unwrap(), Value::Null);
    }

    #[test]
    fn can_encode_compact_json_bytes() {
        let encoded = encode_json_value(&json!({"a": 1, "b": [true, null]})).unwrap();
        assert_eq!(&*encoded, br#"{"a":1,"b":[true,null]}"#);
    }

    #[test]
    fn can_decode_null_into_nullable_destinations() {
        assert_eq!(decode_json_value::<Option<Profile>>(b"null").unwrap(), None);
        assert_eq!(decode_json_value::<Value>(b"null").unwrap(), Value::Null);
    }

    #[test]
    fn can_reject_null_for_non_nullable_destination() {
        let error = decode_json_value::<Profile>(b"null").unwrap_err();
        assert_eq!(error, DatatypeError::ValueConversion(String::new()));
    }

    fn non_string_key_map() -> std::collections::BTreeMap<(u8, u8), &'static str> {
        std::collections::BTreeMap::from([((1, 2), "x")])
    }

    #[test]
    fn can_reject_value_that_fails_to_encode() {
        let error = encode_json_value(&non_string_key_map()).unwrap_err();
        assert_eq!(error, DatatypeError::ValueConversion(String::new()));
    }

    #[test]
    fn can_reject_input_that_is_not_exactly_one_json_value() {
        for bytes in [
            b"".as_slice(),
            b"{\"a\":".as_slice(),
            b"1 2".as_slice(),
            b"null null".as_slice(),
            b"\xff".as_slice(),
        ] {
            let error = decode_json_value::<Value>(bytes).unwrap_err();
            assert_eq!(error, DatatypeError::ValueConversion(String::new()));
        }
    }

    #[test]
    fn can_report_conversion_direction_in_the_message() {
        let DatatypeError::ValueConversion(message) =
            encode_json_value(&non_string_key_map()).unwrap_err()
        else {
            panic!("expected ValueConversion");
        };
        assert!(message.starts_with("encode: "));

        let DatatypeError::ValueConversion(message) = decode_json_value::<Value>(b"").unwrap_err()
        else {
            panic!("expected ValueConversion");
        };
        assert!(message.starts_with("decode: "));
    }
}
