use std::error::Error;
use std::future::pending;
use std::io;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::Duration;

use ployz_internal_proxy::{
    AcceptFuture, AsyncStream, BoxStream, CancellationToken, ConnectionClosed, DialContext,
    DialFuture, Dialer, ListenerAddress, Proxy, ProxyError, ProxyListener, TcpProxyListener,
    is_connection_closed_error,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, oneshot};
use tokio::time::timeout;

const TEST_TIMEOUT: Duration = Duration::from_secs(3);

#[test]
fn closed_connection_error_classification_is_exact_and_recursive() {
    assert!(is_connection_closed_error(&ConnectionClosed));
    assert!(is_connection_closed_error(&io::Error::new(
        io::ErrorKind::BrokenPipe,
        "broken pipe"
    )));
    assert!(is_connection_closed_error(&io::Error::new(
        io::ErrorKind::ConnectionReset,
        "reset"
    )));
    let wrapped = ProxyError::DataCopy(io::Error::new(io::ErrorKind::BrokenPipe, "broken pipe"));
    assert!(is_connection_closed_error(&wrapped));

    for kind in [
        io::ErrorKind::NotConnected,
        io::ErrorKind::ConnectionAborted,
        io::ErrorKind::TimedOut,
        io::ErrorKind::Interrupted,
        io::ErrorKind::Other,
    ] {
        assert!(!is_connection_closed_error(&io::Error::new(kind, "other")));
    }
}

#[tokio::test]
async fn proxies_tcp_bidirectionally_and_half_closes() {
    let remote = TcpListener::bind("127.0.0.1:0").await.expect("bind remote");
    let remote_address = remote.local_addr().expect("remote address");
    let remote_task = tokio::spawn(async move {
        let (mut connection, _) = remote.accept().await.expect("accept remote");
        let mut request = Vec::new();
        connection
            .read_to_end(&mut request)
            .await
            .expect("read request");
        assert_eq!(request, b"request");
        connection.write_all(b"response").await.expect("response");
        connection.shutdown().await.expect("half-close");
    });

    let listener = TcpProxyListener::bind("127.0.0.1:0")
        .await
        .expect("bind proxy");
    let local_address = listener.socket_addr();
    let cancellation = CancellationToken::new();
    let proxy = Arc::new(Proxy::new(listener, remote_address.to_string()));
    let run_task = spawn_proxy(Arc::clone(&proxy), cancellation.clone());

    let mut client = TcpStream::connect(local_address)
        .await
        .expect("connect local");
    client.write_all(b"request").await.expect("request");
    client.shutdown().await.expect("half-close local");
    let mut response = Vec::new();
    timeout(TEST_TIMEOUT, client.read_to_end(&mut response))
        .await
        .expect("response timeout")
        .expect("read response");
    assert_eq!(response, b"response");
    remote_task.await.expect("remote task");

    cancellation.cancel();
    assert!(
        timeout(TEST_TIMEOUT, run_task)
            .await
            .expect("stop timeout")
            .expect("run task")
            .is_ok()
    );
    assert!(TcpStream::connect(local_address).await.is_err());
}

#[tokio::test]
async fn listener_failure_keeps_source_and_closes_listener() {
    let closed = Arc::new(AtomicBool::new(false));
    let listener = ErrorListener {
        source: Mutex::new(Some(io::Error::other("listener failed"))),
        closed: Arc::clone(&closed),
    };
    let proxy = Proxy::new(listener, "127.0.0.1:1");
    let error = proxy
        .run(&CancellationToken::new())
        .await
        .expect_err("listener error");
    assert_eq!(
        error.to_string(),
        "accept local connection: listener failed"
    );
    assert_eq!(
        error.source().expect("source").to_string(),
        "listener failed"
    );
    assert!(closed.load(Ordering::SeqCst));
}

