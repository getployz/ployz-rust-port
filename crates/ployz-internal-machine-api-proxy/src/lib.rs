//! Schema-independent gRPC routing for the machine API.
//!
//! The proxy keeps Tonic responsible for gRPC framing, compression rejection,
//! metadata, status, and message limits. It only resolves target machines,
//! forwards raw protobuf messages, and injects the source metadata required by
//! broadcast responses.

use std::collections::HashMap;
use std::convert::Infallible;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, RwLock};
use std::task::{Context, Poll};
use std::time::Duration;

use bytes::Bytes;
use http::uri::PathAndQuery;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{Instant, Sleep};
use tokio_stream::Stream;
use tokio_stream::wrappers::ReceiverStream;
use tonic::body::Body;
use tonic::codec::Streaming;
use tonic::metadata::{KeyAndValueRef, MetadataMap};
use tonic::{Code, Request, Response, Status};
use tower::Service;

mod backend;
mod codec;
mod mapper;
mod payload;
pub use backend::Backend;
pub use codec::{RawCodec, RawDecoder, RawEncoder};
use mapper::AddressKey;
pub use mapper::{
    CorrosionMapper, MachineMapper, MachineStore, MachineTarget, MachinesNotFoundError,
    MapMachinesError, StoreFuture,
};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;
type ResponseStream = DeadlineStream;

const REQUEST_BUFFER: usize = 8;
const RESPONSE_BUFFER: usize = 8;

/// Proxy fan-out mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Mode {
    OneToOne,
    OneToMany,
}

/// A completed routing decision.
#[derive(Clone, Debug)]
pub struct Route {
    pub mode: Mode,
    backends: Vec<Backend>,
}

impl Route {
    #[must_use]
    pub fn mode(&self) -> Mode {
        self.mode
    }

    #[must_use]
    pub fn backends(&self) -> &[Backend] {
        &self.backends
    }
}

/// Resolves gRPC metadata into cached local or remote backends.
#[derive(Debug)]
pub struct Director<M> {
    local_backend: Backend,
    remote_port: u16,
    remote_backends: Mutex<HashMap<AddressKey, Backend>>,
    local_address: RwLock<Option<String>>,
    mapper: M,
}

impl<M> Director<M>
where
    M: MachineMapper,
{
    #[must_use]
    pub fn new(local_socket_path: impl AsRef<str>, remote_port: u16, mapper: M) -> Self {
        Self {
            local_backend: Backend::local(local_socket_path),
            remote_port,
            remote_backends: Mutex::new(HashMap::new()),
            local_address: RwLock::new(None),
            mapper,
        }
    }

    pub fn update_local_address(&self, address: impl Into<String>) {
        *self
            .local_address
            .write()
            .expect("local address lock poisoned") = Some(address.into());
    }

    pub async fn route(&self, metadata: &MetadataMap) -> Result<Route, Status> {
        if metadata.contains_key("proxy-authority") {
            return Ok(self.local_route());
        }

        let machines = metadata_values(metadata, "machines");
        let machine = metadata_values(metadata, "machine");
        if machines.is_none() && machine.is_none() {
            return Ok(self.local_route());
        }

        if let Some(machine) = machine {
            if machine.len() != 1 {
                return Err(Status::invalid_argument(
                    "proxy metadata 'machine' must have exactly one value",
                ));
            }
            if machines.is_some() {
                return Err(Status::invalid_argument(
                    "both 'machine' and 'machines' proxy metadata are set",
                ));
            }
            let targets = self
                .mapper
                .map_machines(&machine)
                .await
                .map_err(map_resolution_error)?;
            let target = targets.first().expect("mapper returned no targets");
            return Ok(Route {
                mode: Mode::OneToOne,
                backends: vec![self.backend_for(target)?],
            });
        }

        let machines = machines.expect("one proxy metadata key was present");
        if machines.is_empty() {
            return Err(Status::invalid_argument(
                "proxy metadata 'machines' is empty",
            ));
        }
        let targets = self
            .mapper
            .map_machines(&machines)
            .await
            .map_err(map_resolution_error)?;
        let mut backends = Vec::with_capacity(targets.len());
        for target in targets {
            backends.push(self.backend_for(&target)?.with_machine(target));
        }
        Ok(Route {
            mode: Mode::OneToMany,
            backends,
        })
    }

    pub fn flush_remote_backends(&self) {
        let mut backends = self
            .remote_backends
            .lock()
            .expect("remote backend lock poisoned");
        for backend in backends.values() {
            backend.close();
        }
        backends.clear();
    }

    pub fn close(&self) {
        self.local_backend.close();
        self.flush_remote_backends();
    }

    fn local_route(&self) -> Route {
        Route {
            mode: Mode::OneToOne,
            backends: vec![self.local_backend.clone()],
        }
    }

    fn backend_for(&self, target: &MachineTarget) -> Result<Backend, Status> {
        let address = target.address();
        if self
            .local_address
            .read()
            .expect("local address lock poisoned")
            .as_deref()
            == Some(address)
        {
            return Ok(self.local_backend.clone());
        }
        let mut backends = self
            .remote_backends
            .lock()
            .expect("remote backend lock poisoned");
        let key = target.address_key();
        if let Some(backend) = backends.get(&key) {
            return Ok(backend.clone());
        }
        let backend = Backend::remote_target(target, self.remote_port)?;
        backends.insert(key, backend.clone());
        Ok(backend)
    }
}

