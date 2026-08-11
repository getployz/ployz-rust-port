//! Version compatibility policy for Ployz gRPC clients and proxy frontends.
//!
//! The ordinary machine API server deliberately does not use this policy. The
//! two transparent proxy frontends attach [`ServerVersionLayer`], while every
//! connector attaches [`ClientVersionInterceptor`].

use std::cmp::Ordering;
use std::convert::Infallible;
use std::future::Future;
use std::io::{self, Write};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::task::{Context, Poll};

use semver::Version;
use tonic::body::Body;
use tonic::codegen::http::{
    HeaderMap, HeaderValue, Request as HttpRequest, Response as HttpResponse,
};
use tonic::codegen::tokio_stream::Stream;
use tonic::metadata::{AsciiMetadataValue, MetadataMap};
use tonic::service::Interceptor;
use tonic::{Request, Response, Status};
use tower::{Layer, Service};

pub const METADATA_KEY_CLIENT_VERSION: &str = "ployz-client-version";
pub const METADATA_KEY_MIN_SERVER_VERSION: &str = "ployz-min-server-version";
pub const METADATA_KEY_SERVER_VERSION: &str = "ployz-server-version";

pub const MIN_CLIENT_VERSION: &str = "0.20.0";
pub const MIN_SERVER_VERSION: &str = "0.20.0";
pub const RELEASE_URL: &str = "https://github.com/getployz/ployz/releases/latest";

static WARNED: AtomicBool = AtomicBool::new(false);

/// Parsed version with the frozen Masterminds v1.5.0 wire behavior retained.
#[derive(Clone, Debug)]
struct ProtocolVersion {
    semantic: Version,
    wire: String,
    prerelease: String,
}

impl ProtocolVersion {
    fn parse(value: &str) -> Option<Self> {
        let (without_build, build) = match value.split_once('+') {
            Some((left, right)) if !right.contains('+') && identifiers_valid(right) => {
                (left, Some(right))
            }
            Some(_) => return None,
            None => (value, None),
        };

        let (core, prerelease) = match without_build.split_once('-') {
            Some((left, right)) if identifiers_valid(right) => (left, right),
            Some(_) => return None,
            None => (without_build, ""),
        };

        let core = core.strip_prefix('v').unwrap_or(core);
        if core.is_empty() || core.starts_with('V') {
            return None;
        }

        let parts = core.split('.').collect::<Vec<_>>();
        if !(1..=3).contains(&parts.len()) {
            return None;
        }

        let mut numbers = [0_i64; 3];
        for (index, part) in parts.iter().enumerate() {
            if part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            numbers[index] = part.parse().ok()?;
        }

        let mut wire = format!("{}.{}.{}", numbers[0], numbers[1], numbers[2]);
        if !prerelease.is_empty() {
            wire.push('-');
            wire.push_str(prerelease);
        }
        if let Some(build) = build {
            wire.push('+');
            wire.push_str(build);
        }

        let normalized_prerelease = if prerelease.is_empty() {
            String::new()
        } else {
            prerelease
                .split('.')
                .map(|part| {
                    if part.bytes().all(|byte| byte.is_ascii_digit()) {
                        let trimmed = part.trim_start_matches('0');
                        if trimmed.is_empty() { "0" } else { trimmed }
                    } else {
                        part
                    }
                })
                .collect::<Vec<_>>()
                .join(".")
        };

        let mut semantic_text = format!("{}.{}.{}", numbers[0], numbers[1], numbers[2]);
        if !normalized_prerelease.is_empty() {
            semantic_text.push('-');
            semantic_text.push_str(&normalized_prerelease);
        }
        if let Some(build) = build {
            semantic_text.push('+');
            semantic_text.push_str(build);
        }

        Some(Self {
            semantic: Version::parse(&semantic_text).ok()?,
            wire,
            prerelease: prerelease.to_owned(),
        })
    }

    fn parse_or_zero(value: Option<&str>) -> Self {
        value.and_then(Self::parse).unwrap_or_else(Self::zero)
    }

    fn zero() -> Self {
        Self::parse("0.0.0").expect("the static zero version is valid")
    }

    fn less_than(&self, other: &Self) -> bool {
        let core = (
            self.semantic.major,
            self.semantic.minor,
            self.semantic.patch,
        )
            .cmp(&(
                other.semantic.major,
                other.semantic.minor,
                other.semantic.patch,
            ));
        core == Ordering::Less
            || (core == Ordering::Equal
                && compare_prerelease(&self.prerelease, &other.prerelease) == Ordering::Less)
    }
}