#[tokio::test]
async fn individual_dial_failure_does_not_stop_acceptance() {
    let listener = TcpProxyListener::bind("127.0.0.1:0")
        .await
        .expect("bind proxy");
    let local_address = listener.socket_addr();
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_for_dial = Arc::clone(&attempts);
    let dialer = move |_: DialContext, _: &'static str, _: String| -> DialFuture {
        let attempt = attempts_for_dial.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if attempt == 0 {
                return Err(io::Error::new(io::ErrorKind::ConnectionRefused, "refused"));
            }
            let (proxy_side, mut server_side) = tokio::io::duplex(32);
            tokio::spawn(async move {
                server_side.write_all(b"ok").await.expect("write ok");
                server_side.shutdown().await.expect("shutdown");
            });
            Ok(Box::new(proxy_side) as BoxStream)
        })
    };
    let (errors_tx, mut errors_rx) = mpsc::unbounded_channel();
    let proxy = Arc::new(
        Proxy::new(listener, "unchanged.example:80")
            .with_dialer(dialer)
            .with_error_handler(move |error| errors_tx.send(error).expect("record error")),
    );
    let cancellation = CancellationToken::new();
    let run_task = spawn_proxy(proxy, cancellation.clone());

    let _first = TcpStream::connect(local_address)
        .await
        .expect("first local");
    let first_error = timeout(TEST_TIMEOUT, errors_rx.recv())
        .await
        .expect("error timeout")
        .expect("dial error");
    assert!(matches!(first_error, ProxyError::Connect { .. }));
    assert_eq!(
        first_error.to_string(),
        "connect remote address 'unchanged.example:80': refused"
    );

    let mut second = TcpStream::connect(local_address)
        .await
        .expect("second local");
    let mut response = [0; 2];
    timeout(TEST_TIMEOUT, second.read_exact(&mut response))
        .await
        .expect("read timeout")
        .expect("read ok");
    assert_eq!(&response, b"ok");
    assert_eq!(attempts.load(Ordering::SeqCst), 2);

    cancellation.cancel();
    assert!(
        timeout(TEST_TIMEOUT, run_task)
            .await
            .expect("stop timeout")
            .expect("run task")
            .is_ok()
    );
}

#[tokio::test]
async fn wrapped_copy_failure_does_not_stop_acceptance() {
    let listener = TcpProxyListener::bind("127.0.0.1:0")
        .await
        .expect("bind proxy");
    let local_address = listener.socket_addr();
    let attempts = Arc::new(AtomicUsize::new(0));
    let attempts_for_dial = Arc::clone(&attempts);
    let dialer = move |_: DialContext, _: &'static str, _: String| -> DialFuture {
        let attempt = attempts_for_dial.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            if attempt == 0 {
                return Ok(Box::new(ReadBrokenPipeStream) as BoxStream);
            }
            let (proxy_side, mut server_side) = tokio::io::duplex(32);
            tokio::spawn(async move {
                server_side.write_all(b"ok").await.expect("write ok");
                server_side.shutdown().await.expect("shutdown");
            });
            Ok(Box::new(proxy_side) as BoxStream)
        })
    };
    let (errors_tx, mut errors_rx) = mpsc::unbounded_channel();
    let proxy = Arc::new(
        Proxy::new(listener, "remote:80")
            .with_dialer(dialer)
            .with_error_handler(move |error| errors_tx.send(error).expect("record error")),
    );
    let cancellation = CancellationToken::new();
    let run_task = spawn_proxy(proxy, cancellation.clone());

    let _first = TcpStream::connect(local_address)
        .await
        .expect("first local");
    let copy_error = timeout(TEST_TIMEOUT, errors_rx.recv())
        .await
        .expect("error timeout")
        .expect("copy error");
    assert!(matches!(copy_error, ProxyError::DataCopy(_)));
    assert_eq!(copy_error.to_string(), "data copy: broken pipe");
    assert!(is_connection_closed_error(&copy_error));

    let mut second = TcpStream::connect(local_address)
        .await
        .expect("second local");
    let mut response = [0; 2];
    timeout(TEST_TIMEOUT, second.read_exact(&mut response))
        .await
        .expect("read timeout")
        .expect("read ok");
    assert_eq!(&response, b"ok");
    assert_eq!(attempts.load(Ordering::SeqCst), 2);

    cancellation.cancel();
    assert!(
        timeout(TEST_TIMEOUT, run_task)
            .await
            .expect("stop timeout")
            .expect("run task")
            .is_ok()
    );
}

