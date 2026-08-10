use std::future::Future;
use std::io;
use std::net::SocketAddr;
#[cfg(unix)]
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::{TcpListener as TokioTcpListener, TcpStream, ToSocketAddrs};
#[cfg(unix)]
use tokio::net::{UnixListener as TokioUnixListener, UnixStream};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::error::{anyhow_context, context};

pub trait Connection: AsyncRead + AsyncWrite + Unpin + Send {
    // halfCloser is an interface for connections that support half-close.
    fn supports_half_close(&self) -> bool;
}

impl Connection for TcpStream {
    fn supports_half_close(&self) -> bool {
        true
    }
}

#[cfg(unix)]
impl Connection for UnixStream {
    fn supports_half_close(&self) -> bool {
        true
    }
}

#[derive(Debug)]
struct ConnectionClosed;

impl std::fmt::Display for ConnectionClosed {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("use of closed network connection")
    }
}

impl std::error::Error for ConnectionClosed {}

#[derive(Debug)]
struct ClosedPipe;

impl std::fmt::Display for ClosedPipe {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("closed pipe")
    }
}

impl std::error::Error for ClosedPipe {}

#[derive(Debug)]
struct ContextDeadlineExceeded;

impl std::fmt::Display for ContextDeadlineExceeded {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("context deadline exceeded")
    }
}

impl std::error::Error for ContextDeadlineExceeded {}

fn connection_closed_error() -> io::Error {
    io::Error::new(io::ErrorKind::NotConnected, ConnectionClosed)
}

fn closed_pipe_error() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, ClosedPipe)
}

#[async_trait]
pub trait Listener: Send + Sync {
    /// Wait for the next connection. `close` must unblock a pending call.
    async fn accept(&self) -> io::Result<Box<dyn Connection>>;
    fn close(&self) -> io::Result<()>;
}

pub struct TcpListener {
    inner: Mutex<Option<Arc<TokioTcpListener>>>,
    closed: CancellationToken,
}

impl TcpListener {
    pub async fn bind(address: impl ToSocketAddrs) -> io::Result<Self> {
        Ok(Self {
            inner: Mutex::new(Some(Arc::new(TokioTcpListener::bind(address).await?))),
            closed: CancellationToken::new(),
        })
    }

    pub fn local_addr(&self) -> io::Result<SocketAddr> {
        self.inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .ok_or_else(connection_closed_error)?
            .local_addr()
    }
}

#[async_trait]
impl Listener for TcpListener {
    async fn accept(&self) -> io::Result<Box<dyn Connection>> {
        let listener = self
            .inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .map(Arc::clone)
            .ok_or_else(connection_closed_error)?;
        tokio::select! {
            result = listener.accept() => {
                let (stream, _) = result?;
                Ok(Box::new(stream))
            }
            () = self.closed.cancelled() => Err(connection_closed_error()),
        }
    }

    fn close(&self) -> io::Result<()> {
        self.closed.cancel();
        self.inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        Ok(())
    }
}

#[cfg(unix)]
pub struct UnixListener {
    inner: Mutex<Option<Arc<TokioUnixListener>>>,
    closed: CancellationToken,
    path: PathBuf,
}

#[cfg(unix)]
impl UnixListener {
    pub fn bind(path: impl AsRef<std::path::Path>) -> io::Result<Self> {
        let path = path.as_ref().to_owned();
        Ok(Self {
            inner: Mutex::new(Some(Arc::new(TokioUnixListener::bind(&path)?))),
            closed: CancellationToken::new(),
            path,
        })
    }
}

#[cfg(unix)]
#[async_trait]
impl Listener for UnixListener {
    async fn accept(&self) -> io::Result<Box<dyn Connection>> {
        let listener = self
            .inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .as_ref()
            .map(Arc::clone)
            .ok_or_else(connection_closed_error)?;
        tokio::select! {
            result = listener.accept() => {
                let (stream, _) = result?;
                Ok(Box::new(stream))
            }
            () = self.closed.cancelled() => Err(connection_closed_error()),
        }
    }