impl<M> Drop for Director<M> {
    fn drop(&mut self) {
        self.local_backend.close();
        let backends = self
            .remote_backends
            .get_mut()
            .expect("remote backend lock poisoned");
        for backend in backends.values() {
            backend.close();
        }
    }
}

fn metadata_values(metadata: &MetadataMap, key: &'static str) -> Option<Vec<String>> {
    if !metadata.contains_key(key) {
        return None;
    }
    Some(
        metadata
            .get_all(key)
            .iter()
            .map(|value| value.to_str().unwrap_or_default().to_owned())
            .collect(),
    )
}

fn map_resolution_error(error: MapMachinesError) -> Status {
    match error {
        MapMachinesError::NotFound(error) => Status::invalid_argument(error.to_string()),
        MapMachinesError::Status(status) => status,
        MapMachinesError::Other(message) => {
            Status::internal(format!("failed to resolve machines: {message}"))
        }
    }
}

/// Tower fallback service for a proxy-only Tonic server.
#[derive(Debug)]
pub struct ProxyService<M> {
    director: Arc<Director<M>>,
}

impl<M> Clone for ProxyService<M> {
    fn clone(&self) -> Self {
        Self {
            director: Arc::clone(&self.director),
        }
    }
}

impl<M> ProxyService<M> {
    #[must_use]
    pub fn new(director: Arc<Director<M>>) -> Self {
        Self { director }
    }

    #[must_use]
    pub fn director(&self) -> &Arc<Director<M>> {
        &self.director
    }
}

impl<M> Service<http::Request<Body>> for ProxyService<M>
where
    M: MachineMapper,
{
    type Response = http::Response<Body>;
    type Error = Infallible;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: http::Request<Body>) -> Self::Future {
        let director = Arc::clone(&self.director);
        let path = request
            .uri()
            .path_and_query()
            .cloned()
            .unwrap_or_else(|| PathAndQuery::from_static("/"));
        let authority = request
            .uri()
            .authority()
            .map(ToString::to_string)
            .or_else(|| {
                request
                    .headers()
                    .get(http::header::HOST)
                    .and_then(|value| value.to_str().ok())
                    .map(ToOwned::to_owned)
            });
        let deadline = parse_ingress_deadline(request.headers().get("grpc-timeout"));

        Box::pin(async move {
            let call = ProxyCall {
                director,
                path,
                authority,
                deadline,
            };
            let response = tonic::server::Grpc::new(RawCodec)
                .streaming(call, request)
                .await;
            Ok(response)
        })
    }
}

#[derive(Debug)]
struct ProxyCall<M> {
    director: Arc<Director<M>>,
    path: PathAndQuery,
    authority: Option<String>,
    deadline: Result<Option<Instant>, Status>,
}

impl<M> Clone for ProxyCall<M> {
    fn clone(&self) -> Self {
        Self {
            director: Arc::clone(&self.director),
            path: self.path.clone(),
            authority: self.authority.clone(),
            deadline: self.deadline.clone(),
        }
    }
}

impl<M> Service<Request<Streaming<Bytes>>> for ProxyCall<M>
where
    M: MachineMapper,
{
    type Response = Response<ResponseStream>;
    type Error = Status;
    type Future = BoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&mut self, _context: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, request: Request<Streaming<Bytes>>) -> Self::Future {
        let this = self.clone();
        Box::pin(async move {
            let deadline = this.deadline.clone()?;
            let route =
                before_deadline(this.director.route(request.metadata()), deadline).await??;
            match route.mode {
                Mode::OneToOne => this.one_to_one(request, route.backends, deadline).await,
                Mode::OneToMany => this.one_to_many(request, route.backends, deadline).await,
            }
        })
    }
}