#[tokio::test]
async fn custom_dialer_gets_exact_contract_and_may_return_non_tcp_stream() {
    let listener = TcpProxyListener::bind("127.0.0.1:0")
        .await
        .expect("bind proxy");
    let local_address = listener.socket_addr();
    let (observed_tx, observed_rx) = oneshot::channel();
    let observed_tx = Arc::new(Mutex::new(Some(observed_tx)));
    let dialer = {
        let observed_tx = Arc::clone(&observed_tx);
        move |context: DialContext, network: &'static str, address: String| -> DialFuture {
            observed_tx
                .lock()
                .expect("observation lock")
                .take()
                .expect("one dial")
                .send((network, address, context.remaining()))
                .expect("send observation");
            Box::pin(async move {
                let (proxy_side, mut peer) = tokio::io::duplex(32);
                tokio::spawn(async move {
                    let mut request = [0; 4];
                    peer.read_exact(&mut request).await.expect("read ping");
                    assert_eq!(&request, b"ping");
                    peer.write_all(b"pong").await.expect("write pong");
                    peer.shutdown().await.expect("shutdown peer");
                });
                Ok(Box::new(proxy_side) as BoxStream)
            })
        }
    };
    let proxy = Arc::new(Proxy::new(listener, "remote-name:80").with_dialer(dialer));
    let cancellation = CancellationToken::new();
    let run_task = spawn_proxy(proxy, cancellation.clone());
    let mut client = TcpStream::connect(local_address)
        .await
        .expect("connect local");
    client.write_all(b"ping").await.expect("write ping");
    client.shutdown().await.expect("shutdown local");
    let mut response = Vec::new();
    client.read_to_end(&mut response).await.expect("read pong");
    assert_eq!(response, b"pong");
    let (network, address, remaining) = observed_rx.await.expect("dial observation");
    assert_eq!(network, "tcp");
    assert_eq!(address, "remote-name:80");
    assert!(remaining <= Duration::from_secs(10));
    assert!(remaining > Duration::from_secs(9));

    cancellation.cancel();
    assert!(run_task.await.expect("run task").is_ok());
}

