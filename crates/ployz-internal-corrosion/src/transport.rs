use std::{
    error::Error,
    fmt, io,
    pin::Pin,
    sync::{Arc, Mutex},
    time::Duration,
};

use async_compression::tokio::bufread::GzipDecoder;
use backon::Retryable as _;
use bytes::Bytes;
use futures_util::{StreamExt as _, future};
use h2::Reason;
use http_body_util::{BodyExt as _, Full};
use hyper::{
    Method, Request, StatusCode, Uri, Version,
    body::Incoming,
    header::{self, HeaderMap, HeaderValue},
};
use hyper_util::{
    client::legacy::{Client, Error as HyperClientError, connect::HttpConnector},
    rt::TokioExecutor,
};
use tokio::io::{AsyncRead, AsyncReadExt as _, BufReader};
use tokio_util::io::StreamReader;

use crate::backoff::RandomizedBackoff;

type H2Client = Client<HttpConnector, Full<Bytes>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ClientErrorKind {
    Admin,
    Http,
    Json,
    Protocol,
    SubscriptionNotFound,
}

#[derive(Debug)]
pub struct ClientError {
    kind: ClientErrorKind,
    message: String,
    source: Option<Box<dyn Error + Send + Sync>>,
}

impl ClientError {
    pub(crate) fn new(kind: ClientErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            source: None,
        }
    }

    pub(crate) fn with_source(
        kind: ClientErrorKind,
        message: impl Into<String>,
        source: impl Error + Send + Sync + 'static,
    ) -> Self {
        Self {
            kind,
            message: message.into(),
            source: Some(Box::new(source)),
        }
    }

    pub fn kind(&self) -> ClientErrorKind {
        self.kind
    }

    pub fn is_subscription_not_found(&self) -> bool {
        if self.kind == ClientErrorKind::SubscriptionNotFound {
            return true;
        }
        let mut source = self.source();
        while let Some(error) = source {
            if error
                .downcast_ref::<Self>()
                .is_some_and(|error| error.kind == ClientErrorKind::SubscriptionNotFound)
            {
                return true;
            }
            source = error.source();
        }
        false
    }
}

impl fmt::Display for ClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)?;
        if let Some(source) = &self.source {
            write!(formatter, ": {source}")?;
        }
        Ok(())
    }
}

impl Error for ClientError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        self.source.as_deref().map(|source| source as _)
    }
}

pub(crate) struct HttpResponse {
    pub(crate) status: StatusCode,
    pub(crate) headers: HeaderMap,
    pub(crate) body: ResponseReader,
}

pub(crate) struct ResponseReader {
    reader: Pin<Box<dyn AsyncRead + Send>>,
    staged_error: Arc<Mutex<Option<io::Error>>>,
}

impl ResponseReader {
    fn new(body: Incoming, gzip: bool) -> Self {
        let staged_error = Arc::new(Mutex::new(None));
        let staged_for_stream = Arc::clone(&staged_error);
        let stream = body.into_data_stream().filter_map(move |item| {
            let staged_error = Arc::clone(&staged_for_stream);
            future::ready(match item {
                Ok(bytes) => Some(Ok::<Bytes, io::Error>(bytes)),
                Err(error) => {
                    let mut slot = staged_error.lock().expect("body error lock poisoned");
                    if slot.is_none() {
                        *slot = Some(io::Error::other(error));
                    }
                    None
                }
            })
        });
        let reader = StreamReader::new(stream);
        let reader: Pin<Box<dyn AsyncRead + Send>> = if gzip {
            let mut decoder = GzipDecoder::new(BufReader::new(reader));
            decoder.multiple_members(true);
            Box::pin(decoder)
        } else {
            Box::pin(reader)
        };
        Self {
            reader,
            staged_error,
        }
    }

    pub(crate) async fn read(&mut self, buffer: &mut [u8]) -> Result<usize, ClientError> {
        match self.reader.read(buffer).await {
            Ok(0) => {
                if let Some(error) = self.take_staged_error() {
                    Err(ClientError::with_source(
                        ClientErrorKind::Http,
                        "read response body",
                        error,
                    ))
                } else {
                    Ok(0)
                }
            }
            Ok(read) => Ok(read),
            Err(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
                if let Some(staged) = self.take_staged_error() {
                    Err(ClientError::with_source(
                        ClientErrorKind::Http,
                        "read response body",
                        staged,
                    ))
                } else {
                    Err(ClientError::with_source(
                        ClientErrorKind::Http,
                        "read response body",
                        error,
                    ))
                }
            }
            Err(error) => Err(ClientError::with_source(
                ClientErrorKind::Http,
                "read response body",
                error,
            )),
        }
    }

    pub(crate) async fn read_to_end(mut self) -> Result<Vec<u8>, ClientError> {
        let mut body = Vec::new();
        let mut chunk = [0_u8; 8192];
        loop {
            match self.read(&mut chunk).await? {
                0 => return Ok(body),
                read => body.extend_from_slice(&chunk[..read]),
            }
        }
    }

