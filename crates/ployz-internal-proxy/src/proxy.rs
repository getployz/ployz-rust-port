use std::error::Error;
use std::fmt;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::panic::resume_unwind;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpSocket;
use tokio::sync::{OwnedSemaphorePermit, Semaphore};
use tokio::task::{JoinError, JoinSet};
use tokio::time::{Instant, sleep_until};
use tokio_util::sync::CancellationToken;

use crate::{ConnectionClosed, ListenerAddress, ProxyListener};

const DIAL_TIMEOUT: Duration = Duration::from_secs(10);

/// Default maximum number of accepted connections with live handler tasks.
pub const DEFAULT_MAX_CONNECTIONS: usize = 256;

/// A full-duplex asynchronous byte stream usable by the proxy.
pub trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send + 'static {}

impl<T> AsyncStream for T where T: AsyncRead + AsyncWrite + Unpin + Send + 'static {}

/// Type-erased asynchronous stream used by listeners and custom dialers.
pub type BoxStream = Box<dyn AsyncStream>;

/// Future returned by a custom dialer.
pub type DialFuture = Pin<Box<dyn Future<Output = io::Result<BoxStream>> + Send + 'static>>;

/// Cancellation and deadline information supplied to a proxy dialer.
#[derive(Clone)]
pub struct DialContext {
    token: CancellationToken,
    deadline: Instant,
}

impl DialContext {
    fn with_timeout(parent: &CancellationToken, timeout: Duration) -> Self {
        Self {
            token: parent.child_token(),
            deadline: Instant::now() + timeout,
        }
    }

    /// Returns a token cancelled when the run ends or the dial deadline elapses.
    #[must_use]
    pub fn cancellation_token(&self) -> CancellationToken {
        self.token.clone()
    }

    /// Reports whether the run ended or the dial deadline elapsed.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.token.is_cancelled()
    }

    /// Returns the absolute dial deadline.
    #[must_use]
    pub fn deadline(&self) -> Instant {
        self.deadline
    }

    /// Returns the time remaining before the dial deadline.
    #[must_use]
    pub fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
    }

    /// Resolves when the run ends or the dial deadline elapses.
    pub async fn cancelled(&self) {
        self.token.cancelled().await;
    }
}

/// Establishes outbound connections for a proxy.
pub trait Dialer: Send + Sync + 'static {
    /// Starts one nonblocking dial operation.
    ///
    /// The returned future must own all work and resources for the operation.
    /// Dropping it must prevent any later connection or side effect; it must not
    /// detach tasks, start unmanaged threads, or perform blocking work in `poll`.
    fn dial(&self, context: DialContext, network: &'static str, address: String) -> DialFuture;
}

impl<F> Dialer for F
where
    F: Fn(DialContext, &'static str, String) -> DialFuture + Send + Sync + 'static,
{
    fn dial(&self, context: DialContext, network: &'static str, address: String) -> DialFuture {
        self(context, network, address)
    }
}

/// The default direct TCP dialer.
#[derive(Clone, Copy, Debug, Default)]
pub struct TcpDialer;

impl Dialer for TcpDialer {
    fn dial(&self, context: DialContext, network: &'static str, address: String) -> DialFuture {
        Box::pin(async move {
            if network != "tcp" {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unsupported network type: {network}"),
                ));
            }
            let address: SocketAddr = address.parse().map_err(|source| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("remote address must be numeric: {source}"),
                )
            })?;
            let socket = if address.is_ipv4() {
                TcpSocket::new_v4()?
            } else {
                TcpSocket::new_v6()?
            };
            tokio::select! {
                result = socket.connect(address) => {
                    result.map(|stream| Box::new(stream) as BoxStream)
                }
                () = context.cancelled() => Err(dial_context_error(&context)),
            }
        })
    }
}

/// An error returned from the accept loop or reported for one connection.
#[derive(Debug)]
pub enum ProxyError {
    /// Accepting a local connection failed.
    Accept(io::Error),
    /// Connecting to the remote address failed.
    Connect {
        /// The requested remote address.
        address: String,
        /// The dial failure.
        source: io::Error,
    },
    /// Copying bytes in either direction failed.
    DataCopy(io::Error),
}

impl fmt::Display for ProxyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Accept(source) => write!(formatter, "accept local connection: {source}"),
            Self::Connect { address, source } => {
                write!(formatter, "connect remote address '{address}': {source}")
            }
            Self::DataCopy(source) => write!(formatter, "data copy: {source}"),
        }
    }
}

