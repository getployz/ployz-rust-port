use std::error::Error as StdError;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use async_compression::tokio::bufread::GzipDecoder;
use futures_util::TryStreamExt;
use reqwest::{Client, StatusCode};
use serde::{
    Deserialize, Deserializer,
    de::{IgnoredAny, MapAccess, Visitor},
};
use serde_json::value::RawValue;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::net::UnixStream;
use tokio::time::{Instant, timeout, timeout_at};
use tokio_util::io::StreamReader;

const AVAILABILITY_TIMEOUT: Duration = Duration::from_secs(1);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

type BoxError = Box<dyn StdError + Send + Sync>;

#[derive(Debug)]
pub enum CaddyAdminClientError {
    Construction {
        message: &'static str,
        source: Option<BoxError>,
    },
    Request {
        operation: &'static str,
        source: BoxError,
    },
    ReadResponse {
        source: BoxError,
    },
    ParseAdapt {
        source: serde_json::Error,
    },
    AdaptRejected {
        status: u16,
        body: Vec<u8>,
        message: Option<String>,
    },
    AdaptBeforeLoad {
        source: Box<CaddyAdminClientError>,
    },
    LoadRejected {
        status: u16,
        body: Vec<u8>,
        message: Option<String>,
        source: Option<BoxError>,
    },
}

impl CaddyAdminClientError {
    #[must_use]
    pub fn status(&self) -> Option<u16> {
        match self {
            Self::AdaptRejected { status, .. } | Self::LoadRejected { status, .. } => Some(*status),
            _ => None,
        }
    }

    #[must_use]
    pub fn body(&self) -> Option<&[u8]> {
        match self {
            Self::AdaptRejected { body, .. } | Self::LoadRejected { body, .. } => Some(body),
            _ => None,
        }
    }
}

impl fmt::Display for CaddyAdminClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Construction { message, source } => {
                formatter.write_str(message)?;
                if let Some(source) = source {
                    write!(formatter, ": {source}")?;
                }
                Ok(())
            }
            Self::Request { operation, source } => {
                write!(formatter, "send {operation} request: {source}")
            }
            Self::ReadResponse { source } => write!(formatter, "read response body: {source}"),
            Self::ParseAdapt { source } => write!(formatter, "parse adapt response: {source}"),
            Self::AdaptRejected { body, message, .. } => match message {
                Some(message) => formatter.write_str(message),
                None => write!(formatter, "{}", String::from_utf8_lossy(body)),
            },
            Self::AdaptBeforeLoad { source } => {
                write!(formatter, "adapt Caddyfile to JSON config: {source}")
            }
            Self::LoadRejected {
                status,
                body,
                message,
                ..
            } => {
                if let Some(message) = message {
                    write!(formatter, "caddy responded with error: {message}")
                } else {
                    write!(
                        formatter,
                        "caddy responded with error: HTTP {status}: {}",
                        String::from_utf8_lossy(body)
                    )
                }
            }
        }
    }
}

impl StdError for CaddyAdminClientError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Construction {
                source: Some(source),
                ..
            }
            | Self::Request { source, .. }
            | Self::ReadResponse { source } => Some(source.as_ref()),
            Self::LoadRejected {
                source: Some(source),
                ..
            } => Some(source.as_ref()),
            Self::ParseAdapt { source } => Some(source),
            Self::AdaptBeforeLoad { source } => Some(source.as_ref()),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct CaddyAdminClient {
    socket_path: PathBuf,
    client: Client,
}

impl CaddyAdminClient {
    pub fn new(socket_path: impl Into<PathBuf>) -> Result<Self, CaddyAdminClientError> {
        ensure_ring_provider()?;
        let socket_path = socket_path.into();
        let client = Client::builder()
            .unix_socket(socket_path.as_path())
            .http1_only()
            .http1_title_case_headers()
            .no_proxy()
            .no_gzip()
            .no_brotli()
            .no_deflate()
            .no_zstd()
            .retry(reqwest::retry::never())
            .timeout(REQUEST_TIMEOUT)
            .user_agent("Go-http-client/1.1")
            .build()
            .map_err(|source| CaddyAdminClientError::Construction {
                message: "create Caddy admin client",
                source: Some(Box::new(source)),
            })?;
        Ok(Self {
            socket_path,
            client,
        })
    }