impl<M> ProxyCall<M>
where
    M: MachineMapper,
{
    async fn one_to_one(
        &self,
        request: Request<Streaming<Bytes>>,
        backends: Vec<Backend>,
        deadline: Option<Instant>,
    ) -> Result<Response<ResponseStream>, Status> {
        if backends.len() != 1 {
            return Err(Status::internal(format!(
                "one2one proxying should have exactly one connection (got {})",
                backends.len()
            )));
        }
        let backend = backends.into_iter().next().expect("length was checked");
        let (metadata, _extensions, incoming) = request.into_parts();
        let (request_tx, request_rx) = mpsc::channel(REQUEST_BUFFER);
        let (request_error_tx, request_error_rx) = mpsc::channel(1);
        let request_task = tokio::spawn(async move {
            if let Err(status) = pump_request(incoming, vec![request_tx]).await {
                let _ = request_error_tx.send(status).await;
            }
        });
        let mut request_guard = AbortOnDrop::new(request_task);
        let mut request_errors = Some(request_error_rx);

        let outgoing = self.outgoing_request(
            &backend,
            metadata,
            ReceiverStream::new(request_rx),
            deadline,
        )?;
        let channel =
            before_deadline_or_request_error(backend.channel(), &mut request_errors, deadline)
                .await??;
        let mut grpc = tonic::client::Grpc::new(channel);
        if let Err(error) =
            before_deadline_or_request_error(grpc.ready(), &mut request_errors, deadline).await?
        {
            let status = Status::unavailable(error.to_string());
            return Err(status);
        }
        let response = match before_deadline_or_request_error(
            grpc.streaming(outgoing, self.path.clone(), RawCodec),
            &mut request_errors,
            deadline,
        )
        .await?
        {
            Ok(response) => response,
            Err(status) => return Err(status),
        };
        let (initial_metadata, mut upstream, _extensions) = response.into_parts();
        let first = message_or_request_error(&mut upstream, &mut request_errors, deadline).await;

        let (response_tx, response_rx) = mpsc::channel(RESPONSE_BUFFER);
        let mut response = Response::new(DeadlineStream::new(response_rx, deadline));
        match first {
            Ok(Some(message)) => {
                *response.metadata_mut() = initial_metadata;
                let request_task = request_guard.take();
                tokio::spawn(async move {
                    forward_one_to_one_response(
                        message,
                        upstream,
                        response_tx,
                        request_task,
                        request_errors,
                        deadline,
                    )
                    .await;
                });
            }
            Ok(None) => {
                let trailers = before_deadline_or_request_error(
                    upstream.trailers(),
                    &mut request_errors,
                    deadline,
                )
                .await??;
                if let Some(status) = ready_request_error(&mut request_errors)? {
                    return Err(status);
                }
                if let Some(trailers) = trailers
                    && !trailers.is_empty()
                {
                    let _ = response_tx
                        .send(Err(Status::with_metadata(Code::Ok, "", trailers)))
                        .await;
                }
            }
            Err(status) => {
                return Err(status);
            }
        }
        Ok(response)
    }

    async fn one_to_many(
        &self,
        request: Request<Streaming<Bytes>>,
        backends: Vec<Backend>,
        deadline: Option<Instant>,
    ) -> Result<Response<ResponseStream>, Status> {
        if backends.is_empty() {
            return Err(Status::unavailable("no backend connections for proxying"));
        }
        let (metadata, _extensions, incoming) = request.into_parts();
        let (event_tx, mut event_rx) = mpsc::channel(RESPONSE_BUFFER.max(backends.len() * 2));
        let mut request_senders = Vec::with_capacity(backends.len());
        let mut backend_tasks = Vec::with_capacity(backends.len());
        for backend in backends {
            let (request_tx, request_rx) = mpsc::channel(REQUEST_BUFFER);
            request_senders.push(request_tx);
            let outgoing = self.outgoing_request(
                &backend,
                metadata.clone(),
                ReceiverStream::new(request_rx),
                deadline,
            );
            let path = self.path.clone();
            let events = event_tx.clone();
            backend_tasks.push(tokio::spawn(async move {
                run_broadcast_backend(backend, outgoing, path, events, deadline).await;
            }));
        }
        drop(event_tx);
        let (request_result_tx, mut request_results) = mpsc::channel(1);
        let request_task = tokio::spawn(async move {
            let _ = request_result_tx
                .send(pump_request_many(incoming, request_senders).await)
                .await;
        });
        let _request_guard = AbortOnDrop::new(request_task);
        let task_guards = backend_tasks
            .into_iter()
            .map(AbortOnDrop::new)
            .collect::<Vec<_>>();

        let mut response_metadata = MetadataMap::new();
        let mut trailers = MetadataMap::new();
        let mut merged = Vec::new();
        let mut backend_failures = Vec::new();
        let mut finished = 0;
        let mut request_finished = false;
        while finished < task_guards.len() {
            let event = match event_or_request_result(
                &mut event_rx,
                &mut request_results,
                &mut request_finished,
                deadline,
            )
            .await
            {
                Ok(event) => event,
                Err(mut status) => {
                    append_metadata(status.metadata_mut(), &trailers);
                    return response_or_status(response_metadata, status, deadline).await;
                }
            };
            let Some(event) = event else {
                return Err(Status::internal(
                    "all broadcast backend tasks stopped before completion",
                ));
            };
            match event {
                BackendEvent::InitialMetadata(metadata) => {
                    append_metadata(&mut response_metadata, &metadata);
                }
                BackendEvent::Payload(payload) => merged.extend_from_slice(&payload),
                BackendEvent::Trailers(metadata) => append_metadata(&mut trailers, &metadata),
                BackendEvent::Fatal(status) => backend_failures.push(status),
                BackendEvent::Finished => finished += 1,
            }
        }
        if !request_finished {
            match request_results.try_recv() {
                Ok(Ok(())) => {}
                Ok(Err(mut status)) => {
                    append_metadata(status.metadata_mut(), &trailers);
                    return response_or_status(response_metadata, status, deadline).await;
                }
                Err(mpsc::error::TryRecvError::Empty) => {}
                Err(mpsc::error::TryRecvError::Disconnected) => {
                    let mut status = Status::internal("request pump stopped without a result");
                    append_metadata(status.metadata_mut(), &trailers);
                    return response_or_status(response_metadata, status, deadline).await;
                }
            }
        }
        if !backend_failures.is_empty() {
            let count = backend_failures.len();
            let noun = if count == 1 { "error" } else { "errors" };
            let bullets = backend_failures
                .iter()
                .map(Status::message)
                .collect::<Vec<_>>()
                .join("\n\t* ");
            let mut status =
                Status::unknown(format!("{count} {noun} occurred:\n\t* {bullets}\n\n"));
            append_metadata(status.metadata_mut(), &trailers);
            return response_or_status(response_metadata, status, deadline).await;
        }
        let (response_tx, response_rx) = mpsc::channel(2);
        response_tx
            .send(Ok(Bytes::from(merged)))
            .await
            .map_err(|_| Status::cancelled("downstream closed"))?;
        if !trailers.is_empty() {
            response_tx
                .send(Err(Status::with_metadata(Code::Ok, "", trailers)))
                .await
                .map_err(|_| Status::cancelled("downstream closed"))?;
        }
        drop(response_tx);
        let mut response = Response::new(DeadlineStream::new(response_rx, deadline));
        *response.metadata_mut() = response_metadata;
        Ok(response)
    }

    fn outgoing_request<S>(
        &self,
        backend: &Backend,
        incoming_metadata: MetadataMap,
        stream: S,
        deadline: Option<Instant>,
    ) -> Result<Request<S>, Status> {
        let metadata = backend.outgoing_metadata(&incoming_metadata, self.authority.as_deref());
        let mut request = Request::from_parts(metadata, http::Extensions::new(), stream);
        if let Some(deadline) = deadline {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Status::deadline_exceeded("context deadline exceeded"));
            }
            request.set_timeout(remaining);
        }
        Ok(request)
    }
}

