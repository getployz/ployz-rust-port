use std::error::Error;
use std::ffi::CString;
use std::net::{Ipv6Addr, SocketAddr, SocketAddrV6};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use hyper_util::rt::tokio::WithHyperIo;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::task::JoinHandle;
use tonic::Status;
use tonic::metadata::{AsciiMetadataValue, MetadataMap};
use tonic::transport::{Channel, Endpoint};
use tower::service_fn;

use crate::MachineTarget;
use crate::payload::{PayloadError, append_machine_info, build_machine_error};

const REMOTE_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const REMOTE_INITIAL_BACKOFF: Duration = Duration::from_secs(1);
const REMOTE_MAX_BACKOFF: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BackendKind {
    Local,
    Remote,
}

/// Shared connection state for one backend.
#[derive(Debug)]
struct BackendInner {
    kind: BackendKind,
    target: String,
    remote: Option<RemoteSocket>,
    connection: Mutex<ConnectionState>,
    connection_changed: tokio::sync::Notify,
}

#[derive(Clone, Debug)]
struct RemoteSocket {
    ip: Ipv6Addr,
    zone: Vec<u8>,
    port: u16,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum ConnectionPhase {
    #[default]
    Idle,
    InitialConnect,
    Backoff,
}

#[derive(Debug, Default)]
struct ConnectionState {
    channel: Option<Channel>,
    phase: ConnectionPhase,
    reconnect_task: Option<JoinHandle<()>>,
    closed: bool,
    generation: u64,
}

impl BackendInner {
    fn new(kind: BackendKind, target: String, remote: Option<RemoteSocket>) -> Self {
        Self {
            kind,
            target,
            remote,
            connection: Mutex::new(ConnectionState::default()),
            connection_changed: tokio::sync::Notify::new(),
        }
    }

    fn endpoint(&self) -> Result<Endpoint, Status> {
        match self.kind {
            BackendKind::Local => Endpoint::from_shared(self.target.clone()),
            BackendKind::Remote => Endpoint::from_shared(
                self.remote
                    .as_ref()
                    .expect("remote backend socket not set")
                    .endpoint_uri(),
            ),
        }
        .map_err(|error| Status::internal(error.to_string()))
        .map(|endpoint| endpoint.connect_timeout(REMOTE_CONNECT_TIMEOUT))
    }

    async fn connect(
        self: &Arc<Self>,
        generation: u64,
    ) -> Result<Channel, Box<dyn Error + Send + Sync>> {
        let endpoint = self
            .endpoint()
            .map_err(|error| Box::new(error) as Box<dyn Error + Send + Sync>)?;
        let socket = self
            .remote
            .as_ref()
            .expect("remote backend socket not set")
            .socket_addr()
            .map_err(|error| Box::new(error) as Box<dyn Error + Send + Sync>)?;
        let owner = Arc::downgrade(self);
        endpoint
            .connect_with_connector(service_fn(move |_| {
                let owner = owner.clone();
                async move {
                    let allowed = owner.upgrade().is_some_and(|inner| {
                        let state = inner
                            .connection
                            .lock()
                            .expect("backend connection lock poisoned");
                        state.generation == generation && state.channel.is_none() && !state.closed
                    });
                    if !allowed {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::ConnectionAborted,
                            "connection superseded by owner lifecycle",
                        ));
                    }
                    tokio::net::TcpStream::connect(socket).await.map(|stream| {
                        WithHyperIo::new(TrackedTcp {
                            stream,
                            owner,
                            generation,
                        })
                    })
                }
            }))
            .await
            .map_err(|error| Box::new(error) as Box<dyn Error + Send + Sync>)
    }

    fn local_channel(&self) -> Result<Channel, Status> {
        let mut state = self
            .connection
            .lock()
            .expect("backend connection lock poisoned");
        if let Some(channel) = state.channel.as_ref() {
            return Ok(channel.clone());
        }
        if state.closed {
            return Err(Status::unavailable("backend is closed"));
        }
        let channel = self.endpoint()?.connect_lazy();
        state.channel = Some(channel.clone());
        Ok(channel)
    }

    fn close(&self) {
        let mut state = self
            .connection
            .lock()
            .expect("backend connection lock poisoned");
        state.closed = true;
        state.channel = None;
        state.phase = ConnectionPhase::Idle;
        if let Some(task) = state.reconnect_task.take() {
            task.abort();
        }
        self.connection_changed.notify_waiters();
    }
}