    #[must_use]
    pub fn socket_path(&self) -> &Path {
        &self.socket_path
    }

    pub async fn is_available(&self) -> bool {
        matches!(
            timeout(AVAILABILITY_TIMEOUT, UnixStream::connect(&self.socket_path)).await,
            Ok(Ok(_))
        )
    }

    pub async fn adapt(&self, caddyfile: &str) -> Result<Vec<u8>, CaddyAdminClientError> {
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        let response = self
            .send(
                "adapt",
                deadline,
                self.client
                    .post("http://localhost/adapt")
                    .header("Accept-Encoding", "gzip")
                    .header("Content-Type", "text/caddyfile")
                    .body(caddyfile.as_bytes().to_vec()),
            )
            .await?;
        let status = response.status();
        let body = read_body(response, deadline).await.map_err(|failure| {
            CaddyAdminClientError::ReadResponse {
                source: failure.source,
            }
        })?;

        if status == StatusCode::OK {
            let envelope: AdaptEnvelope<'_> = serde_json::from_slice(&body)
                .map_err(|source| CaddyAdminClientError::ParseAdapt { source })?;
            return Ok(match envelope.result {
                RawField::Missing => Vec::new(),
                RawField::Present(raw) => raw.get().as_bytes().to_vec(),
            });
        }

        let message = (status == StatusCode::BAD_REQUEST)
            .then(|| serde_json::from_slice::<ApiErrorEnvelope>(&body).ok())
            .flatten()
            .map(|envelope| envelope.error);
        Err(CaddyAdminClientError::AdaptRejected {
            status: status.as_u16(),
            body,
            message,
        })
    }

    pub async fn validate(&self, caddyfile: &str) -> Result<(), CaddyAdminClientError> {
        self.adapt(caddyfile).await.map(|_| ())
    }

    pub async fn load(&self, caddyfile: &str) -> Result<(), CaddyAdminClientError> {
        let json = self.adapt(caddyfile).await.map_err(|source| {
            CaddyAdminClientError::AdaptBeforeLoad {
                source: Box::new(source),
            }
        })?;
        let deadline = Instant::now() + REQUEST_TIMEOUT;
        let response = self
            .send(
                "load",
                deadline,
                self.client
                    .post("http://localhost/load")
                    .header("Accept-Encoding", "gzip")
                    .header("Content-Type", "application/json")
                    .body(json),
            )
            .await?;
        let status = response.status();
        if status == StatusCode::OK {
            return Ok(());
        }

        let (body, source) = match read_body(response, deadline).await {
            Ok(body) => (body, None),
            Err(failure) => (failure.partial, Some(failure.source)),
        };
        let message = (status == StatusCode::BAD_REQUEST)
            .then(|| serde_json::from_slice::<ApiErrorEnvelope>(&body).ok())
            .flatten()
            .map(|envelope| envelope.error);
        Err(CaddyAdminClientError::LoadRejected {
            status: status.as_u16(),
            body,
            message,
            source,
        })
    }

    async fn send(
        &self,
        operation: &'static str,
        deadline: Instant,
        request: reqwest::RequestBuilder,
    ) -> Result<reqwest::Response, CaddyAdminClientError> {
        match timeout_at(deadline, request.send()).await {
            Ok(Ok(response)) => Ok(response),
            Ok(Err(source)) => Err(CaddyAdminClientError::Request {
                operation,
                source: Box::new(source),
            }),
            Err(source) => Err(CaddyAdminClientError::Request {
                operation,
                source: Box::new(source),
            }),
        }
    }
}

fn ring_compatible(provider: &rustls::crypto::CryptoProvider) -> bool {
    let ring = rustls::crypto::ring::default_provider();
    std::ptr::eq(provider.secure_random, ring.secure_random)
        && std::ptr::eq(provider.key_provider, ring.key_provider)
}

