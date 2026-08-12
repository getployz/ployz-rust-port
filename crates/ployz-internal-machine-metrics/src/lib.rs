//! Plain-HTTP Prometheus endpoint for a Ployz machine.

mod encoding;
mod routing;

use std::convert::Infallible;
use std::fmt;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::sync::LazyLock;
use std::task::{Context, Poll};
use std::time::Duration;

use http_body_util::{BodyExt as _, Full, combinators::BoxBody};
use hyper::body::{Body, Bytes, Frame, SizeHint};
use hyper::header::{
    ACCEPT, ACCEPT_ENCODING, CONNECTION, CONTENT_ENCODING, CONTENT_TYPE, LOCATION,
};
use hyper::server::conn::http1;
use hyper::service::service_fn;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::{TokioIo, TokioTimer};
use ployz_internal_metrics::CreatedIntCounterVec;
use prometheus::{Gauge, Opts};
use tokio::net::{TcpListener, TcpStream};
use tokio::task::{JoinError, JoinSet};
use tokio::time::{Instant, sleep, timeout_at};
use tokio_util::sync::CancellationToken;

use crate::routing::Route;

#[cfg(test)]
use tokio::sync::Notify;

/// The fixed port used by the machine metrics endpoint.
pub const PORT: u16 = 51_090;

const READ_TIMEOUT: Duration = Duration::from_secs(5);
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
const SMALL_RESPONSE_LIMIT: usize = 2_048;
const MAX_POST_HANDLER_READ_BYTES: u64 = 256 << 10;
const MAX_OPTIONS_BODY_BYTES: u64 = 4 << 10;
// The approved request-limit deviation cannot reproduce Go's shared head
// budget. Keep this well above Hyper's 100-field default while bounding its
// two proportional parser scratch arrays to about 256 KiB per parse.
const MAX_REQUEST_HEADERS: usize = 4_096;
const _: () = assert!(MAX_REQUEST_HEADERS > 100);
const _: () = assert!(MAX_REQUEST_HEADERS * 64 <= 256 * 1_024);

/// A metrics server bound to one concrete machine address.
#[derive(Debug, Clone)]
pub struct Server {
    listen_addr: IpAddr,
    port: u16,
}

impl Server {
    /// Creates a server that will bind `listen_addr` on [`PORT`].
    ///
    /// Construction initializes the process-wide handler metrics, matching the
    /// oracle's `promhttp.Handler()` construction semantics.
    pub fn new(listen_addr: IpAddr) -> Self {
        LazyLock::force(&HANDLER_METRICS);
        Self {
            listen_addr,
            port: PORT,
        }
    }

    #[cfg(test)]
    fn with_port(listen_addr: IpAddr, port: u16) -> Self {
        LazyLock::force(&HANDLER_METRICS);
        Self { listen_addr, port }
    }

    /// Binds and serves until cancellation, then drains connections for up to
    /// five seconds.
    ///
    /// Binding happens before cancellation is observed, so a pre-cancelled
    /// call still reports an occupied or otherwise invalid address.
    pub async fn run(&self, cancellation: &CancellationToken) -> Result<(), ServerError> {
        let address = SocketAddr::new(self.listen_addr, self.port);
        let listener = TcpListener::bind(address)
            .await
            .map_err(|source| ServerError::Bind { address, source })?;
        self.serve(listener, cancellation).await
    }

    async fn serve(
        &self,
        listener: TcpListener,
        cancellation: &CancellationToken,
    ) -> Result<(), ServerError> {
        let connection_cancellation = CancellationToken::new();
        let mut connections = JoinSet::new();
        let mut retry_delay = Duration::ZERO;

        loop {
            tokio::select! {
                _ = cancellation.cancelled() => break,
                completed = connections.join_next(), if !connections.is_empty() => {
                    report_supervisor_result(completed.expect("non-empty JoinSet returned None"));
                }
                accepted = listener.accept() => {
                    match accepted {
                        Ok((stream, _)) => {
                            retry_delay = Duration::ZERO;
                            spawn_connection(
                                &mut connections,
                                stream,
                                connection_cancellation.clone(),
                            );
                        }
                        Err(error) if is_temporary_accept_error(&error) => {
                            retry_delay = next_retry_delay(retry_delay);
                            eprintln!("metrics server temporary accept error: {error}; retrying in {retry_delay:?}");
                            sleep(retry_delay).await;
                        }
                        Err(source) => {
                            connections.detach_all();
                            return Err(ServerError::Accept(source));
                        }
                    }
                }
            }
        }

        drop(listener);
        connection_cancellation.cancel();
        let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
        while !connections.is_empty() {
            match timeout_at(deadline, connections.join_next()).await {
                Ok(Some(result)) => report_supervisor_result(result),
                Ok(None) => break,
                Err(_) => {
                    connections.detach_all();
                    return Err(ServerError::ShutdownTimeout);
                }
            }
        }
        Ok(())
    }
}

