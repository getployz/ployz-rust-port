use std::error::Error;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::task::{Context, Poll};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ployz_internal_sshexec::{
    Cancellation, CommandError, ConnectError, Remote, StreamError, TunnelError, connect,
};
use russh::keys::ssh_key::{Algorithm, LineEnding};
use russh::server::{self, Auth, ChannelOpenHandle, Session};
use russh::{Channel, ChannelId, Sig};
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::time::{sleep, timeout};

#[derive(Clone)]
struct TestHandler {
    withhold_streamlocal: bool,
    signals: Arc<AtomicUsize>,
}

impl server::Handler for TestHandler {
    type Error = russh::Error;

    async fn auth_publickey(
        &mut self,
        _user: &str,
        _public_key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<Auth, Self::Error> {
        Ok(Auth::Accept)
    }

    async fn channel_open_session(
        &mut self,
        _channel: Channel<server::Msg>,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        reply.accept().await;
        Ok(())
    }

    async fn exec_request(
        &mut self,
        channel: ChannelId,
        command: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if command == b"reject" {
            session.channel_failure(channel)?;
            return Ok(());
        }
        if command == b"withhold-start" {
            return Ok(());
        }
        session.channel_success(channel)?;
        match command {
            b"combined" => {
                session.data(channel, b" \xe2\x80\x83out\n".to_vec())?;
                session.extended_data(channel, 1, b"err \xc2\xa0".to_vec())?;
                session.exit_status_request(channel, 0)?;
                session.eof(channel)?;
                session.close(channel)?;
            }
            b"split" => {
                session.data(channel, b"stdout".to_vec())?;
                session.extended_data(channel, 1, b"stderr".to_vec())?;
                session.exit_status_request(channel, 0)?;
                session.eof(channel)?;
                session.close(channel)?;
            }
            b"fail" => {
                session.data(channel, b"  failure output \n".to_vec())?;
                session.exit_status_request(channel, 7)?;
                session.eof(channel)?;
                session.close(channel)?;
            }
            b"wait" | b"write-wait" => {
                if command == b"write-wait" {
                    session.data(channel, b"blocks".to_vec())?;
                }
            }
            _ => {
                session.exit_status_request(channel, 127)?;
                session.close(channel)?;
            }
        }
        Ok(())
    }

    async fn signal(
        &mut self,
        channel: ChannelId,
        signal: Sig,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        if matches!(signal, Sig::INT) {
            self.signals.fetch_add(1, Ordering::SeqCst);
            session.exit_status_request(channel, 130)?;
            session.eof(channel)?;
            session.close(channel)?;
        }
        Ok(())
    }

    async fn channel_open_direct_streamlocal(
        &mut self,
        _channel: Channel<server::Msg>,
        _socket_path: &str,
        reply: ChannelOpenHandle,
        _session: &mut Session,
    ) -> Result<(), Self::Error> {
        if self.withhold_streamlocal {
            // The adversarial server never confirms or rejects this open.
            std::mem::forget(reply);
        } else {
            reply.accept().await;
        }
        Ok(())
    }

    async fn data(
        &mut self,
        channel: ChannelId,
        data: &[u8],
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.data(channel, data.to_vec())?;
        Ok(())
    }

    async fn channel_eof(
        &mut self,
        channel: ChannelId,
        session: &mut Session,
    ) -> Result<(), Self::Error> {
        session.eof(channel)?;
        session.close(channel)?;
        Ok(())
    }
}

struct TestServer {
    port: u16,
    key_path: PathBuf,
    signals: Arc<AtomicUsize>,
    finished: oneshot::Receiver<()>,
}

impl TestServer {
    async fn start(withhold_streamlocal: bool) -> Self {
        Self::start_with_algorithm(withhold_streamlocal, Algorithm::Ed25519).await
    }

    async fn start_with_algorithm(withhold_streamlocal: bool, algorithm: Algorithm) -> Self {
        let mut rng = russh::keys::key::safe_rng();
        let host_key = russh::keys::PrivateKey::random(&mut rng, Algorithm::Ed25519).unwrap();
        let client_key = russh::keys::PrivateKey::random(&mut rng, algorithm).unwrap();
        let key_path = unique_key_path();
        std::fs::write(
            &key_path,
            client_key.to_openssh(LineEnding::LF).unwrap().as_bytes(),
        )
        .unwrap();

        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let signals = Arc::new(AtomicUsize::new(0));
        let handler = TestHandler {
            withhold_streamlocal,
            signals: Arc::clone(&signals),
        };
        let config = server::Config {
            auth_rejection_time: Duration::ZERO,
            keys: vec![host_key],
            ..server::Config::default()
        };
        let config = Arc::new(config);
        let (finished_tx, finished) = oneshot::channel();
        tokio::spawn(async move {
            let result = async {
                let (stream, _) = listener.accept().await?;
                let running = server::run_stream(config, stream, handler).await?;
                running.await
            }
            .await;
            assert!(result.is_ok(), "test SSH server failed: {result:?}");
            let _ = finished_tx.send(());
        });

        Self {
            port,
            key_path,
            signals,
            finished,
        }
    }

