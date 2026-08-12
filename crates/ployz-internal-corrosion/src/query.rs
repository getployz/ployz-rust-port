use std::{fmt, net::SocketAddr};

use backon::Retryable as _;
use bytes::Bytes;
#[cfg(test)]
use hyper::Uri;
use hyper::{Method, StatusCode};
use serde::{
    Deserialize, Deserializer, Serialize,
    de::{DeserializeOwned, MapAccess, Visitor},
};
use serde_json::value::RawValue;

use crate::{
    backoff::RandomizedBackoff,
    json::{JsonColumn, SqlValue, go_json_bytes},
    transport::{ClientError, ClientErrorKind, HttpResponse, ResponseReader, Transport},
};

#[derive(Clone, Debug, Serialize)]
pub struct Statement {
    pub query: String,
    /// `None` is the Go nil-slice wire form (`null`); `Some(vec![])` is `[]`.
    pub params: Option<Vec<SqlValue>>,
}

impl Statement {
    pub fn new(query: impl Into<String>, params: Option<Vec<SqlValue>>) -> Self {
        Self {
            query: query.into(),
            params,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExecResponse {
    pub results: Vec<ExecResult>,
    pub time: f64,
    pub version: Option<u64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct ExecResult {
    pub rows_affected: u64,
    pub time: f64,
    pub error: Option<String>,
}

#[derive(Debug)]
pub struct ExecError {
    pub response: Option<ExecResponse>,
    pub error: ClientError,
}

impl fmt::Display for ExecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.error.fmt(formatter)
    }
}

impl std::error::Error for ExecError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.error)
    }
}

impl From<ClientError> for ExecError {
    fn from(error: ClientError) -> Self {
        Self {
            response: None,
            error,
        }
    }
}

#[derive(Clone)]
pub struct ApiClient {
    transport: Transport,
    resubscribe_backoff: Option<RandomizedBackoff>,
}

impl fmt::Debug for ApiClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ApiClient")
            .field("resubscribe_enabled", &self.resubscribe_backoff.is_some())
            .finish_non_exhaustive()
    }
}

impl ApiClient {
    pub fn new(address: SocketAddr, bearer_token: impl Into<String>) -> Result<Self, ClientError> {
        Ok(Self {
            transport: Transport::new(address, bearer_token)?,
            resubscribe_backoff: Some(RandomizedBackoff::subscription()),
        })
    }

    pub fn with_resubscribe(mut self, enabled: bool) -> Self {
        self.resubscribe_backoff = enabled.then(RandomizedBackoff::subscription);
        self
    }

    #[cfg(test)]
    fn test_client(base: Uri, resubscribe_backoff: Option<RandomizedBackoff>) -> Self {
        Self {
            transport: Transport::with_base(base, "test-token"),
            resubscribe_backoff,
        }
    }

    pub async fn exec(
        &self,
        query: impl Into<String>,
        params: Option<Vec<SqlValue>>,
    ) -> Result<ExecResult, ExecError> {
        let statements = [Statement::new(query, params)];
        let response = self.exec_multi(Some(&statements)).await?;
        if let Some(result) = response.results.first().cloned() {
            Ok(result)
        } else {
            let message = format!("no results: {response:?}");
            Err(ExecError {
                response: Some(response),
                error: ClientError::new(ClientErrorKind::Protocol, message),
            })
        }
    }

    /// Executes an optional statement list. `None` preserves Go's zero-variadic
    /// `null` wire form; `Some(&[])` sends an explicit non-nil empty list.
    pub async fn exec_multi(
        &self,
        statements: Option<&[Statement]>,
    ) -> Result<ExecResponse, ExecError> {
        let body = go_json_bytes(&statements)?;
        let uri = self.transport.endpoint("/v1/transactions")?;
        let response = self
            .transport
            .request(Method::POST, uri, Bytes::from(body))
            .await?;
        let status = response.status;
        let response_body = response.body.read_to_end().await?;

        if status == StatusCode::OK {
            let response = parse_exec_response(&response_body).map_err(ExecError::from)?;
            let errors: Vec<String> = response
                .results
                .iter()
                .filter_map(|result| result.error.clone())
                .collect();
            if errors.is_empty() {
                Ok(response)
            } else {
                Err(ExecError {
                    response: Some(response),
                    error: ClientError::new(ClientErrorKind::Protocol, errors.join("\n")),
                })
            }
        } else if status == StatusCode::INTERNAL_SERVER_ERROR {
            match parse_exec_response(&response_body) {
                Ok(response)
                    if response
                        .results
                        .first()
                        .and_then(|result| result.error.as_ref())
                        .is_some() =>
                {
                    let message = response.results[0].error.clone().expect("checked above");
                    Err(ExecError {
                        response: None,
                        error: ClientError::new(ClientErrorKind::Protocol, message),
                    })
                }
                _ => Err(ExecError {
                    response: None,
                    error: ClientError::new(
                        ClientErrorKind::Http,
                        format!(
                            "internal server error: {}",
                            String::from_utf8_lossy(&response_body)
                        ),
                    ),
                }),
            }
        } else {
            Err(ExecError {
                response: None,
                error: unexpected_status(status, &response_body),
            })
        }
    }

    pub async fn query(
        &self,
        query: impl Into<String>,
        params: Option<Vec<SqlValue>>,
    ) -> Result<Rows, ClientError> {
        let body = go_json_bytes(&Statement::new(query, params))?;
        let uri = self.transport.endpoint("/v1/queries")?;
        let response = self
            .transport
            .request(Method::POST, uri, Bytes::from(body))
            .await?;
        rows_from_response(response, true).await
    }

