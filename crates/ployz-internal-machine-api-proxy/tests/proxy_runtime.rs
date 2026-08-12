use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use bytes::Bytes;
use http::uri::PathAndQuery;
use ployz_internal_machine_api_pb::{Empty, EmptyResponse};
use ployz_internal_machine_api_proxy::{
    Director, MachineMapper, MachineTarget, MapMachinesError, ProxyService, RawCodec,
};
use prost::Message;
use tokio::sync::mpsc;
use tokio_stream::wrappers::{ReceiverStream, TcpListenerStream};
use tonic::body::Body;
use tonic::codec::Streaming;
use tonic::metadata::{BinaryMetadataValue, MetadataMap};
use tonic::transport::{Channel, Endpoint, Server};
use tonic::{Code, Request, Response, Status};
use tower::Service;

type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send + 'static>>;
type RawStream = ReceiverStream<Result<Bytes, Status>>;

#[derive(Clone, Debug)]
struct StaticMapper {
    targets: Vec<MachineTarget>,
}

#[derive(Clone, Debug)]
struct HangingMapper;

impl MachineMapper for HangingMapper {
    fn map_machines<'a>(
        &'a self,
        _names_or_ids: &'a [String],
    ) -> Pin<Box<dyn Future<Output = Result<Vec<MachineTarget>, MapMachinesError>> + Send + 'a>>
    {
        Box::pin(std::future::pending())
    }
}

impl MachineMapper for StaticMapper {
    fn map_machines<'a>(
        &'a self,
        _names_or_ids: &'a [String],
    ) -> Pin<Box<dyn Future<Output = Result<Vec<MachineTarget>, MapMachinesError>> + Send + 'a>>
    {
        let targets = self.targets.clone();
        Box::pin(async move { Ok(targets) })
    }
}

#[derive(Clone, Debug)]
enum UpstreamBehavior {
    Echo,
    RespondOnce,
    AlternateFailure,
    UnavailableOnce,
    MalformedResponse,
    CancellationProbe(Arc<tokio::sync::Notify>),
    ErrorBeforeMessage,
    ErrorAfterMessage,
}

#[derive(Clone, Debug)]
struct RawUpstream {
    behavior: UpstreamBehavior,
    calls: Arc<AtomicUsize>,
    paths: Arc<Mutex<Vec<String>>>,
    requests: Arc<Mutex<Vec<MetadataMap>>>,
}