async fn response_or_status(
    metadata: MetadataMap,
    status: Status,
    deadline: Option<Instant>,
) -> Result<Response<ResponseStream>, Status> {
    if metadata.is_empty() {
        return Err(status);
    }
    let (response_tx, response_rx) = mpsc::channel(1);
    response_tx
        .send(Err(status))
        .await
        .map_err(|_| Status::cancelled("downstream closed"))?;
    drop(response_tx);
    let mut response = Response::new(DeadlineStream::new(response_rx, deadline));
    *response.metadata_mut() = metadata;
    Ok(response)
}

async fn pump_request(
    mut incoming: Streaming<Bytes>,
    destinations: Vec<mpsc::Sender<Bytes>>,
) -> Result<(), Status> {
    loop {
        match incoming.message().await {
            Ok(Some(message)) => {
                for destination in &destinations {
                    destination.send(message.clone()).await.map_err(|_| {
                        Status::internal("failed proxying s2c: backend request stream closed")
                    })?;
                }
            }
            Ok(None) => return Ok(()),
            Err(status) => return Err(frontend_stream_error(&status)),
        }
    }
}

async fn pump_request_many(
    mut incoming: Streaming<Bytes>,
    mut destinations: Vec<mpsc::Sender<Bytes>>,
) -> Result<(), Status> {
    loop {
        match incoming.message().await {
            Ok(Some(message)) => {
                let mut live = Vec::with_capacity(destinations.len());
                for destination in destinations {
                    if destination.send(message.clone()).await.is_ok() {
                        live.push(destination);
                    }
                }
                destinations = live;
                if destinations.is_empty() {
                    return Ok(());
                }
            }
            Ok(None) => return Ok(()),
            Err(status) => return Err(frontend_stream_error(&status)),
        }
    }
}

