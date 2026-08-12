use std::fmt;

use base64::{Engine as _, alphabet, engine};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_json::value::RawValue;

use crate::transport::{ClientError, ClientErrorKind};

/// A SQL parameter with the same reachable wire cases as Go's `encoding/json` values.
#[derive(Clone, Debug)]
pub enum SqlValue {
    Null,
    Bool(bool),
    I64(i64),
    U64(u64),
    F64(f64),
    String(String),
    Bytes(Vec<u8>),
    Raw(RawSqlValue),
}

/// A validated, compacted raw JSON SQL parameter.
#[derive(Clone, Debug)]
pub struct RawSqlValue(Box<RawValue>);

impl SqlValue {
    pub fn float(value: f64) -> Result<Self, ClientError> {
        if value.is_finite() {
            Ok(Self::F64(value))
        } else {
            Err(ClientError::new(
                ClientErrorKind::Json,
                "unsupported non-finite SQL parameter",
            ))
        }
    }

    pub fn raw(value: &str) -> Result<Self, ClientError> {
        let compact = compact_raw(value)?;
        let raw = RawValue::from_string(compact).map_err(|error| {
            ClientError::with_source(ClientErrorKind::Json, "invalid raw SQL parameter", error)
        })?;
        Ok(Self::Raw(RawSqlValue(raw)))
    }
}

impl From<bool> for SqlValue {
    fn from(value: bool) -> Self {
        Self::Bool(value)
    }
}

impl From<i64> for SqlValue {
    fn from(value: i64) -> Self {
        Self::I64(value)
    }
}

impl From<u64> for SqlValue {
    fn from(value: u64) -> Self {
        Self::U64(value)
    }
}

impl From<String> for SqlValue {
    fn from(value: String) -> Self {
        Self::String(value)
    }
}

impl From<&str> for SqlValue {
    fn from(value: &str) -> Self {
        Self::String(value.to_owned())
    }
}

impl Serialize for SqlValue {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Null => serializer.serialize_none(),
            Self::Bool(value) => serializer.serialize_bool(*value),
            Self::I64(value) => serializer.serialize_i64(*value),
            Self::U64(value) => serializer.serialize_u64(*value),
            Self::F64(value) if value.is_finite() => serializer.serialize_f64(*value),
            Self::F64(_) => Err(serde::ser::Error::custom(
                "unsupported non-finite SQL parameter",
            )),
            Self::String(value) => serializer.serialize_str(value),
            Self::Bytes(value) => {
                serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(value))
            }
            Self::Raw(value) => value.0.serialize(serializer),
        }
    }
}

/// A nullable byte slice using Go's padded standard Base64 JSON representation.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct GoBytes(pub Option<Vec<u8>>);

impl<'de> Deserialize<'de> for GoBytes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor;

        impl<'de> de::Visitor<'de> for Visitor {
            type Value = GoBytes;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("null or a padded standard Base64 string")
            }

            fn visit_none<E>(self) -> Result<Self::Value, E> {
                Ok(GoBytes(None))
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(GoBytes(None))
            }

            fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
            where
                D: Deserializer<'de>,
            {
                deserializer.deserialize_str(self)
            }

            fn visit_borrowed_str<E>(self, value: &'de str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                decode_go_base64(value)
                    .map(|bytes| GoBytes(Some(bytes)))
                    .map_err(E::custom)
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                decode_go_base64(value)
                    .map(|bytes| GoBytes(Some(bytes)))
                    .map_err(E::custom)
            }
        }

        deserializer.deserialize_option(Visitor)
    }
}

/// A deferred JSON column preserving its original number and tuple spelling.
#[derive(Clone, Debug)]
pub struct JsonColumn(pub(crate) Box<RawValue>);

impl JsonColumn {
    pub fn raw(&self) -> &str {
        self.0.get()
    }

    pub fn decode<T: serde::de::DeserializeOwned>(&self) -> Result<T, ClientError> {
        serde_json::from_str(self.raw()).map_err(|error| {
            ClientError::with_source(ClientErrorKind::Json, "decode column value", error)
        })
    }