/// A local or remote raw gRPC backend, optionally decorated with broadcast
/// response metadata.
#[derive(Clone, Debug)]
pub struct Backend {
    inner: Arc<BackendInner>,
    machine: Option<MachineTarget>,
}

impl Backend {
    #[must_use]
    pub fn local(socket_path: impl AsRef<str>) -> Self {
        let target = format!("unix://{}", socket_path.as_ref());
        Self {
            inner: Arc::new(BackendInner::new(BackendKind::Local, target, None)),
            machine: None,
        }
    }

    pub fn remote(address: &str, port: u16) -> Result<Self, Status> {
        let target = MachineTarget::new("", "", address);
        Self::remote_target(&target, port)
    }

    pub(crate) fn remote_target(target: &MachineTarget, port: u16) -> Result<Self, Status> {
        let Some(address) = target.remote_address.as_ref() else {
            return Err(Status::internal(format!(
                "address must be a valid IPv6 address: {}",
                target.address()
            )));
        };
        let zone = if address.zone.is_empty() {
            String::new()
        } else {
            format!("%{}", String::from_utf8_lossy(&address.zone))
        };
        let remote = RemoteSocket {
            ip: address.ip,
            zone: address.zone.clone(),
            port,
        };
        Ok(Self {
            inner: Arc::new(BackendInner::new(
                BackendKind::Remote,
                format!("[{}{zone}]:{port}", address.ip),
                Some(remote),
            )),
            machine: None,
        })
    }

    pub(crate) fn with_machine(&self, machine: MachineTarget) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            machine: Some(machine),
        }
    }

    #[must_use]
    pub fn is_local(&self) -> bool {
        self.inner.kind == BackendKind::Local
    }

    #[must_use]
    pub fn target(&self) -> &str {
        &self.inner.target
    }

    #[must_use]
    pub fn machine(&self) -> Option<&MachineTarget> {
        self.machine.as_ref()
    }

    #[must_use]
    pub fn outgoing_metadata(
        &self,
        incoming: &MetadataMap,
        authority: Option<&str>,
    ) -> MetadataMap {
        let mut outgoing = incoming.clone();
        if self.inner.kind == BackendKind::Remote {
            outgoing.remove("machine");
            outgoing.remove("machines");
            let authority = authority
                .and_then(|value| AsciiMetadataValue::try_from(value).ok())
                .unwrap_or_else(|| AsciiMetadataValue::from_static("unknown"));
            outgoing.insert("proxy-authority", authority);
        }
        outgoing
    }

    pub(crate) fn append_info(
        &self,
        streaming: bool,
        response: &[u8],
    ) -> Result<Vec<u8>, PayloadError> {
        match &self.machine {
            Some(machine) => append_machine_info(machine, streaming, response),
            None => Ok(response.to_vec()),
        }
    }

    pub(crate) fn build_error(
        &self,
        streaming: bool,
        error: &(dyn Error + 'static),
    ) -> Result<Option<Vec<u8>>, PayloadError> {
        self.machine
            .as_ref()
            .map(|machine| build_machine_error(machine, streaming, error))
            .transpose()
    }

    pub(crate) async fn channel(&self) -> Result<Channel, Status> {
        if self.inner.kind == BackendKind::Local {
            return self.inner.local_channel();
        }

        let generation = loop {
            let changed = self.inner.connection_changed.notified();
            let action = {
                let mut state = self
                    .inner
                    .connection
                    .lock()
                    .expect("backend connection lock poisoned");
                if let Some(channel) = state.channel.as_ref() {
                    return Ok(channel.clone());
                }
                if state.closed {
                    return Err(Status::unavailable("backend is closed"));
                }
                match state.phase {
                    ConnectionPhase::Idle => {
                        state.phase = ConnectionPhase::InitialConnect;
                        state.generation = state.generation.wrapping_add(1);
                        Some(state.generation)
                    }
                    ConnectionPhase::InitialConnect => None,
                    ConnectionPhase::Backoff => {
                        return Err(Status::unavailable(format!(
                            "backend {} is reconnecting",
                            self.target()
                        )));
                    }
                }
            };
            if let Some(generation) = action {
                break generation;
            }
            changed.await;
        };

        let mut connect_guard = InitialConnectGuard::new(Arc::clone(&self.inner));
        match self.inner.connect(generation).await {
            Ok(channel) => {
                let mut state = self
                    .inner
                    .connection
                    .lock()
                    .expect("backend connection lock poisoned");
                if state.closed {
                    drop(state);
                    drop(channel);
                    return Err(Status::unavailable("backend is closed"));
                }
                if state.generation != generation || state.phase != ConnectionPhase::InitialConnect
                {
                    drop(state);
                    drop(channel);
                    return Err(Status::unavailable("backend connection was superseded"));
                }
                state.phase = ConnectionPhase::Idle;
                state.channel = Some(channel.clone());
                connect_guard.disarm();
                self.inner.connection_changed.notify_waiters();
                Ok(channel)
            }
            Err(error) => {
                let should_reconnect = {
                    let mut state = self
                        .inner
                        .connection
                        .lock()
                        .expect("backend connection lock poisoned");
                    if !state.closed
                        && state.generation == generation
                        && state.phase == ConnectionPhase::InitialConnect
                    {
                        state.phase = ConnectionPhase::Backoff;
                        true
                    } else {
                        false
                    }
                };
                connect_guard.disarm();
                if should_reconnect {
                    self.inner.connection_changed.notify_waiters();
                    start_reconnecting(Arc::clone(&self.inner));
                }
                Err(Status::unavailable(error.to_string()))
            }
        }
    }

    pub(crate) fn close(&self) {
        self.inner.close();
    }
}