    async fn connect(&self) -> ployz_internal_sshexec::Client {
        connect("tester", "127.0.0.1", self.port, &self.key_path)
            .await
            .unwrap()
    }

    async fn finish(self) {
        timeout(Duration::from_secs(3), self.finished)
            .await
            .expect("server connection task did not terminate")
            .unwrap();
        std::fs::remove_file(self.key_path).unwrap();
    }
}

#[tokio::test]
async fn on_disk_rsa_is_enabled_and_on_disk_dsa_is_rejected() {
    let rsa_server = TestServer::start_with_algorithm(false, Algorithm::Rsa { hash: None }).await;
    let rsa_client = rsa_server.connect().await;
    rsa_client.close().await.unwrap();
    rsa_server.finish().await;

    let dsa_path = unique_key_path();
    std::fs::write(&dsa_path, include_bytes!("fixtures/id_dsa_1024")).unwrap();
    let error = connect("tester", "127.0.0.1", 1, &dsa_path)
        .await
        .unwrap_err();
    assert!(matches!(error, ConnectError::ParsePrivateKey { .. }));
    assert!(error.to_string().contains("parse private key"));
    std::fs::remove_file(dsa_path).unwrap();
}

fn unique_key_path() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!("ployz-sshexec-{}-{nonce}.key", std::process::id()))
}

#[tokio::test]
async fn reusable_remote_preserves_output_exit_and_cancellation_contracts() {
    let server = TestServer::start(false).await;
    let client = server.connect().await;
    assert_eq!(client.username(), "tester");
    let remote = Remote::new(client);
    let active = Cancellation::new();

    let already_cancelled = Cancellation::new();
    already_cancelled.cancel();
    assert!(matches!(
        remote
            .client()
            .dial_streamlocal("/run/not-opened.sock", &already_cancelled)
            .await,
        Err(TunnelError::Cancelled(_))
    ));

    assert_eq!(remote.run(&active, "combined").await.unwrap(), "out\nerr");

    let failure = remote.run_bytes(&active, "fail").await.unwrap_err();
    match failure {
        CommandError::Command { output, failure } => {
            assert_eq!(output, b"failure output");
            assert_eq!(failure.exit_status, Some(7));
        }
        other => panic!("unexpected error: {other}"),
    }

    let mut stdout = FlushFailWriter::default();
    let mut stderr = FlushFailWriter::default();
    remote
        .stream(&active, "split", &mut stdout, &mut stderr)
        .await
        .unwrap();
    assert_eq!(stdout.0, b"stdout");
    assert_eq!(stderr.0, b"stderr");

    let cancellation = Cancellation::new();
    let cancel_from_task = cancellation.clone();
    tokio::spawn(async move {
        sleep(Duration::from_millis(25)).await;
        cancel_from_task.cancel();
    });
    assert!(matches!(
        remote.run(&cancellation, "wait").await,
        Err(CommandError::Cancelled(_))
    ));
    assert_eq!(server.signals.load(Ordering::SeqCst), 1);

    let mut stream = remote
        .client()
        .dial_streamlocal("/run/uncloud.sock", &active)
        .await
        .unwrap();
    stream.write_all(b"echo through tunnel").await.unwrap();
    stream.shutdown().await.unwrap();
    let mut echoed = Vec::new();
    stream.read_to_end(&mut echoed).await.unwrap();
    assert_eq!(echoed, b"echo through tunnel");

    remote.close().await.unwrap();
    server.finish().await;
}

#[tokio::test]
async fn cancelled_unconfirmed_streamlocal_open_retires_connection_without_orphan() {
    let server = TestServer::start(true).await;
    let client = server.connect().await;
    let cancellation = Cancellation::new();
    let cancel_from_task = cancellation.clone();
    tokio::spawn(async move {
        sleep(Duration::from_millis(25)).await;
        cancel_from_task.cancel();
    });

    assert!(matches!(
        client
            .dial_streamlocal("/run/withheld.sock", &cancellation)
            .await,
        Err(TunnelError::Cancelled(_) | TunnelError::ConnectionRetired)
    ));
    timeout(Duration::from_secs(3), client.close())
        .await
        .expect("connection owner did not terminate")
        .unwrap();
    server.finish().await;
}