fn ensure_ring_provider() -> Result<(), CaddyAdminClientError> {
    if let Some(provider) = rustls::crypto::CryptoProvider::get_default() {
        return ring_compatible(provider).then_some(()).ok_or(
            CaddyAdminClientError::Construction {
                message: "incompatible Rustls crypto provider already installed",
                source: None,
            },
        );
    }
    if rustls::crypto::ring::default_provider()
        .install_default()
        .is_ok()
    {
        return Ok(());
    }
    rustls::crypto::CryptoProvider::get_default()
        .is_some_and(|provider| ring_compatible(provider))
        .then_some(())
        .ok_or(CaddyAdminClientError::Construction {
            message: "Rustls provider installation race installed an incompatible provider",
            source: None,
        })
}

struct BodyFailure {
    partial: Vec<u8>,
    source: BoxError,
}

async fn read_body(response: reqwest::Response, deadline: Instant) -> Result<Vec<u8>, BodyFailure> {
    let gzip = response
        .headers()
        .get("Content-Encoding")
        .is_some_and(|value| value.as_bytes().eq_ignore_ascii_case(b"gzip"));
    let stream = response.bytes_stream().map_err(io::Error::other);
    let reader = StreamReader::new(stream);
    if gzip {
        let mut decoder = GzipDecoder::new(reader);
        decoder.multiple_members(true);
        collect_body(decoder, deadline).await
    } else {
        collect_body(reader, deadline).await
    }
}

async fn collect_body<R>(mut reader: R, deadline: Instant) -> Result<Vec<u8>, BodyFailure>
where
    R: AsyncRead + Unpin,
{
    let mut body = Vec::new();
    let mut buffer = [0_u8; 8192];
    loop {
        tokio::task::yield_now().await;
        let result = match timeout_at(deadline, reader.read(&mut buffer)).await {
            Ok(result) => result.map_err(|source| Box::new(source) as BoxError),
            Err(source) => Err(Box::new(source) as BoxError),
        };
        match result {
            Ok(0) => return Ok(body),
            Ok(count) => body.extend_from_slice(&buffer[..count]),
            Err(source) => {
                return Err(BodyFailure {
                    partial: body,
                    source,
                });
            }
        }
    }
}

#[derive(Default)]
enum RawField<'a> {
    #[default]
    Missing,
    Present(&'a RawValue),
}

struct AdaptEnvelope<'a> {
    result: RawField<'a>,
}

impl<'de> Deserialize<'de> for AdaptEnvelope<'de> {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct AdaptVisitor;
        impl<'de> Visitor<'de> for AdaptVisitor {
            type Value = AdaptEnvelope<'de>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an adapt response object")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut result = RawField::Missing;
                while let Some(key) = map.next_key::<String>()? {
                    if go_field_eq(&key, "result") {
                        result = RawField::Present(map.next_value::<&RawValue>()?);
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
                Ok(AdaptEnvelope { result })
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(AdaptEnvelope {
                    result: RawField::Missing,
                })
            }
        }
        deserializer.deserialize_any(AdaptVisitor)
    }
}

struct ApiErrorEnvelope {
    error: String,
}

impl<'de> Deserialize<'de> for ApiErrorEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct ErrorVisitor;
        impl<'de> Visitor<'de> for ErrorVisitor {
            type Value = ApiErrorEnvelope;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a Caddy API error object")
            }

            fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
            where
                M: MapAccess<'de>,
            {
                let mut error = String::new();
                while let Some(key) = map.next_key::<String>()? {
                    if go_field_eq(&key, "error") {
                        if let Some(value) = map.next_value::<Option<String>>()? {
                            error = value;
                        }
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
                Ok(ApiErrorEnvelope { error })
            }

            fn visit_unit<E>(self) -> Result<Self::Value, E> {
                Ok(ApiErrorEnvelope {
                    error: String::new(),
                })
            }
        }
        deserializer.deserialize_any(ErrorVisitor)
    }
}

fn go_field_eq(input: &str, field: &str) -> bool {
    let mut normalized = String::with_capacity(input.len());
    for character in input.chars() {
        match character {
            '\u{212a}' => normalized.push('k'),
            '\u{017f}' => normalized.push('s'),
            character if character.is_ascii() => {
                normalized.push(character.to_ascii_lowercase());
            }
            character => normalized.push(character),
        }
    }
    normalized == field
}

