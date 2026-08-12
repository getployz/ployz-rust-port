use std::error::Error;
use std::fmt;

use ployz_internal_machine_api_pb::{Empty, EmptyResponse, Metadata, google};
use prost::Message;
use tonic::{Code, Status};

use crate::MachineTarget;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PayloadError {
    message: String,
}

impl PayloadError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for PayloadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for PayloadError {}

pub(crate) fn append_machine_info(
    machine: &MachineTarget,
    streaming: bool,
    response: &[u8],
) -> Result<Vec<u8>, PayloadError> {
    let payload = Empty {
        metadata: Some(machine_metadata(machine, String::new(), None)?),
    }
    .encode_to_vec();

    if streaming {
        let mut enhanced = Vec::with_capacity(response.len() + payload.len());
        enhanced.extend_from_slice(response);
        enhanced.extend_from_slice(&payload);
        return Ok(enhanced);
    }

    let (tag, tag_len) = consume_varint(response)?;
    let (_, length_len) = consume_varint(&response[tag_len..])?;
    if tag != 10 {
        return Err(PayloadError::new(format!(
            "unexpected message format: {tag}"
        )));
    }
    if tag_len + length_len > response.len() {
        return Err(PayloadError::new(format!(
            "unexpected message size: {}",
            response.len()
        )));
    }

    let embedded = &response[tag_len + length_len..];
    let mut enhanced = Vec::with_capacity(embedded.len() + payload.len() + 12);
    encode_varint(10, &mut enhanced);
    encode_varint((embedded.len() + payload.len()) as u64, &mut enhanced);
    enhanced.extend_from_slice(embedded);
    enhanced.extend_from_slice(&payload);
    Ok(enhanced)
}