    pub async fn subscribe(
        &self,
        query: impl Into<String>,
        params: Option<Vec<SqlValue>>,
        skip_rows: bool,
    ) -> Result<Subscription, ClientError> {
        let body = go_json_bytes(&Statement::new(query, params))?;
        let endpoint = if skip_rows {
            "/v1/subscriptions?skip_rows=true"
        } else {
            "/v1/subscriptions"
        };
        let uri = self.transport.endpoint(endpoint)?;
        let response = self
            .transport
            .request(Method::POST, uri, Bytes::from(body))
            .await?;
        if response.status != StatusCode::OK {
            return Err(status_with_body(response).await);
        }
        let id = response
            .headers
            .get("corro-query-id")
            .and_then(|value| value.to_str().ok())
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                ClientError::new(
                    ClientErrorKind::Protocol,
                    "missing corro-query-id header in response",
                )
            })?
            .to_owned();

        let (rows, decoder) = if skip_rows {
            (None, Some(NdjsonDecoder::new(response.body)))
        } else {
            (Some(Rows::new(response.body, false).await?), None)
        };
        Ok(Subscription {
            id,
            rows,
            decoder,
            client: self.clone(),
        })
    }

    async fn resubscribe_once(
        &self,
        id: &str,
        from_change: u64,
    ) -> Result<NdjsonDecoder, ClientError> {
        let id = encode_path_segment(id);
        let uri = self
            .transport
            .endpoint(&format!("/v1/subscriptions/{id}?from={from_change}"))?;
        let response = self
            .transport
            .request(Method::GET, uri, Bytes::new())
            .await?;
        if response.status == StatusCode::NOT_FOUND {
            return Err(ClientError::new(
                ClientErrorKind::SubscriptionNotFound,
                "subscription not found",
            ));
        }
        if response.status != StatusCode::OK {
            return Err(status_with_body(response).await);
        }

        if from_change == 0 {
            let mut rows = Rows::new(response.body, false).await.map_err(|error| {
                ClientError::with_source(
                    ClientErrorKind::Protocol,
                    "parse resubscribe response",
                    error,
                )
            })?;
            while rows.next().await?.is_some() {}
            rows.take_decoder().ok_or_else(|| {
                ClientError::new(
                    ClientErrorKind::Protocol,
                    "drain resubscribe snapshot closed the response",
                )
            })
        } else {
            Ok(NdjsonDecoder::new(response.body))
        }
    }
}

pub struct Rows {
    columns: Vec<String>,
    decoder: Option<NdjsonDecoder>,
    end: Option<EndOfQuery>,
    close_on_end: bool,
    failed: bool,
}

impl fmt::Debug for Rows {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Rows")
            .field("columns", &self.columns)
            .field("end", &self.end)
            .field("closed", &self.decoder.is_none())
            .finish()
    }
}

impl Rows {
    async fn new(body: ResponseReader, close_on_end: bool) -> Result<Self, ClientError> {
        let mut decoder = NdjsonDecoder::new(body);
        let raw = decoder.next_raw().await?.ok_or_else(|| {
            ClientError::new(ClientErrorKind::Protocol, "decode query event: EOF")
        })?;
        let event = parse_query_event(raw.get())?;
        let columns = match event.columns {
            Some(columns) => columns,
            None => {
                return Err(ClientError::new(
                    ClientErrorKind::Protocol,
                    format!("expected columns event, got: {event:?}"),
                ));
            }
        };
        Ok(Self {
            columns,
            decoder: Some(decoder),
            end: None,
            close_on_end,
            failed: false,
        })
    }

    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    pub async fn next(&mut self) -> Result<Option<Row>, ClientError> {
        if self.failed || self.end.is_some() {
            return Ok(None);
        }
        let Some(decoder) = self.decoder.as_mut() else {
            return Ok(None);
        };
        let raw = match decoder.next_raw().await {
            Ok(Some(raw)) => raw,
            Ok(None) => {
                self.failed = true;
                self.decoder = None;
                return Err(ClientError::new(
                    ClientErrorKind::Protocol,
                    "decode query event: EOF",
                ));
            }
            Err(error) => {
                self.failed = true;
                self.decoder = None;
                return Err(error);
            }
        };
        let event = match parse_query_event(raw.get()) {
            Ok(event) => event,
            Err(error) => {
                self.failed = true;
                self.decoder = None;
                return Err(error);
            }
        };
        if let Some(error) = event.error {
            self.failed = true;
            self.decoder = None;
            return Err(ClientError::new(
                ClientErrorKind::Protocol,
                format!("query error: {error}"),
            ));
        }
        if let Some(row) = event.row {
            let value_count = row.values.as_ref().map_or(0, Vec::len);
            if value_count != self.columns.len() {
                self.failed = true;
                self.decoder = None;
                return Err(ClientError::new(
                    ClientErrorKind::Protocol,
                    format!(
                        "expected {} column values, got {}",
                        self.columns.len(),
                        value_count
                    ),
                ));
            }
            return Ok(Some(row));
        }
        if let Some(end) = event.end {
            self.end = Some(end);
            if self.close_on_end {
                self.decoder = None;
            }
            return Ok(None);
        }

        self.failed = true;
        self.decoder = None;
        Err(ClientError::new(
            ClientErrorKind::Protocol,
            format!("expected row or eof event, got: {event:?}"),
        ))
    }

    pub fn time(&self) -> Result<f64, ClientError> {
        if let Some(end) = &self.end {
            Ok(end.time)
        } else if self.failed {
            Err(ClientError::new(
                ClientErrorKind::Protocol,
                "time is not available: row iteration failed",
            ))
        } else {
            Err(ClientError::new(
                ClientErrorKind::Protocol,
                "time is not available until all rows are consumed",
            ))
        }
    }

    pub fn close(&mut self) {
        self.decoder = None;
    }

    fn take_decoder(&mut self) -> Option<NdjsonDecoder> {
        self.decoder.take()
    }
}

#[derive(Clone, Debug)]
pub struct Row {
    pub id: u64,
    pub values: Option<Vec<JsonColumn>>,
}

impl Row {
    pub fn get<T: DeserializeOwned + Default>(&self, index: usize) -> Result<T, ClientError> {
        let mut target = T::default();
        self.decode_into(index, &mut target)?;
        Ok(target)
    }

    /// Decodes a column, leaving the destination untouched for JSON `null` like Go.
    pub fn decode_into<T: DeserializeOwned>(
        &self,
        index: usize,
        target: &mut T,
    ) -> Result<(), ClientError> {
        let value = self
            .values
            .as_ref()
            .and_then(|values| values.get(index))
            .ok_or_else(|| {
                ClientError::new(
                    ClientErrorKind::Protocol,
                    format!("column index {index} is out of bounds"),
                )
            })?;
        if value.is_null() {
            return Ok(());
        }
        *target = value.decode().map_err(|error| {
            ClientError::with_source(
                ClientErrorKind::Json,
                format!("unmarshal column value #{index}"),
                error,
            )
        })?;
        Ok(())
    }

