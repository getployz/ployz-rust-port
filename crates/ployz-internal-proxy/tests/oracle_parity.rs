use std::collections::BTreeMap;
use std::error::Error;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex};

use ployz_internal_proxy::{
    AcceptFuture, CancellationToken, ConnectionClosed, ListenerAddress, Proxy, ProxyError,
    ProxyListener, TcpProxyListener, is_connection_closed_error,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

#[cfg(unix)]
use ployz_internal_proxy::UnixProxyListener;

#[tokio::test]
async fn error_and_network_contract_matches_pinned_go_oracle() {
    let oracle = go_oracle();
    let mut rust = BTreeMap::new();
    rust.insert(
        "PLOYZ_ORACLE_CLOSED_net_closed".to_owned(),
        is_connection_closed_error(&ConnectionClosed).to_string(),
    );
    for (name, error) in [
        (
            "closed_pipe",
            io::Error::new(io::ErrorKind::BrokenPipe, "closed pipe"),
        ),
        (
            "epipe",
            io::Error::new(io::ErrorKind::BrokenPipe, "broken pipe"),
        ),
        (
            "reset",
            io::Error::new(io::ErrorKind::ConnectionReset, "reset"),
        ),
        ("other", io::Error::other("copy failed")),
    ] {
        rust.insert(
            format!("PLOYZ_ORACLE_CLOSED_{name}"),
            is_connection_closed_error(&error).to_string(),
        );
    }
    let wrapped = ProxyError::DataCopy(io::Error::new(io::ErrorKind::BrokenPipe, "broken pipe"));
    rust.insert(
        "PLOYZ_ORACLE_CLOSED_wrapped_epipe".to_owned(),
        is_connection_closed_error(&wrapped).to_string(),
    );

    let accept_error = Proxy::new(
        ErrorListener(Mutex::new(Some(io::Error::other("listener failed")))),
        "",
    )
    .run(&CancellationToken::new())
    .await
    .expect_err("accept error");
    rust.insert(
        "PLOYZ_ORACLE_ACCEPT_ERROR".to_owned(),
        accept_error.to_string(),
    );

    rust_network_contract(&mut rust).await;
    assert_eq!(rust, oracle);
    assert_eq!(
        wrapped.source().expect("wrapped source").to_string(),
        "broken pipe"
    );
}

async fn rust_network_contract(output: &mut BTreeMap<String, String>) {
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
    let proxy = Arc::new(Proxy::new(listener, remote_address.to_string()));
    let cancellation = CancellationToken::new();
    let run_proxy = Arc::clone(&proxy);
    let run_cancellation = cancellation.clone();
    let run_task = tokio::spawn(async move { run_proxy.run(&run_cancellation).await });
    let mut client = TcpStream::connect(local_address)
        .await
        .expect("connect proxy");
    client.write_all(b"request").await.expect("write request");
    client.shutdown().await.expect("half-close client");
    let mut response = String::new();
    client
        .read_to_string(&mut response)
        .await
        .expect("read response");
    remote_task.await.expect("remote task");
    output.insert(
        "PLOYZ_ORACLE_BIDIRECTIONAL".to_owned(),
        format!("request->{response}"),
    );
    cancellation.cancel();
    output.insert(
        "PLOYZ_ORACLE_CANCELLATION_NIL".to_owned(),
        run_task.await.expect("run task").is_ok().to_string(),
    );

    #[cfg(unix)]
    {
        let socket_path = std::env::temp_dir().join(format!(
            "ployz-proxy-oracle-{}-{}.sock",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = std::fs::remove_file(&socket_path);
        let listener = UnixProxyListener::bind(&socket_path).expect("bind Unix proxy");
        let proxy = Arc::new(Proxy::new(listener, "127.0.0.1:1"));
        let cancellation = CancellationToken::new();
        let run_proxy = Arc::clone(&proxy);
        let run_cancellation = cancellation.clone();
        let run_task = tokio::spawn(async move { run_proxy.run(&run_cancellation).await });
        cancellation.cancel();
        assert!(run_task.await.expect("Unix run task").is_ok());
        output.insert(
            "PLOYZ_ORACLE_UNIX_UNLINKED".to_owned(),
            (!socket_path.exists()).to_string(),
        );
    }
}

struct ErrorListener(Mutex<Option<io::Error>>);

impl ProxyListener for ErrorListener {
    fn accept(&self) -> AcceptFuture<'_> {
        Box::pin(async move {
            Err(self
                .0
                .lock()
                .expect("listener lock")
                .take()
                .unwrap_or_else(|| io::Error::other("listener failed again")))
        })
    }

    fn close(&self) -> io::Result<()> {
        Ok(())
    }

    fn local_addr(&self) -> ListenerAddress {
        ListenerAddress::Tcp("127.0.0.1:0".parse().expect("address"))
    }
}

fn go_oracle() -> BTreeMap<String, String> {
    let root = repository_root();
    let package = root.join("upstream/uncloud/internal/proxy");
    let imaginary_test = package.join("ployz_oracle_test.go");
    let actual_test = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/oracle_proxy_test.go");
    let unique = format!("{}", std::process::id());
    let overlay = std::env::temp_dir().join(format!("ployz-proxy-go-overlay-{unique}.json"));
    let go_cache = std::env::temp_dir().join(format!("ployz-proxy-go-cache-{unique}"));
    std::fs::create_dir_all(&go_cache).expect("create writable Go cache");
    let overlay_json = format!(
        "{{\"Replace\":{{\"{}\":\"{}\"}}}}",
        imaginary_test.display(),
        actual_test.display()
    );
    std::fs::write(&overlay, overlay_json).expect("write Go overlay");

    let go = Path::new("/opt/go1.26.1/bin/go");
    assert!(
        go.is_file(),
        "pinned Go toolchain is missing at {}",
        go.display()
    );
    let output = Command::new(go)
        .args([
            "test",
            "-race",
            "-run=^TestPloyzProxyOracle$",
            "-count=1",
            "-v",
            "-overlay",
        ])
        .arg(&overlay)
        .arg("./internal/proxy")
        .current_dir(root.join("upstream/uncloud"))
        .env("GOCACHE", &go_cache)
        .output()
        .expect("run pinned Go proxy oracle");
    let _ = std::fs::remove_file(overlay);
    let _ = std::fs::remove_dir_all(go_cache);
    assert!(
        output.status.success(),
        "Go oracle failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );

    String::from_utf8(output.stdout)
        .expect("Go output is UTF-8")
        .lines()
        .filter_map(|line| line.split_once('='))
        .filter(|(key, _)| key.starts_with("PLOYZ_ORACLE_"))
        .map(|(key, value)| (key.to_owned(), value.to_owned()))
        .collect()
}

fn repository_root() -> PathBuf {
    if let Some(root) = std::env::var_os("PLOYZ_REPOSITORY_ROOT") {
        return PathBuf::from(root);
    }
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crate lives beneath repository root")
        .to_owned()
}