/// Fatal server startup, accept-loop, or shutdown failures.
#[derive(Debug)]
pub enum ServerError {
    Bind {
        address: SocketAddr,
        source: io::Error,
    },
    Accept(io::Error),
    ShutdownTimeout,
}

impl fmt::Display for ServerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bind { address, source } => {
                write!(formatter, "listen metrics server on {address}: {source}")
            }
            Self::Accept(source) => write!(formatter, "metrics server failed: {source}"),
            Self::ShutdownTimeout => {
                formatter.write_str("shut down metrics server: deadline exceeded")
            }
        }
    }
}

impl std::error::Error for ServerError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Bind { source, .. } | Self::Accept(source) => Some(source),
            Self::ShutdownTimeout => None,
        }
    }
}

fn is_temporary_accept_error(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted
    )
}

fn next_retry_delay(previous: Duration) -> Duration {
    if previous.is_zero() {
        Duration::from_millis(5)
    } else {
        (previous * 2).min(Duration::from_secs(1))
    }
}

fn spawn_connection(
    supervisors: &mut JoinSet<()>,
    stream: TcpStream,
    cancellation: CancellationToken,
) {
    supervisors.spawn(async move {
        let inner = tokio::spawn(run_connection(stream, cancellation));
        match inner.await {
            Ok(Ok(())) => {}
            Ok(Err(error)) => eprintln!("metrics connection failed: {error}"),
            Err(error) => eprintln!("metrics connection task failed: {error}"),
        }
    });
}

async fn run_connection(
    stream: TcpStream,
    cancellation: CancellationToken,
) -> Result<(), hyper::Error> {
    let service = service_fn(handle_request);
    let mut builder = http1::Builder::new();
    builder
        .timer(TokioTimer::new())
        .header_read_timeout(READ_TIMEOUT)
        .max_headers(MAX_REQUEST_HEADERS)
        .max_buf_size(1_052_672);
    let connection = builder.serve_connection(TokioIo::new(stream), service);
    tokio::pin!(connection);
    tokio::select! {
        result = &mut connection => result,
        _ = cancellation.cancelled() => {
            connection.as_mut().graceful_shutdown();
            connection.await
        }
    }
}

fn report_supervisor_result(result: Result<(), JoinError>) {
    if let Err(error) = result {
        eprintln!("metrics connection supervisor failed: {error}");
    }
}

struct HandlerMetrics {
    requests: CreatedIntCounterVec,
    in_flight: Gauge,
}

impl HandlerMetrics {
    fn register() -> Self {
        let registry = ployz_internal_metrics::registry();
        let requests = CreatedIntCounterVec::new(
            Opts::new(
                "promhttp_metric_handler_requests_total",
                "Total number of scrapes by HTTP status code.",
            ),
            &["code"],
        )
        .expect("create metrics handler request counter");
        for code in ["200", "500", "503"] {
            requests.with_label_values(&[code]);
        }
        let in_flight = Gauge::new(
            "promhttp_metric_handler_requests_in_flight",
            "Current number of scrapes being served.",
        )
        .expect("create metrics handler in-flight gauge");
        registry
            .register(Box::new(requests.clone()))
            .unwrap_or_else(|error| panic!("register metrics handler request counter: {error}"));
        registry
            .register(Box::new(in_flight.clone()))
            .unwrap_or_else(|error| panic!("register metrics handler in-flight gauge: {error}"));
        Self {
            requests,
            in_flight,
        }
    }
}

static HANDLER_METRICS: LazyLock<HandlerMetrics> = LazyLock::new(HandlerMetrics::register);

type ResponseBody = BoxBody<Bytes, Infallible>;

