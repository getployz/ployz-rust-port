#![cfg(unix)]

use std::fs;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ployz_internal_proxy::{
    CancellationToken, ListenerAddress, Proxy, ProxyListener, UnixProxyListener,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UnixStream};
use tokio::time::timeout;

#[tokio::test]
async fn unix_listener_proxies_half_close_and_unlinks_socket_on_cancellation() {
    let remote = TcpListener::bind("127.0.0.1:0").await.expect("bind remote");
    let remote_address = remote.local_addr().expect("remote address");
    let remote_task = tokio::spawn(async move {
        let (mut connection, _) = remote.accept().await.expect("accept remote");
        let mut request = Vec::new();
        connection
            .read_to_end(&mut request)
            .await
            .expect("read request");
        assert_eq!(request, b"unix-request");
        connection
            .write_all(b"unix-response")
            .await
            .expect("write response");
        connection.shutdown().await.expect("half-close");
    });

    let socket_path = temporary_socket_path();
    let listener = UnixProxyListener::bind(&socket_path).expect("bind Unix socket");
    let proxy = Arc::new(Proxy::new(listener, remote_address.to_string()));
    assert_eq!(
        proxy.local_addr(),
        ListenerAddress::Unix(socket_path.clone())
    );
    let cancellation = CancellationToken::new();
    let run_proxy = Arc::clone(&proxy);
    let run_cancellation = cancellation.clone();
    let run_task = tokio::spawn(async move { run_proxy.run(&run_cancellation).await });

    let mut local = UnixStream::connect(&socket_path)
        .await
        .expect("connect Unix socket");
    local
        .write_all(b"unix-request")
        .await
        .expect("write request");
    local.shutdown().await.expect("half-close local");
    let mut response = Vec::new();
    timeout(Duration::from_secs(3), local.read_to_end(&mut response))
        .await
        .expect("response timeout")
        .expect("read response");
    assert_eq!(response, b"unix-response");

    cancellation.cancel();
    assert!(run_task.await.expect("run task").is_ok());
    remote_task.await.expect("remote task");
    assert!(!socket_path.exists());
}

#[tokio::test]
async fn unix_cancellation_does_not_depend_on_the_socket_path() {
    let socket_path = temporary_socket_path();
    let listener = UnixProxyListener::bind(&socket_path).expect("bind Unix socket");
    let proxy = Arc::new(Proxy::new(listener, "127.0.0.1:1"));
    let cancellation = CancellationToken::new();
    let run_proxy = Arc::clone(&proxy);
    let run_cancellation = cancellation.clone();
    let run_task = tokio::spawn(async move { run_proxy.run(&run_cancellation).await });

    fs::remove_file(&socket_path).expect("remove live socket path");
    cancellation.cancel();
    assert!(
        timeout(Duration::from_secs(3), run_task)
            .await
            .expect("stop timeout")
            .expect("run task")
            .is_ok()
    );
}

#[tokio::test]
async fn dropping_closed_listener_does_not_unlink_rebound_socket() {
    let socket_path = temporary_socket_path();
    let old_listener =
        Arc::new(UnixProxyListener::bind(&socket_path).expect("bind old Unix socket"));
    let close_one = {
        let listener = Arc::clone(&old_listener);
        std::thread::spawn(move || listener.close())
    };
    let close_two = {
        let listener = Arc::clone(&old_listener);
        std::thread::spawn(move || listener.close())
    };
    close_one
        .join()
        .expect("first close thread")
        .expect("first close");
    close_two
        .join()
        .expect("second close thread")
        .expect("second close");
    assert!(!socket_path.exists());

    let replacement = UnixProxyListener::bind(&socket_path).expect("bind replacement socket");
    drop(old_listener);
    assert!(socket_path.exists());

    let connect = UnixStream::connect(&socket_path);
    let (client, accepted) = tokio::join!(connect, replacement.accept());
    let client = client.expect("connect replacement");
    let accepted = accepted.expect("accept replacement");
    drop(client);
    drop(accepted);
    replacement.close().expect("close replacement");
}

#[cfg(target_os = "linux")]
#[tokio::test]
async fn abstract_unix_listener_cancels_without_a_filesystem_wake_address() {
    use std::os::linux::net::SocketAddrExt;
    use std::os::unix::net::{SocketAddr, UnixListener as StdUnixListener};

    let name = format!(
        "ployz-proxy-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("clock after epoch")
            .as_nanos()
    );
    let address = SocketAddr::from_abstract_name(&name).expect("abstract address");
    let expected_name = address.as_abstract_name().expect("abstract name").to_vec();
    let listener = StdUnixListener::bind_addr(&address).expect("bind abstract Unix socket");
    listener
        .set_nonblocking(true)
        .expect("nonblocking listener");
    let listener = tokio::net::UnixListener::from_std(listener).expect("Tokio listener");
    let listener = UnixProxyListener::new(listener).expect("wrap abstract listener");
    assert_eq!(listener.path(), None);
    let proxy = Arc::new(Proxy::new(listener, "127.0.0.1:1"));
    assert_eq!(
        proxy.local_addr(),
        ListenerAddress::AbstractUnix(expected_name)
    );
    assert_eq!(proxy.local_addr().to_string(), format!("@{name}"));
    let cancellation = CancellationToken::new();
    let run_proxy = Arc::clone(&proxy);
    let run_cancellation = cancellation.clone();
    let run_task = tokio::spawn(async move { run_proxy.run(&run_cancellation).await });

    cancellation.cancel();
    assert!(run_task.await.expect("run task").is_ok());
}

fn temporary_socket_path() -> PathBuf {
    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock after epoch")
        .as_nanos();
    std::env::temp_dir().join(format!("ployz-proxy-{}-{unique}.sock", std::process::id()))
}