fn start_reconnecting(inner: Arc<BackendInner>) {
    let weak = Arc::downgrade(&inner);
    let (start_tx, start_rx) = tokio::sync::oneshot::channel();
    let task = tokio::spawn(async move {
        if start_rx.await.is_err() {
            return;
        }
        let mut attempt = 0;
        loop {
            tokio::time::sleep(remote_backoff(attempt)).await;
            let Some(inner) = weak.upgrade() else {
                return;
            };
            let generation = {
                let mut state = inner
                    .connection
                    .lock()
                    .expect("backend connection lock poisoned");
                if state.closed {
                    return;
                }
                state.generation = state.generation.wrapping_add(1);
                state.generation
            };
            if let Ok(channel) = inner.connect(generation).await {
                let installed = {
                    let mut state = inner
                        .connection
                        .lock()
                        .expect("backend connection lock poisoned");
                    if state.closed {
                        state.reconnect_task = None;
                        false
                    } else if state.generation == generation {
                        state.channel = Some(channel.clone());
                        state.phase = ConnectionPhase::Idle;
                        state.reconnect_task = None;
                        inner.connection_changed.notify_waiters();
                        true
                    } else {
                        false
                    }
                };
                drop(channel);
                if installed {
                    return;
                }
            }
            attempt = attempt.saturating_add(1);
        }
    });
    let mut state = inner
        .connection
        .lock()
        .expect("backend connection lock poisoned");
    if state.closed {
        task.abort();
        state.phase = ConnectionPhase::Idle;
    } else if state.reconnect_task.is_none() {
        state.reconnect_task = Some(task);
        drop(state);
        let _ = start_tx.send(());
    } else {
        task.abort();
    }
}

struct TrackedTcp {
    stream: tokio::net::TcpStream,
    owner: std::sync::Weak<BackendInner>,
    generation: u64,
}

impl AsyncRead for TrackedTcp {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_read(context, buffer)
    }
}

impl AsyncWrite for TrackedTcp {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.stream).poll_write(context, buffer)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_flush(context)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.stream).poll_shutdown(context)
    }
}

impl Drop for TrackedTcp {
    fn drop(&mut self) {
        let Some(inner) = self.owner.upgrade() else {
            return;
        };
        let (stale_channel, should_reconnect) = {
            let mut state = inner
                .connection
                .lock()
                .expect("backend connection lock poisoned");
            if state.closed || state.generation != self.generation {
                return;
            }
            state.generation = state.generation.wrapping_add(1);
            state.phase = ConnectionPhase::Backoff;
            (state.channel.take(), state.reconnect_task.is_none())
        };
        inner.connection_changed.notify_waiters();
        drop(stale_channel);
        if should_reconnect && tokio::runtime::Handle::try_current().is_ok() {
            start_reconnecting(inner);
        }
    }
}