    fn take_staged_error(&self) -> Option<io::Error> {
        self.staged_error
            .lock()
            .expect("body error lock poisoned")
            .take()
    }
}

#[derive(Clone)]
pub(crate) struct Transport {
    client: H2Client,
    base: Uri,
    token: Option<HeaderValue>,
}

impl Transport {
    pub(crate) fn new(
        address: std::net::SocketAddr,
        bearer_token: impl Into<String>,
    ) -> Result<Self, ClientError> {
        let base: Uri = format!("http://{address}").parse().map_err(|error| {
            ClientError::with_source(ClientErrorKind::Http, "invalid Corrosion URL", error)
        })?;

        let mut connector = HttpConnector::new();
        // The oracle also speaks cleartext prior-knowledge HTTP/2 to an https-spelled redirect.
        connector.enforce_http(false);
        connector.set_connect_timeout(Some(Duration::from_secs(3)));
        let mut builder = Client::builder(TokioExecutor::new());
        builder
            .http2_only(true)
            .retry_canceled_requests(false)
            .pool_idle_timeout(None)
            .http2_max_header_list_size(10 << 20)
            .http2_initial_connection_window_size((1 << 30) + 65_535)
            .http2_initial_stream_window_size(4 << 20);

        let bearer_token = bearer_token.into();
        let token = if bearer_token.is_empty() {
            None
        } else {
            Some(
                HeaderValue::from_str(&format!("Bearer {bearer_token}")).map_err(|error| {
                    ClientError::with_source(
                        ClientErrorKind::Http,
                        "invalid bearer token header",
                        error,
                    )
                })?,
            )
        };
        Ok(Self {
            client: builder.build(connector),
            base,
            token,
        })
    }

    #[cfg(test)]
    pub(crate) fn with_base(base: Uri, bearer_token: &str) -> Self {
        let mut connector = HttpConnector::new();
        connector.enforce_http(false);
        connector.set_connect_timeout(Some(Duration::from_secs(3)));
        let mut builder = Client::builder(TokioExecutor::new());
        builder.http2_only(true).retry_canceled_requests(false);
        Self {
            client: builder.build(connector),
            base,
            token: (!bearer_token.is_empty()).then(|| {
                HeaderValue::from_str(&format!("Bearer {bearer_token}"))
                    .expect("test token is a valid header")
            }),
        }
    }

    pub(crate) fn endpoint(&self, path_and_query: &str) -> Result<Uri, ClientError> {
        resolve_uri(&self.base, path_and_query)
    }

    pub(crate) async fn request(
        &self,
        method: Method,
        uri: Uri,
        body: Bytes,
    ) -> Result<HttpResponse, ClientError> {
        let mut current_method = method;
        let mut current_uri = uri;
        let mut current_body = body;

        for redirects in 0..10 {
            let response = self
                .request_with_transport_retries(
                    current_method.clone(),
                    current_uri.clone(),
                    current_body.clone(),
                )
                .await?;
            let status = response.status();
            if !matches!(status.as_u16(), 301 | 302 | 303 | 307 | 308) {
                return Ok(response_reader(response));
            }

            let Some(location) = response.headers().get(header::LOCATION) else {
                return Ok(response_reader(response));
            };
            if location.is_empty() {
                return Ok(response_reader(response));
            }
            if redirects == 9 {
                return Err(ClientError::new(
                    ClientErrorKind::Http,
                    "stopped after 10 redirects",
                ));
            }
            let location = location.to_str().map_err(|error| {
                ClientError::with_source(ClientErrorKind::Http, "invalid redirect location", error)
            })?;
            current_uri = resolve_uri(&current_uri, location)?;
            if matches!(status.as_u16(), 301..=303)
                && current_method != Method::GET
                && current_method != Method::HEAD
            {
                current_method = Method::GET;
                current_body = Bytes::new();
            }
            drop(response);
        }
        unreachable!("redirect loop returns at its boundary")
    }

    async fn request_with_transport_retries(
        &self,
        method: Method,
        uri: Uri,
        body: Bytes,
    ) -> Result<hyper::Response<Incoming>, ClientError> {
        (|| self.request_with_protocol_retries(method.clone(), uri.clone(), body.clone()))
            .retry(RandomizedBackoff::transport())
            .when(HyperClientError::is_connect)
            .await
            .map_err(|error| ClientError::with_source(ClientErrorKind::Http, "send request", error))
    }

    async fn request_with_protocol_retries(
        &self,
        method: Method,
        uri: Uri,
        body: Bytes,
    ) -> Result<hyper::Response<Incoming>, HyperClientError> {
        for attempt in 0_u32..8 {
            if attempt >= 2 {
                let base = Duration::from_secs(1_u64 << (attempt - 2));
                let jitter = Duration::from_secs_f64(base.as_secs_f64() * fastrand::f64() * 0.1);
                tokio::time::sleep(base.saturating_add(jitter)).await;
            }
            let request = self.build_request(method.clone(), uri.clone(), body.clone());
            match self.client.request(request).await {
                Ok(response) => return Ok(response),
                Err(error) if attempt < 7 && retryable_h2_error(&error) => continue,
                Err(error) => return Err(error),
            }
        }
        unreachable!("bounded retry loop returns on its final attempt")
    }