    pub fn expect_columns(&self, count: usize) -> Result<(), ClientError> {
        let actual = self.values.as_ref().map_or(0, Vec::len);
        if actual == count {
            Ok(())
        } else {
            Err(ClientError::new(
                ClientErrorKind::Protocol,
                format!("expected {count} values, got {actual}"),
            ))
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EndOfQuery {
    pub time: f64,
    pub change_id: Option<u64>,
}

pub struct Subscription {
    id: String,
    rows: Option<Rows>,
    decoder: Option<NdjsonDecoder>,
    client: ApiClient,
}

impl fmt::Debug for Subscription {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Subscription")
            .field("id", &self.id)
            .field("has_rows", &self.rows.is_some())
            .finish()
    }
}

impl Subscription {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn rows(&self) -> Option<&Rows> {
        self.rows.as_ref()
    }

    pub fn rows_mut(&mut self) -> Option<&mut Rows> {
        self.rows.as_mut()
    }

    pub fn into_changes(mut self) -> Result<ChangeStream, ClientError> {
        let (decoder, last_change_id) = if let Some(mut rows) = self.rows.take() {
            let end = rows.end.as_ref().ok_or_else(|| {
                ClientError::new(
                    ClientErrorKind::Protocol,
                    "changes are not available until all rows are consumed",
                )
            })?;
            let change_id = end.change_id.ok_or_else(|| {
                ClientError::new(
                    ClientErrorKind::Protocol,
                    "end-of-query event has no change ID",
                )
            })?;
            let decoder = rows.take_decoder().ok_or_else(|| {
                ClientError::new(ClientErrorKind::Protocol, "subscription response is closed")
            })?;
            (decoder, change_id)
        } else {
            (
                self.decoder.take().ok_or_else(|| {
                    ClientError::new(ClientErrorKind::Protocol, "subscription response is closed")
                })?,
                0,
            )
        };
        Ok(ChangeStream {
            id: self.id,
            decoder: Some(decoder),
            last_change_id,
            client: self.client,
            needs_resubscribe: false,
            finished: false,
        })
    }
}

pub struct ChangeStream {
    id: String,
    decoder: Option<NdjsonDecoder>,
    last_change_id: u64,
    client: ApiClient,
    needs_resubscribe: bool,
    finished: bool,
}

impl fmt::Debug for ChangeStream {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ChangeStream")
            .field("id", &self.id)
            .field("last_change_id", &self.last_change_id)
            .field("needs_resubscribe", &self.needs_resubscribe)
            .field("finished", &self.finished)
            .finish()
    }
}

impl ChangeStream {
    pub fn id(&self) -> &str {
        &self.id
    }

    pub fn last_change_id(&self) -> u64 {
        self.last_change_id
    }

    pub async fn next(&mut self) -> Option<Result<ChangeEvent, ClientError>> {
        if self.finished {
            return None;
        }
        loop {
            if self.needs_resubscribe {
                match self.resubscribe().await {
                    Ok(decoder) => {
                        self.decoder = Some(decoder);
                        self.needs_resubscribe = false;
                    }
                    Err(error) => {
                        self.finished = true;
                        self.decoder = None;
                        return Some(Err(ClientError::with_source(
                            ClientErrorKind::Protocol,
                            "resubscribe to query with backoff",
                            error,
                        )));
                    }
                }
            }

            let result = match self
                .decoder
                .as_mut()
                .expect("active stream has a decoder")
                .next_raw()
                .await
            {
                Ok(Some(raw)) => parse_query_event(raw.get()).and_then(|event| {
                    if let Some(error) = event.error {
                        Err(ClientError::new(
                            ClientErrorKind::Protocol,
                            format!("query error: {error}"),
                        ))
                    } else if let Some(change) = event.change {
                        let expected = expected_change_id(self.last_change_id);
                        if expected.is_some_and(|expected| change.change_id != expected) {
                            Err(ClientError::new(
                                ClientErrorKind::Protocol,
                                format!(
                                    "missed a change: expected change ID {}, got {}",
                                    expected.expect("a mismatched ID has an expectation"),
                                    change.change_id
                                ),
                            ))
                        } else {
                            Ok(change)
                        }
                    } else {
                        Err(ClientError::new(
                            ClientErrorKind::Protocol,
                            format!("expected change event, got: {event:?}"),
                        ))
                    }
                }),
                Ok(None) => Err(ClientError::new(
                    ClientErrorKind::Protocol,
                    "decode query event: EOF",
                )),
                Err(error) => Err(error),
            };

            match result {
                Ok(change) => {
                    self.last_change_id = change.change_id;
                    return Some(Ok(change));
                }
                Err(error) if self.client.resubscribe_backoff.is_none() => {
                    self.finished = true;
                    self.decoder = None;
                    return Some(Err(error));
                }
                Err(_) => {
                    self.decoder = None;
                    self.needs_resubscribe = true;
                }
            }
        }
    }