async fn handle_request<B>(request: Request<B>) -> Result<Response<ResponseBody>, Infallible>
where
    B: Body<Data = Bytes> + Send + Unpin + 'static,
{
    let route = routing::route(&request);
    let drain_policy = if matches!(&route, Route::OptionsStar) {
        DrainPolicy::OptionsStar
    } else {
        DrainPolicy::IgnoredBody
    };
    let response = match route {
        Route::OptionsStar => response(StatusCode::OK, Bytes::new(), true),
        Route::Redirect(location) => {
            let is_get = request.method() == Method::GET;
            let is_head = request.method() == Method::HEAD;
            let payload = if is_get {
                Bytes::from(format!(
                    "<a href=\"{}\">Temporary Redirect</a>.\n\n",
                    html_escape(&location)
                ))
            } else {
                Bytes::new()
            };
            let mut response = response(StatusCode::TEMPORARY_REDIRECT, payload, true);
            response.headers_mut().insert(
                LOCATION,
                location
                    .parse()
                    .expect("cleaned request path is a valid Location"),
            );
            if is_get || is_head {
                response
                    .headers_mut()
                    .insert(CONTENT_TYPE, "text/html; charset=utf-8".parse().unwrap());
            }
            response
        }
        Route::NotFound => {
            let mut response = response(
                StatusCode::NOT_FOUND,
                Bytes::from_static(b"404 page not found\n"),
                true,
            );
            response
                .headers_mut()
                .insert(CONTENT_TYPE, "text/plain; charset=utf-8".parse().unwrap());
            response
                .headers_mut()
                .insert("x-content-type-options", "nosniff".parse().unwrap());
            response
        }
        Route::Metrics => serve_metrics(&request).await,
    };
    let reusable = drain_ignored_body(request.into_body(), drain_policy).await;
    Ok(if reusable {
        response
    } else {
        close_response(response)
    })
}

#[derive(Clone, Copy)]
enum DrainPolicy {
    IgnoredBody,
    OptionsStar,
}

impl DrainPolicy {
    const fn limit(self) -> u64 {
        match self {
            Self::IgnoredBody => MAX_POST_HANDLER_READ_BYTES,
            Self::OptionsStar => MAX_OPTIONS_BODY_BYTES,
        }
    }

    const fn rejects_exact_limit(self) -> bool {
        matches!(self, Self::IgnoredBody)
    }
}

async fn drain_ignored_body<B>(mut body: B, policy: DrainPolicy) -> bool
where
    B: Body<Data = Bytes> + Unpin,
{
    let limit = policy.limit();
    if let Some(length) = body.size_hint().exact()
        && (length > limit || (length == limit && policy.rejects_exact_limit()))
    {
        return false;
    }

    let mut drained = 0_u64;
    while let Some(frame) = body.frame().await {
        let Ok(frame) = frame else {
            return false;
        };
        if let Some(data) = frame.data_ref() {
            drained = drained.saturating_add(data.len() as u64);
            if drained > limit {
                return false;
            }
        }
    }
    true
}

fn close_response(mut response: Response<ResponseBody>) -> Response<ResponseBody> {
    response
        .headers_mut()
        .insert(CONNECTION, "close".parse().unwrap());
    response
}

async fn serve_metrics<B>(request: &Request<B>) -> Response<ResponseBody> {
    let accept = request
        .headers()
        .get(ACCEPT)
        .and_then(|value| value.to_str().ok())
        .unwrap_or("")
        .to_owned();
    let accept_encodings = request
        .headers()
        .get_all(ACCEPT_ENCODING)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .map(str::to_owned)
        .collect::<Vec<_>>();
    let is_head = request.method() == Method::HEAD;
    let render_delay = test_render_delay(request);
    let reports_negotiation_start = test_reports_negotiation_start(request);
    let instrumentation = ScrapeInstrumentation::new();
    let rendered = run_blocking(move || {
        apply_test_render_delay(render_delay);
        #[cfg(test)]
        if reports_negotiation_start {
            TEST_NEGOTIATION_STARTED.notify_one();
        }
        #[cfg(not(test))]
        let _ = reports_negotiation_start;
        let (format, escaping) = encoding::negotiate_format(&accept);
        let content_type = format.content_type(escaping);
        let use_gzip = encoding::accepts_gzip(
            &accept_encodings
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
        );
        let families = ployz_internal_metrics::registry().gather();
        let result = encoding::encode(&families, format, escaping).and_then(|payload| {
            if use_gzip {
                encoding::gzip(&payload).map_err(encoding::EncodeError::Compression)
            } else {
                Ok(payload)
            }
        });
        (result, instrumentation, content_type, use_gzip)
    })
    .await;

    match rendered {
        Ok((Ok(payload), instrumentation, content_type, use_gzip)) => {
            let length = payload.len();
            let mut response = response(
                StatusCode::OK,
                if is_head {
                    Bytes::new()
                } else {
                    Bytes::from(payload)
                },
                length <= SMALL_RESPONSE_LIMIT,
            );
            if is_head && length <= SMALL_RESPONSE_LIMIT {
                response.headers_mut().insert(
                    hyper::header::CONTENT_LENGTH,
                    length.to_string().parse().unwrap(),
                );
            }
            response
                .headers_mut()
                .insert(CONTENT_TYPE, content_type.parse().unwrap());
            if use_gzip {
                response
                    .headers_mut()
                    .insert(CONTENT_ENCODING, "gzip".parse().unwrap());
            }
            instrument_response(response, instrumentation, "200")
        }
        Ok((Err(error), instrumentation, _, _)) => {
            internal_error(error.to_string(), Some(instrumentation))
        }
        Err(error) => internal_error(format!("metrics rendering task failed: {error}"), None),
    }
}