impl Error for ProxyError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Accept(source) | Self::DataCopy(source) => Some(source),
            Self::Connect { source, .. } => Some(source),
        }
    }
}

/// Reports whether an error indicates routine connection shutdown by either peer.
#[must_use]
pub fn is_connection_closed_error(mut error: &(dyn Error + 'static)) -> bool {
    loop {
        if error.is::<ConnectionClosed>() {
            return true;
        }
        if let Some(error) = error.downcast_ref::<io::Error>()
            && matches!(
                error.kind(),
                io::ErrorKind::BrokenPipe | io::ErrorKind::ConnectionReset
            )
        {
            return true;
        }
        let Some(source) = error.source() else {
            return false;
        };
        error = source;
    }
}

type ErrorHandler = dyn Fn(ProxyError) + Send + Sync;

/// Proxies local connections to one remote TCP address.
pub struct Proxy {
    listener: Arc<dyn ProxyListener>,
    remote_address: String,
    dialer: Arc<dyn Dialer>,
    on_error: Option<Arc<ErrorHandler>>,
    max_connections: usize,
}

impl Proxy {
    /// Creates a proxy using the direct TCP dialer.
    #[must_use]
    pub fn new(listener: impl ProxyListener, remote_address: impl Into<String>) -> Self {
        Self {
            listener: Arc::new(listener),
            remote_address: remote_address.into(),
            dialer: Arc::new(TcpDialer),
            on_error: None,
            max_connections: DEFAULT_MAX_CONNECTIONS,
        }
    }

    /// Replaces the direct TCP dialer.
    #[must_use]
    pub fn with_dialer(mut self, dialer: impl Dialer) -> Self {
        self.dialer = Arc::new(dialer);
        self
    }

    /// Installs the concurrently callable per-connection error handler.
    #[must_use]
    pub fn with_error_handler(
        mut self,
        handler: impl Fn(ProxyError) + Send + Sync + 'static,
    ) -> Self {
        self.on_error = Some(Arc::new(handler));
        self
    }

    /// Sets the maximum number of accepted connections with live handler tasks.
    ///
    /// The proxy acquires capacity before accepting, so excess clients remain in
    /// the operating-system listener backlog. `max_connections` must be nonzero.
    #[must_use]
    pub fn with_max_connections(mut self, max_connections: usize) -> Self {
        assert!(max_connections != 0, "max_connections must be nonzero");
        self.max_connections = max_connections;
        self
    }

    /// Returns the local listener address, including after the proxy stops.
    #[must_use]
    pub fn local_addr(&self) -> ListenerAddress {
        self.listener.local_addr()
    }

    /// Returns the remote address passed unchanged to every dial.
    #[must_use]
    pub fn remote_addr(&self) -> &str {
        &self.remote_address
    }

    /// Runs until cancellation or listener failure.
    ///
    /// Cancellation returns `Ok(())`. A listener failure is returned after all
    /// accepted connections have stopped. Individual connection failures are
    /// reported to the error handler and do not stop acceptance.
    pub async fn run(&self, parent: &CancellationToken) -> Result<(), ProxyError> {
        let context = parent.child_token();
        let capacity = Arc::new(Semaphore::new(self.max_connections));
        let mut handlers = JoinSet::new();
        let mut run_error = None;
        let mut panic = None;

        loop {
            let permit = tokio::select! {
                () = context.cancelled() => break,
                result = handlers.join_next(), if !handlers.is_empty() => {
                    record_task_result(result, &mut panic);
                    if panic.is_some() {
                        context.cancel();
                        break;
                    }
                    continue;
                }
                permit = Arc::clone(&capacity).acquire_owned() => {
                    permit.expect("proxy capacity semaphore is never closed")
                }
            };

            let connection = tokio::select! {
                () = context.cancelled() => {
                    drop(permit);
                    break;
                }
                result = handlers.join_next(), if !handlers.is_empty() => {
                    drop(permit);
                    record_task_result(result, &mut panic);
                    if panic.is_some() {
                        context.cancel();
                        break;
                    }
                    continue;
                }
                result = self.listener.accept() => {
                    match result {
                        Ok(connection) => connection,
                        Err(source) => {
                            drop(permit);
                            if !context.is_cancelled() {
                                run_error = Some(ProxyError::Accept(source));
                            }
                            context.cancel();
                            break;
                        }
                    }
                }
            };

            let handler_context = context.clone();
            let dialer = Arc::clone(&self.dialer);
            let remote_address = self.remote_address.clone();
            let on_error = self.on_error.clone();
            handlers.spawn(async move {
                handle_connection(
                    handler_context,
                    connection,
                    dialer,
                    remote_address,
                    on_error,
                    permit,
                )
                .await;
            });
        }

        context.cancel();
        let _ = self.listener.close();
        while let Some(result) = handlers.join_next().await {
            record_join_result(result, &mut panic);
        }
        if let Some(panic) = panic {
            resume_unwind(panic.into_panic());
        }
        run_error.map_or(Ok(()), Err)
    }
}

fn record_task_result(result: Option<Result<(), JoinError>>, panic: &mut Option<JoinError>) {
    if let Some(result) = result {
        record_join_result(result, panic);
    }
}

fn record_join_result(result: Result<(), JoinError>, panic: &mut Option<JoinError>) {
    if let Err(error) = result
        && error.is_panic()
        && panic.is_none()
    {
        *panic = Some(error);
    }
}

async fn handle_connection(
    context: CancellationToken,
    local: BoxStream,
    dialer: Arc<dyn Dialer>,
    remote_address: String,
    on_error: Option<Arc<ErrorHandler>>,
    _permit: OwnedSemaphorePermit,
) {
    let dial_context = DialContext::with_timeout(&context, DIAL_TIMEOUT);
    let deadline = dial_context.deadline();
    let dial = dialer.dial(dial_context.clone(), "tcp", remote_address.clone());
    let remote = tokio::select! {
        () = context.cancelled() => {
            dial_context.token.cancel();
            return;
        }
        () = sleep_until(deadline) => {
            dial_context.token.cancel();
            report_connect_error(
                &context,
                on_error.as_deref(),
                remote_address,
                io::Error::new(io::ErrorKind::TimedOut, "context deadline exceeded"),
            );
            return;
        }
        result = dial => {
            dial_context.token.cancel();
            match result {
                Ok(connection) => connection,
                Err(source) => {
                    report_connect_error(
                        &context,
                        on_error.as_deref(),
                        remote_address,
                        source,
                    );
                    return;
                }
            }
        }
    };

    let mut local = IgnoreShutdown(local);
    let mut remote = IgnoreShutdown(remote);
    let result = tokio::select! {
        () = context.cancelled() => None,
        result = tokio::io::copy_bidirectional(&mut local, &mut remote) => Some(result),
    };
    drop(local);
    drop(remote);

    if let Some(Err(source)) = result
        && !context.is_cancelled()
        && let Some(on_error) = on_error
    {
        on_error(ProxyError::DataCopy(source));
    }
}

fn report_connect_error(
    context: &CancellationToken,
    on_error: Option<&ErrorHandler>,
    remote_address: String,
    source: io::Error,
) {
    if !context.is_cancelled()
        && let Some(on_error) = on_error
    {
        on_error(ProxyError::Connect {
            address: remote_address,
            source,
        });
    }
}

fn dial_context_error(context: &DialContext) -> io::Error {
    if Instant::now() >= context.deadline() {
        io::Error::new(io::ErrorKind::TimedOut, "context deadline exceeded")
    } else {
        io::Error::new(io::ErrorKind::Interrupted, "dial cancelled")
    }
}

struct IgnoreShutdown<T>(T);

impl<T: AsyncRead + Unpin> AsyncRead for IgnoreShutdown<T> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(context, buffer)
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for IgnoreShutdown<T> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<Result<usize, io::Error>> {
        Pin::new(&mut self.0).poll_write(context, buffer)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        Pin::new(&mut self.0).poll_flush(context)
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Result<(), io::Error>> {
        match Pin::new(&mut self.0).poll_shutdown(context) {
            Poll::Pending => Poll::Pending,
            Poll::Ready(_) => Poll::Ready(Ok(())),
        }
    }

    fn is_write_vectored(&self) -> bool {
        self.0.is_write_vectored()
    }

    fn poll_write_vectored(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffers: &[io::IoSlice<'_>],
    ) -> Poll<Result<usize, io::Error>> {
        Pin::new(&mut self.0).poll_write_vectored(context, buffers)
    }
}