    async fn resubscribe(&self) -> Result<NdjsonDecoder, ClientError> {
        let backoff = self
            .client
            .resubscribe_backoff
            .clone()
            .expect("resubscribe checked as enabled");
        (|| self.client.resubscribe_once(&self.id, self.last_change_id))
            .retry(backoff)
            .when(|error| !error.is_subscription_not_found())
            .await
    }
}

fn expected_change_id(last_change_id: u64) -> Option<u64> {
    (last_change_id != 0).then(|| last_change_id.wrapping_add(1))
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChangeType {
    Insert,
    Update,
    Delete,
    Other(String),
}

impl ChangeType {
    pub fn as_str(&self) -> &str {
        match self {
            Self::Insert => "insert",
            Self::Update => "update",
            Self::Delete => "delete",
            Self::Other(value) => value,
        }
    }
}

#[derive(Clone, Debug)]
pub struct ChangeEvent {
    pub change_type: ChangeType,
    pub row_id: u64,
    pub values: Option<Vec<JsonColumn>>,
    pub change_id: u64,
}

impl ChangeEvent {
    pub fn get<T: DeserializeOwned + Default>(&self, index: usize) -> Result<T, ClientError> {
        let mut target = T::default();
        self.decode_into(index, &mut target)?;
        Ok(target)
    }

    pub fn decode_into<T: DeserializeOwned>(
        &self,
        index: usize,
        target: &mut T,
    ) -> Result<(), ClientError> {
        let value = self
            .values
            .as_ref()
            .and_then(|values| values.get(index))
            .ok_or_else(|| {
                ClientError::new(
                    ClientErrorKind::Protocol,
                    format!("column index {index} is out of bounds"),
                )
            })?;
        if value.is_null() {
            return Ok(());
        }
        *target = value.decode().map_err(|error| {
            ClientError::with_source(
                ClientErrorKind::Json,
                format!("unmarshal column value #{index}"),
                error,
            )
        })?;
        Ok(())
    }

    pub fn to_json(&self) -> Result<Vec<u8>, ClientError> {
        #[derive(Serialize)]
        struct Wire<'a>(&'a str, u64, Option<Vec<&'a RawValue>>, u64);
        go_json_bytes(&Wire(
            self.change_type.as_str(),
            self.row_id,
            self.values
                .as_ref()
                .map(|values| values.iter().map(|value| value.0.as_ref()).collect()),
            self.change_id,
        ))
    }
}

struct NdjsonDecoder {
    body: ResponseReader,
    buffer: Vec<u8>,
    eof: bool,
}

impl NdjsonDecoder {
    fn new(body: ResponseReader) -> Self {
        Self {
            body,
            buffer: Vec::new(),
            eof: false,
        }
    }

    async fn next_raw(&mut self) -> Result<Option<Box<RawValue>>, ClientError> {
        loop {
            let whitespace = self
                .buffer
                .iter()
                .position(|byte| !byte.is_ascii_whitespace())
                .unwrap_or(self.buffer.len());
            if whitespace != 0 {
                self.buffer.drain(..whitespace);
            }

            if !self.buffer.is_empty() {
                let mut stream =
                    serde_json::Deserializer::from_slice(&self.buffer).into_iter::<Box<RawValue>>();
                match stream.next() {
                    Some(Ok(value)) => {
                        let consumed = stream.byte_offset();
                        self.buffer.drain(..consumed);
                        return Ok(Some(value));
                    }
                    Some(Err(error)) if error.is_eof() && !self.eof => {}
                    Some(Err(error)) => {
                        return Err(ClientError::with_source(
                            ClientErrorKind::Json,
                            "decode query event",
                            error,
                        ));
                    }
                    None => {}
                }
            }

            if self.eof {
                return Ok(None);
            }
            let mut chunk = [0_u8; 8192];
            match self.body.read(&mut chunk).await? {
                0 => self.eof = true,
                read => self.buffer.extend_from_slice(&chunk[..read]),
            }
        }
    }
}

#[derive(Debug, Default)]
struct QueryEvent {
    columns: Option<Vec<String>>,
    row: Option<Row>,
    end: Option<EndOfQuery>,
    change: Option<ChangeEvent>,
    error: Option<String>,
}

#[derive(Debug)]
struct ObjectPairs(Vec<(String, Box<RawValue>)>);

impl<'de> Deserialize<'de> for ObjectPairs {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct PairsVisitor;

        impl<'de> Visitor<'de> for PairsVisitor {
            type Value = ObjectPairs;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a JSON object")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut pairs = Vec::new();
                while let Some(key) = map.next_key::<String>()? {
                    pairs.push((key, map.next_value::<Box<RawValue>>()?));
                }
                Ok(ObjectPairs(pairs))
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(ObjectPairs(Vec::new()))
            }

            fn visit_none<E>(self) -> Result<Self::Value, E> {
                Ok(ObjectPairs(Vec::new()))
            }
        }

        deserializer.deserialize_any(PairsVisitor)
    }
}

fn parse_query_event(data: &str) -> Result<QueryEvent, ClientError> {
    let ObjectPairs(pairs) = decode(data, "decode query event")?;
    let mut event = QueryEvent::default();
    for (key, value) in pairs {
        if key.eq_ignore_ascii_case("columns") {
            event.columns = decode_nullable(value.get(), "invalid columns event")?;
        } else if key.eq_ignore_ascii_case("row") {
            event.row = if raw_is_null(&value) {
                None
            } else {
                Some(parse_row_event(value.get())?)
            };
        } else if key.eq_ignore_ascii_case("eoq") {
            if raw_is_null(&value) {
                event.end = None;
            } else {
                let mut end = event.end.take().unwrap_or_default();
                merge_end_of_query(&mut end, value.get())?;
                event.end = Some(end);
            }
        } else if key.eq_ignore_ascii_case("change") {
            event.change = if raw_is_null(&value) {
                None
            } else {
                Some(parse_change_event(value.get())?)
            };
        } else if key.eq_ignore_ascii_case("error") {
            event.error = decode_nullable(value.get(), "invalid query error")?;
        }
    }
    Ok(event)
}

fn parse_row_event(data: &str) -> Result<Row, ClientError> {
    let raw: Vec<Box<RawValue>> = decode(data, "invalid row event")?;
    if raw.len() != 2 {
        return Err(ClientError::new(
            ClientErrorKind::Protocol,
            "invalid row event: expected an array of 2 elements",
        ));
    }
    let id = decode(raw[0].get(), "invalid row event row ID")?;
    let values = parse_values(raw[1].get(), "invalid row event values")?;
    Ok(Row { id, values })
}