#[cfg(test)]
mod tests {
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Instant as StdInstant;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixListener;
    use tokio::sync::{mpsc, oneshot};

    use super::*;

    static NEXT_SOCKET: AtomicU64 = AtomicU64::new(0);
    const GZIP_ADAPT_BODY: &[u8] = &[
        0x1f, 0x8b, 0x08, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x03, 0xab, 0x56, 0x2a, 0x4a, 0x2d,
        0x2e, 0xcd, 0x29, 0x51, 0xb2, 0xaa, 0x56, 0x4a, 0xaf, 0xca, 0x2c, 0x50, 0xb2, 0x2a, 0x29,
        0x2a, 0x4d, 0xad, 0xad, 0x05, 0x00, 0x24, 0xe3, 0x70, 0x52, 0x18, 0x00, 0x00, 0x00,
    ];

    fn socket_path(label: &str) -> PathBuf {
        let id = NEXT_SOCKET.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "ployz-caddy-admin-{}-{id}-{label}.sock",
            std::process::id()
        ))
    }

    async fn read_request(stream: &mut UnixStream) -> Vec<u8> {
        let mut request = Vec::new();
        loop {
            let mut buffer = [0_u8; 4096];
            let count = stream.read(&mut buffer).await.unwrap();
            assert!(count > 0, "EOF before complete request");
            request.extend_from_slice(&buffer[..count]);
            if let Some(head_end) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                let head_end = head_end + 4;
                let head = String::from_utf8_lossy(&request[..head_end]);
                let content_length = head
                    .lines()
                    .find_map(|line| {
                        line.strip_prefix("Content-Length: ")
                            .or_else(|| line.strip_prefix("content-length: "))
                    })
                    .and_then(|value| value.trim().parse::<usize>().ok())
                    .unwrap_or(0);
                if request.len() >= head_end + content_length {
                    return request;
                }
            }
        }
    }

    fn response(status: &str, body: &[u8]) -> Vec<u8> {
        let mut response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        response.extend_from_slice(body);
        response
    }

    fn gzip_response(status: &str, body: &[u8]) -> Vec<u8> {
        let mut response = format!(
            "HTTP/1.1 {status}\r\nContent-Length: {}\r\nContent-Encoding: gzip\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .into_bytes();
        response.extend_from_slice(body);
        response
    }

    fn scripted_server(
        path: &Path,
        responses: Vec<Vec<u8>>,
    ) -> (
        tokio::task::JoinHandle<()>,
        mpsc::UnboundedReceiver<Vec<u8>>,
    ) {
        let listener = UnixListener::bind(path).unwrap();
        let (sender, receiver) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            for response in responses {
                let (mut stream, _) = listener.accept().await.unwrap();
                let _ = sender.send(read_request(&mut stream).await);
                stream.write_all(&response).await.unwrap();
                stream.shutdown().await.unwrap();
            }
        });
        (task, receiver)
    }

    fn assert_wire(request: &[u8], target: &str, content_type: &str, body: &[u8]) {
        let head_end = request
            .windows(4)
            .position(|part| part == b"\r\n\r\n")
            .unwrap()
            + 4;
        let head = String::from_utf8_lossy(&request[..head_end]);
        assert!(
            head.starts_with(&format!("POST {target} HTTP/1.1\r\n")),
            "{head}"
        );
        assert!(head.contains("Host: localhost\r\n"), "{head}");
        assert!(
            head.contains(&format!("Content-Type: {content_type}\r\n")),
            "{head}"
        );
        assert!(
            head.contains("User-Agent: Go-http-client/1.1\r\n"),
            "{head}"
        );
        assert!(head.contains("Accept-Encoding: gzip\r\n"), "{head}");
        assert_eq!(&request[head_end..], body);
    }

    #[test]
    fn adapt_envelope_matches_go_field_rules_and_raw_spelling() {
        let cases = [
            (r#"null"#, ""),
            (r#"{}"#, ""),
            (r#"{"result":null}"#, "null"),
            (r#"{"Result": { "x" : 1 }}"#, r#"{ "x" : 1 }"#),
            (r#"{"result":1,"reſult":2}"#, "2"),
        ];
        for (body, expected) in cases {
            let envelope: AdaptEnvelope<'_> = serde_json::from_str(body).unwrap();
            let actual = match envelope.result {
                RawField::Missing => "",
                RawField::Present(value) => value.get(),
            };
            assert_eq!(actual, expected);
        }
    }

    #[test]
    fn api_error_envelope_preserves_duplicate_null_rule() {
        let envelope: ApiErrorEnvelope =
            serde_json::from_str(r#"{"error":"first","ERROR":null}"#).unwrap();
        assert_eq!(envelope.error, "first");
        assert!(serde_json::from_str::<ApiErrorEnvelope>(r#"{"error":1}"#).is_err());
        assert_eq!(
            serde_json::from_str::<ApiErrorEnvelope>("null")
                .unwrap()
                .error,
            ""
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn availability_handles_missing_live_stale_and_denied_sockets() {
        let path = socket_path("availability");
        let client = CaddyAdminClient::new(&path).unwrap();
        assert!(!client.is_available().await);

        let listener = UnixListener::bind(&path).unwrap();
        assert!(client.is_available().await);
        let _ = listener.accept().await.unwrap();
        drop(listener);
        assert!(!client.is_available().await);
        std::fs::remove_file(&path).unwrap();

        let denied_dir = socket_path("permission-dir");
        std::fs::create_dir(&denied_dir).unwrap();
        let denied_path = denied_dir.join("admin.sock");
        let listener = UnixListener::bind(&denied_path).unwrap();
        std::fs::set_permissions(&denied_dir, std::fs::Permissions::from_mode(0o000)).unwrap();
        assert!(
            !CaddyAdminClient::new(&denied_path)
                .unwrap()
                .is_available()
                .await
        );
        std::fs::set_permissions(&denied_dir, std::fs::Permissions::from_mode(0o700)).unwrap();
        drop(listener);
        std::fs::remove_file(denied_path).unwrap();
        std::fs::remove_dir(denied_dir).unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn adapt_and_load_use_exact_wire_and_raw_result() {
        let path = socket_path("wire");
        let raw_result = br#"{ "apps": {"http": [1, 2]} }"#;
        let mut adapt_body = br#"{"result":"#.to_vec();
        adapt_body.extend_from_slice(raw_result);
        adapt_body.extend_from_slice(br#", "warnings": []}"#);
        let (server, mut requests) = scripted_server(
            &path,
            vec![
                response("200 OK", &adapt_body),
                response("200 OK", &adapt_body),
                response("200 OK", b"body must remain unread"),
            ],
        );
        let client = CaddyAdminClient::new(&path).unwrap();

        assert_eq!(client.adapt("example.test").await.unwrap(), raw_result);
        assert_wire(
            &requests.recv().await.unwrap(),
            "/adapt",
            "text/caddyfile",
            b"example.test",
        );
        client.load("load.test").await.unwrap();
        assert_wire(
            &requests.recv().await.unwrap(),
            "/adapt",
            "text/caddyfile",
            b"load.test",
        );
        assert_wire(
            &requests.recv().await.unwrap(),
            "/load",
            "application/json",
            raw_result,
        );
        server.await.unwrap();
        std::fs::remove_file(path).unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn adapt_status_json_and_body_precedence_matches_go() {
        type AdaptCase<'a> = (&'a str, Vec<u8>, Result<&'a [u8], &'a str>);
        let cases: Vec<AdaptCase<'_>> = vec![
            ("missing", response("200 OK", br#"{}"#), Ok(b"")),
            ("top-null", response("200 OK", b"null"), Ok(b"")),
            (
                "raw-null",
                response("200 OK", br#"{"result":null}"#),
                Ok(b"null"),
            ),
            (
                "fold-last",
                response("200 OK", br#"{"RESULT":1,"result": { "last": true }}"#),
                Ok(br#"{ "last": true }"#),
            ),
            (
                "unicode-fold",
                response("200 OK", "{\"reſult\":2}".as_bytes()),
                Ok(b"2"),
            ),
            (
                "bad-request",
                response("400 Bad Request", br#"{"error":"bad caddy"}"#),
                Err("bad caddy"),
            ),
            (
                "duplicate-null",
                response("400 Bad Request", br#"{"ERROR":"first","error":null}"#),
                Err("first"),
            ),
            (
                "wrong-type",
                response("400 Bad Request", br#"{"error":7}"#),
                Err(r#"{"error":7}"#),
            ),
            (
                "other-status",
                response("422 Unprocessable Entity", b"raw body"),
                Err("raw body"),
            ),
        ];

        for (label, wire_response, expected) in cases {
            let path = socket_path(label);
            let (server, _requests) = scripted_server(&path, vec![wire_response]);
            let result = CaddyAdminClient::new(&path).unwrap().adapt("x").await;
            match expected {
                Ok(expected) => assert_eq!(result.unwrap(), expected, "{label}"),
                Err(expected) => assert_eq!(result.unwrap_err().to_string(), expected, "{label}"),
            }
            server.await.unwrap();
            std::fs::remove_file(path).unwrap();
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn gzip_multistream_and_malformed_suffix_precedence_matches_go() {
        const GZIP_FIRST: &[u8] = &[
            0x1f, 0x8b, 0x08, 0, 0, 0, 0, 0, 0, 3, 0xab, 0x56, 0x2a, 0x4a, 0x2d, 0x06, 0, 0x2c,
            0x42, 0xe8, 0xc4, 0x05, 0, 0, 0,
        ];
        const GZIP_SECOND: &[u8] = &[
            0x1f, 0x8b, 0x08, 0, 0, 0, 0, 0, 0, 3, 0x2b, 0xcd, 0x29, 0x51, 0xb2, 0xaa, 0x56, 0xca,
            0x2d, 0xcd, 0x29, 0xc9, 0x54, 0xb2, 0x2a, 0x29, 0x2a, 0x4d, 0xad, 0xad, 0x05, 0, 0x5c,
            0x2e, 0x2d, 0x0b, 0x14, 0, 0, 0,
        ];
        let mut multi = GZIP_FIRST.to_vec();
        multi.extend_from_slice(GZIP_SECOND);
        let path = socket_path("multistream");
        let (server, _) = scripted_server(&path, vec![gzip_response("200 OK", &multi)]);
        assert_eq!(
            CaddyAdminClient::new(&path)
                .unwrap()
                .adapt("x")
                .await
                .unwrap(),
            br#"{"multi":true}"#
        );
        server.await.unwrap();
        std::fs::remove_file(path).unwrap();

        for (label, suffix) in [
            ("trailing-junk", b"junk".as_slice()),
            ("truncated-member", &GZIP_SECOND[..GZIP_SECOND.len() - 5]),
        ] {
            let mut malformed = GZIP_ADAPT_BODY.to_vec();
            malformed.extend_from_slice(suffix);
            let path = socket_path(label);
            let (server, _) = scripted_server(&path, vec![gzip_response("200 OK", &malformed)]);
            assert!(matches!(
                CaddyAdminClient::new(&path).unwrap().adapt("x").await,
                Err(CaddyAdminClientError::ReadResponse { .. })
            ));
            server.await.unwrap();
            std::fs::remove_file(path).unwrap();
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn load_uses_partial_error_body_and_never_runs_after_failed_adapt() {
        let path = socket_path("partial-load");
        let partial =
            b"HTTP/1.1 500 Oops\r\nContent-Length: 10\r\nConnection: close\r\n\r\nabc".to_vec();
        let (server, _) = scripted_server(
            &path,
            vec![response("200 OK", br#"{"result":{}}"#), partial],
        );
        let error = CaddyAdminClient::new(&path)
            .unwrap()
            .load("x")
            .await
            .unwrap_err();
        assert_eq!(
            error.to_string(),
            "caddy responded with error: HTTP 500: abc"
        );
        assert_eq!(error.body(), Some(b"abc".as_slice()));
        assert!(error.source().is_some());
        server.await.unwrap();
        std::fs::remove_file(path).unwrap();

        let path = socket_path("no-load");
        let (server, mut requests) = scripted_server(
            &path,
            vec![response("400 Bad Request", br#"{"error":"invalid"}"#)],
        );
        let error = CaddyAdminClient::new(&path)
            .unwrap()
            .load("bad")
            .await
            .unwrap_err();
        assert_eq!(error.to_string(), "adapt Caddyfile to JSON config: invalid");
        assert_wire(
            &requests.recv().await.unwrap(),
            "/adapt",
            "text/caddyfile",
            b"bad",
        );
        server.await.unwrap();
        assert!(requests.try_recv().is_err());
        std::fs::remove_file(path).unwrap();
    }

    #[derive(Clone, Copy, Debug)]
    enum HangingPhase {
        Headers,
        PlainBody,
        GzipBody,
    }

    async fn hanging_server(
        path: &Path,
        phase: HangingPhase,
        accepted: oneshot::Sender<()>,
        eof: oneshot::Sender<()>,
    ) -> tokio::task::JoinHandle<()> {
        let listener = UnixListener::bind(path).unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let _ = read_request(&mut stream).await;
            match phase {
                HangingPhase::Headers => {}
                HangingPhase::PlainBody => {
                    stream
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nabc")
                        .await
                        .unwrap();
                }
                HangingPhase::GzipBody => {
                    stream
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Encoding: gzip\r\n\r\n")
                        .await
                        .unwrap();
                    stream.write_all(GZIP_ADAPT_BODY).await.unwrap();
                    stream.write_all(&[0x1f, 0x8b, 0x08]).await.unwrap();
                }
            }
            accepted.send(()).ok();
            let mut byte = [0_u8];
            loop {
                match stream.read(&mut byte).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {}
                }
            }
            eof.send(()).ok();
        })
    }

    #[tokio::test(flavor = "current_thread")]
    async fn dropping_future_closes_peer_in_every_response_phase() {
        for phase in [
            HangingPhase::Headers,
            HangingPhase::PlainBody,
            HangingPhase::GzipBody,
        ] {
            let path = socket_path(&format!("cancel-{phase:?}"));
            let (accepted_sender, accepted) = oneshot::channel();
            let (eof_sender, eof) = oneshot::channel();
            let server = hanging_server(&path, phase, accepted_sender, eof_sender).await;
            let client = CaddyAdminClient::new(&path).unwrap();
            let mut operation = Box::pin(client.adapt("cancel"));
            tokio::select! {
                result = &mut operation => panic!("operation unexpectedly finished in {phase:?}: {result:?}"),
                result = accepted => result.unwrap(),
            }
            drop(operation);
            timeout(Duration::from_secs(1), eof)
                .await
                .expect("server did not observe cancellation EOF")
                .unwrap();
            server.await.unwrap();
            std::fs::remove_file(path).unwrap();
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn deadline_is_one_absolute_five_second_window_in_every_response_phase() {
        for phase in [
            HangingPhase::Headers,
            HangingPhase::PlainBody,
            HangingPhase::GzipBody,
        ] {
            let path = socket_path(&format!("timeout-{phase:?}"));
            let (accepted_sender, accepted) = oneshot::channel();
            let (eof_sender, eof) = oneshot::channel();
            let server = hanging_server(&path, phase, accepted_sender, eof_sender).await;
            let client = CaddyAdminClient::new(&path).unwrap();
            let started = StdInstant::now();
            let mut operation = Box::pin(client.adapt("timeout"));
            tokio::select! {
                result = &mut operation => panic!("operation unexpectedly finished in {phase:?}: {result:?}"),
                result = accepted => result.unwrap(),
            }
            let error = operation.await.unwrap_err();
            assert!(
                matches!(
                    (phase, error),
                    (HangingPhase::Headers, CaddyAdminClientError::Request { .. })
                        | (
                            HangingPhase::PlainBody | HangingPhase::GzipBody,
                            CaddyAdminClientError::ReadResponse { .. }
                        )
                ),
                "wrong timeout phase classification for {phase:?}"
            );
            assert!(
                (Duration::from_millis(4_800)..Duration::from_millis(5_800))
                    .contains(&started.elapsed()),
                "{phase:?} elapsed: {:?}",
                started.elapsed()
            );
            timeout(Duration::from_secs(1), eof)
                .await
                .expect("server did not observe timeout EOF")
                .unwrap();
            server.await.unwrap();
            std::fs::remove_file(path).unwrap();
        }
    }
}