impl RemoteSocket {
    fn endpoint_uri(&self) -> String {
        if self.zone.is_empty() {
            return format!("http://[{}]:{}", self.ip, self.port);
        }
        let mut zone = String::from("%25");
        for byte in &self.zone {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
                zone.push(char::from(*byte));
            } else {
                use std::fmt::Write as _;
                write!(zone, "%{byte:02X}").expect("writing to String cannot fail");
            }
        }
        format!("http://[{}{zone}]:{}", self.ip, self.port)
    }

    fn socket_addr(&self) -> Result<SocketAddr, std::io::Error> {
        let scope_id = match std::str::from_utf8(&self.zone)
            .ok()
            .and_then(|zone| zone.parse::<u32>().ok())
        {
            _ if self.zone.is_empty() => 0,
            Some(index) => index,
            None => interface_index(&self.zone)?,
        };
        Ok(SocketAddr::V6(SocketAddrV6::new(
            self.ip, self.port, 0, scope_id,
        )))
    }
}

fn interface_index(zone: &[u8]) -> Result<u32, std::io::Error> {
    let name = CString::new(zone)
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid IPv6 zone"))?;
    // SAFETY: `name` is a live NUL-terminated byte string and `if_nametoindex`
    // does not retain its pointer. The symbol and ABI are shared by Linux and macOS.
    let index = unsafe { if_nametoindex(name.as_ptr().cast()) };
    if index == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(index)
    }
}

unsafe extern "C" {
    fn if_nametoindex(interface_name: *const core::ffi::c_char) -> core::ffi::c_uint;
}

struct InitialConnectGuard {
    inner: Arc<BackendInner>,
    armed: bool,
}

impl InitialConnectGuard {
    fn new(inner: Arc<BackendInner>) -> Self {
        Self { inner, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for InitialConnectGuard {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let mut state = self
            .inner
            .connection
            .lock()
            .expect("backend connection lock poisoned");
        if state.phase == ConnectionPhase::InitialConnect {
            state.phase = ConnectionPhase::Idle;
            drop(state);
            self.inner.connection_changed.notify_waiters();
        }
    }
}

fn remote_backoff(retries: u32) -> Duration {
    if retries == 0 {
        return REMOTE_INITIAL_BACKOFF;
    }
    let base = REMOTE_INITIAL_BACKOFF.as_secs_f64()
        * 1.6_f64.powi(i32::try_from(retries).unwrap_or(i32::MAX));
    let capped = base.min(REMOTE_MAX_BACKOFF.as_secs_f64());
    let jitter = 1.0 + 0.2 * (fastrand::f64() * 2.0 - 1.0);
    Duration::from_secs_f64(capped * jitter)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn cancelled_initial_connect_restores_idle_and_wakes_waiters() {
        let inner = Arc::new(BackendInner::new(
            BackendKind::Remote,
            "[::1]:1".to_owned(),
            Some(RemoteSocket {
                ip: Ipv6Addr::LOCALHOST,
                zone: Vec::new(),
                port: 1,
            }),
        ));
        inner.connection.lock().unwrap().phase = ConnectionPhase::InitialConnect;
        let notified = inner.connection_changed.notified();

        drop(InitialConnectGuard::new(Arc::clone(&inner)));

        tokio::time::timeout(Duration::from_millis(50), notified)
            .await
            .expect("connection waiter was not notified");
        assert_eq!(
            inner.connection.lock().unwrap().phase,
            ConnectionPhase::Idle
        );
    }

    #[test]
    fn numeric_scoped_ipv6_target_preserves_scope_for_dialing() {
        let backend = Backend::remote("fe80::1%7", 8080).unwrap();
        assert_eq!(backend.target(), "[fe80::1%7]:8080");
        assert_eq!(
            backend.inner.endpoint().unwrap().uri().to_string(),
            "http://[fe80::1%257]:8080/"
        );
        let socket = backend
            .inner
            .remote
            .as_ref()
            .unwrap()
            .socket_addr()
            .unwrap();
        let SocketAddr::V6(socket) = socket else {
            panic!("expected IPv6 socket");
        };
        assert_eq!(socket.ip(), &"fe80::1".parse::<Ipv6Addr>().unwrap());
        assert_eq!(socket.scope_id(), 7);
    }
}