#[cfg(test)]
static TEST_RENDER_STARTED: LazyLock<Notify> = LazyLock::new(Notify::new);

#[cfg(test)]
static TEST_NEGOTIATION_STARTED: LazyLock<Notify> = LazyLock::new(Notify::new);

#[cfg(test)]
fn test_render_delay<B>(request: &Request<B>) -> Duration {
    request
        .headers()
        .get("x-ployz-test-render-delay")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse().ok())
        .map_or(Duration::ZERO, Duration::from_millis)
}

#[cfg(test)]
fn test_reports_negotiation_start<B>(request: &Request<B>) -> bool {
    request
        .headers()
        .contains_key("x-ployz-test-negotiation-start")
}

#[cfg(not(test))]
fn test_reports_negotiation_start<B>(_request: &Request<B>) -> bool {
    false
}

#[cfg(not(test))]
fn test_render_delay<B>(_request: &Request<B>) -> Duration {
    Duration::ZERO
}

#[cfg(test)]
fn apply_test_render_delay(delay: Duration) {
    if !delay.is_zero() {
        TEST_RENDER_STARTED.notify_one();
        std::thread::sleep(delay);
    }
}

#[cfg(not(test))]
fn apply_test_render_delay(_delay: Duration) {}

struct ScrapeInstrumentation {
    in_flight: Gauge,
    requests: CreatedIntCounterVec,
    status: Option<&'static str>,
}

impl ScrapeInstrumentation {
    fn new() -> Self {
        let in_flight = HANDLER_METRICS.in_flight.clone();
        in_flight.inc();
        Self {
            in_flight,
            requests: HANDLER_METRICS.requests.clone(),
            status: None,
        }
    }
}

impl Drop for ScrapeInstrumentation {
    fn drop(&mut self) {
        self.in_flight.dec();
        if let Some(status) = self.status {
            self.requests.with_label_values(&[status]).inc();
        }
    }
}

fn instrument_response(
    response: Response<ResponseBody>,
    mut instrumentation: ScrapeInstrumentation,
    status: &'static str,
) -> Response<ResponseBody> {
    instrumentation.status = Some(status);
    let (parts, body) = response.into_parts();
    Response::from_parts(
        parts,
        CompletionBody {
            inner: body,
            instrumentation: Some(instrumentation),
        }
        .boxed(),
    )
}

fn internal_error(
    error: String,
    instrumentation: Option<ScrapeInstrumentation>,
) -> Response<ResponseBody> {
    eprintln!("error encoding and sending metric family: {error}");
    let mut response = response(
        StatusCode::INTERNAL_SERVER_ERROR,
        Bytes::from(format!(
            "An error has occurred while serving metrics:\n\n{error}\n"
        )),
        true,
    );
    response
        .headers_mut()
        .insert(CONTENT_TYPE, "text/plain; charset=utf-8".parse().unwrap());
    response
        .headers_mut()
        .insert("x-content-type-options", "nosniff".parse().unwrap());
    match instrumentation {
        Some(instrumentation) => instrument_response(response, instrumentation, "500"),
        None => response,
    }
}