pub(crate) fn build_machine_error(
    machine: &MachineTarget,
    streaming: bool,
    error: &(dyn Error + 'static),
) -> Result<Vec<u8>, PayloadError> {
    let (error_text, rpc_status) = if let Some(status) = error.downcast_ref::<Status>() {
        (
            format!(
                "rpc error: code = {:?} desc = {}",
                status.code(),
                status.message()
            ),
            google_status(status),
        )
    } else {
        (
            error.to_string(),
            google::rpc::Status {
                code: Code::Unknown as i32,
                message: error.to_string(),
                details: Vec::new(),
            },
        )
    };
    let empty = Empty {
        metadata: Some(machine_metadata(machine, error_text, Some(rpc_status))?),
    };
    if streaming {
        Ok(empty.encode_to_vec())
    } else {
        Ok(EmptyResponse {
            messages: vec![empty],
        }
        .encode_to_vec())
    }
}

fn google_status(status: &Status) -> google::rpc::Status {
    if !status.details().is_empty()
        && let Ok(decoded) = google::rpc::Status::decode(status.details())
    {
        return decoded;
    }
    google::rpc::Status {
        code: status.code() as i32,
        message: status.message().to_owned(),
        details: Vec::new(),
    }
}

fn machine_metadata(
    machine: &MachineTarget,
    error: String,
    status: Option<google::rpc::Status>,
) -> Result<Metadata, PayloadError> {
    if !machine.address_is_utf8 {
        return Err(PayloadError::new("string field contains invalid UTF-8"));
    }
    Ok(Metadata {
        machine_id: machine.id.clone(),
        machine_name: machine.name.clone(),
        machine_addr: machine.address.clone(),
        error,
        status,
    })
}

fn consume_varint(bytes: &[u8]) -> Result<(u64, usize), PayloadError> {
    let mut value = 0_u64;
    for (index, byte) in bytes.iter().copied().enumerate().take(10) {
        if index == 9 && byte > 1 {
            return Err(PayloadError::new("variable length integer overflow"));
        }
        value |= u64::from(byte & 0x7f) << (index * 7);
        if byte < 0x80 {
            return Ok((value, index + 1));
        }
    }
    Err(PayloadError::new("unexpected EOF"))
}

fn encode_varint(mut value: u64, output: &mut Vec<u8>) {
    while value >= 0x80 {
        output.push((value as u8) | 0x80);
        value >>= 7;
    }
    output.push(value as u8);
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    fn machine() -> MachineTarget {
        MachineTarget::new("id-2", "machine-b", "fd00::2")
    }

    #[test]
    fn injects_unary_and_streaming_payloads_and_builds_errors() {
        let machine = machine();
        let reply = Empty::default().encode_to_vec();
        let wrapped = EmptyResponse {
            messages: vec![Empty::default()],
        }
        .encode_to_vec();

        let streaming = append_machine_info(&machine, true, &reply).unwrap();
        let decoded = Empty::decode(streaming.as_slice()).unwrap();
        assert_eq!(decoded.metadata.unwrap().machine_id, "id-2");

        let unary = append_machine_info(&machine, false, &wrapped).unwrap();
        let decoded = EmptyResponse::decode(unary.as_slice()).unwrap();
        assert_eq!(
            decoded.messages[0].metadata.as_ref().unwrap().machine_name,
            "machine-b"
        );

        let status = Status::permission_denied("denied");
        let error = build_machine_error(&machine, false, &status).unwrap();
        let decoded = EmptyResponse::decode(error.as_slice()).unwrap();
        let metadata = decoded.messages[0].metadata.as_ref().unwrap();
        assert_eq!(
            metadata.error,
            "rpc error: code = PermissionDenied desc = denied"
        );
        assert_eq!(
            metadata.status.as_ref().unwrap().code,
            Code::PermissionDenied as i32
        );
        assert_eq!(metadata.status.as_ref().unwrap().message, "denied");
    }

    #[test]
    fn matches_oracle_errors_for_malformed_unary_envelopes() {
        let machine = machine();
        assert_eq!(
            append_machine_info(&machine, false, &[0x10, 0])
                .unwrap_err()
                .to_string(),
            "unexpected message format: 16"
        );
        let rewritten = append_machine_info(&machine, false, &[0x0a, 0x7f]).unwrap();
        let decoded = EmptyResponse::decode(rewritten.as_slice()).unwrap();
        assert_eq!(
            decoded.messages[0].metadata.as_ref().unwrap().machine_id,
            "id-2"
        );

        let mut overflow = vec![0x80; 9];
        overflow.push(0x02);
        assert_eq!(
            append_machine_info(&machine, false, &overflow)
                .unwrap_err()
                .to_string(),
            "variable length integer overflow"
        );
        assert_eq!(
            append_machine_info(&machine, false, &[0x80; 9])
                .unwrap_err()
                .to_string(),
            "unexpected EOF"
        );
    }

    #[test]
    fn rejects_non_utf8_scoped_address_when_protobuf_string_is_required() {
        let machine = MachineTarget::from_management_address(
            "id",
            "name",
            "fe80::1".parse().unwrap(),
            vec![b'e', b'n', 0xff],
        );
        assert_eq!(
            append_machine_info(&machine, true, &[])
                .unwrap_err()
                .to_string(),
            "string field contains invalid UTF-8"
        );
    }

    #[test]
    fn go_payload_fixtures_match_byte_for_byte() {
        let fixtures = std::env::var("PLOYZ_GO_FIXTURES_IN").map_or_else(
            |_| include_str!("../tests/fixtures/go_payloads.tsv").to_owned(),
            |path| std::fs::read_to_string(path).unwrap(),
        );
        let machine = machine();
        let error = Status::permission_denied("denied");
        let expected = BTreeMap::from([
            (
                "append_streaming",
                append_machine_info(&machine, true, &[0x08, 0x01]).unwrap(),
            ),
            (
                "append_unary",
                append_machine_info(&machine, false, &[0x0a, 0x00]).unwrap(),
            ),
            (
                "build_error_streaming",
                build_machine_error(&machine, true, &error).unwrap(),
            ),
            (
                "build_error_unary",
                build_machine_error(&machine, false, &error).unwrap(),
            ),
        ]);
        let actual = fixtures
            .lines()
            .map(|line| {
                let (name, payload) = line.split_once('\t').unwrap();
                (name, decode_hex(payload))
            })
            .collect::<BTreeMap<_, _>>();
        assert_eq!(actual, expected);
    }

    fn decode_hex(value: &str) -> Vec<u8> {
        assert_eq!(value.len() % 2, 0);
        value
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                let text = std::str::from_utf8(pair).unwrap();
                u8::from_str_radix(text, 16).unwrap()
            })
            .collect()
    }
}