fn identifiers_valid(value: &str) -> bool {
    !value.is_empty()
        && value.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

fn compare_prerelease(left: &str, right: &str) -> Ordering {
    if left.is_empty() && right.is_empty() {
        return Ordering::Equal;
    }
    if left.is_empty() {
        return Ordering::Greater;
    }
    if right.is_empty() {
        return Ordering::Less;
    }

    let left = left.split('.').collect::<Vec<_>>();
    let right = right.split('.').collect::<Vec<_>>();
    for index in 0..left.len().max(right.len()) {
        let Some(left_part) = left.get(index) else {
            return Ordering::Less;
        };
        let Some(right_part) = right.get(index) else {
            return Ordering::Greater;
        };
        if left_part == right_part {
            continue;
        }

        return match (left_part.parse::<u64>(), right_part.parse::<u64>()) {
            (Ok(left_number), Ok(right_number)) if left_number > right_number => Ordering::Greater,
            (Ok(_), Ok(_)) => Ordering::Less,
            (Ok(_), Err(_)) => Ordering::Less,
            (Err(_), Ok(_)) => Ordering::Greater,
            (Err(_), Err(_)) => left_part.cmp(right_part),
        };
    }
    Ordering::Equal
}

/// Immutable version policy shared by client and server middleware.
#[derive(Clone, Debug)]
pub struct VersionPolicy {
    current: ProtocolVersion,
    current_metadata: AsciiMetadataValue,
    current_header: HeaderValue,
    minimum_client: ProtocolVersion,
    minimum_server: ProtocolVersion,
}

impl VersionPolicy {
    /// Constructs a policy for an injected binary version.
    ///
    /// Invalid injected versions have the oracle's `0.0.0` fallback.
    #[must_use]
    pub fn new(current_version: &str) -> Self {
        let current = ProtocolVersion::parse(current_version).unwrap_or_else(ProtocolVersion::zero);
        let current_metadata = current
            .wire
            .parse()
            .expect("normalized versions are valid ASCII metadata");
        let current_header = current
            .wire
            .parse()
            .expect("normalized versions are valid HTTP headers");
        Self {
            current,
            current_metadata,
            current_header,
            minimum_client: ProtocolVersion::parse(MIN_CLIENT_VERSION)
                .expect("the static client minimum is valid"),
            minimum_server: ProtocolVersion::parse(MIN_SERVER_VERSION)
                .expect("the static server minimum is valid"),
        }
    }

    /// Returns the normalized version emitted on the wire.
    #[must_use]
    pub fn current_version(&self) -> &str {
        &self.current.wire
    }

    /// Appends client compatibility metadata without replacing caller values.
    pub fn append_client_metadata(&self, metadata: &mut MetadataMap) {
        metadata.append(METADATA_KEY_CLIENT_VERSION, self.current_metadata.clone());
        metadata.append(
            METADATA_KEY_MIN_SERVER_VERSION,
            AsciiMetadataValue::from_static(MIN_SERVER_VERSION),
        );
    }

    /// Validates the first client/minimum-server values in a Tonic request.
    pub fn validate_request<T>(&self, request: &Request<T>) -> Result<(), Status> {
        self.validate_values(
            request
                .metadata()
                .get(METADATA_KEY_CLIENT_VERSION)
                .and_then(|value| value.to_str().ok()),
            request
                .metadata()
                .get(METADATA_KEY_MIN_SERVER_VERSION)
                .and_then(|value| value.to_str().ok()),
        )
    }

    fn validate_headers(&self, headers: &HeaderMap) -> Result<(), Status> {
        self.validate_values(
            headers
                .get(METADATA_KEY_CLIENT_VERSION)
                .and_then(|value| value.to_str().ok()),
            headers
                .get(METADATA_KEY_MIN_SERVER_VERSION)
                .and_then(|value| value.to_str().ok()),
        )
    }

    fn validate_values(
        &self,
        client: Option<&str>,
        required_server: Option<&str>,
    ) -> Result<(), Status> {
        let client = ProtocolVersion::parse_or_zero(client);
        if client.less_than(&self.minimum_client) {
            return Err(Status::failed_precondition(format!(
                "version check failed: client version is below minimum {}. Please upgrade: {RELEASE_URL}",
                self.minimum_client.wire
            )));
        }

        let required_server = ProtocolVersion::parse_or_zero(required_server);
        if self.current.less_than(&required_server) {
            return Err(Status::failed_precondition(format!(
                "version check failed: daemon version {} is below client's minimum required version {}. Please upgrade the daemon: {RELEASE_URL}",
                self.current.wire, required_server.wire
            )));
        }
        Ok(())
    }

    fn prepend_server_header(&self, headers: &mut HeaderMap) {
        let existing = headers
            .get_all(METADATA_KEY_SERVER_VERSION)
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        headers.remove(METADATA_KEY_SERVER_VERSION);
        headers.append(METADATA_KEY_SERVER_VERSION, self.current_header.clone());
        for value in existing {
            headers.append(METADATA_KEY_SERVER_VERSION, value);
        }
    }

    fn warning_message(&self, metadata: &MetadataMap) -> Option<String> {
        let server = ProtocolVersion::parse_or_zero(
            metadata
                .get(METADATA_KEY_SERVER_VERSION)
                .and_then(|value| value.to_str().ok()),
        );
        server.less_than(&self.minimum_server).then(|| {
            format!(
                "WARNING: daemon version is below minimum required version {}. The daemon did not verify this CLI's minimum version requirement, so the operation may not have behaved as intended. Please upgrade the daemon: {RELEASE_URL}\n",
                self.minimum_server.wire
            )
        })
    }

    fn warn_once<W: Write>(
        &self,
        metadata: &MetadataMap,
        warned: &AtomicBool,
        writer: &mut W,
    ) -> io::Result<bool> {
        let Some(message) = self.warning_message(metadata) else {
            return Ok(false);
        };
        if warned.swap(true, AtomicOrdering::SeqCst) {
            return Ok(false);
        }
        writer.write_all(message.as_bytes())?;
        Ok(true)
    }

    fn warn_to_stderr(&self, metadata: &MetadataMap) {
        let mut stderr = io::stderr().lock();
        // grpc-go deliberately ignores WarnWriter failures after marking the
        // process warned. Preserve that limitation without changing RPC success.
        drop(self.warn_once(metadata, &WARNED, &mut stderr));
    }
}

impl Default for VersionPolicy {
    fn default() -> Self {
        Self::new(ployz_internal_version::version())
    }
}

/// Tonic request interceptor used by every Ployz client connector.
#[derive(Clone, Debug, Default)]
pub struct ClientVersionInterceptor {
    policy: VersionPolicy,
}

impl ClientVersionInterceptor {
    #[must_use]
    pub fn new(policy: VersionPolicy) -> Self {
        Self { policy }
    }
}

impl Interceptor for ClientVersionInterceptor {
    fn call(&mut self, mut request: Request<()>) -> Result<Request<()>, Status> {
        self.policy.append_client_metadata(request.metadata_mut());
        Ok(request)
    }
}

/// Inspects a successful unary response and leaves error responses untouched.
pub fn inspect_unary_response<T>(
    result: Result<Response<T>, Status>,
) -> Result<Response<T>, Status> {
    inspect_unary_response_with_policy(result, &VersionPolicy::default())
}

/// Policy-explicit form of [`inspect_unary_response`].
pub fn inspect_unary_response_with_policy<T>(
    result: Result<Response<T>, Status>,
    policy: &VersionPolicy,
) -> Result<Response<T>, Status> {
    if let Ok(response) = &result {
        policy.warn_to_stderr(response.metadata());
    }
    result
}

/// Wraps a successful streaming response without inspecting its header.
///
/// A stream-creation status is returned unchanged and cannot trigger a
/// compatibility warning.
pub fn wrap_streaming_response<S>(
    result: Result<Response<S>, Status>,
) -> Result<VersionedStreaming<S>, Status> {
    result.map(VersionedStreaming::new)
}

/// Policy-explicit form of [`wrap_streaming_response`].
pub fn wrap_streaming_response_with_policy<S>(
    result: Result<Response<S>, Status>,
    policy: VersionPolicy,
) -> Result<VersionedStreaming<S>, Status> {
    result.map(|response| VersionedStreaming::with_policy(response, policy))
}

/// A response stream whose initial metadata is checked only on `header()`.
#[derive(Debug)]
pub struct VersionedStreaming<S> {
    inner: S,
    metadata: MetadataMap,
    extensions: tonic::codegen::http::Extensions,
    policy: VersionPolicy,
}

impl<S> VersionedStreaming<S> {
    /// Wraps an already-successful streaming response without inspecting it.
    #[must_use]
    pub fn new(response: Response<S>) -> Self {
        Self::with_policy(response, VersionPolicy::default())
    }

    /// Wraps a response with an explicit policy, useful for injected builds.
    #[must_use]
    pub fn with_policy(response: Response<S>, policy: VersionPolicy) -> Self {
        let (metadata, inner, extensions) = response.into_parts();
        Self {
            inner,
            metadata,
            extensions,
            policy,
        }
    }

    /// Returns initial metadata and performs the temporary compatibility check.
    #[must_use]
    pub fn header(&self) -> &MetadataMap {
        self.policy.warn_to_stderr(&self.metadata);
        &self.metadata
    }

    #[must_use]
    pub fn get_ref(&self) -> &S {
        &self.inner
    }

    pub fn get_mut(&mut self) -> &mut S {
        &mut self.inner
    }

    #[must_use]
    pub fn extensions(&self) -> &tonic::codegen::http::Extensions {
        &self.extensions
    }

    pub fn extensions_mut(&mut self) -> &mut tonic::codegen::http::Extensions {
        &mut self.extensions
    }

    #[must_use]
    pub fn into_inner(self) -> S {
        self.inner
    }
}

impl<T> VersionedStreaming<tonic::Streaming<T>> {
    pub async fn message(&mut self) -> Result<Option<T>, Status> {
        self.inner.message().await
    }

    pub async fn trailers(&mut self) -> Result<Option<MetadataMap>, Status> {
        self.inner.trailers().await
    }
}

impl<S> Stream for VersionedStreaming<S>
where
    S: Stream + Unpin,
{
    type Item = S::Item;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        Pin::new(&mut self.inner).poll_next(context)
    }
}