#[tokio::test]
async fn cancellation_drops_in_flight_dial_without_reporting_an_error() {
    let listener = TcpProxyListener::bind("127.0.0.1:0")
        .await
        .expect("bind proxy");
    let local_address = listener.socket_addr();
    let dropped = Arc::new(AtomicBool::new(false));
    let (started_tx, started_rx) = oneshot::channel();
    let started_tx = Arc::new(Mutex::new(Some(started_tx)));
    let dialer = {
        let dropped = Arc::clone(&dropped);
        move |_: DialContext, _: &'static str, _: String| -> DialFuture {
            started_tx
                .lock()
                .expect("started lock")
                .take()
                .expect("one dial")
                .send(())
                .expect("send started");
            let guard = DropFlag(Arc::clone(&dropped));
            Box::pin(async move {
                let _guard = guard;
                pending::<()>().await;
                unreachable!()
            })
        }
    };
    let errors = Arc::new(AtomicUsize::new(0));
    let errors_for_handler = Arc::clone(&errors);
    let proxy = Arc::new(
        Proxy::new(listener, "remote:80")
            .with_dialer(dialer)
            .with_error_handler(move |_| {
                errors_for_handler.fetch_add(1, Ordering::SeqCst);
            }),
    );
    let cancellation = CancellationToken::new();
    let run_task = spawn_proxy(proxy, cancellation.clone());
    let _client = TcpStream::connect(local_address)
        .await
        .expect("connect local");
    started_rx.await.expect("dial started");
    cancellation.cancel();
    assert!(
        timeout(TEST_TIMEOUT, run_task)
            .await
            .expect("stop timeout")
            .expect("run task")
            .is_ok()
    );
    assert!(dropped.load(Ordering::SeqCst));
    assert_eq!(errors.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn cancellation_drops_live_copy_streams_and_suppresses_errors() {
    let listener = TcpProxyListener::bind("127.0.0.1:0")
        .await
        .expect("bind proxy");
    let local_address = listener.socket_addr();
    let (dialed_tx, dialed_rx) = oneshot::channel();
    let dialed_tx = Arc::new(Mutex::new(Some(dialed_tx)));
    let dialer = move |_: DialContext, _: &'static str, _: String| -> DialFuture {
        let dialed_tx = Arc::clone(&dialed_tx);
        Box::pin(async move {
            let (proxy_side, peer) = tokio::io::duplex(32);
            dialed_tx
                .lock()
                .expect("dialed lock")
                .take()
                .expect("one dial")
                .send(peer)
                .expect("send peer");
            Ok(Box::new(proxy_side) as BoxStream)
        })
    };
    let errors = Arc::new(AtomicUsize::new(0));
    let errors_for_handler = Arc::clone(&errors);
    let proxy = Arc::new(
        Proxy::new(listener, "remote:80")
            .with_dialer(dialer)
            .with_error_handler(move |_| {
                errors_for_handler.fetch_add(1, Ordering::SeqCst);
            }),
    );
    let cancellation = CancellationToken::new();
    let run_task = spawn_proxy(proxy, cancellation.clone());
    let mut client = TcpStream::connect(local_address)
        .await
        .expect("connect local");
    let mut peer = dialed_rx.await.expect("dial completed");
    client.write_all(b"left").await.expect("write local");
    let mut left = [0; 4];
    peer.read_exact(&mut left).await.expect("read remote");
    peer.write_all(b"right").await.expect("write remote");
    let mut right = [0; 5];
    client.read_exact(&mut right).await.expect("read local");
    assert_eq!(&left, b"left");
    assert_eq!(&right, b"right");

    cancellation.cancel();
    assert!(
        timeout(TEST_TIMEOUT, run_task)
            .await
            .expect("stop timeout")
            .expect("run task")
            .is_ok()
    );
    let mut byte = [0; 1];
    assert_eq!(peer.read(&mut byte).await.expect("remote EOF"), 0);
    assert_eq!(client.read(&mut byte).await.expect("local EOF"), 0);
    assert_eq!(errors.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn capacity_is_acquired_before_accept_and_cancellation_drains_handler() {
    let listener = TcpProxyListener::bind("127.0.0.1:0")
        .await
        .expect("bind proxy");
    let local_address = listener.socket_addr();
    let attempts = Arc::new(AtomicUsize::new(0));
    let active = Arc::new(AtomicUsize::new(0));
    let dialer = PendingDialer {
        attempts: Arc::clone(&attempts),
        active: Arc::clone(&active),
    };
    let proxy = Arc::new(
        Proxy::new(listener, "remote:80")
            .with_dialer(dialer)
            .with_max_connections(1),
    );
    let cancellation = CancellationToken::new();
    let run_task = spawn_proxy(proxy, cancellation.clone());
    let _first = TcpStream::connect(local_address)
        .await
        .expect("first local");
    wait_until(|| attempts.load(Ordering::SeqCst) == 1).await;
    let _second = TcpStream::connect(local_address)
        .await
        .expect("second local");
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert_eq!(active.load(Ordering::SeqCst), 1);

    cancellation.cancel();
    assert!(
        timeout(TEST_TIMEOUT, run_task)
            .await
            .expect("stop timeout")
            .expect("run task")
            .is_ok()
    );
    assert_eq!(active.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn shutdown_errors_after_clean_eof_are_ignored() {
    let listener = TcpProxyListener::bind("127.0.0.1:0")
        .await
        .expect("bind proxy");
    let local_address = listener.socket_addr();
    let dialer = |_: DialContext, _: &'static str, _: String| -> DialFuture {
        Box::pin(async move {
            let (proxy_side, peer) = tokio::io::duplex(32);
            drop(peer);
            Ok(Box::new(ShutdownErrorStream(proxy_side)) as BoxStream)
        })
    };
    let errors = Arc::new(AtomicUsize::new(0));
    let errors_for_handler = Arc::clone(&errors);
    let proxy = Arc::new(
        Proxy::new(listener, "remote:80")
            .with_dialer(dialer)
            .with_error_handler(move |_| {
                errors_for_handler.fetch_add(1, Ordering::SeqCst);
            }),
    );
    let cancellation = CancellationToken::new();
    let run_task = spawn_proxy(proxy, cancellation.clone());
    let mut client = TcpStream::connect(local_address)
        .await
        .expect("connect local");
    let mut sink = Vec::new();
    timeout(TEST_TIMEOUT, client.read_to_end(&mut sink))
        .await
        .expect("EOF timeout")
        .expect("read EOF");
    tokio::task::yield_now().await;
    assert_eq!(errors.load(Ordering::SeqCst), 0);
    cancellation.cancel();
    assert!(run_task.await.expect("run task").is_ok());
}

fn spawn_proxy(
    proxy: Arc<Proxy>,
    cancellation: CancellationToken,
) -> tokio::task::JoinHandle<Result<(), ProxyError>> {
    tokio::spawn(async move { proxy.run(&cancellation).await })
}

async fn wait_until(predicate: impl Fn() -> bool) {
    timeout(TEST_TIMEOUT, async {
        while !predicate() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .expect("condition timeout");
}

struct ErrorListener {
    source: Mutex<Option<io::Error>>,
    closed: Arc<AtomicBool>,
}

impl ProxyListener for ErrorListener {
    fn accept(&self) -> AcceptFuture<'_> {
        Box::pin(async move {
            Err(self
                .source
                .lock()
                .expect("source lock")
                .take()
                .unwrap_or_else(|| io::Error::other("listener failed again")))
        })
    }

    fn close(&self) -> io::Result<()> {
        self.closed.store(true, Ordering::SeqCst);
        Ok(())
    }

    fn local_addr(&self) -> ListenerAddress {
        ListenerAddress::Tcp("127.0.0.1:0".parse().expect("address"))
    }
}

struct DropFlag(Arc<AtomicBool>);

impl Drop for DropFlag {
    fn drop(&mut self) {
        self.0.store(true, Ordering::SeqCst);
    }
}

struct PendingDialer {
    attempts: Arc<AtomicUsize>,
    active: Arc<AtomicUsize>,
}

impl Dialer for PendingDialer {
    fn dial(&self, _: DialContext, _: &'static str, _: String) -> DialFuture {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        let guard = ActiveGuard::new(Arc::clone(&self.active));
        Box::pin(async move {
            let _guard = guard;
            pending::<()>().await;
            unreachable!()
        })
    }
}

struct ActiveGuard(Arc<AtomicUsize>);

impl ActiveGuard {
    fn new(active: Arc<AtomicUsize>) -> Self {
        active.fetch_add(1, Ordering::SeqCst);
        Self(active)
    }
}

impl Drop for ActiveGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

impl From<Arc<AtomicUsize>> for ActiveGuard {
    fn from(active: Arc<AtomicUsize>) -> Self {
        Self::new(active)
    }
}

struct ShutdownErrorStream(tokio::io::DuplexStream);

struct ReadBrokenPipeStream;

impl AsyncRead for ReadBrokenPipeStream {
    fn poll_read(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        _: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Poll::Ready(Err(io::Error::new(
            io::ErrorKind::BrokenPipe,
            "broken pipe",
        )))
    }
}

impl AsyncWrite for ReadBrokenPipeStream {
    fn poll_write(
        self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        Poll::Ready(Ok(buffer.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

impl AsyncRead for ShutdownErrorStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(context, buffer)
    }
}

impl AsyncWrite for ShutdownErrorStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(context, buffer)
    }

    fn poll_flush(mut self: Pin<&mut Self>, context: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(context)
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Err(io::Error::other("shutdown failed")))
    }
}

fn _assert_async_stream<T: AsyncStream>() {}
