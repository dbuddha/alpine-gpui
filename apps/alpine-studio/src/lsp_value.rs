//! Unambiguous values at diagnostic trust boundaries, within caller wire limits.
//!
//! Ordinary `Value` decoding discards duplicate keys, including escaped aliases.
//! Reject them in the same decoding pass rather than validating a lossy value.

use std::fmt;

use serde::de::{self, Deserialize, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value, value::RawValue};

pub(crate) fn parse(raw: &RawValue) -> Result<Value, serde_json::Error> {
    serde_json::from_str::<UniqueValue>(raw.get()).map(|value| value.0)
}

struct UniqueValue(Value);

impl<'de> Deserialize<'de> for UniqueValue {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        deserializer.deserialize_any(ValueVisitor)
    }
}

struct ValueVisitor;

impl<'de> Visitor<'de> for ValueVisitor {
    type Value = UniqueValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value with unique decoded object keys")
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::Bool(value)))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::Number(value.into())))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::Number(value.into())))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
        Number::from_f64(value)
            .map(|number| UniqueValue(Value::Number(number)))
            .ok_or_else(|| E::custom("non-finite JSON number"))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::String(value.into())))
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::String(value)))
    }

    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(UniqueValue(Value::Null))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<Self::Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element::<UniqueValue>()? {
            values.push(value.0);
        }
        Ok(UniqueValue(Value::Array(values)))
    }

    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
        let mut values = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if values.contains_key(&key) {
                return Err(de::Error::custom("duplicate JSON object key"));
            }
            values.insert(key, map.next_value::<UniqueValue>()?.0);
        }
        Ok(UniqueValue(Value::Object(values)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_input_reports_the_unique_key_json_contract()
    -> Result<(), Box<dyn std::error::Error>> {
        let input = de::value::BytesDeserializer::<de::value::Error>::new(b"not JSON");
        let error = UniqueValue::deserialize(input)
            .err()
            .ok_or("raw bytes were admitted as a JSON value")?
            .to_string();
        assert!(error.contains("byte array"));
        assert!(error.contains("expected a JSON value with unique decoded object keys"));
        Ok(())
    }

    #[test]
    fn owned_strings_are_preserved_and_non_finite_numbers_are_rejected()
    -> Result<(), Box<dyn std::error::Error>> {
        let text = String::from("owned string with \"quotes\", a newline\n and a NUL\0");
        let input = de::value::StringDeserializer::<de::value::Error>::new(text.clone());
        assert_eq!(UniqueValue::deserialize(input)?.0, Value::String(text));
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let input = de::value::F64Deserializer::<de::value::Error>::new(value);
            let error = UniqueValue::deserialize(input)
                .err()
                .ok_or("a non-finite number was admitted as JSON")?;
            assert_eq!(error.to_string(), "non-finite JSON number");
        }
        Ok(())
    }

    #[test]
    fn preserves_value_semantics_but_rejects_nested_and_escaped_duplicates()
    -> Result<(), Box<dyn std::error::Error>> {
        for source in [
            r#"[null,true,false,-1,18446744073709551615,1.25,"text","a\nb",{}]"#,
            r#"{"a":[{"x":1},{"x":2}],"b":{"x":3}}"#,
        ] {
            let raw = RawValue::from_string(source.into())?;
            assert_eq!(parse(&raw)?, serde_json::from_str::<Value>(source)?);
        }
        for source in [
            r#"{"x":0,"x":1}"#,
            r#"{"outer":[{"x":0,"x":1}]}"#,
            r#"{"x":0,"\u0078":1}"#,
        ] {
            let raw = RawValue::from_string(source.into())?;
            assert!(parse(&raw).is_err());
        }
        let deep = format!("{}0{}", "[".repeat(129), "]".repeat(129));
        assert!(parse(&RawValue::from_string(deep)?).is_err());
        Ok(())
    }
}