struct CompletionBody<B> {
    inner: B,
    instrumentation: Option<ScrapeInstrumentation>,
}

impl<B> Body for CompletionBody<B>
where
    B: Body<Data = Bytes, Error = Infallible> + Unpin,
{
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let this = self.get_mut();
        let polled = Pin::new(&mut this.inner).poll_frame(context);
        if matches!(polled, Poll::Ready(None)) {
            this.instrumentation.take();
        }
        polled
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> SizeHint {
        self.inner.size_hint()
    }
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&#34;")
        .replace('\'', "&#39;")
}

async fn run_blocking<F, T>(work: F) -> Result<T, JoinError>
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    tokio::task::spawn_blocking(work).await
}

fn response(status: StatusCode, payload: Bytes, exact_size: bool) -> Response<ResponseBody> {
    let body = if exact_size {
        Full::new(payload).boxed()
    } else {
        UnknownSizeBody(Some(payload)).boxed()
    };
    Response::builder().status(status).body(body).unwrap()
}

struct UnknownSizeBody(Option<Bytes>);

impl Body for UnknownSizeBody {
    type Data = Bytes;
    type Error = Infallible;

    fn poll_frame(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        Poll::Ready(self.get_mut().0.take().map(|data| Ok(Frame::data(data))))
    }

    fn is_end_stream(&self) -> bool {
        self.0.is_none()
    }

    fn size_hint(&self) -> SizeHint {
        SizeHint::default()
    }
}

#[cfg(test)]
mod tests {
    use std::future::Future as _;
    use std::net::{Ipv4Addr, TcpListener as StdTcpListener};
    use std::time::Instant as StdInstant;

    use http_body_util::Empty;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use tokio::sync::oneshot;

    use super::*;

    static HANDLER_TEST_LOCK: LazyLock<tokio::sync::Mutex<()>> =
        LazyLock::new(|| tokio::sync::Mutex::new(()));

    async fn raw_request(port: u16, request: &[u8]) -> Vec<u8> {
        let mut stream = loop {
            match TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await {
                Ok(stream) => break stream,
                Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {
                    tokio::task::yield_now().await;
                }
                Err(error) => panic!("connect to metrics server: {error}"),
            }
        };
        stream.write_all(request).await.unwrap();
        let mut response = Vec::new();
        stream.read_to_end(&mut response).await.unwrap();
        response
    }

    fn empty_request(method: Method, uri: &str) -> Request<Empty<Bytes>> {
        Request::builder()
            .method(method)
            .uri(uri)
            .body(Empty::new())
            .unwrap()
    }

    fn family_value(name: &str, label: Option<(&str, &str)>) -> f64 {
        LazyLock::force(&HANDLER_METRICS);
        let families = ployz_internal_metrics::registry().gather();
        let family = families
            .iter()
            .find(|family| family.name() == name)
            .unwrap();
        let metric = family
            .metric
            .iter()
            .find(|metric| {
                label.is_none_or(|(label_name, label_value)| {
                    metric
                        .label
                        .iter()
                        .any(|label| label.name() == label_name && label.value() == label_value)
                })
            })
            .unwrap();
        metric.gauge.as_ref().map_or_else(
            || metric.counter.as_ref().unwrap().value(),
            |gauge| gauge.value(),
        )
    }

    struct GatedBody {
        release: Option<oneshot::Receiver<()>>,
        payload: Option<Bytes>,
    }

    impl Body for GatedBody {
        type Data = Bytes;
        type Error = Infallible;