fn frontend_stream_error(status: &Status) -> Status {
    Status::internal(format!(
        "failed proxying s2c: rpc error: code = {:?} desc = {}",
        status.code(),
        status.message()
    ))
}

async fn forward_one_to_one_response(
    first: Bytes,
    mut upstream: Streaming<Bytes>,
    response: mpsc::Sender<Result<Bytes, Status>>,
    request_task: JoinHandle<()>,
    mut request_errors: Option<mpsc::Receiver<Status>>,
    deadline: Option<Instant>,
) {
    let request_guard = AbortOnDrop::new(request_task);
    if send_before_deadline(&response, Ok(first), deadline)
        .await
        .is_err()
    {
        return;
    }
    loop {
        match message_before_deadline_or_closed(
            &mut upstream,
            &response,
            &mut request_errors,
            deadline,
        )
        .await
        {
            None => break,
            Some(Ok(Some(message))) => {
                if send_before_deadline(&response, Ok(message), deadline)
                    .await
                    .is_err()
                {
                    return;
                }
            }
            Some(Ok(None)) => {
                match before_deadline(upstream.trailers(), deadline).await {
                    Ok(Ok(Some(trailers))) if !trailers.is_empty() => {
                        let _ = send_before_deadline(
                            &response,
                            Err(Status::with_metadata(Code::Ok, "", trailers)),
                            deadline,
                        )
                        .await;
                    }
                    Ok(Ok(_)) => {}
                    Ok(Err(status)) | Err(status) => {
                        let _ = send_before_deadline(&response, Err(status), deadline).await;
                    }
                }
                break;
            }
            Some(Err(status)) => {
                let _ = send_before_deadline(&response, Err(status), deadline).await;
                break;
            }
        }
    }
    drop(request_guard);
}

async fn message_before_deadline_or_closed(
    stream: &mut Streaming<Bytes>,
    response: &mpsc::Sender<Result<Bytes, Status>>,
    request_errors: &mut Option<mpsc::Receiver<Status>>,
    deadline: Option<Instant>,
) -> Option<Result<Option<Bytes>, Status>> {
    loop {
        if let Some(errors) = request_errors {
            let event = match deadline {
                Some(deadline) => tokio::select! {
                    biased;
                    status = errors.recv() => status,
                    () = response.closed() => return None,
                    result = stream.message() => return Some(result),
                    () = tokio::time::sleep_until(deadline) => {
                        return Some(Err(Status::deadline_exceeded("context deadline exceeded")));
                    }
                },
                None => tokio::select! {
                    biased;
                    status = errors.recv() => status,
                    () = response.closed() => return None,
                    result = stream.message() => return Some(result),
                },
            };
            if let Some(status) = event {
                return Some(Err(status));
            }
            *request_errors = None;
            continue;
        }
        return match deadline {
            Some(deadline) => tokio::select! {
                () = response.closed() => None,
                result = stream.message() => Some(result),
                () = tokio::time::sleep_until(deadline) => {
                    Some(Err(Status::deadline_exceeded("context deadline exceeded")))
                }
            },
            None => tokio::select! {
                () = response.closed() => None,
                result = stream.message() => Some(result),
            },
        };
    }
}

async fn message_or_request_error(
    stream: &mut Streaming<Bytes>,
    request_errors: &mut Option<mpsc::Receiver<Status>>,
    deadline: Option<Instant>,
) -> Result<Option<Bytes>, Status> {
    loop {
        let Some(errors) = request_errors else {
            return message_before_deadline(stream, deadline).await;
        };
        let event = match deadline {
            Some(deadline) => tokio::select! {
                biased;
                status = errors.recv() => status,
                result = stream.message() => return result,
                () = tokio::time::sleep_until(deadline) => {
                    return Err(Status::deadline_exceeded("context deadline exceeded"));
                }
            },
            None => tokio::select! {
                biased;
                status = errors.recv() => status,
                result = stream.message() => return result,
            },
        };
        if let Some(status) = event {
            return Err(status);
        }
        *request_errors = None;
    }
}

#[derive(Debug)]
enum BackendEvent {
    InitialMetadata(MetadataMap),
    Payload(Bytes),
    Trailers(MetadataMap),
    Fatal(Status),
    Finished,
}

#[derive(Debug)]
enum BroadcastFailure {
    Backend(Status),
    Transform(Status),
}