    pub fn is_null(&self) -> bool {
        self.raw().trim() == "null"
    }
}

pub(crate) fn go_json_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, ClientError> {
    let encoded = serde_json::to_vec(value)
        .map_err(|error| ClientError::with_source(ClientErrorKind::Json, "encode JSON", error))?;
    Ok(go_escape(encoded))
}

fn go_escape(encoded: Vec<u8>) -> Vec<u8> {
    let mut out = Vec::with_capacity(encoded.len());
    let mut index = 0;
    while index < encoded.len() {
        match encoded[index] {
            b'<' => out.extend_from_slice(br"\u003c"),
            b'>' => out.extend_from_slice(br"\u003e"),
            b'&' => out.extend_from_slice(br"\u0026"),
            0xe2 if encoded.get(index..index + 3) == Some(&[0xe2, 0x80, 0xa8]) => {
                out.extend_from_slice(br"\u2028");
                index += 2;
            }
            0xe2 if encoded.get(index..index + 3) == Some(&[0xe2, 0x80, 0xa9]) => {
                out.extend_from_slice(br"\u2029");
                index += 2;
            }
            byte => out.push(byte),
        }
        index += 1;
    }
    out
}

fn compact_raw(value: &str) -> Result<String, ClientError> {
    serde_json::from_str::<Box<RawValue>>(value).map_err(|error| {
        ClientError::with_source(ClientErrorKind::Json, "invalid raw SQL parameter", error)
    })?;

    let mut compact = String::with_capacity(value.len());
    let mut quoted = false;
    let mut escaped = false;
    for character in value.chars() {
        if quoted {
            compact.push(character);
            if escaped {
                escaped = false;
            } else if character == '\\' {
                escaped = true;
            } else if character == '"' {
                quoted = false;
            }
        } else if character == '"' {
            quoted = true;
            compact.push(character);
        } else if !character.is_ascii_whitespace() {
            compact.push(character);
        }
    }
    Ok(String::from_utf8(go_escape(compact.into_bytes())).expect("JSON remains UTF-8"))
}

fn decode_go_base64(value: &str) -> Result<Vec<u8>, base64::DecodeError> {
    let filtered: String = value
        .chars()
        .filter(|character| !matches!(character, '\r' | '\n'))
        .collect();
    let config = engine::GeneralPurposeConfig::new()
        .with_decode_padding_mode(engine::DecodePaddingMode::RequireCanonical)
        .with_decode_allow_trailing_bits(true);
    engine::GeneralPurpose::new(&alphabet::STANDARD, config).decode(filtered)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sql_values_match_go_wire_forms() {
        let values = vec![
            SqlValue::Null,
            SqlValue::Bytes(vec![0, 255]),
            SqlValue::String("<>&\u{2028}\u{2029}".to_owned()),
            SqlValue::raw(r#" { "n" : 1.2300, "s":"<" } "#).unwrap(),
        ];
        assert_eq!(
            String::from_utf8(go_json_bytes(&values).unwrap()).unwrap(),
            r#"[null,"AP8=","\u003c\u003e\u0026\u2028\u2029",{"n":1.2300,"s":"\u003c"}]"#
        );
        assert!(SqlValue::float(f64::NAN).is_err());
    }

    #[test]
    fn go_bytes_accepts_crlf_and_noncanonical_trailing_bits() {
        let bytes: GoBytes = serde_json::from_str(r#""/\r\nx==""#).unwrap();
        assert_eq!(bytes, GoBytes(Some(vec![255])));
        assert!(serde_json::from_str::<GoBytes>(r#""/w""#).is_err());
        assert_eq!(
            serde_json::from_str::<GoBytes>("null").unwrap(),
            GoBytes(None)
        );
        assert_eq!(
            serde_json::from_str::<GoBytes>("\"\"").unwrap(),
            GoBytes(Some(Vec::new()))
        );
    }
}
