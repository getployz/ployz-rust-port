use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, de};

/// DNS record type. The service currently defines A and AAAA, but the wire
/// contract deliberately permits future string values.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum RecordType {
    A,
    Aaaa,
    Other(String),
}

impl Default for RecordType {
    fn default() -> Self {
        Self::Other(String::new())
    }
}

impl RecordType {
    pub fn new(value: impl Into<String>) -> Self {
        match value.into() {
            value if value == "A" => Self::A,
            value if value == "AAAA" => Self::Aaaa,
            value => Self::Other(value),
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Self::A => "A",
            Self::Aaaa => "AAAA",
            Self::Other(value) => value,
        }
    }

    fn is_empty(&self) -> bool {
        self.as_str().is_empty()
    }
}

impl fmt::Display for RecordType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl From<&str> for RecordType {
    fn from(value: &str) -> Self {
        Self::new(value)
    }
}

impl From<String> for RecordType {
    fn from(value: String) -> Self {
        Self::new(value)
    }
}

impl<'de> Deserialize<'de> for RecordType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        String::deserialize(deserializer).map(Self::new)
    }
}

impl Serialize for RecordType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct DomainResponse {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub token: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct RecordRequest {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(rename = "type", default, skip_serializing_if = "RecordType::is_empty")]
    pub record_type: RecordType,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub values: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct RecordResponse {
    #[serde(flatten)]
    pub record: RecordRequest,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub fqdn: String,
}

fn folded(name: &str, expected: &str) -> bool {
    name.eq_ignore_ascii_case(expected)
}

impl<'de> Deserialize<'de> for DomainResponse {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor;
        impl<'de> de::Visitor<'de> for Visitor {
            type Value = DomainResponse;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a DNS domain response")
            }
            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut out = DomainResponse::default();
                while let Some(key) = map.next_key::<String>()? {
                    if folded(&key, "name") {
                        if let Some(value) = map.next_value::<Option<String>>()? {
                            out.name = value;
                        }
                    } else if folded(&key, "token") {
                        if let Some(value) = map.next_value::<Option<String>>()? {
                            out.token = value;
                        }
                    } else {
                        map.next_value::<de::IgnoredAny>()?;
                    }
                }
                Ok(out)
            }
        }
        deserializer.deserialize_map(Visitor)
    }
}

impl<'de> Deserialize<'de> for RecordRequest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserialize_record(deserializer).map(|(record, _)| record)
    }
}

impl<'de> Deserialize<'de> for RecordResponse {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let (record, fqdn) = deserialize_record(deserializer)?;
        Ok(Self { record, fqdn })
    }
}

fn deserialize_record<'de, D>(deserializer: D) -> Result<(RecordRequest, String), D::Error>
where
    D: Deserializer<'de>,
{
    struct Visitor;
    impl<'de> de::Visitor<'de> for Visitor {
        type Value = (RecordRequest, String);
        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("a DNS record")
        }
        fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
        where
            A: de::MapAccess<'de>,
        {
            let mut record = RecordRequest::default();
            let mut fqdn = String::new();
            while let Some(key) = map.next_key::<String>()? {
                if folded(&key, "name") {
                    if let Some(value) = map.next_value::<Option<String>>()? {
                        record.name = value;
                    }
                } else if folded(&key, "type") {
                    if let Some(value) = map.next_value::<Option<RecordType>>()? {
                        record.record_type = value;
                    }
                } else if folded(&key, "values") {
                    record.values = map.next_value::<Option<Vec<String>>>()?.unwrap_or_default();
                } else if folded(&key, "fqdn") {
                    if let Some(value) = map.next_value::<Option<String>>()? {
                        fqdn = value;
                    }
                } else {
                    map.next_value::<de::IgnoredAny>()?;
                }
            }
            Ok((record, fqdn))
        }
    }
    deserializer.deserialize_map(Visitor)
}