impl From<Status> for BroadcastFailure {
    fn from(status: Status) -> Self {
        Self::Backend(status)
    }
}

async fn run_broadcast_backend<S>(
    backend: Backend,
    outgoing: Result<Request<S>, Status>,
    path: PathAndQuery,
    events: mpsc::Sender<BackendEvent>,
    deadline: Option<Instant>,
) where
    S: tokio_stream::Stream<Item = Bytes> + Send + 'static,
{
    let result = async {
        let outgoing = outgoing?;
        let channel = before_deadline(backend.channel(), deadline).await??;
        let mut grpc = tonic::client::Grpc::new(channel);
        before_deadline(grpc.ready(), deadline)
            .await?
            .map_err(|error| Status::unavailable(error.to_string()))?;
        let response =
            before_deadline(grpc.streaming(outgoing, path, RawCodec), deadline).await??;
        let (metadata, mut stream, _extensions) = response.into_parts();
        let mut first = true;
        loop {
            match message_before_deadline(&mut stream, deadline).await {
                Ok(Some(payload)) => {
                    if first {
                        first = false;
                        events
                            .send(BackendEvent::InitialMetadata(metadata.clone()))
                            .await
                            .map_err(|_| Status::cancelled("downstream closed"))?;
                    }
                    let payload = backend.append_info(false, &payload).map_err(|error| {
                        BroadcastFailure::Transform(Status::unknown(format!(
                            "error appending info for {}: {error}",
                            backend.target()
                        )))
                    })?;
                    events
                        .send(BackendEvent::Payload(Bytes::from(payload)))
                        .await
                        .map_err(|_| Status::cancelled("downstream closed"))?;
                }
                Ok(None) => {
                    if let Some(trailers) = before_deadline(stream.trailers(), deadline).await?? {
                        events
                            .send(BackendEvent::Trailers(trailers))
                            .await
                            .map_err(|_| Status::cancelled("downstream closed"))?;
                    }
                    return Ok::<(), BroadcastFailure>(());
                }
                Err(status) => return Err(BroadcastFailure::Backend(status)),
            }
        }
    }
    .await;

    match result {
        Err(BroadcastFailure::Transform(status)) => {
            let _ = events.send(BackendEvent::Fatal(status)).await;
        }
        Err(BroadcastFailure::Backend(error)) => match backend.build_error(false, &error) {
            Ok(Some(payload)) => {
                let _ = events
                    .send(BackendEvent::Payload(Bytes::from(payload)))
                    .await;
            }
            Ok(None) => {
                let _ = events.send(BackendEvent::Fatal(error)).await;
            }
            Err(build_error) => {
                let _ = events
                    .send(BackendEvent::Fatal(Status::unknown(format!(
                        "error building error for {}: {build_error}",
                        backend.target()
                    ))))
                    .await;
            }
        },
        Ok(()) => {}
    }
    let _ = events.send(BackendEvent::Finished).await;
}

async fn message_before_deadline(
    stream: &mut Streaming<Bytes>,
    deadline: Option<Instant>,
) -> Result<Option<Bytes>, Status> {
    match deadline {
        Some(deadline) => tokio::select! {
            result = stream.message() => result,
            () = tokio::time::sleep_until(deadline) => {
                Err(Status::deadline_exceeded("context deadline exceeded"))
            }
        },
        None => stream.message().await,
    }
}

async fn event_before_deadline(
    events: &mut mpsc::Receiver<BackendEvent>,
    deadline: Option<Instant>,
) -> Result<Option<BackendEvent>, Status> {
    match deadline {
        Some(deadline) => tokio::select! {
            event = events.recv() => Ok(event),
            () = tokio::time::sleep_until(deadline) => {
                Err(Status::deadline_exceeded("context deadline exceeded"))
            }
        },
        None => Ok(events.recv().await),
    }
}

async fn event_or_request_result(
    events: &mut mpsc::Receiver<BackendEvent>,
    request_results: &mut mpsc::Receiver<Result<(), Status>>,
    request_finished: &mut bool,
    deadline: Option<Instant>,
) -> Result<Option<BackendEvent>, Status> {
    if *request_finished {
        return event_before_deadline(events, deadline).await;
    }
    match deadline {
        Some(deadline) => tokio::select! {
            biased;
            result = request_results.recv() => match result {
                Some(Ok(())) => {
                    *request_finished = true;
                    event_before_deadline(events, Some(deadline)).await
                },
                Some(Err(status)) => Err(status),
                None => Err(Status::internal("request pump stopped without a result")),
            },
            event = events.recv() => Ok(event),
            () = tokio::time::sleep_until(deadline) => {
                Err(Status::deadline_exceeded("context deadline exceeded"))
            }
        },
        None => tokio::select! {
            biased;
            result = request_results.recv() => match result {
                Some(Ok(())) => {
                    *request_finished = true;
                    Ok(events.recv().await)
                },
                Some(Err(status)) => Err(status),
                None => Err(Status::internal("request pump stopped without a result")),
            },
            event = events.recv() => Ok(event),
        },
    }
}