#[tokio::test]
async fn rejected_exec_returns_without_waiting_for_server_close() {
    let server = TestServer::start(false).await;
    let remote = Remote::new(server.connect().await);
    let active = Cancellation::new();

    let error = timeout(Duration::from_secs(3), remote.run_bytes(&active, "reject"))
        .await
        .expect("rejected exec remained blocked")
        .unwrap_err();
    assert!(matches!(
        error,
        CommandError::Command {
            failure,
            ..
        } if failure.request_failed
    ));

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let error = timeout(
        Duration::from_secs(3),
        remote.stream(&active, "reject", &mut stdout, &mut stderr),
    )
    .await
    .expect("rejected streaming exec remained blocked")
    .unwrap_err();
    assert!(matches!(
        error,
        StreamError::Command { failure } if failure.request_failed
    ));

    remote.close().await.unwrap();
    server.finish().await;
}

#[tokio::test]
async fn close_and_queued_cancellation_preempt_withheld_open() {
    let server = TestServer::start(true).await;
    let client = server.connect().await;
    let first_client = client.clone();
    let first_cancellation = Cancellation::new();
    let first = tokio::spawn(async move {
        first_client
            .dial_streamlocal("/run/first-withheld.sock", &first_cancellation)
            .await
    });
    sleep(Duration::from_millis(25)).await;

    let queued_cancellation = Cancellation::new();
    queued_cancellation.cancel();
    assert!(matches!(
        timeout(
            Duration::from_millis(250),
            client.dial_streamlocal("/run/queued.sock", &queued_cancellation),
        )
        .await
        .expect("queued cancellation remained behind an earlier open"),
        Err(TunnelError::Cancelled(_))
    ));

    timeout(Duration::from_secs(3), client.close())
        .await
        .expect("close remained behind an in-flight open")
        .unwrap();
    assert!(matches!(
        timeout(Duration::from_secs(1), first)
            .await
            .unwrap()
            .unwrap(),
        Err(TunnelError::ConnectionClosed | TunnelError::ConnectionRetired)
    ));
    server.finish().await;
}

struct PendingWriter;

#[derive(Default)]
struct FlushFailWriter(Vec<u8>);

impl AsyncWrite for FlushFailWriter {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _context: &mut Context<'_>,
        buffer: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        self.0.extend_from_slice(buffer);
        Poll::Ready(Ok(buffer.len()))
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Poll::Ready(Err(std::io::Error::other("flush must not be called")))
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Poll::Ready(Ok(()))
    }
}

impl AsyncWrite for PendingWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
        _buffer: &[u8],
    ) -> Poll<Result<usize, std::io::Error>> {
        Poll::Pending
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Poll::Pending
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        _context: &mut Context<'_>,
    ) -> Poll<Result<(), std::io::Error>> {
        Poll::Ready(Ok(()))
    }
}

#[tokio::test]
async fn streaming_writer_cannot_mask_command_cancellation() {
    let server = TestServer::start(false).await;
    let remote = Remote::new(server.connect().await);
    let cancellation = Cancellation::new();
    let cancel_from_task = cancellation.clone();
    tokio::spawn(async move {
        sleep(Duration::from_millis(25)).await;
        cancel_from_task.cancel();
    });
    let mut stdout = PendingWriter;
    let mut stderr = tokio::io::sink();

    let error = timeout(
        Duration::from_secs(3),
        remote.stream(&cancellation, "write-wait", &mut stdout, &mut stderr),
    )
    .await
    .expect("pending output writer masked cancellation")
    .unwrap_err();
    assert!(matches!(error, StreamError::Cancelled(_)));
    assert!(error.source().is_some());
    assert_eq!(server.signals.load(Ordering::SeqCst), 1);

    remote.close().await.unwrap();
    server.finish().await;
}

#[tokio::test]
async fn withheld_exec_reply_observes_cancellation() {
    let server = TestServer::start(false).await;
    let remote = Remote::new(server.connect().await);
    let cancellation = Cancellation::new();
    let cancel_from_task = cancellation.clone();
    tokio::spawn(async move {
        sleep(Duration::from_millis(25)).await;
        cancel_from_task.cancel();
    });

    let error = timeout(
        Duration::from_secs(3),
        remote.run(&cancellation, "withhold-start"),
    )
    .await
    .expect("withheld exec response masked cancellation")
    .unwrap_err();
    assert!(matches!(error, CommandError::Cancelled(_)));
    assert!(error.source().is_some());
    assert_eq!(server.signals.load(Ordering::SeqCst), 1);

    remote.close().await.unwrap();
    server.finish().await;
}