    fn close(&self) -> io::Result<()> {
        self.closed.cancel();
        self.inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take();
        match std::fs::remove_file(&self.path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }
}

pub type DialContext = Arc<
    dyn Fn(
            CancellationToken,
            String,
            String,
        ) -> Pin<Box<dyn Future<Output = Result<Box<dyn Connection>>> + Send>>
        + Send
        + Sync,
>;

pub type OnError = Arc<dyn Fn(anyhow::Error) + Send + Sync>;

/// Proxy proxies local connections to a remote TCP address optionally using a custom dialer.
pub struct Proxy {
    pub listener: Arc<dyn Listener>,
    pub remote_addr: String,
    pub dial_context: Option<DialContext>,
    // OnError is called for errors that occur during proxying individual connections. It may be called concurrently
    // for different connections.
    pub on_error: Option<OnError>,
}

/// is_connection_closed_error reports whether err indicates that a connection was closed or aborted by either peer.
/// Callers can use it to ignore routine connection shutdown or broken pipe errors reported to Proxy.OnError.
pub fn is_connection_closed_error(err: &anyhow::Error) -> bool {
    err.chain().any(|cause| {
        cause.is::<ConnectionClosed>()
            || cause.is::<ClosedPipe>()
            || cause.downcast_ref::<io::Error>().is_some_and(|err| {
                matches!(
                    err.raw_os_error(),
                    Some(nix::libc::EPIPE) | Some(nix::libc::ECONNRESET)
                ) || err.get_ref().is_some_and(|source| {
                    source.is::<ConnectionClosed>() || source.is::<ClosedPipe>()
                })
            })
    })
}

impl Proxy {
    /// run starts the proxy and runs until the context is canceled or the listener fails. It returns nil when the context
    /// is canceled. Errors handling individual connections are reported to OnError and do not stop the proxy.
    pub async fn run(&mut self, parent_ctx: CancellationToken) -> Result<()> {
        let dial_context = if let Some(dial_context) = &self.dial_context {
            Arc::clone(dial_context)
        } else {
            let dial_context: DialContext = Arc::new(|ctx, _network, address| {
                Box::pin(async move {
                    let stream = tokio::select! {
                        result = TcpStream::connect(address) => result.map_err(anyhow::Error::new)?,
                        () = ctx.cancelled() => return Err(anyhow::Error::msg("operation canceled")),
                    };
                    Ok(Box::new(stream) as Box<dyn Connection>)
                })
            });
            self.dial_context = Some(Arc::clone(&dial_context));
            dial_context
        };

        let ctx = parent_ctx.child_token();
        let mut active_conns = JoinSet::new();

        // Closing the listener unblocks Accept when the context is canceled. This works for both TCP and Unix listeners
        // and avoids polling with listener deadlines.
        let close_ctx = ctx.clone();
        let close_listener = Arc::clone(&self.listener);
        let stop_close = tokio::spawn(async move {
            close_ctx.cancelled().await;
            let _ = close_listener.close();
        });

        let mut run_err = None;
        loop {
            let conn = match self.listener.accept().await {
                Ok(conn) => conn,
                Err(err) => {
                    if ctx.is_cancelled() {
                        break;
                    }

                    run_err = Some(context("accept local connection", err));
                    ctx.cancel();
                    break;
                }
            };

            let connection = self.handle_connection(ctx.clone(), conn, Arc::clone(&dial_context));
            active_conns.spawn(connection);
        }

        // Wait for all connections to finish.
        while let Some(result) = active_conns.join_next().await {
            if let Err(err) = result
                && err.is_panic()
            {
                std::panic::resume_unwind(err.into_panic());
            }
        }
        ctx.cancel();
        let _ = stop_close.await;
        let _ = self.listener.close();
        match run_err {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    fn handle_connection(
        &self,
        ctx: CancellationToken,
        local_conn: Box<dyn Connection>,
        dial_context: DialContext,
    ) -> impl Future<Output = ()> + Send + 'static {
        let remote_addr = self.remote_addr.clone();
        let on_error = self.on_error.clone();

        async move {
            // Use a separate context with timeout for dialing the remote address.
            let dial_ctx = ctx.child_token();
            let dial = dial_context(dial_ctx.clone(), "tcp".to_owned(), remote_addr.clone());
            let remote_conn = tokio::select! {
                result = tokio::time::timeout(Duration::from_secs(10), dial) => {
                    match result {
                        Ok(Ok(conn)) => conn,
                        Ok(Err(err)) => {
                            dial_ctx.cancel();
                            if !ctx.is_cancelled()
                                && let Some(on_error) = &on_error
                            {
                                on_error(anyhow_context(format!("connect remote address '{remote_addr}'"), err));
                            }
                            return;
                        }
                        Err(_) => {
                            dial_ctx.cancel();
                            if !ctx.is_cancelled()
                                && let Some(on_error) = &on_error
                            {
                                on_error(context(
                                    format!("connect remote address '{remote_addr}'"),
                                    ContextDeadlineExceeded,
                                ));
                            }
                            return;
                        }
                    }
                }
                () = ctx.cancelled() => {
                    dial_ctx.cancel();
                    return;
                },
            };
            dial_ctx.cancel();

            // Dropping both connections aborts both copies after cancellation or a copy error. A clean EOF still uses
            // half-close so the other direction can finish sending any remaining data.
            let local_half_closer = local_conn.supports_half_close();
            let remote_half_closer = remote_conn.supports_half_close();
            let (local_read, local_write) = tokio::io::split(local_conn);
            let (remote_read, remote_write) = tokio::io::split(remote_conn);
            let (done_tx, mut done_rx) = tokio::sync::mpsc::channel(2);
            let mut copies = JoinSet::new();

            let first_done = done_tx.clone();
            copies.spawn(async move {
                let mut local_read = local_read;
                let mut remote_write = remote_write;
                let result = tokio::io::copy(&mut local_read, &mut remote_write).await;
                if result.is_ok() && remote_half_closer {
                    // Close write half of remote connection if supported.
                    let _ = remote_write.shutdown().await;
                }
                let _ = first_done.send(result.map(|_| ())).await;
            });

            let second_done = done_tx.clone();
            copies.spawn(async move {
                let mut remote_read = remote_read;
                let mut local_write = local_write;
                let result = tokio::io::copy(&mut remote_read, &mut local_write).await;
                if result.is_ok() && local_half_closer {
                    // Close write half of local connection if supported.
                    let _ = local_write.shutdown().await;
                }
                let _ = second_done.send(result.map(|_| ())).await;
            });
            drop(done_tx);

            // Wait for both copies to complete. The first error is the original failure because a copy reports it before
            // closing the connections to unblock the other copy.
            let mut copy_err = None;
            for _ in 0..2 {
                let result = tokio::select! {
                    result = done_rx.recv() => result,
                    () = ctx.cancelled() => {
                        copies.abort_all();
                        break;
                    }
                };
                match result {
                    Some(Err(err)) => {
                        if copy_err.is_none() {
                            copy_err = Some(err);
                        }
                        copies.abort_all();
                    }
                    Some(Ok(())) => {}
                    None => break,
                }
            }
            while let Some(result) = copies.join_next().await {
                if let Err(err) = result
                    && err.is_panic()
                {
                    std::panic::resume_unwind(err.into_panic());
                }
            }

            if let Some(copy_err) = copy_err
                && !ctx.is_cancelled()
                && let Some(on_error) = on_error
            {
                on_error(context("data copy", copy_err));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::task::{Context, Poll};

    use anyhow::anyhow;
    use nix::libc;
    use tokio::io::{AsyncReadExt, AsyncWriteExt, DuplexStream, ReadBuf};
    use tokio::sync::{Mutex, mpsc};

    use super::*;

    #[test]
    fn is_connection_closed_error_cases() {
        let tests = [
            ("closed network connection", connection_closed_error(), true),
            ("closed pipe", closed_pipe_error(), true),
            (
                "broken pipe",
                io::Error::from_raw_os_error(libc::EPIPE),
                true,
            ),
            (
                "connection reset",
                io::Error::from_raw_os_error(libc::ECONNRESET),
                true,
            ),
            (
                "connection aborted is distinct",
                io::Error::from_raw_os_error(libc::ECONNABORTED),
                false,
            ),
            (
                "not connected is distinct",
                io::Error::from_raw_os_error(libc::ENOTCONN),
                false,
            ),
            (
                "synthetic broken-pipe kind is distinct",
                io::Error::new(io::ErrorKind::BrokenPipe, "synthetic"),
                false,
            ),
            (
                "synthetic connection-reset kind is distinct",
                io::Error::new(io::ErrorKind::ConnectionReset, "synthetic"),
                false,
            ),
            ("other error", io::Error::other("copy failed"), false),
        ];
        for (name, source, want) in tests {
            let err = anyhow::Error::new(source);
            assert_eq!(is_connection_closed_error(&err), want, "{name}");
        }

        let wrapped = context("copy data", io::Error::from_raw_os_error(libc::EPIPE));
        assert!(
            is_connection_closed_error(&wrapped),
            "wrapped connection error"
        );
        assert!(wrapped.to_string().starts_with("copy data: "));
        assert!(!is_connection_closed_error(&anyhow!("copy failed")));
    }

    #[tokio::test]
    async fn tcp_listener_accepts_and_close_unblocks_accept() {
        let listener = Arc::new(TcpListener::bind("127.0.0.1:0").await.unwrap());
        let address = listener.local_addr().unwrap();
        let client = TcpStream::connect(address).await.unwrap();
        let accepted = listener.accept().await.unwrap();
        drop(client);
        drop(accepted);

        let accepting_listener = Arc::clone(&listener);
        let accepting = tokio::spawn(async move { accepting_listener.accept().await });
        listener.close().unwrap();
        let error = match accepting.await.unwrap() {
            Ok(_) => panic!("accept unexpectedly succeeded after close"),
            Err(error) => error,
        };
        assert!(is_connection_closed_error(&anyhow::Error::new(error)));

        let rebound = TcpListener::bind(address).await.unwrap();
        rebound.close().unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unix_listener_accepts_connections() {
        static NEXT_SOCKET: AtomicU32 = AtomicU32::new(0);

        let path = std::env::temp_dir().join(format!(
            "ployz-proxy-{}-{}.sock",
            std::process::id(),
            NEXT_SOCKET.fetch_add(1, Ordering::Relaxed)
        ));
        let listener = UnixListener::bind(&path).unwrap();
        let client = UnixStream::connect(&path).await.unwrap();
        let accepted = listener.accept().await.unwrap();
        drop(client);
        drop(accepted);
        listener.close().unwrap();
        assert!(!path.exists());

        let rebound = UnixListener::bind(&path).unwrap();
        rebound.close().unwrap();
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn run_continues_after_closed_connection_error() {
        let listener = Arc::new(TestListener::new());
        let ctx = CancellationToken::new();
        let (closed_err_tx, mut closed_err_rx) = mpsc::channel(1);
        let (unexpected_err_tx, mut unexpected_err_rx) = mpsc::channel(1);
        let dial_count = Arc::new(AtomicU32::new(0));

        let dial_context: DialContext = Arc::new(move |_, _, _| {
            let dial_count = Arc::clone(&dial_count);
            Box::pin(async move {
                if dial_count.fetch_add(1, Ordering::SeqCst) == 0 {
                    return Ok(Box::new(ReadErrorConn) as Box<dyn Connection>);
                }

                let (proxy_conn, mut upstream_conn) = tokio::io::duplex(64);
                tokio::spawn(async move {
                    let _ = upstream_conn.write_all(b"ok").await;
                });
                Ok(Box::new(proxy_conn) as Box<dyn Connection>)
            })
        });
        let on_error: OnError = Arc::new(move |err| {
            if is_connection_closed_error(&err) {
                let _ = closed_err_tx.try_send(err);
                return;
            }
            let _ = unexpected_err_tx.try_send(err);
        });
        let mut proxy = Proxy {
            listener: listener.clone(),
            remote_addr: "remote:80".to_owned(),
            dial_context: Some(dial_context),
            on_error: Some(on_error),
        };
        let run_ctx = ctx.clone();
        let run_task = tokio::spawn(async move { proxy.run(run_ctx).await });

        let first_conn = listener.connect();
        let conn_err = tokio::time::timeout(Duration::from_secs(1), closed_err_rx.recv())
            .await
            .expect("timed out waiting for closed connection error")
            .expect("closed error channel closed");
        let copy_error = conn_err
            .chain()
            .find_map(|cause| cause.downcast_ref::<io::Error>())
            .expect("copy error chain contains io::Error");
        assert_eq!(copy_error.raw_os_error(), Some(libc::EPIPE));
        assert!(conn_err.to_string().starts_with("data copy: "));
        drop(first_conn);

        let mut second_conn = listener.connect();
        let mut got = [0; 2];
        tokio::time::timeout(Duration::from_secs(1), second_conn.read_exact(&mut got))
            .await
            .expect("timed out reading second connection")
            .expect("read second connection");
        assert_eq!(&got, b"ok");
        assert!(unexpected_err_rx.try_recv().is_err());

        ctx.cancel();
        tokio::time::timeout(Duration::from_secs(1), run_task)
            .await
            .expect("timed out waiting for proxy to stop")
            .expect("run task panicked")
            .expect("proxy run failed");
    }

    #[tokio::test]
    async fn run_returns_listener_error() {
        let listener = Arc::new(ErrorListener);
        let mut proxy = Proxy {
            listener,
            remote_addr: String::new(),
            dial_context: None,
            on_error: None,
        };
        let err = proxy
            .run(CancellationToken::new())
            .await
            .expect_err("listener error expected");
        assert_eq!(err.to_string(), "accept local connection: listener failed");
        let listener_error = err
            .chain()
            .find_map(|cause| cause.downcast_ref::<io::Error>())
            .expect("listener io::Error remains in the source chain");
        assert!(
            listener_error
                .get_ref()
                .is_some_and(|source| source.is::<ListenerFailure>())
        );
    }

    #[tokio::test(start_paused = true)]
    async fn dial_timeout_cancels_token_and_reports_context_deadline() {
        let listener = Arc::new(TestListener::new());
        let ctx = CancellationToken::new();
        let (dial_token_tx, mut dial_token_rx) = mpsc::unbounded_channel();
        let (error_tx, mut error_rx) = mpsc::unbounded_channel();
        let dial_context: DialContext = Arc::new(move |dial_ctx, _, _| {
            dial_token_tx.send(dial_ctx.clone()).unwrap();
            Box::pin(std::future::pending())
        });
        let mut proxy = Proxy {
            listener: listener.clone(),
            remote_addr: "remote:80".to_owned(),
            dial_context: Some(dial_context),
            on_error: Some(Arc::new(move |error| {
                error_tx.send(error).unwrap();
            })),
        };
        let run_ctx = ctx.clone();
        let run_task = tokio::spawn(async move { proxy.run(run_ctx).await });
        let client = listener.connect();
        let dial_token = dial_token_rx.recv().await.unwrap();

        tokio::time::advance(Duration::from_secs(10)).await;
        let error = error_rx.recv().await.unwrap();
        assert!(dial_token.is_cancelled());
        assert_eq!(
            error.to_string(),
            "connect remote address 'remote:80': context deadline exceeded"
        );
        assert!(
            error
                .chain()
                .any(|source| source.is::<ContextDeadlineExceeded>())
        );

        drop(client);
        ctx.cancel();
        run_task.await.unwrap().unwrap();
    }

    struct TestListener {
        conns_tx: mpsc::UnboundedSender<DuplexStream>,
        conns_rx: Mutex<mpsc::UnboundedReceiver<DuplexStream>>,
        closed: CancellationToken,
    }

    impl Connection for DuplexStream {
        fn supports_half_close(&self) -> bool {
            false
        }
    }

    impl TestListener {
        fn new() -> Self {
            let (conns_tx, conns_rx) = mpsc::unbounded_channel();
            Self {
                conns_tx,
                conns_rx: Mutex::new(conns_rx),
                closed: CancellationToken::new(),
            }
        }

        fn connect(&self) -> DuplexStream {
            let (client_conn, proxy_conn) = tokio::io::duplex(64);
            self.conns_tx
                .send(proxy_conn)
                .expect("listener is not closed");
            client_conn
        }
    }

    #[async_trait]
    impl Listener for TestListener {
        async fn accept(&self) -> io::Result<Box<dyn Connection>> {
            let mut conns = self.conns_rx.lock().await;
            tokio::select! {
                conn = conns.recv() => conn
                    .map(|conn| Box::new(conn) as Box<dyn Connection>)
                    .ok_or_else(connection_closed_error),
                () = self.closed.cancelled() => Err(connection_closed_error()),
            }
        }

        fn close(&self) -> io::Result<()> {
            self.closed.cancel();
            Ok(())
        }
    }

    #[derive(Debug)]
    struct ListenerFailure;

    impl std::fmt::Display for ListenerFailure {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("listener failed")
        }
    }

    impl std::error::Error for ListenerFailure {}

    struct ErrorListener;

    #[async_trait]
    impl Listener for ErrorListener {
        async fn accept(&self) -> io::Result<Box<dyn Connection>> {
            Err(io::Error::other(ListenerFailure))
        }

        fn close(&self) -> io::Result<()> {
            Ok(())
        }
    }

    // read_error_conn fails reads immediately so tests can deterministically exercise a proxy copy failure.
    struct ReadErrorConn;

    impl Connection for ReadErrorConn {
        fn supports_half_close(&self) -> bool {
            false
        }
    }

    impl AsyncRead for ReadErrorConn {
        fn poll_read(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            _buf: &mut ReadBuf<'_>,
        ) -> Poll<io::Result<()>> {
            Poll::Ready(Err(io::Error::from_raw_os_error(libc::EPIPE)))
        }
    }

    impl AsyncWrite for ReadErrorConn {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            buf: &[u8],
        ) -> Poll<io::Result<usize>> {
            Poll::Ready(Ok(buf.len()))
        }

        fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }
}