fn parse_change_event(data: &str) -> Result<ChangeEvent, ClientError> {
    let raw: Vec<Box<RawValue>> = decode(data, "invalid change event")?;
    if raw.len() != 4 {
        return Err(ClientError::new(
            ClientErrorKind::Protocol,
            "invalid change event: expected an array of 4 elements",
        ));
    }
    let change_type: String = decode(raw[0].get(), "invalid change event type")?;
    let change_type = match change_type.as_str() {
        "insert" => ChangeType::Insert,
        "update" => ChangeType::Update,
        "delete" => ChangeType::Delete,
        _ => ChangeType::Other(change_type),
    };
    Ok(ChangeEvent {
        change_type,
        row_id: decode(raw[1].get(), "invalid change event row ID")?,
        values: parse_values(raw[2].get(), "invalid change event values")?,
        change_id: decode(raw[3].get(), "invalid change event change ID")?,
    })
}

fn parse_values(data: &str, context: &str) -> Result<Option<Vec<JsonColumn>>, ClientError> {
    let raw: Option<Vec<Box<RawValue>>> = decode(data, context)?;
    Ok(raw.map(|values| values.into_iter().map(JsonColumn).collect()))
}

fn merge_end_of_query(end: &mut EndOfQuery, data: &str) -> Result<(), ClientError> {
    let ObjectPairs(pairs) = decode(data, "invalid end-of-query event")?;
    for (key, value) in pairs {
        if key.eq_ignore_ascii_case("time") && !raw_is_null(&value) {
            end.time = decode(value.get(), "invalid end-of-query time")?;
        } else if key.eq_ignore_ascii_case("change_id") {
            end.change_id = decode_nullable(value.get(), "invalid end-of-query change ID")?;
        }
    }
    Ok(())
}

fn parse_exec_response(data: &[u8]) -> Result<ExecResponse, ClientError> {
    let text = std::str::from_utf8(data).map_err(|error| {
        ClientError::with_source(ClientErrorKind::Json, "decode response", error)
    })?;
    let ObjectPairs(pairs) = decode(text, "decode response")?;
    let mut response = ExecResponse::default();
    for (key, value) in pairs {
        if key.eq_ignore_ascii_case("results") {
            if !raw_is_null(&value) {
                let results: Vec<Box<RawValue>> = decode(value.get(), "invalid results")?;
                response.results = results
                    .into_iter()
                    .map(|value| parse_exec_result(value.get()))
                    .collect::<Result<_, _>>()?;
            }
        } else if key.eq_ignore_ascii_case("time") && !raw_is_null(&value) {
            response.time = decode(value.get(), "invalid execution time")?;
        } else if key.eq_ignore_ascii_case("version") {
            response.version = decode_nullable(value.get(), "invalid execution version")?;
        }
    }
    Ok(response)
}

fn parse_exec_result(data: &str) -> Result<ExecResult, ClientError> {
    let ObjectPairs(pairs) = decode(data, "invalid execution result")?;
    let mut result = ExecResult::default();
    for (key, value) in pairs {
        if key.eq_ignore_ascii_case("rows_affected") && !raw_is_null(&value) {
            result.rows_affected = decode(value.get(), "invalid rows_affected")?;
        } else if key.eq_ignore_ascii_case("time") && !raw_is_null(&value) {
            result.time = decode(value.get(), "invalid result time")?;
        } else if key.eq_ignore_ascii_case("error") {
            result.error = decode_nullable(value.get(), "invalid result error")?;
        }
    }
    Ok(result)
}

fn decode<T: DeserializeOwned>(data: &str, context: &str) -> Result<T, ClientError> {
    serde_json::from_str(data)
        .map_err(|error| ClientError::with_source(ClientErrorKind::Json, context, error))
}

fn decode_nullable<T: DeserializeOwned>(
    data: &str,
    context: &str,
) -> Result<Option<T>, ClientError> {
    decode(data, context)
}

fn raw_is_null(value: &RawValue) -> bool {
    value.get().trim() == "null"
}

async fn rows_from_response(
    response: HttpResponse,
    close_on_end: bool,
) -> Result<Rows, ClientError> {
    if response.status != StatusCode::OK {
        return Err(status_with_body(response).await);
    }
    Rows::new(response.body, close_on_end)
        .await
        .map_err(|error| {
            ClientError::with_source(ClientErrorKind::Protocol, "parse query response", error)
        })
}

async fn status_with_body(response: HttpResponse) -> ClientError {
    let status = response.status;
    match response.body.read_to_end().await {
        Ok(body) => unexpected_status(status, &body),
        Err(error) => ClientError::with_source(ClientErrorKind::Http, "read response body", error),
    }
}

fn unexpected_status(status: StatusCode, body: &[u8]) -> ClientError {
    ClientError::new(
        ClientErrorKind::Http,
        format!(
            "unexpected status code {}: {}",
            status.as_u16(),
            String::from_utf8_lossy(body)
        ),
    )
}