/// Layer for the two proxy-only server frontends.
#[derive(Clone, Debug, Default)]
pub struct ServerVersionLayer {
    policy: VersionPolicy,
}

impl ServerVersionLayer {
    #[must_use]
    pub fn new(policy: VersionPolicy) -> Self {
        Self { policy }
    }
}

impl<S> Layer<S> for ServerVersionLayer {
    type Service = ServerVersionService<S>;

    fn layer(&self, inner: S) -> Self::Service {
        ServerVersionService {
            inner,
            policy: self.policy.clone(),
        }
    }
}

/// Conditional response-aware service produced by [`ServerVersionLayer`].
#[derive(Clone, Debug)]
pub struct ServerVersionService<S> {
    inner: S,
    policy: VersionPolicy,
}

impl<S, B> Service<HttpRequest<B>> for ServerVersionService<S>
where
    S: Service<HttpRequest<B>, Response = HttpResponse<Body>, Error = Infallible>,
    S::Future: Send + 'static,
{
    type Response = HttpResponse<Body>;
    type Error = Infallible;
    type Future =
        Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send + 'static>>;

    fn poll_ready(&mut self, context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(context)
    }

    fn call(&mut self, request: HttpRequest<B>) -> Self::Future {
        if let Err(status) = self.policy.validate_headers(request.headers()) {
            return Box::pin(async move { Ok(status.into_http::<Body>()) });
        }

        let future = self.inner.call(request);
        let policy = self.policy.clone();
        Box::pin(async move {
            let mut response = future.await?;
            policy.prepend_server_header(response.headers_mut());
            Ok(response)
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::Mutex;
    use std::task::Waker;

    use super::*;

    static WARNING_TEST_LOCK: Mutex<()> = Mutex::new(());
    use tonic::Code;
    use tonic::metadata::BinaryMetadataValue;

    fn policy() -> VersionPolicy {
        VersionPolicy::new("999.0.0-dev")
    }

    fn request(pairs: &[(&'static str, &'static str)]) -> Request<()> {
        let mut request = Request::new(());
        for (key, value) in pairs {
            request.metadata_mut().append(*key, value.parse().unwrap());
        }
        request
    }

    #[test]
    fn parser_matches_frozen_normalization_and_invalid_fallback_cases() {
        let accepted = [
            ("1", "1.0.0"),
            ("v01.002", "1.2.0"),
            ("001.002.0003-00.alpha+001.x", "1.2.3-00.alpha+001.x"),
            ("0.19.0-nightly-abc1234", "0.19.0-nightly-abc1234"),
            ("999.0.0-dev", "999.0.0-dev"),
            ("9223372036854775807", "9223372036854775807.0.0"),
        ];
        for (input, expected) in accepted {
            assert_eq!(
                ProtocolVersion::parse(input).unwrap().wire,
                expected,
                "{input}"
            );
        }
        assert!(
            ProtocolVersion::parse("1.2.3")
                .unwrap()
                .semantic
                .pre
                .is_empty()
        );
        assert_eq!(
            ProtocolVersion::parse("1.2.3-00")
                .unwrap()
                .semantic
                .pre
                .as_str(),
            "0"
        );

        for input in [
            "",
            "V1.2.3",
            " 1.2.3",
            "1.2.3 ",
            "1..3",
            "1.2.3.4",
            "1.2.3-",
            "1.2.3-a..b",
            "1.2.3+",
            "1.2.3+a+b",
            "1.2.3-é",
            "9223372036854775808",
        ] {
            assert!(ProtocolVersion::parse(input).is_none(), "{input}");
        }
    }

    #[test]
    fn comparator_preserves_build_insensitivity_and_zero_padding_flaw() {
        let less = |left: &str, right: &str| {
            ProtocolVersion::parse(left)
                .unwrap()
                .less_than(&ProtocolVersion::parse(right).unwrap())
        };
        assert!(less("1.0.0-alpha", "1.0.0"));
        assert!(less("1.0.0-1", "1.0.0-alpha"));
        assert!(less("1.0.0-alpha", "1.0.0-alpha.1"));
        assert!(!less("1.0.0+one", "1.0.0+two"));
        assert!(!less("1.0.0+two", "1.0.0+one"));
        assert!(less("1.0.0-0", "1.0.0-00"));
        assert!(less("1.0.0-00", "1.0.0-0"));
        assert!(less("1.0.0-18446744073709551615", "1.0.0-x"));
        assert!(less(
            "1.0.0-18446744073709551616",
            "1.0.0-18446744073709551617"
        ));
    }

    #[test]
    fn validation_uses_first_values_and_exact_error_precedence() {
        let policy = policy();
        let cases = [
            (vec![], Some("client version is below minimum")),
            (
                vec![(METADATA_KEY_CLIENT_VERSION, "bad")],
                Some("client version is below minimum"),
            ),
            (
                vec![(METADATA_KEY_CLIENT_VERSION, "0.19.9")],
                Some("client version is below minimum"),
            ),
            (
                vec![
                    (METADATA_KEY_CLIENT_VERSION, "999.0.0"),
                    (METADATA_KEY_MIN_SERVER_VERSION, "999.0.0"),
                ],
                Some(
                    "daemon version 999.0.0-dev is below client's minimum required version 999.0.0",
                ),
            ),
            (
                vec![
                    (METADATA_KEY_CLIENT_VERSION, "999.0.0"),
                    (METADATA_KEY_MIN_SERVER_VERSION, "0.0.1"),
                ],
                None,
            ),
        ];

        for (pairs, expected_message) in cases {
            let result = policy.validate_request(&request(&pairs));
            match expected_message {
                Some(fragment) => {
                    let status = result.unwrap_err();
                    assert_eq!(status.code(), Code::FailedPrecondition);
                    assert!(status.message().contains(fragment), "{}", status.message());
                    assert!(status.message().ends_with(RELEASE_URL));
                    assert!(status.details().is_empty());
                    assert!(status.metadata().is_empty());
                }
                None => result.unwrap(),
            }
        }

        let duplicate = request(&[
            (METADATA_KEY_CLIENT_VERSION, "999.0.0"),
            (METADATA_KEY_CLIENT_VERSION, "0.0.0"),
            (METADATA_KEY_MIN_SERVER_VERSION, "0.0.1"),
        ]);
        policy.validate_request(&duplicate).unwrap();
    }

    #[test]
    fn client_interceptor_appends_and_preserves_every_existing_value() {
        #[derive(Clone, Debug, PartialEq)]
        struct DeadlineMarker(u8);

        let mut request = request(&[(METADATA_KEY_CLIENT_VERSION, "caller-first")]);
        request
            .metadata_mut()
            .append_bin("trace-bin", BinaryMetadataValue::from_bytes(&[0, 255]));
        request
            .metadata_mut()
            .insert("grpc-timeout", "60000000u".parse().unwrap());
        request.extensions_mut().insert(DeadlineMarker(7));

        let mut interceptor = ClientVersionInterceptor::new(policy());
        let request = interceptor.call(request).unwrap();
        assert_eq!(
            request
                .metadata()
                .get_all(METADATA_KEY_CLIENT_VERSION)
                .iter()
                .map(|value| value.to_str().unwrap())
                .collect::<Vec<_>>(),
            ["caller-first", "999.0.0-dev"]
        );
        assert_eq!(
            request
                .metadata()
                .get_all(METADATA_KEY_MIN_SERVER_VERSION)
                .iter()
                .map(|value| value.to_str().unwrap())
                .collect::<Vec<_>>(),
            [MIN_SERVER_VERSION]
        );
        assert_eq!(
            request
                .metadata()
                .get_bin("trace-bin")
                .unwrap()
                .to_bytes()
                .unwrap()
                .as_ref(),
            [0, 255]
        );
        assert_eq!(request.metadata().get("grpc-timeout").unwrap(), "60000000u");
        assert_eq!(
            request.extensions().get::<DeadlineMarker>(),
            Some(&DeadlineMarker(7))
        );
    }

    #[test]
    fn warnings_are_exact_once_and_only_for_old_missing_or_malformed_first_values() {
        let policy = policy();
        let expected = format!(
            "WARNING: daemon version is below minimum required version 0.20.0. The daemon did not verify this CLI's minimum version requirement, so the operation may not have behaved as intended. Please upgrade the daemon: {RELEASE_URL}\n"
        );

        for first in [None, Some("bad"), Some("0.19.9")] {
            let mut metadata = MetadataMap::new();
            if let Some(first) = first {
                metadata.append(METADATA_KEY_SERVER_VERSION, first.parse().unwrap());
                metadata.append(METADATA_KEY_SERVER_VERSION, "999.0.0".parse().unwrap());
            }
            let warned = AtomicBool::new(false);
            let mut output = Vec::new();
            assert!(policy.warn_once(&metadata, &warned, &mut output).unwrap());
            assert!(!policy.warn_once(&metadata, &warned, &mut output).unwrap());
            assert_eq!(output, expected.as_bytes());
        }

        let mut current = MetadataMap::new();
        current.insert(METADATA_KEY_SERVER_VERSION, "999.0.0".parse().unwrap());
        assert!(
            !policy
                .warn_once(&current, &AtomicBool::new(false), &mut Vec::new())
                .unwrap()
        );
    }

    #[derive(Clone, Default)]
    struct SharedWriter(Arc<Mutex<Vec<u8>>>);

    impl Write for SharedWriter {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0
                .lock()
                .expect("warning buffer lock")
                .extend_from_slice(bytes);
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn concurrent_warning_stress_emits_one_complete_line() {
        let gate = Arc::new(AtomicBool::new(false));
        let writer = SharedWriter::default();
        let mut metadata = MetadataMap::new();
        metadata.insert(METADATA_KEY_SERVER_VERSION, "0.0.0".parse().unwrap());
        let policy = policy();

        let threads = (0..256)
            .map(|_| {
                let gate = gate.clone();
                let mut writer = writer.clone();
                let metadata = metadata.clone();
                let policy = policy.clone();
                std::thread::spawn(move || policy.warn_once(&metadata, &gate, &mut writer).unwrap())
            })
            .collect::<Vec<_>>();
        assert_eq!(
            threads
                .into_iter()
                .map(|thread| thread.join().unwrap())
                .filter(|emitted| *emitted)
                .count(),
            1
        );
        let output = writer.0.lock().expect("warning buffer lock").clone();
        assert_eq!(output.iter().filter(|byte| **byte == b'\n').count(), 1);
        assert!(String::from_utf8(output).unwrap().starts_with("WARNING:"));
    }

    #[derive(Clone)]
    struct Handler {
        calls: Arc<std::sync::atomic::AtomicUsize>,
        response_headers: HeaderMap,
    }

    impl<B> Service<HttpRequest<B>> for Handler {
        type Response = HttpResponse<Body>;
        type Error = Infallible;
        type Future = std::future::Ready<Result<Self::Response, Self::Error>>;

        fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, _request: HttpRequest<B>) -> Self::Future {
            self.calls.fetch_add(1, AtomicOrdering::SeqCst);
            let mut response = HttpResponse::new(Body::empty());
            *response.headers_mut() = self.response_headers.clone();
            std::future::ready(Ok(response))
        }
    }

    #[test]
    fn server_layer_rejects_before_handler_and_has_no_version_header() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let handler = Handler {
            calls: calls.clone(),
            response_headers: HeaderMap::new(),
        };
        let mut service = ServerVersionLayer::new(policy()).layer(handler);
        let response = block_on(service.call(HttpRequest::new(()))).unwrap();
        assert_eq!(calls.load(AtomicOrdering::SeqCst), 0);
        assert_eq!(response.headers().get("grpc-status").unwrap(), "9");
        assert!(
            response
                .headers()
                .get("grpc-message")
                .unwrap()
                .to_str()
                .unwrap()
                .contains("client%20version%20is%20below%20minimum")
        );
        assert!(
            response
                .headers()
                .get(METADATA_KEY_SERVER_VERSION)
                .is_none()
        );

        let mut non_text = HttpRequest::new(());
        non_text.headers_mut().insert(
            METADATA_KEY_CLIENT_VERSION,
            HeaderValue::from_bytes(&[0xff]).unwrap(),
        );
        let response = block_on(service.call(non_text)).unwrap();
        assert_eq!(response.headers().get("grpc-status").unwrap(), "9");
        assert_eq!(calls.load(AtomicOrdering::SeqCst), 0);
    }

    #[test]
    fn server_layer_prepends_version_and_preserves_duplicate_and_binary_headers() {
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut response_headers = HeaderMap::new();
        response_headers.append(METADATA_KEY_SERVER_VERSION, "handler-one".parse().unwrap());
        response_headers.append(METADATA_KEY_SERVER_VERSION, "handler-two".parse().unwrap());
        response_headers.append("opaque-bin", "AAH/".parse().unwrap());
        response_headers.insert("grpc-status", "10".parse().unwrap());
        response_headers.insert("grpc-message", "handler%20detail".parse().unwrap());
        response_headers.insert("grpc-status-details-bin", "CgA".parse().unwrap());
        let handler = Handler {
            calls: calls.clone(),
            response_headers,
        };
        let mut service = ServerVersionLayer::new(policy()).layer(handler);
        let mut request = HttpRequest::new(());
        request
            .headers_mut()
            .append(METADATA_KEY_CLIENT_VERSION, "999.0.0".parse().unwrap());
        request
            .headers_mut()
            .append(METADATA_KEY_MIN_SERVER_VERSION, "0.20.0".parse().unwrap());
        let response = block_on(service.call(request)).unwrap();
        assert_eq!(calls.load(AtomicOrdering::SeqCst), 1);
        assert_eq!(
            response
                .headers()
                .get_all(METADATA_KEY_SERVER_VERSION)
                .iter()
                .map(|value| value.to_str().unwrap())
                .collect::<Vec<_>>(),
            ["999.0.0-dev", "handler-one", "handler-two"]
        );
        assert_eq!(response.headers().get("grpc-status").unwrap(), "10");
        assert_eq!(
            response.headers().get("grpc-message").unwrap(),
            "handler%20detail"
        );
        assert_eq!(
            response.headers().get("grpc-status-details-bin").unwrap(),
            "CgA"
        );
        assert_eq!(response.headers().get("opaque-bin").unwrap(), "AAH/");
    }

    struct PendingHandler {
        called: Arc<AtomicBool>,
        dropped: Arc<AtomicBool>,
    }

    struct PendingResponse {
        dropped: Arc<AtomicBool>,
    }

    impl Future for PendingResponse {
        type Output = Result<HttpResponse<Body>, Infallible>;

        fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
            Poll::Pending
        }
    }

    impl Drop for PendingResponse {
        fn drop(&mut self) {
            self.dropped.store(true, AtomicOrdering::SeqCst);
        }
    }

    impl Service<HttpRequest<()>> for PendingHandler {
        type Response = HttpResponse<Body>;
        type Error = Infallible;
        type Future = PendingResponse;

        fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
            Poll::Ready(Ok(()))
        }

        fn call(&mut self, _request: HttpRequest<()>) -> Self::Future {
            self.called.store(true, AtomicOrdering::SeqCst);
            PendingResponse {
                dropped: self.dropped.clone(),
            }
        }
    }

    #[test]
    fn accepted_request_future_directly_owns_and_cancels_inner_future() {
        let called = Arc::new(AtomicBool::new(false));
        let dropped = Arc::new(AtomicBool::new(false));
        let handler = PendingHandler {
            called: called.clone(),
            dropped: dropped.clone(),
        };
        let mut service = ServerVersionLayer::new(policy()).layer(handler);
        let mut request = HttpRequest::new(());
        request
            .headers_mut()
            .append(METADATA_KEY_CLIENT_VERSION, "999.0.0".parse().unwrap());
        request
            .headers_mut()
            .append(METADATA_KEY_MIN_SERVER_VERSION, "0.20.0".parse().unwrap());
        let future = service.call(request);
        assert!(called.load(AtomicOrdering::SeqCst));
        assert!(!dropped.load(AtomicOrdering::SeqCst));
        drop(future);
        assert!(dropped.load(AtomicOrdering::SeqCst));
    }

    #[test]
    fn unary_error_is_returned_without_warning_inspection() {
        let mut metadata = MetadataMap::new();
        metadata.append(METADATA_KEY_SERVER_VERSION, "0.0.0".parse().unwrap());
        let status = Status::with_details_and_metadata(
            Code::Aborted,
            "handler detail",
            tonic::codegen::Bytes::from_static(&[10, 0]),
            metadata,
        );
        let returned =
            inspect_unary_response_with_policy::<()>(Err(status), &policy()).unwrap_err();
        assert_eq!(returned.code(), Code::Aborted);
        assert_eq!(returned.message(), "handler detail");
        assert_eq!(returned.details(), [10, 0]);
        assert_eq!(
            returned
                .metadata()
                .get(METADATA_KEY_SERVER_VERSION)
                .unwrap(),
            "0.0.0"
        );
    }

    #[derive(Debug)]
    struct NeverReady;

    impl Stream for NeverReady {
        type Item = Result<u8, Status>;

        fn poll_next(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            Poll::Pending
        }
    }

    #[test]
    fn stream_creation_and_poll_do_not_inspect_header() {
        let _guard = WARNING_TEST_LOCK.lock().expect("warning test lock");
        WARNED.store(false, AtomicOrdering::SeqCst);
        let mut response = Response::new(NeverReady);
        response
            .metadata_mut()
            .insert(METADATA_KEY_SERVER_VERSION, "0.0.0".parse().unwrap());
        response.extensions_mut().insert(17_u8);
        let mut stream = VersionedStreaming::with_policy(response, policy());
        assert_eq!(stream.extensions().get::<u8>(), Some(&17));
        let warned = AtomicBool::new(false);
        let mut output = Vec::new();
        let mut context = Context::from_waker(Waker::noop());
        assert!(Pin::new(&mut stream).poll_next(&mut context).is_pending());
        assert!(!WARNED.load(AtomicOrdering::SeqCst));
        assert_eq!(
            stream.header().get(METADATA_KEY_SERVER_VERSION).unwrap(),
            "0.0.0"
        );
        assert!(WARNED.load(AtomicOrdering::SeqCst));
        let _ = stream.header();

        assert!(!warned.load(AtomicOrdering::SeqCst));
        assert!(output.is_empty());
        assert!(
            stream
                .policy
                .warn_once(&stream.metadata, &warned, &mut output)
                .unwrap()
        );
        assert!(
            !stream
                .policy
                .warn_once(&stream.metadata, &warned, &mut output)
                .unwrap()
        );
        assert!(String::from_utf8(output).unwrap().starts_with("WARNING:"));
        WARNED.store(false, AtomicOrdering::SeqCst);
    }

    #[test]
    fn stream_creation_error_is_unchanged_and_never_warns() {
        let _guard = WARNING_TEST_LOCK.lock().expect("warning test lock");
        WARNED.store(false, AtomicOrdering::SeqCst);
        let mut metadata = MetadataMap::new();
        metadata.insert(METADATA_KEY_SERVER_VERSION, "0.0.0".parse().unwrap());
        let status = Status::with_details_and_metadata(
            Code::Unavailable,
            "header failed",
            tonic::codegen::Bytes::from_static(&[1, 2, 3]),
            metadata,
        );
        let status =
            wrap_streaming_response_with_policy::<NeverReady>(Err(status), policy()).unwrap_err();
        assert_eq!(status.code(), Code::Unavailable);
        assert_eq!(status.message(), "header failed");
        assert_eq!(status.details(), [1, 2, 3]);
        assert_eq!(
            status.metadata().get(METADATA_KEY_SERVER_VERSION).unwrap(),
            "0.0.0"
        );
        assert!(!WARNED.load(AtomicOrdering::SeqCst));
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        let mut context = Context::from_waker(Waker::noop());
        let mut future = Box::pin(future);
        loop {
            match future.as_mut().poll(&mut context) {
                Poll::Ready(output) => return output,
                Poll::Pending => std::thread::yield_now(),
            }
        }
    }
}
