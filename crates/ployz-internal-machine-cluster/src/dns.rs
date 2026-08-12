use std::fmt;

use ployz_internal_dns::{
    Client, CreateRecordsError, Error, RecordRequest, RecordResponse, RecordType,
};
use ployz_internal_machine_api_pb::{DnsRecord, dns_record};
use serde::{Deserialize, Deserializer, de};

pub(crate) trait DnsAccess: Send + Sync + 'static {
    fn reserve_domain(&self, endpoint: &str) -> Result<(String, String), Error>;
    fn create_records(
        &self,
        endpoint: &str,
        domain: &str,
        token: &str,
        records: &[RecordRequest],
    ) -> Result<Vec<RecordResponse>, CreateRecordsError>;
}

impl DnsAccess for Client {
    fn reserve_domain(&self, endpoint: &str) -> Result<(String, String), Error> {
        self.reserve_domain(endpoint)
    }

    fn create_records(
        &self,
        endpoint: &str,
        domain: &str,
        token: &str,
        records: &[RecordRequest],
    ) -> Result<Vec<RecordResponse>, CreateRecordsError> {
        self.create_records(endpoint, domain, token, records)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct StoredDomain {
    pub endpoint: String,
    pub name: String,
    pub token: String,
}

pub(crate) fn encode_stored_domain(domain: &StoredDomain) -> Vec<u8> {
    let mut encoded = String::new();
    encoded.push_str("{\"Endpoint\":");
    push_go_json_string(&mut encoded, &domain.endpoint);
    encoded.push_str(",\"Name\":");
    push_go_json_string(&mut encoded, &domain.name);
    encoded.push_str(",\"Token\":");
    push_go_json_string(&mut encoded, &domain.token);
    encoded.push('}');
    encoded.into_bytes()
}

fn push_go_json_string(encoded: &mut String, value: &str) {
    use std::fmt::Write as _;

    encoded.push('"');
    for character in value.chars() {
        match character {
            '"' => encoded.push_str("\\\""),
            '\\' => encoded.push_str("\\\\"),
            '\u{0008}' => encoded.push_str("\\b"),
            '\u{000c}' => encoded.push_str("\\f"),
            '\n' => encoded.push_str("\\n"),
            '\r' => encoded.push_str("\\r"),
            '\t' => encoded.push_str("\\t"),
            '<' => encoded.push_str("\\u003c"),
            '>' => encoded.push_str("\\u003e"),
            '&' => encoded.push_str("\\u0026"),
            '\u{2028}' => encoded.push_str("\\u2028"),
            '\u{2029}' => encoded.push_str("\\u2029"),
            character if character <= '\u{001f}' => {
                write!(encoded, "\\u{:04x}", u32::from(character))
                    .expect("writing to String cannot fail");
            }
            character => encoded.push(character),
        }
    }
    encoded.push('"');
}

impl<'de> Deserialize<'de> for StoredDomain {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct Visitor;
        impl<'de> de::Visitor<'de> for Visitor {
            type Value = StoredDomain;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a stored Uncloud DNS domain")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: de::MapAccess<'de>,
            {
                let mut domain = StoredDomain {
                    endpoint: String::new(),
                    name: String::new(),
                    token: String::new(),
                };
                while let Some(key) = map.next_key::<String>()? {
                    if key.eq_ignore_ascii_case("Endpoint") {
                        if let Some(value) = map.next_value::<Option<String>>()? {
                            domain.endpoint = value;
                        }
                    } else if key.eq_ignore_ascii_case("Name") {
                        if let Some(value) = map.next_value::<Option<String>>()? {
                            domain.name = value;
                        }
                    } else if key.eq_ignore_ascii_case("Token") {
                        if let Some(value) = map.next_value::<Option<String>>()? {
                            domain.token = value;
                        }
                    } else {
                        map.next_value::<de::IgnoredAny>()?;
                    }
                }
                Ok(domain)
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                Ok(StoredDomain {
                    endpoint: String::new(),
                    name: String::new(),
                    token: String::new(),
                })
            }

            fn visit_none<E>(self) -> Result<Self::Value, E>
            where
                E: de::Error,
            {
                self.visit_unit()
            }
        }
        deserializer.deserialize_any(Visitor)
    }
}

pub(crate) fn map_record_request(record: &DnsRecord) -> RecordRequest {
    RecordRequest {
        name: record.name.clone(),
        record_type: RecordType::new(match dns_record::RecordType::try_from(record.r#type) {
            Ok(value) => value.as_str_name().to_owned(),
            Err(_) => record.r#type.to_string(),
        }),
        values: record.values.clone(),
    }
}

pub(crate) fn map_record_response(record: &RecordResponse) -> DnsRecord {
    let record_type = match record.record.record_type {
        RecordType::A => dns_record::RecordType::A,
        RecordType::Aaaa => dns_record::RecordType::Aaaa,
        RecordType::Other(_) => dns_record::RecordType::Unspecified,
    };
    DnsRecord {
        name: record.fqdn.clone(),
        r#type: record_type as i32,
        values: record.record.values.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stored_domain_json_matches_go_and_accepts_folded_duplicate_fields() {
        let domain = StoredDomain {
            endpoint: "https://dns.example".into(),
            name: "example.test".into(),
            token: "token".into(),
        };
        assert_eq!(
            String::from_utf8(encode_stored_domain(&domain)).unwrap(),
            r#"{"Endpoint":"https://dns.example","Name":"example.test","Token":"token"}"#
        );
        let decoded: StoredDomain = serde_json::from_str(
            r#"{"endpoint":"old","ENDPOINT":"new","name":null,"NAME":"example.test","unknown":1}"#,
        )
        .unwrap();
        assert_eq!(decoded.endpoint, "new");
        assert_eq!(decoded.name, "example.test");
        assert_eq!(decoded.token, "");

        let null: StoredDomain = serde_json::from_str("null").unwrap();
        assert_eq!(null.endpoint, "");
        assert_eq!(null.name, "");
        assert_eq!(null.token, "");
    }

    #[test]
    fn stored_domain_json_uses_go_html_and_javascript_escaping() {
        let domain = StoredDomain {
            endpoint: "<&>\u{2028}\u{2029}".into(),
            name: "line\nname".into(),
            token: "\u{0001}".into(),
        };
        assert_eq!(
            String::from_utf8(encode_stored_domain(&domain)).unwrap(),
            r#"{"Endpoint":"\u003c\u0026\u003e\u2028\u2029","Name":"line\nname","Token":"\u0001"}"#
        );
    }

    #[test]
    fn record_mapping_preserves_unknown_request_number_and_zeroes_unknown_response_type() {
        let request = map_record_request(&DnsRecord {
            name: "x".into(),
            r#type: 99,
            values: vec!["v".into()],
        });
        assert_eq!(request.record_type, RecordType::Other("99".into()));
        let response = map_record_response(&RecordResponse {
            record: request,
            fqdn: "x.example".into(),
        });
        assert_eq!(response.r#type, dns_record::RecordType::Unspecified as i32);
    }
}
