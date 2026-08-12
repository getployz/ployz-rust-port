use std::{collections::BTreeMap, pin::Pin};

use futures_util::{Stream, stream};
use serde::{Deserialize, Serialize};

use crate::{Cancellation, CancellationError, ProgressError};

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressDetail {
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current: Option<i64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<i64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start: Option<i64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hidecounts: Option<bool>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub units: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
pub struct ErrorDetail {
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub code: Option<i64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProgressMessage {
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub progress_detail: Option<ProgressDetail>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error_detail: Option<ErrorDetail>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time: Option<i64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_nano: Option<i64>,
    #[serde(default)]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub aux: Option<serde_json::Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Debug)]
pub struct ProgressItem {
    pub message: ProgressMessage,
    pub error: Option<ProgressError>,
}

pub type ProgressStream = Pin<Box<dyn Stream<Item = ProgressItem> + Send>>;

struct DecodeState {
    response: Option<reqwest::Response>,
    cancellation: Cancellation,
    buffer: Vec<u8>,
    eof: bool,
    finished: bool,
}

pub(crate) fn progress_stream(
    response: reqwest::Response,
    cancellation: Cancellation,
) -> ProgressStream {
    let state = DecodeState {
        response: Some(response),
        cancellation,
        buffer: Vec::new(),
        eof: false,
        finished: false,
    };
    Box::pin(stream::unfold(state, |mut state| async move {
        let item = state.next().await?;
        Some((item, state))
    }))
}

impl DecodeState {
    async fn next(&mut self) -> Option<ProgressItem> {
        if self.finished {
            return None;
        }

        loop {
            match decode_one(&mut self.buffer) {
                Ok(Some(message)) => {
                    if self.cancellation.is_cancelled() {
                        self.finished = true;
                        self.response.take();
                        return Some(error_item(ProgressError::Cancelled(CancellationError)));
                    }
                    let error = message.error_detail.as_ref().map(|detail| {
                        ProgressError::Embedded(detail.message.clone().unwrap_or_default())
                    });
                    return Some(ProgressItem { message, error });
                }
                Ok(None) if self.eof => {
                    self.finished = true;
                    self.response.take();
                    if self.buffer.iter().all(u8::is_ascii_whitespace) {
                        return None;
                    }
                    return Some(error_item(ProgressError::DecodeIo(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "unexpected EOF",
                    ))));
                }
                Ok(None) => {}
                Err(error) => {
                    self.finished = true;
                    self.response.take();
                    return Some(error_item(ProgressError::DecodeJson(error)));
                }
            }

            let Some(response) = self.response.as_mut() else {
                self.finished = true;
                return None;
            };
            let chunk = tokio::select! {
                biased;
                chunk = response.chunk() => chunk,
                _ = self.cancellation.cancelled() => {
                    self.finished = true;
                    self.response.take();
                    return Some(error_item(ProgressError::DecodeCancelled(CancellationError)));
                }
            };
            match chunk {
                Ok(Some(bytes)) => self.buffer.extend_from_slice(&bytes),
                Ok(None) => self.eof = true,
                Err(error) => {
                    self.finished = true;
                    self.response.take();
                    return Some(error_item(ProgressError::DecodeTransport(error)));
                }
            }
        }
    }
}

fn error_item(error: ProgressError) -> ProgressItem {
    ProgressItem {
        message: ProgressMessage::default(),
        error: Some(error),
    }
}

fn decode_one(buffer: &mut Vec<u8>) -> Result<Option<ProgressMessage>, serde_json::Error> {
    let mut values =
        serde_json::Deserializer::from_slice(buffer).into_iter::<Option<ProgressMessage>>();
    match values.next() {
        Some(Ok(message)) => {
            let consumed = values.byte_offset();
            buffer.drain(..consumed);
            Ok(Some(message.unwrap_or_default()))
        }
        Some(Err(error)) if error.is_eof() => Ok(None),
        Some(Err(error)) => Err(error),
        None => {
            buffer.clear();
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fmt::Write as _;

    use super::*;

    #[test]
    fn decoder_is_lossless_across_values_and_null() {
        let mut buffer = br#"{"id":"layer","progress":"2/4","progressDetail":{"current":2,"total":4,"start":1,"hidecounts":false,"units":"B","future":7},"futureTop":true}null{"errorDetail":{"code":500,"future":true}}"#.to_vec();
        let first = decode_one(&mut buffer).unwrap().unwrap();
        assert_eq!(first.id.as_deref(), Some("layer"));
        assert_eq!(first.progress_detail.as_ref().unwrap().start, Some(1));
        assert_eq!(first.progress_detail.as_ref().unwrap().extra["future"], 7);
        assert_eq!(first.extra["futureTop"], true);
        assert_eq!(
            decode_one(&mut buffer).unwrap(),
            Some(ProgressMessage::default())
        );
        let third = decode_one(&mut buffer).unwrap().unwrap();
        assert_eq!(third.error_detail.as_ref().unwrap().code, Some(500));
        assert_eq!(third.error_detail.as_ref().unwrap().extra["future"], true);
    }

    #[test]
    fn incomplete_value_waits_for_more_bytes() {
        let mut buffer = br#"{"id":"partial"#.to_vec();
        assert_eq!(decode_one(&mut buffer).unwrap(), None);
        buffer.extend_from_slice(b"\"}");
        assert_eq!(
            decode_one(&mut buffer).unwrap().unwrap().id.as_deref(),
            Some("partial")
        );
    }

    #[test]
    fn writes_progress_fixture_for_go_differential_check() {
        let Ok(output_path) = std::env::var("PLOYZ_RUST_PROGRESS_OUT") else {
            return;
        };
        let mut buffer = include_bytes!("../tests/fixtures/progress.stream.json").to_vec();
        let mut output = String::new();
        while let Some(message) = decode_one(&mut buffer).unwrap() {
            let detail = message.progress_detail.as_ref();
            let error = message.error_detail.as_ref();
            let aux = message
                .aux
                .as_ref()
                .map(serde_json::Value::to_string)
                .unwrap_or_default();
            writeln!(
                output,
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                message.id.as_deref().unwrap_or_default(),
                message.status.as_deref().unwrap_or_default(),
                message.progress.as_deref().unwrap_or_default(),
                detail.and_then(|value| value.current).unwrap_or_default(),
                detail.and_then(|value| value.total).unwrap_or_default(),
                detail.and_then(|value| value.start).unwrap_or_default(),
                detail
                    .and_then(|value| value.hidecounts)
                    .unwrap_or_default(),
                detail
                    .and_then(|value| value.units.as_deref())
                    .unwrap_or_default(),
                error.and_then(|value| value.code).unwrap_or_default(),
                error
                    .and_then(|value| value.message.as_deref())
                    .unwrap_or_default(),
                message.error.as_deref().unwrap_or_default(),
                message.stream.as_deref().unwrap_or_default(),
                message.from.as_deref().unwrap_or_default(),
                message.time.unwrap_or_default(),
                message.time_nano.unwrap_or_default(),
                aux,
            )
            .unwrap();
        }
        assert!(buffer.iter().all(u8::is_ascii_whitespace));
        std::fs::write(output_path, output).unwrap();
    }
}