fn encode_path_segment(value: &str) -> String {
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            encoded.push(char::from(byte));
        } else {
            use std::fmt::Write as _;
            write!(encoded, "%{byte:02X}").expect("writing to String cannot fail");
        }
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::GoBytes;
    use futures_util::future::{Either, select};
    use hyper::header;
    use std::{future::Future, time::Duration};
    use tokio::{net::TcpListener, sync::oneshot};

    fn run<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    async fn send_response(
        mut respond: h2::server::SendResponse<Bytes>,
        status: StatusCode,
        headers: &[(&str, &str)],
        body: &'static [u8],
    ) {
        let mut response = hyper::Response::builder().status(status);
        for (name, value) in headers {
            response = response.header(*name, *value);
        }
        let mut stream = respond
            .send_response(response.body(()).unwrap(), body.is_empty())
            .unwrap();
        if !body.is_empty() {
            stream.send_data(Bytes::from_static(body), true).unwrap();
        }
    }

    async fn request_body(mut body: h2::RecvStream) -> Vec<u8> {
        let mut data = Vec::new();
        while let Some(chunk) = body.data().await {
            data.extend_from_slice(&chunk.unwrap());
        }
        data
    }

    #[test]
    fn parses_rows_and_changes_with_exact_tuple_errors() {
        let row = parse_row_event(r#"[42,["hello","AP8="]]"#).unwrap();
        assert_eq!(row.id, 42);
        assert_eq!(row.get::<String>(0).unwrap(), "hello");
        assert_eq!(row.get::<GoBytes>(1).unwrap().0, Some(vec![0, 255]));
        assert!(
            parse_row_event("[1]")
                .unwrap_err()
                .to_string()
                .contains("2 elements")
        );

        let change = parse_change_event(r#"["insert",7,["x"],9]"#).unwrap();
        assert_eq!(change.change_type, ChangeType::Insert);
        assert_eq!(change.row_id, 7);
        assert_eq!(change.change_id, 9);
        assert_eq!(change.to_json().unwrap(), br#"["insert",7,["x"],9]"#);
        assert!(
            parse_change_event(r#"["insert",7,[]]"#)
                .unwrap_err()
                .to_string()
                .contains("4 elements")
        );

        let null_row = parse_row_event("[1,[null]]").unwrap();
        let mut existing = String::from("unchanged");
        null_row.decode_into(0, &mut existing).unwrap();
        assert_eq!(existing, "unchanged");
        assert!(parse_row_event("[1,null]").unwrap().values.is_none());
        let nil_change = parse_change_event(r#"["delete",1,null,2]"#).unwrap();
        assert_eq!(nil_change.to_json().unwrap(), br#"["delete",1,null,2]"#);
    }

    #[test]
    fn change_id_rollover_matches_go_uint64_arithmetic() {
        assert_eq!(expected_change_id(0), None);
        assert_eq!(expected_change_id(1), Some(2));
        assert_eq!(expected_change_id(u64::MAX), Some(0));
    }

    #[test]
    fn query_objects_fold_names_apply_duplicates_and_merge_nested_values() {
        let event = parse_query_event(
            r#"{"EOQ":{"time":1,"change_id":4},"eoq":{"time":2},"ERROR":"old","error":null}"#,
        )
        .unwrap();
        assert_eq!(
            event.end,
            Some(EndOfQuery {
                time: 2.0,
                change_id: Some(4)
            })
        );
        assert_eq!(event.error, None);

        let response = parse_exec_response(
            br#"{"RESULTS":[{"ROWS_AFFECTED":1,"error":null}],"time":1,"TIME":2,"version":3}"#,
        )
        .unwrap();
        assert_eq!(response.results[0].rows_affected, 1);
        assert_eq!(response.time, 2.0);
        assert_eq!(response.version, Some(3));
    }

    #[test]
    fn path_segment_escaping_matches_url_path_expectations() {
        assert_eq!(encode_path_segment("id/a b"), "id%2Fa%20b");
    }

    #[test]
    fn statement_preserves_nil_and_non_nil_empty_params() {
        assert_eq!(
            go_json_bytes(&Statement::new("SELECT 1", None)).unwrap(),
            br#"{"query":"SELECT 1","params":null}"#
        );
        assert_eq!(
            go_json_bytes(&Statement::new("SELECT 1", Some(Vec::new()))).unwrap(),
            br#"{"query":"SELECT 1","params":[]}"#
        );
        let no_statements: Option<&[Statement]> = None;
        assert_eq!(go_json_bytes(&no_statements).unwrap(), b"null");
        assert_eq!(go_json_bytes(&Some(&[] as &[Statement])).unwrap(), b"[]");
    }

    #[test]
    fn top_level_null_has_go_zero_struct_semantics() {
        let event = parse_query_event("null").unwrap();
        assert!(event.columns.is_none());
        assert_eq!(
            parse_exec_response(b"null").unwrap(),
            ExecResponse::default()
        );
    }

    #[test]
    fn h2c_query_sets_wire_headers_and_incrementally_decodes_gzip() {
        run(async {
            const GZIP: &[u8] = &[
                0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x02, 0x03, 0xab, 0x56, 0x4a, 0xce,
                0xcf, 0x29, 0xcd, 0xcd, 0x2b, 0x56, 0xb2, 0x8a, 0x56, 0xca, 0x4c, 0x51, 0xd2, 0x51,
                0x4a, 0xca, 0xc9, 0x4f, 0x52, 0x8a, 0xad, 0xe5, 0xaa, 0x56, 0x2a, 0xca, 0x2f, 0x07,
                0x8a, 0x1a, 0xea, 0x44, 0x2b, 0x65, 0xa4, 0xe6, 0xe4, 0xe4, 0x03, 0xe5, 0x1c, 0x03,
                0x2c, 0x6c, 0x95, 0x62, 0xc1, 0x92, 0xa9, 0xf9, 0x85, 0x4a, 0x56, 0xd5, 0x4a, 0x25,
                0x99, 0xb9, 0xa9, 0x4a, 0x56, 0x06, 0x7a, 0x46, 0xa6, 0xb5, 0xb5, 0x5c, 0x00, 0x77,
                0xd9, 0x22, 0xf6, 0x4d, 0x00, 0x00, 0x00,
            ];
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (socket, _) = listener.accept().await.unwrap();
                let mut connection = h2::server::handshake(socket).await.unwrap();
                let (request, respond) = connection.accept().await.unwrap().unwrap();
                assert_eq!(request.method(), Method::POST);
                assert_eq!(request.uri().path(), "/v1/queries");
                assert_eq!(
                    request.headers()[header::AUTHORIZATION],
                    "Bearer test-token"
                );
                assert_eq!(request.headers()[header::ACCEPT_ENCODING], "gzip");
                assert_eq!(request.headers()[header::USER_AGENT], "Go-http-client/2.0");
                let body = request_body(request.into_body()).await;
                assert_eq!(body, br#"{"query":"SELECT \u003c ?","params":["AP8="]}"#);
                send_response(
                    respond,
                    StatusCode::OK,
                    &[("content-encoding", "gzip")],
                    GZIP,
                )
                .await;
                connection.graceful_shutdown();
                while connection.accept().await.is_some() {}
            });

            let client = ApiClient::test_client(
                format!("http://{address}").parse().unwrap(),
                Some(RandomizedBackoff::deterministic(
                    Duration::from_millis(1),
                    Duration::from_millis(2),
                    Some(Duration::from_secs(1)),
                )),
            );
            let mut rows = client
                .query("SELECT < ?", Some(vec![SqlValue::Bytes(vec![0, 255])]))
                .await
                .unwrap();
            assert_eq!(rows.columns(), ["id", "blob"]);
            let row = rows.next().await.unwrap().unwrap();
            assert_eq!(row.id, 1);
            assert_eq!(row.get::<String>(0).unwrap(), "hello");
            assert_eq!(row.get::<GoBytes>(1).unwrap().0, Some(vec![0, 255]));
            assert!(rows.next().await.unwrap().is_none());
            assert_eq!(rows.time().unwrap(), 0.25);
            server.await.unwrap();
        });
    }

    #[test]
    fn subscription_from_zero_drains_replayed_snapshot() {
        run(async {
            const SNAPSHOT: &[u8] = b"{\"columns\":[\"id\",\"info\"]}\n{\"row\":[1,[\"a\",\"b\"]]}\n{\"eoq\":{\"time\":0.1,\"change_id\":0}}\n";
            const REPLAY: &[u8] = b"{\"columns\":[\"id\",\"info\"]}\n{\"row\":[1,[\"a\",\"b\"]]}\n{\"eoq\":{\"time\":0.1,\"change_id\":0}}\n{\"change\":[\"insert\",2,[\"c\",\"d\"],1]}\n";
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (socket, _) = listener.accept().await.unwrap();
                let mut connection = h2::server::handshake(socket).await.unwrap();
                let (request, respond) = connection.accept().await.unwrap().unwrap();
                assert_eq!(request.method(), Method::POST);
                send_response(
                    respond,
                    StatusCode::OK,
                    &[("corro-query-id", "test-sub")],
                    SNAPSHOT,
                )
                .await;

                let (request, respond) = connection.accept().await.unwrap().unwrap();
                assert_eq!(request.method(), Method::GET);
                assert_eq!(
                    request.uri().path_and_query().unwrap().as_str(),
                    "/v1/subscriptions/test-sub?from=0"
                );
                send_response(respond, StatusCode::OK, &[], REPLAY).await;
                connection.graceful_shutdown();
                while connection.accept().await.is_some() {}
            });
            let client = ApiClient::test_client(
                format!("http://{address}").parse().unwrap(),
                Some(RandomizedBackoff::deterministic(
                    Duration::from_millis(1),
                    Duration::from_millis(2),
                    Some(Duration::from_secs(1)),
                )),
            );
            let mut subscription = client
                .subscribe("SELECT id, info FROM machines", None, false)
                .await
                .unwrap();
            let rows = subscription.rows_mut().unwrap();
            assert!(rows.next().await.unwrap().is_some());
            assert!(rows.next().await.unwrap().is_none());
            let mut changes = subscription.into_changes().unwrap();
            let change = changes.next().await.unwrap().unwrap();
            assert_eq!(change.change_type, ChangeType::Insert);
            assert_eq!(change.row_id, 2);
            assert_eq!(change.change_id, 1);
            server.await.unwrap();
        });
    }

    #[test]
    fn gone_subscription_fails_without_retrying() {
        run(async {
            const SNAPSHOT: &[u8] = b"{\"columns\":[\"id\"]}\n{\"row\":[1,[\"a\"]]}\n{\"eoq\":{\"time\":0.1,\"change_id\":5}}\n";
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (socket, _) = listener.accept().await.unwrap();
                let mut connection = h2::server::handshake(socket).await.unwrap();
                let (request, respond) = connection.accept().await.unwrap().unwrap();
                assert_eq!(request.method(), Method::POST);
                send_response(
                    respond,
                    StatusCode::OK,
                    &[("corro-query-id", "gone")],
                    SNAPSHOT,
                )
                .await;
                let (request, respond) = connection.accept().await.unwrap().unwrap();
                assert_eq!(request.uri().query(), Some("from=5"));
                send_response(respond, StatusCode::NOT_FOUND, &[], b"").await;
                assert!(
                    tokio::time::timeout(Duration::from_millis(50), connection.accept())
                        .await
                        .is_err()
                );
            });
            let client = ApiClient::test_client(
                format!("http://{address}").parse().unwrap(),
                Some(RandomizedBackoff::deterministic(
                    Duration::from_millis(1),
                    Duration::from_millis(2),
                    Some(Duration::from_secs(1)),
                )),
            );
            let mut subscription = client.subscribe("SELECT id", None, false).await.unwrap();
            while subscription
                .rows_mut()
                .unwrap()
                .next()
                .await
                .unwrap()
                .is_some()
            {}
            let mut changes = subscription.into_changes().unwrap();
            let error = changes.next().await.unwrap().unwrap_err();
            assert!(error.is_subscription_not_found());
            assert!(changes.next().await.is_none());
            server.await.unwrap();
        });
    }

    #[test]
    fn cancelled_resubscribe_is_resumed_before_polling_changes() {
        run(async {
            const SNAPSHOT: &[u8] =
                b"{\"columns\":[\"id\"]}\n{\"eoq\":{\"time\":0.1,\"change_id\":5}}\n";
            const CHANGE: &[u8] = b"{\"change\":[\"update\",1,[\"new\"],6]}\n";
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let (first_accepted_tx, first_accepted_rx) = oneshot::channel();
            let server = tokio::spawn(async move {
                let (socket, _) = listener.accept().await.unwrap();
                let mut connection = h2::server::handshake(socket).await.unwrap();
                let (_, respond) = connection.accept().await.unwrap().unwrap();
                send_response(
                    respond,
                    StatusCode::OK,
                    &[("corro-query-id", "cancel-safe")],
                    SNAPSHOT,
                )
                .await;

                let (first, held_response) = connection.accept().await.unwrap().unwrap();
                assert_eq!(first.uri().query(), Some("from=5"));
                first_accepted_tx.send(()).unwrap();

                let (second, respond) = connection.accept().await.unwrap().unwrap();
                assert_eq!(second.uri().query(), Some("from=5"));
                send_response(respond, StatusCode::OK, &[], CHANGE).await;
                drop(held_response);
                connection.graceful_shutdown();
                while connection.accept().await.is_some() {}
            });

            let client = ApiClient::test_client(
                format!("http://{address}").parse().unwrap(),
                Some(RandomizedBackoff::deterministic(
                    Duration::from_millis(1),
                    Duration::from_millis(2),
                    Some(Duration::from_secs(1)),
                )),
            );
            let mut subscription = client.subscribe("SELECT id", None, false).await.unwrap();
            assert!(
                subscription
                    .rows_mut()
                    .unwrap()
                    .next()
                    .await
                    .unwrap()
                    .is_none()
            );
            let mut changes = subscription.into_changes().unwrap();

            match select(Box::pin(changes.next()), Box::pin(first_accepted_rx)).await {
                Either::Left((result, _)) => {
                    panic!("resubscribe unexpectedly completed: {result:?}")
                }
                Either::Right((accepted, pending_next)) => {
                    accepted.unwrap();
                    drop(pending_next);
                }
            }

            let change = tokio::time::timeout(Duration::from_secs(1), changes.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert_eq!(change.change_id, 6);
            server.await.unwrap();
        });
    }

    #[test]
    fn exec_error_response_depends_on_http_status() {
        run(async {
            const EXEC_ERROR: &[u8] = br#"{"results":[{"rows_affected":0,"time":0.1,"error":"database failed"}],"time":0.1,"version":7}"#;
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (socket, _) = listener.accept().await.unwrap();
                let mut connection = h2::server::handshake(socket).await.unwrap();
                for status in [StatusCode::OK, StatusCode::INTERNAL_SERVER_ERROR] {
                    let (_, respond) = connection.accept().await.unwrap().unwrap();
                    send_response(respond, status, &[], EXEC_ERROR).await;
                }
                connection.graceful_shutdown();
                while connection.accept().await.is_some() {}
            });

            let client = ApiClient::test_client(format!("http://{address}").parse().unwrap(), None);
            let ok_error = client.exec("bad statement", None).await.unwrap_err();
            assert_eq!(ok_error.error.to_string(), "database failed");
            assert_eq!(ok_error.response.unwrap().version, Some(7));

            let internal_error = client.exec("bad statement", None).await.unwrap_err();
            assert_eq!(internal_error.error.to_string(), "database failed");
            assert!(internal_error.response.is_none());
            server.await.unwrap();
        });
    }

    #[test]
    fn post_redirect_becomes_bodyless_get_and_reapplies_bearer() {
        run(async {
            const EXEC_RESPONSE: &[u8] =
                br#"{"results":[{"rows_affected":1,"time":0.1,"error":null}],"time":0.1,"version":1}"#;
            let target_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let target_address = target_listener.local_addr().unwrap();
            let source_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let source_address = source_listener.local_addr().unwrap();

            let source = tokio::spawn(async move {
                let (socket, _) = source_listener.accept().await.unwrap();
                let mut connection = h2::server::handshake(socket).await.unwrap();
                let (request, mut respond) = connection.accept().await.unwrap().unwrap();
                assert_eq!(request.method(), Method::POST);
                assert!(!request_body(request.into_body()).await.is_empty());
                let location = format!("http://{target_address}/redirected");
                let response = hyper::Response::builder()
                    .status(StatusCode::FOUND)
                    .header(header::LOCATION, location)
                    .body(())
                    .unwrap();
                respond.send_response(response, true).unwrap();
                connection.graceful_shutdown();
                while connection.accept().await.is_some() {}
            });
            let target = tokio::spawn(async move {
                let (socket, _) = target_listener.accept().await.unwrap();
                let mut connection = h2::server::handshake(socket).await.unwrap();
                let (request, respond) = connection.accept().await.unwrap().unwrap();
                assert_eq!(request.method(), Method::GET);
                assert_eq!(request.uri().path(), "/redirected");
                assert_eq!(
                    request.headers()[header::AUTHORIZATION],
                    "Bearer test-token"
                );
                assert!(request_body(request.into_body()).await.is_empty());
                send_response(respond, StatusCode::OK, &[], EXEC_RESPONSE).await;
                connection.graceful_shutdown();
                while connection.accept().await.is_some() {}
            });

            let client =
                ApiClient::test_client(format!("http://{source_address}").parse().unwrap(), None);
            let result = client.exec("UPDATE x", None).await.unwrap();
            assert_eq!(result.rows_affected, 1);
            source.await.unwrap();
            target.await.unwrap();
        });
    }

    #[test]
    fn empty_location_is_not_followed() {
        run(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (socket, _) = listener.accept().await.unwrap();
                let mut connection = h2::server::handshake(socket).await.unwrap();
                let (request, mut respond) = connection.accept().await.unwrap().unwrap();
                assert_eq!(request.method(), Method::POST);
                let response = hyper::Response::builder()
                    .status(StatusCode::FOUND)
                    .header(header::LOCATION, "")
                    .body(())
                    .unwrap();
                respond.send_response(response, true).unwrap();
                if let Ok(Some(Ok(_))) =
                    tokio::time::timeout(Duration::from_millis(50), connection.accept()).await
                {
                    panic!("empty Location triggered another request");
                }
            });

            let client = ApiClient::test_client(format!("http://{address}").parse().unwrap(), None);
            let error = client.exec("UPDATE x", None).await.unwrap_err();
            assert!(error.to_string().contains("unexpected status code 302"));
            server.await.unwrap();
        });
    }

    #[test]
    fn redirect_limit_stops_before_request_eleven() {
        run(async {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (socket, _) = listener.accept().await.unwrap();
                let mut connection = h2::server::handshake(socket).await.unwrap();
                for request_number in 1..=10 {
                    let (request, mut respond) = connection.accept().await.unwrap().unwrap();
                    if request_number == 1 {
                        assert_eq!(request.method(), Method::POST);
                    } else {
                        assert_eq!(request.method(), Method::GET);
                    }
                    let response = hyper::Response::builder()
                        .status(StatusCode::FOUND)
                        .header(header::LOCATION, "/again")
                        .body(())
                        .unwrap();
                    respond.send_response(response, true).unwrap();
                }
                if let Ok(Some(Ok(_))) =
                    tokio::time::timeout(Duration::from_millis(50), connection.accept()).await
                {
                    panic!("redirect request eleven was sent");
                }
            });

            let client = ApiClient::test_client(format!("http://{address}").parse().unwrap(), None);
            let error = client.exec("UPDATE x", None).await.unwrap_err();
            assert!(error.to_string().contains("stopped after 10 redirects"));
            server.await.unwrap();
        });
    }
}