async fn send_before_deadline(
    response: &mpsc::Sender<Result<Bytes, Status>>,
    item: Result<Bytes, Status>,
    deadline: Option<Instant>,
) -> Result<(), Status> {
    before_deadline(response.send(item), deadline)
        .await?
        .map_err(|_| Status::cancelled("downstream closed"))
}

struct DeadlineStream {
    inner: ReceiverStream<Result<Bytes, Status>>,
    deadline: Option<Pin<Box<Sleep>>>,
    expired: bool,
}

impl DeadlineStream {
    fn new(receiver: mpsc::Receiver<Result<Bytes, Status>>, deadline: Option<Instant>) -> Self {
        Self {
            inner: ReceiverStream::new(receiver),
            deadline: deadline.map(|deadline| Box::pin(tokio::time::sleep_until(deadline))),
            expired: false,
        }
    }
}

impl Stream for DeadlineStream {
    type Item = Result<Bytes, Status>;

    fn poll_next(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.expired {
            return Poll::Ready(None);
        }
        if let Some(deadline) = self.deadline.as_mut()
            && deadline.as_mut().poll(context).is_ready()
        {
            self.inner.close();
            self.expired = true;
            return Poll::Ready(Some(Err(Status::deadline_exceeded(
                "context deadline exceeded",
            ))));
        }
        Pin::new(&mut self.inner).poll_next(context)
    }
}

fn ready_request_error(
    request_errors: &mut Option<mpsc::Receiver<Status>>,
) -> Result<Option<Status>, Status> {
    let Some(errors) = request_errors else {
        return Ok(None);
    };
    match errors.try_recv() {
        Ok(status) => Ok(Some(status)),
        Err(mpsc::error::TryRecvError::Empty) => Ok(None),
        Err(mpsc::error::TryRecvError::Disconnected) => {
            *request_errors = None;
            Ok(None)
        }
    }
}

fn append_metadata(destination: &mut MetadataMap, source: &MetadataMap) {
    for entry in source.iter() {
        match entry {
            KeyAndValueRef::Ascii(key, value) => {
                destination.append(key.clone(), value.clone());
            }
            KeyAndValueRef::Binary(key, value) => {
                destination.append_bin(key.clone(), value.clone());
            }
        }
    }
}

fn parse_ingress_deadline(value: Option<&http::HeaderValue>) -> Result<Option<Instant>, Status> {
    let Some(value) = value else {
        return Ok(None);
    };
    let text = value.to_str().map_err(|_| {
        Status::internal(format!(
            "malformed grpc-timeout: {}",
            String::from_utf8_lossy(value.as_bytes())
        ))
    })?;
    let duration = parse_grpc_timeout(text)
        .map_err(|error| Status::internal(format!("malformed grpc-timeout: {error}")))?;
    Ok(Instant::now().checked_add(duration))
}

fn parse_grpc_timeout(value: &str) -> Result<Duration, String> {
    if value.len() < 2 {
        return Err(format!("transport: timeout string is too short: {value:?}"));
    }
    if value.len() > 9 {
        return Err(format!("transport: timeout string is too long: {value:?}"));
    }
    let (digits, unit) = value.split_at(value.len() - 1);
    let multiplier = match unit.as_bytes()[0] {
        b'H' => Duration::from_secs(3_600),
        b'M' => Duration::from_secs(60),
        b'S' => Duration::from_secs(1),
        b'm' => Duration::from_millis(1),
        b'u' => Duration::from_micros(1),
        b'n' => Duration::from_nanos(1),
        _ => {
            return Err(format!(
                "transport: timeout unit is not recognized: {value:?}"
            ));
        }
    };
    if !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(format!(
            "strconv.ParseUint: parsing {digits:?}: invalid syntax"
        ));
    }
    let amount = digits
        .parse::<u32>()
        .expect("at most eight decimal digits fit in u32");
    let duration = multiplier.saturating_mul(amount);
    let grpc_go_max = Duration::from_nanos(i64::MAX as u64);
    if duration > grpc_go_max {
        Ok(grpc_go_max)
    } else {
        Ok(duration)
    }
}