impl RawUpstream {
    fn echo() -> Self {
        Self {
            behavior: UpstreamBehavior::Echo,
            calls: Arc::new(AtomicUsize::new(0)),
            paths: Arc::new(Mutex::new(Vec::new())),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn alternate_failure() -> Self {
        Self {
            behavior: UpstreamBehavior::AlternateFailure,
            calls: Arc::new(AtomicUsize::new(0)),
            paths: Arc::new(Mutex::new(Vec::new())),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn respond_once() -> Self {
        Self {
            behavior: UpstreamBehavior::RespondOnce,
            calls: Arc::new(AtomicUsize::new(0)),
            paths: Arc::new(Mutex::new(Vec::new())),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn unavailable_once() -> Self {
        Self {
            behavior: UpstreamBehavior::UnavailableOnce,
            calls: Arc::new(AtomicUsize::new(0)),
            paths: Arc::new(Mutex::new(Vec::new())),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn malformed_response() -> Self {
        Self {
            behavior: UpstreamBehavior::MalformedResponse,
            calls: Arc::new(AtomicUsize::new(0)),
            paths: Arc::new(Mutex::new(Vec::new())),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn cancellation_probe(notify: Arc<tokio::sync::Notify>) -> Self {
        Self {
            behavior: UpstreamBehavior::CancellationProbe(notify),
            calls: Arc::new(AtomicUsize::new(0)),
            paths: Arc::new(Mutex::new(Vec::new())),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn error_before_message() -> Self {
        Self {
            behavior: UpstreamBehavior::ErrorBeforeMessage,
            calls: Arc::new(AtomicUsize::new(0)),
            paths: Arc::new(Mutex::new(Vec::new())),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn error_after_message() -> Self {
        Self {
            behavior: UpstreamBehavior::ErrorAfterMessage,
            calls: Arc::new(AtomicUsize::new(0)),
            paths: Arc::new(Mutex::new(Vec::new())),
            requests: Arc::new(Mutex::new(Vec::new())),
        }
    }
}

impl Service<http::Request<Body>> for RawUpstream {
    type Response = http::Response<Body>;
    type Error = Infallible;
    type Future = BoxFuture<Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: http::Request<Body>) -> Self::Future {
        self.paths
            .lock()
            .unwrap()
            .push(request.uri().path().to_owned());
        let call_number = self.calls.fetch_add(1, Ordering::SeqCst);
        let call = RawUpstreamCall {
            fail: matches!(self.behavior, UpstreamBehavior::AlternateFailure)
                && call_number % 2 == 1
                || matches!(self.behavior, UpstreamBehavior::UnavailableOnce) && call_number == 0,
            unavailable: matches!(self.behavior, UpstreamBehavior::UnavailableOnce),
            malformed_response: matches!(self.behavior, UpstreamBehavior::MalformedResponse),
            cancellation: match &self.behavior {
                UpstreamBehavior::CancellationProbe(notify) => Some(Arc::clone(notify)),
                UpstreamBehavior::Echo
                | UpstreamBehavior::RespondOnce
                | UpstreamBehavior::AlternateFailure
                | UpstreamBehavior::UnavailableOnce
                | UpstreamBehavior::MalformedResponse
                | UpstreamBehavior::ErrorBeforeMessage
                | UpstreamBehavior::ErrorAfterMessage => None,
            },
            error_before_message: matches!(self.behavior, UpstreamBehavior::ErrorBeforeMessage),
            error_after_message: matches!(self.behavior, UpstreamBehavior::ErrorAfterMessage),
            respond_once: matches!(self.behavior, UpstreamBehavior::RespondOnce),
            requests: Arc::clone(&self.requests),
        };
        Box::pin(async move {
            Ok(tonic::server::Grpc::new(RawCodec)
                .streaming(call, request)
                .await)
        })
    }
}

#[derive(Clone, Debug)]
struct RawUpstreamCall {
    fail: bool,
    unavailable: bool,
    malformed_response: bool,
    cancellation: Option<Arc<tokio::sync::Notify>>,
    error_before_message: bool,
    error_after_message: bool,
    respond_once: bool,
    requests: Arc<Mutex<Vec<MetadataMap>>>,
}

impl Service<Request<Streaming<Bytes>>> for RawUpstreamCall {
    type Response = Response<RawStream>;
    type Error = Status;
    type Future = BoxFuture<Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Request<Streaming<Bytes>>) -> Self::Future {
        let fail = self.fail;
        let unavailable = self.unavailable;
        let malformed_response = self.malformed_response;
        let cancellation = self.cancellation.clone();
        let error_before_message = self.error_before_message;
        let error_after_message = self.error_after_message;
        let respond_once = self.respond_once;
        let requests = Arc::clone(&self.requests);
        Box::pin(async move {
            if fail {
                if unavailable {
                    return Err(Status::unavailable("application unavailable"));
                }
                return Err(Status::permission_denied("backend denied request"));
            }

            requests.lock().unwrap().push(request.metadata().clone());
            let mut incoming = request.into_inner();
            let (sender, receiver) = mpsc::channel(4);
            tokio::spawn(async move {
                if error_before_message {
                    let _ = sender.send(Err(terminal_status())).await;
                    return;
                }
                if malformed_response {
                    let _ = sender.send(Ok(Bytes::from_static(&[0x10, 0]))).await;
                    return;
                }
                if error_after_message {
                    if let Ok(Some(message)) = incoming.message().await
                        && sender.send(Ok(message)).await.is_err()
                    {
                        return;
                    }
                    let _ = sender.send(Err(terminal_status())).await;
                    return;
                }
                if let Some(notify) = cancellation {
                    if let Ok(Some(message)) = incoming.message().await {
                        if sender.send(Ok(message)).await.is_err() {
                            notify.notify_one();
                            return;
                        }
                        sender.closed().await;
                    }
                    notify.notify_one();
                    return;
                }
                if respond_once {
                    if let Ok(Some(message)) = incoming.message().await {
                        let _ = sender.send(Ok(message)).await;
                    }
                    return;
                }
                while let Ok(Some(message)) = incoming.message().await {
                    if sender.send(Ok(message)).await.is_err() {
                        return;
                    }
                }
                let mut trailers = MetadataMap::new();
                trailers.append("x-trailer", "one".parse().unwrap());
                trailers.append("x-trailer", "two".parse().unwrap());
                trailers.insert_bin("result-bin", BinaryMetadataValue::from_bytes(&[0, 255, 1]));
                let _ = sender
                    .send(Err(Status::with_metadata(Code::Ok, "", trailers)))
                    .await;
            });

            let mut response = Response::new(ReceiverStream::new(receiver));
            response
                .metadata_mut()
                .append("x-initial", "one".parse().unwrap());
            response
                .metadata_mut()
                .append("x-initial", "two".parse().unwrap());
            response
                .metadata_mut()
                .insert_bin("header-bin", BinaryMetadataValue::from_bytes(&[3, 2, 1]));
            Ok(response)
        })
    }
}

fn terminal_status() -> Status {
    let mut metadata = MetadataMap::new();
    metadata.append("x-terminal", "one".parse().unwrap());
    metadata.append("x-terminal", "two".parse().unwrap());
    metadata.insert_bin("terminal-bin", BinaryMetadataValue::from_bytes(&[9, 0, 9]));
    Status::with_details_and_metadata(
        Code::FailedPrecondition,
        "terminal failure",
        Bytes::from_static(&[0, 255, 4, 2]),
        metadata,
    )
}

async fn spawn_server<S>(service: S) -> (u16, tokio::task::JoinHandle<()>)
where
    S: Service<http::Request<Body>, Response = http::Response<Body>> + Clone + Send + 'static,
    S::Error: Into<Box<dyn std::error::Error + Send + Sync>> + Send,
    S::Future: Send,
{
    let listener = tokio::net::TcpListener::bind("[::1]:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let task = tokio::spawn(async move {
        Server::builder()
            .serve_with_incoming(service, TcpListenerStream::new(listener))
            .await
            .unwrap();
    });
    (port, task)
}

async fn spawn_server_on<S>(port: u16, service: S) -> tokio::task::JoinHandle<()>
where
    S: Service<http::Request<Body>, Response = http::Response<Body>> + Clone + Send + 'static,
    S::Error: Into<Box<dyn std::error::Error + Send + Sync>> + Send,
    S::Future: Send,
{
    let listener = tokio::net::TcpListener::bind(("::1", port)).await.unwrap();
    tokio::spawn(async move {
        Server::builder()
            .serve_with_incoming(service, TcpListenerStream::new(listener))
            .await
            .unwrap();
    })
}

#[cfg(unix)]
async fn spawn_unix_server<S>(path: &std::path::Path, service: S) -> tokio::task::JoinHandle<()>
where
    S: Service<http::Request<Body>, Response = http::Response<Body>> + Clone + Send + 'static,
    S::Error: Into<Box<dyn std::error::Error + Send + Sync>> + Send,
    S::Future: Send,
{
    let listener = tokio::net::UnixListener::bind(path).unwrap();
    tokio::spawn(async move {
        Server::builder()
            .serve_with_incoming(
                service,
                tokio_stream::wrappers::UnixListenerStream::new(listener),
            )
            .await
            .unwrap();
    })
}

async fn connect(port: u16) -> Channel {
    Endpoint::from_shared(format!("http://[::1]:{port}"))
        .unwrap()
        .connect()
        .await
        .unwrap()
}

async fn call_proxy(
    channel: Channel,
    metadata: MetadataMap,
    messages: Vec<Bytes>,
    timeout: Option<std::time::Duration>,
) -> Result<Response<Streaming<Bytes>>, Status> {
    let mut grpc = tonic::client::Grpc::new(channel);
    grpc.ready().await.unwrap();
    let mut request = Request::from_parts(
        metadata,
        http::Extensions::new(),
        tokio_stream::iter(messages),
    );
    if let Some(timeout) = timeout {
        request.set_timeout(timeout);
    }
    grpc.streaming(
        request,
        PathAndQuery::from_static("/unregistered.Service/Call"),
        RawCodec,
    )
    .await
}

#[tokio::test]
async fn one_to_one_preserves_unknown_path_messages_metadata_and_trailers() {
    let upstream = RawUpstream::echo();
    let paths = Arc::clone(&upstream.paths);
    let requests = Arc::clone(&upstream.requests);
    let (upstream_port, upstream_task) = spawn_server(upstream).await;
    let mapper = StaticMapper {
        targets: vec![MachineTarget::new("id-1", "remote", "::1")],
    };
    let proxy = ProxyService::new(Arc::new(Director::new(
        "/tmp/unused.sock",
        upstream_port,
        mapper,
    )));
    let (proxy_port, proxy_task) = spawn_server(proxy).await;

    let mut metadata = MetadataMap::new();
    metadata.insert("machine", "remote".parse().unwrap());
    metadata.append("x-repeat", "one".parse().unwrap());
    metadata.append("x-repeat", "two".parse().unwrap());
    metadata.insert_bin("request-bin", BinaryMetadataValue::from_bytes(&[0, 1, 255]));
    let messages = vec![
        Bytes::from_static(b"first"),
        Bytes::from(vec![7; 64 * 1024]),
    ];
    let response = call_proxy(connect(proxy_port).await, metadata, messages.clone(), None)
        .await
        .unwrap();
    assert_eq!(
        response
            .metadata()
            .get_all("x-initial")
            .iter()
            .map(|value| value.to_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["one", "two"]
    );
    assert_eq!(
        response
            .metadata()
            .get_bin("header-bin")
            .unwrap()
            .to_bytes()
            .unwrap(),
        &[3, 2, 1][..]
    );
    let mut stream = response.into_inner();
    assert_eq!(stream.message().await.unwrap(), Some(messages[0].clone()));
    assert_eq!(stream.message().await.unwrap(), Some(messages[1].clone()));
    assert_eq!(stream.message().await.unwrap(), None);
    let trailers = stream.trailers().await.unwrap().unwrap();
    assert_eq!(
        trailers
            .get_all("x-trailer")
            .iter()
            .map(|value| value.to_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["one", "two"]
    );
    assert_eq!(
        trailers.get_bin("result-bin").unwrap().to_bytes().unwrap(),
        &[0, 255, 1][..]
    );
    assert_eq!(
        paths.lock().unwrap().as_slice(),
        ["/unregistered.Service/Call"]
    );
    let captured = requests.lock().unwrap();
    let captured = &captured[0];
    assert_eq!(
        captured
            .get_all("x-repeat")
            .iter()
            .map(|value| value.to_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["one", "two"]
    );
    assert_eq!(
        captured.get_bin("request-bin").unwrap().to_bytes().unwrap(),
        &[0, 1, 255][..]
    );
    assert_eq!(
        captured.get("proxy-authority").unwrap().to_str().unwrap(),
        format!("[::1]:{proxy_port}")
    );
    assert!(captured.get("machine").is_none());

    proxy_task.abort();
    upstream_task.abort();
}

#[tokio::test]
async fn one_to_many_aggregates_success_and_error_payloads_with_machine_metadata() {
    let upstream = RawUpstream::alternate_failure();
    let (upstream_port, upstream_task) = spawn_server(upstream).await;
    let mapper = StaticMapper {
        targets: vec![
            MachineTarget::new("id-1", "machine-a", "::1"),
            MachineTarget::new("id-2", "machine-b", "::1"),
        ],
    };
    let proxy = ProxyService::new(Arc::new(Director::new(
        "/tmp/unused.sock",
        upstream_port,
        mapper,
    )));
    let (proxy_port, proxy_task) = spawn_server(proxy).await;

    let mut metadata = MetadataMap::new();
    metadata.append("machines", "machine-a".parse().unwrap());
    metadata.append("machines", "machine-b".parse().unwrap());
    let request = EmptyResponse {
        messages: vec![Empty::default()],
    }
    .encode_to_vec();
    let response = call_proxy(
        connect(proxy_port).await,
        metadata,
        vec![Bytes::from(request.clone()), Bytes::from(request)],
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        response
            .metadata()
            .get_all("x-initial")
            .iter()
            .map(|value| value.to_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["one", "two"]
    );
    let mut stream = response.into_inner();
    let merged = stream.message().await.unwrap().unwrap();
    assert_eq!(stream.message().await.unwrap(), None);
    let trailers = stream.trailers().await.unwrap().unwrap();
    assert_eq!(
        trailers
            .get_all("x-trailer")
            .iter()
            .map(|value| value.to_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["one", "two"]
    );
    let decoded = EmptyResponse::decode(merged).unwrap();
    assert_eq!(decoded.messages.len(), 3);
    let by_id = decoded
        .messages
        .into_iter()
        .map(|message| {
            let metadata = message.metadata.unwrap();
            (metadata.machine_id.clone(), metadata)
        })
        .collect::<Vec<_>>();
    let successful = by_id
        .iter()
        .filter(|(_, metadata)| metadata.error.is_empty())
        .count();
    assert_eq!(successful, 2);
    let failed = by_id
        .iter()
        .find(|(_, metadata)| !metadata.error.is_empty())
        .unwrap()
        .1
        .clone();
    assert_eq!(
        failed.error,
        "rpc error: code = PermissionDenied desc = backend denied request"
    );
    assert_eq!(failed.status.unwrap().code, Code::PermissionDenied as i32);

    proxy_task.abort();
    upstream_task.abort();
}

#[tokio::test]
async fn one_to_many_finishes_when_backends_finish_before_the_request_stream() {
    let (upstream_port, upstream_task) = spawn_server(RawUpstream::respond_once()).await;
    let proxy = ProxyService::new(Arc::new(Director::new(
        "/tmp/unused.sock",
        upstream_port,
        StaticMapper {
            targets: vec![
                MachineTarget::new("id-1", "machine-a", "::1"),
                MachineTarget::new("id-2", "machine-b", "::1"),
            ],
        },
    )));
    let (proxy_port, proxy_task) = spawn_server(proxy).await;

    let mut metadata = MetadataMap::new();
    metadata.insert("machines", "all".parse().unwrap());
    let payload = EmptyResponse {
        messages: vec![Empty::default()],
    }
    .encode_to_vec();
    let (request_tx, request_rx) = mpsc::channel(1);
    request_tx.send(Bytes::from(payload)).await.unwrap();
    let mut grpc = tonic::client::Grpc::new(connect(proxy_port).await);
    grpc.ready().await.unwrap();
    let request = Request::from_parts(
        metadata,
        http::Extensions::new(),
        ReceiverStream::new(request_rx),
    );
    let response = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        grpc.streaming(
            request,
            PathAndQuery::from_static("/unregistered.Service/Call"),
            RawCodec,
        ),
    )
    .await
    .expect("proxy waited for the still-open frontend request stream")
    .unwrap();
    let mut stream = response.into_inner();
    let merged = stream.message().await.unwrap().unwrap();
    assert_eq!(EmptyResponse::decode(merged).unwrap().messages.len(), 2);
    assert_eq!(stream.message().await.unwrap(), None);
    drop(request_tx);

    proxy_task.abort();
    upstream_task.abort();
}

#[cfg(unix)]
#[tokio::test]
async fn no_routing_metadata_uses_the_local_unix_backend() {
    let socket = std::env::temp_dir().join(format!(
        "ployz-machine-api-proxy-{}-{}.sock",
        std::process::id(),
        fastrand::u64(..)
    ));
    let upstream_task = spawn_unix_server(&socket, RawUpstream::echo()).await;
    let proxy = ProxyService::new(Arc::new(Director::new(
        socket.to_str().unwrap(),
        1,
        StaticMapper { targets: vec![] },
    )));
    let (proxy_port, proxy_task) = spawn_server(proxy).await;
    let response = call_proxy(
        connect(proxy_port).await,
        MetadataMap::new(),
        vec![Bytes::from_static(b"local")],
        None,
    )
    .await
    .unwrap();
    let mut stream = response.into_inner();
    assert_eq!(
        stream.message().await.unwrap(),
        Some(Bytes::from_static(b"local"))
    );

    proxy_task.abort();
    upstream_task.abort();
    std::fs::remove_file(socket).unwrap();
}

#[tokio::test]
async fn one_to_one_hides_initial_metadata_when_backend_fails_before_a_message() {
    let (upstream_port, upstream_task) = spawn_server(RawUpstream::error_before_message()).await;
    let proxy = ProxyService::new(Arc::new(Director::new(
        "/tmp/unused.sock",
        upstream_port,
        StaticMapper {
            targets: vec![MachineTarget::new("id-1", "remote", "::1")],
        },
    )));
    let (proxy_port, proxy_task) = spawn_server(proxy).await;
    let mut metadata = MetadataMap::new();
    metadata.insert("machine", "remote".parse().unwrap());
    let status = call_proxy(
        connect(proxy_port).await,
        metadata,
        vec![Bytes::from_static(b"request")],
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition);
    assert!(status.metadata().get("x-initial").is_none());
    assert_eq!(status.details(), &[0, 255, 4, 2]);

    proxy_task.abort();
    upstream_task.abort();
}

#[tokio::test]
async fn one_to_one_preserves_message_then_terminal_status_details_and_metadata() {
    let (upstream_port, upstream_task) = spawn_server(RawUpstream::error_after_message()).await;
    let proxy = ProxyService::new(Arc::new(Director::new(
        "/tmp/unused.sock",
        upstream_port,
        StaticMapper {
            targets: vec![MachineTarget::new("id-1", "remote", "::1")],
        },
    )));
    let (proxy_port, proxy_task) = spawn_server(proxy).await;
    let mut metadata = MetadataMap::new();
    metadata.insert("machine", "remote".parse().unwrap());
    let response = call_proxy(
        connect(proxy_port).await,
        metadata,
        vec![Bytes::from_static(b"request")],
        None,
    )
    .await
    .unwrap();
    assert_eq!(response.metadata().get("x-initial").unwrap(), "one");
    let mut stream = response.into_inner();
    assert_eq!(
        stream.message().await.unwrap(),
        Some(Bytes::from_static(b"request"))
    );
    let status = stream.message().await.unwrap_err();
    assert_eq!(status.code(), Code::FailedPrecondition);
    assert_eq!(status.message(), "terminal failure");
    assert_eq!(status.details(), &[0, 255, 4, 2]);
    assert_eq!(
        status
            .metadata()
            .get_all("x-terminal")
            .iter()
            .map(|value| value.to_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["one", "two"]
    );
    assert_eq!(
        status
            .metadata()
            .get_bin("terminal-bin")
            .unwrap()
            .to_bytes()
            .unwrap(),
        &[9, 0, 9][..]
    );

    proxy_task.abort();
    upstream_task.abort();
}

#[tokio::test]
async fn dropping_downstream_stream_cancels_the_upstream_stream() {
    let cancelled = Arc::new(tokio::sync::Notify::new());
    let upstream = RawUpstream::cancellation_probe(Arc::clone(&cancelled));
    let (upstream_port, upstream_task) = spawn_server(upstream).await;
    let proxy = ProxyService::new(Arc::new(Director::new(
        "/tmp/unused.sock",
        upstream_port,
        StaticMapper {
            targets: vec![MachineTarget::new("id-1", "remote", "::1")],
        },
    )));
    let (proxy_port, proxy_task) = spawn_server(proxy).await;
    let mut metadata = MetadataMap::new();
    metadata.insert("machine", "remote".parse().unwrap());
    let response = call_proxy(
        connect(proxy_port).await,
        metadata,
        vec![Bytes::from_static(b"first")],
        None,
    )
    .await
    .unwrap();
    let mut stream = response.into_inner();
    assert_eq!(
        stream.message().await.unwrap(),
        Some(Bytes::from_static(b"first"))
    );
    drop(stream);
    tokio::time::timeout(std::time::Duration::from_secs(1), cancelled.notified())
        .await
        .expect("upstream stream was not cancelled");

    proxy_task.abort();
    upstream_task.abort();
}

#[tokio::test]
async fn ingress_deadline_covers_the_whole_upstream_stream() {
    let cancelled = Arc::new(tokio::sync::Notify::new());
    let upstream = RawUpstream::cancellation_probe(Arc::clone(&cancelled));
    let (upstream_port, upstream_task) = spawn_server(upstream).await;
    let proxy = ProxyService::new(Arc::new(Director::new(
        "/tmp/unused.sock",
        upstream_port,
        StaticMapper {
            targets: vec![MachineTarget::new("id-1", "remote", "::1")],
        },
    )));
    let (proxy_port, proxy_task) = spawn_server(proxy).await;
    let mut metadata = MetadataMap::new();
    metadata.insert("machine", "remote".parse().unwrap());
    let response = call_proxy(
        connect(proxy_port).await,
        metadata,
        vec![Bytes::from_static(b"first")],
        Some(std::time::Duration::from_millis(250)),
    )
    .await
    .unwrap();
    let mut stream = response.into_inner();
    assert_eq!(
        stream.message().await.unwrap(),
        Some(Bytes::from_static(b"first"))
    );
    let status = stream.message().await.unwrap_err();
    assert_eq!(status.code(), Code::DeadlineExceeded);
    tokio::time::timeout(std::time::Duration::from_secs(1), cancelled.notified())
        .await
        .expect("deadline did not cancel upstream");

    proxy_task.abort();
    upstream_task.abort();
}

#[tokio::test]
async fn ingress_deadline_covers_machine_resolution() {
    let mut proxy = ProxyService::new(Arc::new(Director::new(
        "/tmp/unused.sock",
        1,
        HangingMapper,
    )));
    let request = http::Request::builder()
        .uri("/unregistered.Service/Call")
        .header("content-type", "application/grpc")
        .header("te", "trailers")
        .header("machine", "remote")
        .header("grpc-timeout", "100m")
        .body(Body::empty())
        .unwrap();
    let response = tokio::time::timeout(std::time::Duration::from_secs(1), proxy.call(request))
        .await
        .expect("proxy ignored the ingress deadline")
        .unwrap();
    assert_eq!(response.headers().get("grpc-status").unwrap(), "4");
    assert_eq!(
        response.headers().get("grpc-message").unwrap(),
        "context%20deadline%20exceeded"
    );
}

#[tokio::test]
async fn malformed_ingress_timeout_is_rejected_before_routing() {
    let proxy = ProxyService::new(Arc::new(Director::new(
        "/tmp/unused.sock",
        1,
        HangingMapper,
    )));
    let (proxy_port, proxy_task) = spawn_server(proxy).await;
    let mut metadata = MetadataMap::new();
    metadata.insert("machine", "remote".parse().unwrap());
    metadata.insert("grpc-timeout", "broken".parse().unwrap());

    let status = call_proxy(
        connect(proxy_port).await,
        metadata,
        vec![Bytes::from_static(b"request")],
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(status.code(), Code::Internal);
    assert_eq!(
        status.message(),
        "malformed grpc-timeout: strconv.ParseUint: parsing \"broke\": invalid syntax"
    );

    proxy_task.abort();
}

#[tokio::test]
async fn application_unavailable_does_not_discard_a_healthy_channel() {
    let upstream = RawUpstream::unavailable_once();
    let (upstream_port, upstream_task) = spawn_server(upstream).await;
    let proxy = ProxyService::new(Arc::new(Director::new(
        "/tmp/unused.sock",
        upstream_port,
        StaticMapper {
            targets: vec![MachineTarget::new("id-1", "remote", "::1")],
        },
    )));
    let (proxy_port, proxy_task) = spawn_server(proxy).await;
    let mut metadata = MetadataMap::new();
    metadata.insert("machine", "remote".parse().unwrap());

    let first = call_proxy(
        connect(proxy_port).await,
        metadata.clone(),
        vec![Bytes::from_static(b"first")],
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(first.code(), Code::Unavailable);

    let mut second = call_proxy(
        connect(proxy_port).await,
        metadata,
        vec![Bytes::from_static(b"second")],
        None,
    )
    .await
    .expect("application status incorrectly put the backend into reconnect backoff")
    .into_inner();
    assert_eq!(
        second.message().await.unwrap(),
        Some(Bytes::from_static(b"second"))
    );

    proxy_task.abort();
    upstream_task.abort();
}

#[tokio::test]
async fn broadcast_append_info_failure_fails_the_frontend_rpc() {
    let (upstream_port, upstream_task) = spawn_server(RawUpstream::malformed_response()).await;
    let proxy = ProxyService::new(Arc::new(Director::new(
        "/tmp/unused.sock",
        upstream_port,
        StaticMapper {
            targets: vec![
                MachineTarget::new("id-1", "remote-a", "::1"),
                MachineTarget::new("id-2", "remote-b", "::1"),
            ],
        },
    )));
    let (proxy_port, proxy_task) = spawn_server(proxy).await;
    let mut metadata = MetadataMap::new();
    metadata.insert("machines", "remote".parse().unwrap());

    let response = call_proxy(
        connect(proxy_port).await,
        metadata,
        vec![Bytes::from_static(b"request")],
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        response
            .metadata()
            .get_all("x-initial")
            .iter()
            .map(|value| value.to_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["one", "two", "one", "two"]
    );
    let status = response.into_inner().message().await.unwrap_err();
    assert_eq!(status.code(), Code::Unknown);
    assert!(
        status
            .message()
            .starts_with("2 errors occurred:\n\t* error appending info for [::1]:")
    );
    assert!(
        status
            .message()
            .ends_with(": unexpected message format: 16\n\n")
    );
    proxy_task.abort();
    upstream_task.abort();
}

#[tokio::test]
async fn remote_backend_reconnects_in_the_background_after_initial_failure() {
    let reservation = tokio::net::TcpListener::bind("[::1]:0").await.unwrap();
    let upstream_port = reservation.local_addr().unwrap().port();
    drop(reservation);

    let proxy = ProxyService::new(Arc::new(Director::new(
        "/tmp/unused.sock",
        upstream_port,
        StaticMapper {
            targets: vec![MachineTarget::new("id-1", "remote", "::1")],
        },
    )));
    let (proxy_port, proxy_task) = spawn_server(proxy).await;
    let mut metadata = MetadataMap::new();
    metadata.insert("machine", "remote".parse().unwrap());
    let first = call_proxy(
        connect(proxy_port).await,
        metadata.clone(),
        vec![Bytes::from_static(b"probe")],
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(first.code(), Code::Unavailable);

    let upstream_task = spawn_server_on(upstream_port, RawUpstream::echo()).await;
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        if let Ok(response) = call_proxy(
            connect(proxy_port).await,
            metadata.clone(),
            vec![Bytes::from_static(b"probe")],
            None,
        )
        .await
        {
            let mut stream = response.into_inner();
            assert_eq!(
                stream.message().await.unwrap(),
                Some(Bytes::from_static(b"probe"))
            );
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "background reconnect did not restore the channel"
        );
    }

    proxy_task.abort();
    upstream_task.abort();
}

#[tokio::test]
async fn remote_backend_reconnects_after_an_established_transport_is_lost() {
    let (upstream_port, upstream_task) = spawn_server(RawUpstream::echo()).await;
    let proxy = ProxyService::new(Arc::new(Director::new(
        "/tmp/unused.sock",
        upstream_port,
        StaticMapper {
            targets: vec![MachineTarget::new("id-1", "remote", "::1")],
        },
    )));
    let (proxy_port, proxy_task) = spawn_server(proxy).await;
    let mut metadata = MetadataMap::new();
    metadata.insert("machine", "remote".parse().unwrap());

    let mut first = call_proxy(
        connect(proxy_port).await,
        metadata.clone(),
        vec![Bytes::from_static(b"first")],
        None,
    )
    .await
    .unwrap()
    .into_inner();
    assert_eq!(
        first.message().await.unwrap(),
        Some(Bytes::from_static(b"first"))
    );
    assert_eq!(first.message().await.unwrap(), None);

    upstream_task.abort();
    let _ = upstream_task.await;
    tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    let replacement_task = spawn_server_on(upstream_port, RawUpstream::echo()).await;

    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    let mut recovered = call_proxy(
        connect(proxy_port).await,
        metadata,
        vec![Bytes::from_static(b"second")],
        None,
    )
    .await
    .expect("owner-managed background reconnect did not replace the lost transport")
    .into_inner();
    assert_eq!(
        recovered.message().await.unwrap(),
        Some(Bytes::from_static(b"second"))
    );

    proxy_task.abort();
    replacement_task.abort();
}