        fn poll_frame(
            self: Pin<&mut Self>,
            context: &mut Context<'_>,
        ) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
            let this = self.get_mut();
            let Some(release) = this.release.as_mut() else {
                return Poll::Ready(None);
            };
            match Pin::new(release).poll(context) {
                Poll::Ready(_) => {
                    this.release = None;
                    Poll::Ready(this.payload.take().map(|data| Ok(Frame::data(data))))
                }
                Poll::Pending => Poll::Pending,
            }
        }
    }

    #[test]
    fn retry_backoff_starts_at_five_milliseconds_and_caps_at_one_second() {
        let expected = [5, 10, 20, 40, 80, 160, 320, 640, 1_000, 1_000];
        let mut delay = Duration::ZERO;
        for milliseconds in expected {
            delay = next_retry_delay(delay);
            assert_eq!(delay, Duration::from_millis(milliseconds));
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn options_and_handler_drains_keep_their_distinct_exact_boundaries() {
        assert!(
            drain_ignored_body(
                Full::new(Bytes::from(vec![0; MAX_OPTIONS_BODY_BYTES as usize])),
                DrainPolicy::OptionsStar,
            )
            .await
        );
        assert!(
            !drain_ignored_body(
                Full::new(Bytes::from(vec![0; MAX_OPTIONS_BODY_BYTES as usize + 1])),
                DrainPolicy::OptionsStar,
            )
            .await
        );
        assert!(
            drain_ignored_body(
                Full::new(Bytes::from(vec![
                    0;
                    MAX_POST_HANDLER_READ_BYTES as usize - 1
                ])),
                DrainPolicy::IgnoredBody,
            )
            .await
        );
        assert!(
            !drain_ignored_body(
                Full::new(Bytes::from(vec![0; MAX_POST_HANDLER_READ_BYTES as usize])),
                DrainPolicy::IgnoredBody,
            )
            .await
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn blocking_metrics_work_does_not_stall_the_runtime() {
        let _serial = HANDLER_TEST_LOCK.lock().await;
        let slow_request = Request::builder()
            .method(Method::GET)
            .uri("/metrics")
            .header("x-ployz-test-render-delay", "250")
            .body(Empty::<Bytes>::new())
            .unwrap();
        let slow = tokio::spawn(handle_request(slow_request));
        TEST_RENDER_STARTED.notified().await;
        let before = StdInstant::now();
        let fast = handle_request(empty_request(Method::GET, "/metrics"))
            .await
            .unwrap();
        assert!(before.elapsed() < Duration::from_millis(75));
        assert!(
            !fast
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .is_empty()
        );
        let slow = slow.await.unwrap().unwrap();
        assert!(
            !slow
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .is_empty()
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn adversarial_accept_negotiation_does_not_stall_the_runtime() {
        let _serial = HANDLER_TEST_LOCK.lock().await;
        let accept = std::iter::repeat_n("*/*,text/plain; escaping=dots", 34_000)
            .collect::<Vec<_>>()
            .join(",");
        assert!(accept.len() > 1_000_000);
        let request = Request::builder()
            .uri("/metrics")
            .header(ACCEPT, accept)
            .header("x-ployz-test-negotiation-start", "1")
            .body(Empty::<Bytes>::new())
            .unwrap();
        let adversarial = tokio::spawn(handle_request(request));
        TEST_NEGOTIATION_STARTED.notified().await;

        let before = StdInstant::now();
        let fast = handle_request(empty_request(Method::GET, "/metrics"))
            .await
            .unwrap();
        assert!(before.elapsed() < Duration::from_millis(75));
        assert!(
            !fast
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .is_empty()
        );
        assert!(
            !adversarial
                .await
                .unwrap()
                .unwrap()
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .is_empty()
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn handler_matches_redirect_head_and_format_negotiation() {
        let _serial = HANDLER_TEST_LOCK.lock().await;
        let redirect = handle_request(
            Request::builder()
                .uri("/foo/../metrics?x=1")
                .body(Empty::<Bytes>::new())
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(redirect.status(), StatusCode::TEMPORARY_REDIRECT);
        assert_eq!(redirect.headers()[LOCATION], "/metrics?x=1");
        assert_eq!(redirect.headers()[CONTENT_TYPE], "text/html; charset=utf-8");
        let body = redirect.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            body,
            Bytes::from_static(b"<a href=\"/metrics?x=1\">Temporary Redirect</a>.\n\n")
        );

        let head = handle_request(
            Request::builder()
                .method(Method::HEAD)
                .uri("/metrics")
                .header(
                    ACCEPT,
                    concat!(
                        "application/vnd.google.protobuf; ",
                        "proto=io.prometheus.client.MetricFamily; encoding=delimited"
                    ),
                )
                .body(Empty::<Bytes>::new())
                .unwrap(),
        )
        .await
        .unwrap();
        let in_flight_before_completion =
            family_value("promhttp_metric_handler_requests_in_flight", None);
        let completed_before = family_value(
            "promhttp_metric_handler_requests_total",
            Some(("code", "200")),
        );
        assert_eq!(head.status(), StatusCode::OK);
        assert_eq!(
            head.headers()[CONTENT_TYPE],
            concat!(
                "application/vnd.google.protobuf; ",
                "proto=io.prometheus.client.MetricFamily; encoding=delimited; ",
                "escaping=underscores"
            )
        );
        assert!(
            head.into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .is_empty()
        );
        assert_eq!(
            family_value("promhttp_metric_handler_requests_in_flight", None),
            in_flight_before_completion - 1.0
        );
        assert_eq!(
            family_value(
                "promhttp_metric_handler_requests_total",
                Some(("code", "200"))
            ),
            completed_before + 1.0
        );

        let repeated_accept = handle_request(
            Request::builder()
                .uri("/metrics")
                .header(ACCEPT, "text/plain; version=0.0.4; q=0.1")
                .header(
                    ACCEPT,
                    concat!(
                        "application/vnd.google.protobuf; ",
                        "proto=io.prometheus.client.MetricFamily; encoding=delimited; q=1"
                    ),
                )
                .body(Empty::<Bytes>::new())
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(
            repeated_accept.headers()[CONTENT_TYPE],
            "text/plain; version=0.0.4; charset=utf-8; escaping=underscores"
        );
        assert!(
            !repeated_accept
                .into_body()
                .collect()
                .await
                .unwrap()
                .to_bytes()
                .is_empty()
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn backpressured_body_keeps_scrape_in_flight_until_completion() {
        let _serial = HANDLER_TEST_LOCK.lock().await;
        let in_flight_before = family_value("promhttp_metric_handler_requests_in_flight", None);
        let completed_before = family_value(
            "promhttp_metric_handler_requests_total",
            Some(("code", "200")),
        );
        let instrumentation = ScrapeInstrumentation::new();
        let (release, wait) = oneshot::channel();
        let response = Response::new(
            GatedBody {
                release: Some(wait),
                payload: Some(Bytes::from_static(b"payload")),
            }
            .boxed(),
        );
        let response = instrument_response(response, instrumentation, "200");
        let collecting = tokio::spawn(response.into_body().collect());
        tokio::task::yield_now().await;

        assert_eq!(
            family_value("promhttp_metric_handler_requests_in_flight", None),
            in_flight_before + 1.0
        );
        assert_eq!(
            family_value(
                "promhttp_metric_handler_requests_total",
                Some(("code", "200"))
            ),
            completed_before
        );

        release.send(()).unwrap();
        assert_eq!(
            collecting.await.unwrap().unwrap().to_bytes(),
            Bytes::from_static(b"payload")
        );
        assert_eq!(
            family_value("promhttp_metric_handler_requests_in_flight", None),
            in_flight_before
        );
        assert_eq!(
            family_value(
                "promhttp_metric_handler_requests_total",
                Some(("code", "200"))
            ),
            completed_before + 1.0
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn pre_cancelled_run_still_reports_an_occupied_port() {
        let occupied = StdTcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let port = occupied.local_addr().unwrap().port();
        let server = Server::with_port(Ipv4Addr::LOCALHOST.into(), port);
        let cancellation = CancellationToken::new();
        cancellation.cancel();

        let error = server.run(&cancellation).await.unwrap_err();
        assert!(matches!(error, ServerError::Bind { .. }));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn serves_metrics_and_stops_accepting_after_cancellation() {
        let _serial = HANDLER_TEST_LOCK.lock().await;
        let reservation = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = reservation.local_addr().unwrap().port();
        drop(reservation);
        let server = Server::with_port(Ipv4Addr::LOCALHOST.into(), port);
        let cancellation = CancellationToken::new();
        let run_cancellation = cancellation.clone();
        let task = tokio::spawn(async move { server.run(&run_cancellation).await });

        let response = raw_request(
            port,
            b"GET /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;
        let response = String::from_utf8(response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(
            response.contains("text/plain; version=0.0.4; charset=utf-8; escaping=underscores")
        );
        assert!(response.contains("promhttp_metric_handler_requests_total"));

        for request in [
            &b"POST /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\nContent-Length: 0\r\n\r\n"[..],
            &b"CONNECT /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"[..],
            &b"GET /m%65trics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"[..],
            &b"OPTIONS * HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n"[..],
        ] {
            let response = raw_request(port, request).await;
            assert!(response.starts_with(b"HTTP/1.1 200 OK\r\n"));
        }

        let response = raw_request(
            port,
            b"HEAD /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;
        let body = response
            .windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|index| &response[index + 4..])
            .unwrap();
        assert!(response.starts_with(b"HTTP/1.1 200 OK\r\n"));
        assert!(body.is_empty());

        let response = raw_request(
            port,
            b"GET /metrics/ HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(response.starts_with(b"HTTP/1.1 404 Not Found\r\n"));

        let response = raw_request(
            port,
            b"GET /metrics/. HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
        )
        .await;
        assert!(response.starts_with(b"HTTP/1.1 307 Temporary Redirect\r\n"));
        assert!(
            response
                .windows(20)
                .any(|window| window == b"location: /metrics\r\n")
        );

        cancellation.cancel();
        task.await.unwrap().unwrap();
        assert!(
            TcpStream::connect((Ipv4Addr::LOCALHOST, port))
                .await
                .is_err()
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn partial_head_concurrency_keeps_parser_memory_and_service_bounded() {
        let _serial = HANDLER_TEST_LOCK.lock().await;
        let reservation = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = reservation.local_addr().unwrap().port();
        drop(reservation);
        let server = Server::with_port(Ipv4Addr::LOCALHOST.into(), port);
        let cancellation = CancellationToken::new();
        let run_cancellation = cancellation.clone();
        let task = tokio::spawn(async move { server.run(&run_cancellation).await });

        let mut partial_heads = Vec::new();
        for _ in 0..64 {
            let mut stream = loop {
                match TcpStream::connect((Ipv4Addr::LOCALHOST, port)).await {
                    Ok(stream) => break stream,
                    Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {
                        tokio::task::yield_now().await;
                    }
                    Err(error) => panic!("connect partial request: {error}"),
                }
            };
            stream
                .write_all(b"GET /metrics HTTP/1.1\r\nX-Partial: ")
                .await
                .unwrap();
            partial_heads.push(stream);
        }

        let response = tokio::time::timeout(
            Duration::from_secs(1),
            raw_request(
                port,
                b"GET /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            ),
        )
        .await
        .expect("partial heads starved a complete request");
        assert!(response.starts_with(b"HTTP/1.1 200 OK\r\n"));

        drop(partial_heads);
        cancellation.cancel();
        task.await.unwrap().unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn ignored_body_drain_matches_known_and_chunked_reuse_boundaries() {
        let _serial = HANDLER_TEST_LOCK.lock().await;
        let reservation = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let port = reservation.local_addr().unwrap().port();
        drop(reservation);
        let server = Server::with_port(Ipv4Addr::LOCALHOST.into(), port);
        let cancellation = CancellationToken::new();
        let run_cancellation = cancellation.clone();
        let task = tokio::spawn(async move { server.run(&run_cancellation).await });

        for (chunked, length, expected_responses) in [
            (false, 262_143, 2),
            (false, 262_144, 1),
            (false, 262_145, 1),
            (true, 262_143, 2),
            (true, 262_144, 2),
            (true, 262_145, 1),
        ] {
            let mut request = if chunked {
                format!(
                    "POST /metrics HTTP/1.1\r\nHost: localhost\r\nTransfer-Encoding: chunked\r\n\r\n{length:x}\r\n"
                )
                .into_bytes()
            } else {
                format!(
                    "POST /metrics HTTP/1.1\r\nHost: localhost\r\nContent-Length: {length}\r\n\r\n"
                )
                .into_bytes()
            };
            request.resize(request.len() + length, b'x');
            if chunked {
                request.extend_from_slice(b"\r\n0\r\n\r\n");
            }
            request.extend_from_slice(
                b"GET /metrics HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            );

            let response = raw_request(port, &request).await;
            assert_eq!(
                response
                    .windows(b"HTTP/1.1 200 OK".len())
                    .filter(|window| *window == b"HTTP/1.1 200 OK")
                    .count(),
                expected_responses,
                "chunked={chunked} length={length}"
            );
        }

        cancellation.cancel();
        task.await.unwrap().unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn two_scrape_requests_render_concurrently() {
        let _serial = HANDLER_TEST_LOCK.lock().await;
        let first = handle_request(empty_request(Method::GET, "/metrics"));
        let second = handle_request(empty_request(Method::GET, "/metrics"));
        let (first, second) = tokio::join!(first, second);
        let first = first
            .unwrap()
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes();
        let second = second
            .unwrap()
            .into_body()
            .collect()
            .await
            .unwrap()
            .to_bytes();
        assert!(!first.is_empty());
        assert!(!second.is_empty());
    }
}