async fn before_deadline<F, T>(future: F, deadline: Option<Instant>) -> Result<T, Status>
where
    F: Future<Output = T>,
{
    match deadline {
        Some(deadline) => tokio::select! {
            output = future => Ok(output),
            () = tokio::time::sleep_until(deadline) => {
                Err(Status::deadline_exceeded("context deadline exceeded"))
            }
        },
        None => Ok(future.await),
    }
}

async fn before_deadline_or_request_error<F, T>(
    future: F,
    request_errors: &mut Option<mpsc::Receiver<Status>>,
    deadline: Option<Instant>,
) -> Result<T, Status>
where
    F: Future<Output = T>,
{
    tokio::pin!(future);
    loop {
        let Some(errors) = request_errors else {
            return before_deadline(future, deadline).await;
        };
        let event = match deadline {
            Some(deadline) => tokio::select! {
                output = &mut future => return Ok(output),
                status = errors.recv() => status,
                () = tokio::time::sleep_until(deadline) => {
                    return Err(Status::deadline_exceeded("context deadline exceeded"));
                }
            },
            None => tokio::select! {
                output = &mut future => return Ok(output),
                status = errors.recv() => status,
            },
        };
        if let Some(status) = event {
            return Err(status);
        }
        *request_errors = None;
    }
}

struct AbortOnDrop<T>(Option<JoinHandle<T>>);

impl<T> AbortOnDrop<T> {
    fn new(handle: JoinHandle<T>) -> Self {
        Self(Some(handle))
    }

    fn take(&mut self) -> JoinHandle<T> {
        self.0.take().expect("task handle already taken")
    }
}

impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        if let Some(handle) = self.0.take() {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod timeout_tests {
    use super::*;
    use tokio_stream::StreamExt;

    #[test]
    fn grpc_timeout_decoding_matches_grpc_go_edges() {
        assert_eq!(
            parse_grpc_timeout("00000001M").unwrap(),
            Duration::from_secs(60)
        );
        assert_eq!(parse_grpc_timeout("0S").unwrap(), Duration::ZERO);
        assert_eq!(
            parse_grpc_timeout("99999999H").unwrap(),
            Duration::from_nanos(i64::MAX as u64)
        );
        assert_eq!(
            parse_grpc_timeout("1").unwrap_err(),
            "transport: timeout string is too short: \"1\""
        );
        assert_eq!(
            parse_grpc_timeout("000000000S").unwrap_err(),
            "transport: timeout string is too long: \"000000000S\""
        );
        assert_eq!(
            parse_grpc_timeout("1234x").unwrap_err(),
            "transport: timeout unit is not recognized: \"1234x\""
        );
        assert_eq!(
            parse_grpc_timeout("9a1S").unwrap_err(),
            "strconv.ParseUint: parsing \"9a1\": invalid syntax"
        );
    }

    #[test]
    fn frontend_stream_errors_use_the_grpc_proxy_status_shape() {
        let status = frontend_stream_error(&Status::data_loss("bad frame"));
        assert_eq!(status.code(), Code::Internal);
        assert_eq!(
            status.message(),
            "failed proxying s2c: rpc error: code = DataLoss desc = bad frame"
        );
    }

    #[tokio::test]
    async fn deadline_interrupts_saturated_downstream_buffering() {
        let deadline = Instant::now() + Duration::from_millis(20);
        let (sender, receiver) = mpsc::channel(1);
        sender
            .send(Ok(Bytes::from_static(b"queued")))
            .await
            .unwrap();
        let blocked = tokio::spawn(async move {
            send_before_deadline(&sender, Ok(Bytes::from_static(b"blocked")), Some(deadline)).await
        });
        tokio::time::sleep(Duration::from_millis(40)).await;

        let mut stream = DeadlineStream::new(receiver, Some(deadline));
        let status = stream.next().await.unwrap().unwrap_err();
        assert_eq!(status.code(), Code::DeadlineExceeded);
        assert_eq!(
            blocked.await.unwrap().unwrap_err().code(),
            Code::DeadlineExceeded
        );
    }

    #[tokio::test]
    async fn fatal_status_uses_initial_metadata_only_after_it_was_staged() {
        let status = Status::unknown("fatal");
        let direct = response_or_status(MetadataMap::new(), status.clone(), None).await;
        match direct {
            Err(error) => assert_eq!(error.code(), Code::Unknown),
            Ok(_) => panic!("unstaged status must be returned directly"),
        }

        let mut metadata = MetadataMap::new();
        metadata.insert("x-initial", "value".parse().unwrap());
        let response = response_or_status(metadata, status, None).await.unwrap();
        assert_eq!(response.metadata().get("x-initial").unwrap(), "value");
        let mut stream = response.into_inner();
        assert_eq!(
            stream.next().await.unwrap().unwrap_err().code(),
            Code::Unknown
        );
    }
}