    fn build_request(&self, method: Method, uri: Uri, body: Bytes) -> Request<Full<Bytes>> {
        let mut builder = Request::builder()
            .method(method)
            .uri(uri)
            .version(Version::HTTP_2)
            .header(header::ACCEPT, "application/json")
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::ACCEPT_ENCODING, "gzip")
            .header(header::USER_AGENT, "Go-http-client/2.0");
        if let Some(token) = &self.token {
            builder = builder.header(header::AUTHORIZATION, token);
        }
        builder
            .body(Full::new(body))
            .expect("fixed request is valid")
    }
}

fn response_reader(mut response: hyper::Response<Incoming>) -> HttpResponse {
    let gzip = response
        .headers()
        .get(header::CONTENT_ENCODING)
        .is_some_and(|value| value.as_bytes().eq_ignore_ascii_case(b"gzip"));
    if gzip {
        response.headers_mut().remove(header::CONTENT_ENCODING);
        response.headers_mut().remove(header::CONTENT_LENGTH);
    }
    let (parts, body) = response.into_parts();
    HttpResponse {
        status: parts.status,
        headers: parts.headers,
        body: ResponseReader::new(body, gzip),
    }
}

fn retryable_h2_error(error: &(dyn Error + 'static)) -> bool {
    let mut source = Some(error);
    while let Some(current) = source {
        if let Some(error) = current.downcast_ref::<h2::Error>() {
            return matches!(
                error.reason(),
                Some(Reason::REFUSED_STREAM | Reason::PROTOCOL_ERROR)
            ) || error.is_go_away();
        }
        source = current.source();
    }
    false
}

fn resolve_uri(base: &Uri, reference: &str) -> Result<Uri, ClientError> {
    let reference = reference.split('#').next().unwrap_or_default();
    if let Some((candidate, _)) = reference.split_once(':')
        && !candidate.is_empty()
        && candidate.bytes().enumerate().all(|(index, byte)| {
            byte.is_ascii_alphabetic()
                || (index != 0 && matches!(byte, b'+' | b'-' | b'.' | b'0'..=b'9'))
        })
        && !matches!(candidate, "http" | "https")
    {
        return Err(ClientError::new(
            ClientErrorKind::Http,
            format!("unsupported redirect scheme in {reference}"),
        ));
    }
    if let Ok(uri) = reference.parse::<Uri>()
        && uri.scheme().is_some()
    {
        validate_scheme(&uri)?;
        return Ok(uri);
    }

    let scheme = base
        .scheme_str()
        .ok_or_else(|| ClientError::new(ClientErrorKind::Http, "base URL has no scheme"))?;
    let authority = base
        .authority()
        .ok_or_else(|| ClientError::new(ClientErrorKind::Http, "base URL has no authority"))?;
    let absolute = if let Some(authority_reference) = reference.strip_prefix("//") {
        format!("{scheme}://{authority_reference}")
    } else if reference.starts_with('/') {
        format!("{scheme}://{authority}{reference}")
    } else if reference.starts_with('?') {
        format!("{scheme}://{authority}{}{reference}", base.path())
    } else {
        let base_path = base.path();
        let directory = base_path
            .rsplit_once('/')
            .map_or("/", |(directory, _)| directory);
        let joined = normalize_path(&format!("{directory}/{reference}"));
        format!("{scheme}://{authority}{joined}")
    };
    let uri: Uri = absolute.parse().map_err(|error| {
        ClientError::with_source(ClientErrorKind::Http, "invalid redirect location", error)
    })?;
    validate_scheme(&uri)?;
    Ok(uri)
}

fn normalize_path(path_and_query: &str) -> String {
    let (path, query) = path_and_query
        .split_once('?')
        .map_or((path_and_query, None), |(path, query)| (path, Some(query)));
    let mut segments = Vec::new();
    for segment in path.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                segments.pop();
            }
            segment => segments.push(segment),
        }
    }
    let mut normalized = format!("/{}", segments.join("/"));
    if let Some(query) = query {
        normalized.push('?');
        normalized.push_str(query);
    }
    normalized
}

fn validate_scheme(uri: &Uri) -> Result<(), ClientError> {
    if matches!(uri.scheme_str(), Some("http" | "https")) {
        Ok(())
    } else {
        Err(ClientError::new(
            ClientErrorKind::Http,
            format!("unsupported redirect scheme in {uri}"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_redirect_references() {
        let base: Uri = "http://127.0.0.1:1234/a/b?old=1".parse().unwrap();
        assert_eq!(
            resolve_uri(&base, "/x?q=1").unwrap(),
            "http://127.0.0.1:1234/x?q=1"
        );
        assert_eq!(
            resolve_uri(&base, "../c").unwrap(),
            "http://127.0.0.1:1234/c"
        );
        assert_eq!(
            resolve_uri(&base, "?new=2").unwrap(),
            "http://127.0.0.1:1234/a/b?new=2"
        );
        assert!(resolve_uri(&base, "file:///secret").is_err());
    }
}
