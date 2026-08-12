use std::error::Error;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use russh::client::{self, AuthResult};
use russh::keys::agent::{AgentIdentity, client::AgentClient};
use russh::keys::{PrivateKeyWithHashAlg, load_secret_key};
use russh::{Channel, Disconnect};
use tokio::net::TcpStream;
use tokio::sync::{Mutex, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::timeout;

use crate::{Cancellation, CancelledError};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const OPEN_CLEANUP_GRACE: Duration = Duration::from_millis(100);
const CONNECTION_CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
const OWNER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(3);

type BoxError = Box<dyn Error + Send + Sync>;
type Handle = client::Handle<AcceptEveryServerKey>;
type SessionChannel = Channel<client::Msg>;

/// A tunneled remote Unix-domain stream.
pub type TunneledStream = russh::ChannelStream<client::Msg>;

#[derive(Clone, Copy, Debug)]
struct AcceptEveryServerKey;

impl client::Handler for AcceptEveryServerKey {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        _server_public_key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        // Deliberate oracle parity: this permits active machine-in-the-middle attacks.
        Ok(true)
    }
}

/// Failure to establish and authenticate an SSH client.
#[derive(Debug)]
pub enum ConnectError {
    Agent { source: BoxError },
    ReadPrivateKey { path: PathBuf, source: BoxError },
    ParsePrivateKey { source: BoxError },
    PrivateKey { path: PathBuf, source: BoxError },
}

impl fmt::Display for ConnectError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Agent { source } => write!(formatter, "connect using SSH agent: {source}"),
            Self::ReadPrivateKey { path, source } => {
                write!(formatter, "read private key file {path:?}: {source}")
            }
            Self::ParsePrivateKey { source } => write!(formatter, "parse private key: {source}"),
            Self::PrivateKey { path, source } => {
                write!(formatter, "connect using private key {path:?}: {source}")
            }
        }
    }
}

impl Error for ConnectError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Agent { source }
            | Self::ReadPrivateKey { source, .. }
            | Self::ParsePrivateKey { source }
            | Self::PrivateKey { source, .. } => Some(source.as_ref()),
        }
    }
}

#[derive(Debug)]
struct StageError {
    context: &'static str,
    source: BoxError,
}

impl fmt::Display for StageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.context, self.source)
    }
}

impl Error for StageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.source.as_ref())
    }
}

#[derive(Clone, Copy, Debug)]
struct AuthenticationRejected(&'static str);

impl fmt::Display for AuthenticationRejected {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{} authentication rejected", self.0)
    }
}

impl Error for AuthenticationRejected {}

#[derive(Clone, Copy, Debug)]
struct UnsupportedOnDiskDsa;

impl fmt::Display for UnsupportedOnDiskDsa {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("on-disk DSA keys are not supported")
    }
}

impl Error for UnsupportedOnDiskDsa {}

/// Opens a reusable in-process SSH connection.
///
/// The agent is attempted first. If it cannot authenticate, a fresh TCP
/// transport is used with the unencrypted key at `key_path`.
pub async fn connect(
    user: &str,
    host: &str,
    port: u16,
    key_path: impl AsRef<Path>,
) -> Result<Client, ConnectError> {
    let username = default_username(user);
    let port = effective_port(port);
    let key_path = key_path.as_ref();

    let agent_error = match connect_agent(&username, host, port).await {
        Ok(handle) => return Ok(Client::from_handle(username, handle)),
        Err(error) => error,
    };

    if key_path.as_os_str().is_empty() {
        return Err(ConnectError::Agent {
            source: agent_error,
        });
    }

    let expanded_path = ployz_internal_fs::expand_home_dir(key_path);
    let key = load_secret_key(&expanded_path, None).map_err(|source| {
        if matches!(source, russh::keys::Error::IO(_)) {
            ConnectError::ReadPrivateKey {
                path: expanded_path.clone(),
                source: Box::new(source),
            }
        } else {
            ConnectError::ParsePrivateKey {
                source: Box::new(source),
            }
        }
    })?;
    if matches!(key.algorithm(), russh::keys::Algorithm::Dsa) {
        return Err(ConnectError::ParsePrivateKey {
            source: Box::new(UnsupportedOnDiskDsa),
        });
    }

    let handle = connect_private_key(&username, host, port, key)
        .await
        .map_err(|source| ConnectError::PrivateKey {
            path: expanded_path,
            source,
        })?;
    Ok(Client::from_handle(username, handle))
}

fn default_username(user: &str) -> String {
    if !user.is_empty() {
        return user.to_owned();
    }
    ployz_internal_fs::current_user()
        .ok()
        .map(|current| current.username.to_string_lossy().into_owned())
        .unwrap_or_default()
}

fn effective_port(port: u16) -> u16 {
    if port == 0 { 22 } else { port }
}

async fn connect_agent(username: &str, host: &str, port: u16) -> Result<Handle, BoxError> {
    let mut agent = AgentClient::connect_env()
        .await
        .map_err(|source| StageError {
            context: "connect to SSH agent",
            source: Box::new(source),
        })?;
    let mut handle = connect_transport(host, port)
        .await
        .map_err(|source| StageError {
            context: "dial SSH server for agent authentication",
            source,
        })?;

    let authentication = authenticate_agent(&mut handle, username, &mut agent).await;
    drop(agent);
    match authentication {
        Ok(()) => Ok(handle),
        Err(source) => {
            retire_handle(&mut handle).await;
            Err(Box::new(StageError {
                context: "authenticate with SSH agent",
                source,
            }))
        }
    }
}

async fn authenticate_agent(
    handle: &mut Handle,
    username: &str,
    agent: &mut AgentClient<tokio::net::UnixStream>,
) -> Result<(), BoxError> {
    let identities = agent.request_identities().await?;
    let mut attempted = false;
    for identity in identities {
        attempted = true;
        let public_key = identity.public_key();
        let hash = if public_key.algorithm().is_rsa() {
            handle.best_supported_rsa_hash().await?.flatten()
        } else {
            None
        };
        drop(public_key);
        let result = match identity {
            AgentIdentity::PublicKey { key, .. } => {
                handle
                    .authenticate_publickey_with(username, key, hash, agent)
                    .await?
            }
            AgentIdentity::Certificate { certificate, .. } => {
                handle
                    .authenticate_certificate_with(username, certificate, hash, agent)
                    .await?
            }
        };
        if result.success() {
            return Ok(());
        }
    }
    let label = if attempted {
        "SSH agent"
    } else {
        "SSH agent (no identities)"
    };
    Err(Box::new(AuthenticationRejected(label)))
}

async fn connect_private_key(
    username: &str,
    host: &str,
    port: u16,
    key: russh::keys::PrivateKey,
) -> Result<Handle, BoxError> {
    let mut handle = connect_transport(host, port).await?;
    let authentication = async {
        let hash = if key.algorithm().is_rsa() {
            handle.best_supported_rsa_hash().await?.flatten()
        } else {
            None
        };
        handle
            .authenticate_publickey(username, PrivateKeyWithHashAlg::new(Arc::new(key), hash))
            .await
    }
    .await;
    match authentication {
        Ok(AuthResult::Success) => Ok(handle),
        Ok(_) => {
            retire_handle(&mut handle).await;
            Err(Box::new(AuthenticationRejected("private key")))
        }
        Err(source) => {
            retire_handle(&mut handle).await;
            Err(Box::new(source))
        }
    }
}

async fn connect_transport(host: &str, port: u16) -> Result<Handle, BoxError> {
    let stream = timeout(CONNECT_TIMEOUT, TcpStream::connect((host, port)))
        .await
        .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "SSH TCP connection timed out"))??;
    Ok(client::connect_stream(
        Arc::new(client::Config::default()),
        stream,
        AcceptEveryServerKey,
    )
    .await?)
}

async fn retire_handle(handle: &mut Handle) {
    let _ = timeout(CONNECTION_CLEANUP_TIMEOUT, async {
        let _ = handle.disconnect(Disconnect::ByApplication, "", "").await;
        let _ = handle.await;
    })
    .await;
}

/// A reusable authenticated SSH connection.
#[derive(Clone)]
pub struct Client {
    inner: Arc<ClientInner>,
}

struct ClientInner {
    username: String,
    requests: mpsc::Sender<OwnerRequest>,
    task: Mutex<Option<JoinHandle<Result<(), russh::Error>>>>,
    shutdown: Cancellation,
    closed: AtomicBool,
}

impl fmt::Debug for Client {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("Client")
            .field("username", &self.inner.username)
            .field("closed", &self.inner.closed.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

impl Client {
    fn from_handle(username: String, handle: Handle) -> Self {
        let (requests, receiver) = mpsc::channel(16);
        let shutdown = Cancellation::new();
        let task = tokio::spawn(connection_owner(handle, receiver, shutdown.clone()));
        Self {
            inner: Arc::new(ClientInner {
                username,
                requests,
                task: Mutex::new(Some(task)),
                shutdown,
                closed: AtomicBool::new(false),
            }),
        }
    }

    #[must_use]
    pub fn username(&self) -> &str {
        &self.inner.username
    }

    pub(crate) async fn open_session(&self) -> Result<SessionChannel, SessionError> {
        let (reply, response) = oneshot::channel();
        tokio::select! {
            result = self.inner.requests.send(OwnerRequest::OpenSession { reply }) => {
                result.map_err(|_| SessionError::Closed)?;
            }
            () = self.inner.shutdown.cancelled() => return Err(SessionError::Closed),
        }
        tokio::select! {
            result = response => result.unwrap_or(Err(SessionError::Closed)),
            () = self.inner.shutdown.cancelled() => Err(SessionError::Closed),
        }
    }

    pub async fn dial_streamlocal(
        &self,
        socket_path: impl Into<String>,
        cancellation: &Cancellation,
    ) -> Result<TunneledStream, TunnelError> {
        if cancellation.is_cancelled() {
            return Err(TunnelError::Cancelled(CancelledError));
        }
        let (reply, response) = oneshot::channel();
        tokio::select! {
            result = self.inner.requests.send(OwnerRequest::OpenStreamlocal {
                socket_path: socket_path.into(),
                cancellation: cancellation.clone(),
                reply,
            }) => result.map_err(|_| TunnelError::ConnectionClosed)?,
            () = cancellation.cancelled() => return Err(TunnelError::Cancelled(CancelledError)),
            () = self.inner.shutdown.cancelled() => return Err(TunnelError::ConnectionClosed),
        }
        let result = tokio::select! {
            result = response => result.unwrap_or(Err(SessionError::Closed)),
            () = cancellation.cancelled() => return Err(TunnelError::Cancelled(CancelledError)),
            () = self.inner.shutdown.cancelled() => return Err(TunnelError::ConnectionClosed),
        };
        match result {
            Ok(channel) => Ok(channel.into_stream()),
            Err(SessionError::Cancelled) => Err(TunnelError::Cancelled(CancelledError)),
            Err(SessionError::Retired) => Err(TunnelError::ConnectionRetired),
            Err(SessionError::Closed) => Err(TunnelError::ConnectionClosed),
            Err(SessionError::Ssh(source)) => Err(TunnelError::Open { source }),
        }
    }

    pub async fn close(&self) -> Result<(), CloseError> {
        self.inner.closed.store(true, Ordering::Release);
        self.inner.shutdown.cancel();
        if let Some(mut task) = self.inner.task.lock().await.take() {
            match timeout(OWNER_SHUTDOWN_TIMEOUT, &mut task).await {
                Err(_) => {
                    task.abort();
                    let _ = task.await;
                    Err(CloseError::Timeout)
                }
                Ok(result) => match result {
                    Ok(Ok(())) => Ok(()),
                    Ok(Err(source)) => Err(CloseError::Ssh(source)),
                    Err(source) => Err(CloseError::OwnerTask(source)),
                },
            }
        } else {
            Ok(())
        }
    }
}

impl Drop for ClientInner {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Release);
        self.shutdown.cancel();
    }
}

enum OwnerRequest {
    OpenSession {
        reply: oneshot::Sender<Result<SessionChannel, SessionError>>,
    },
    OpenStreamlocal {
        socket_path: String,
        cancellation: Cancellation,
        reply: oneshot::Sender<Result<SessionChannel, SessionError>>,
    },
}

#[derive(Debug)]
pub enum SessionError {
    Ssh(russh::Error),
    Closed,
    Cancelled,
    Retired,
}

async fn connection_owner(
    mut handle: Handle,
    mut requests: mpsc::Receiver<OwnerRequest>,
    shutdown: Cancellation,
) -> Result<(), russh::Error> {
    loop {
        tokio::select! {
            result = &mut handle => {
                reject_pending(&mut requests, SessionError::Closed);
                return result;
            }
            () = shutdown.cancelled() => {
                reject_pending(&mut requests, SessionError::Closed);
                return shutdown_handle(&mut handle).await;
            }
            request = requests.recv() => match request {
                Some(OwnerRequest::OpenSession { reply }) => {
                    let result = {
                        let open = handle.channel_open_session();
                        tokio::pin!(open);
                        tokio::select! {
                            result = &mut open => Some(result.map_err(SessionError::Ssh)),
                            () = shutdown.cancelled() => None,
                        }
                    };
                    if let Some(result) = result {
                        let _ = reply.send(result);
                    } else {
                        let _ = reply.send(Err(SessionError::Closed));
                        reject_pending(&mut requests, SessionError::Closed);
                        return shutdown_handle(&mut handle).await;
                    }
                }
                Some(OwnerRequest::OpenStreamlocal {
                    socket_path,
                    cancellation,
                    reply,
                }) => {
                    let (result, disposition) = supervised_streamlocal_open(
                        &handle,
                        socket_path,
                        &cancellation,
                        &shutdown,
                    ).await;
                    let _ = reply.send(result);
                    if disposition == OpenDisposition::Retire {
                        retire_handle(&mut handle).await;
                        reject_pending(&mut requests, SessionError::Retired);
                        return Ok(());
                    }
                }
                None => {
                    return shutdown_handle(&mut handle).await;
                }
            }
        }
    }
}

async fn shutdown_handle(handle: &mut Handle) -> Result<(), russh::Error> {
    match timeout(CONNECTION_CLEANUP_TIMEOUT, async {
        handle.disconnect(Disconnect::ByApplication, "", "").await?;
        handle.await
    })
    .await
    {
        Err(_) => Ok(()),
        Ok(Ok(())) | Ok(Err(russh::Error::Disconnect | russh::Error::HUP)) => Ok(()),
        Ok(Err(source)) => Err(source),
    }
}

async fn supervised_streamlocal_open(
    handle: &Handle,
    socket_path: String,
    cancellation: &Cancellation,
    shutdown: &Cancellation,
) -> (Result<SessionChannel, SessionError>, OpenDisposition) {
    if cancellation.is_cancelled() {
        return (Err(SessionError::Cancelled), OpenDisposition::Reusable);
    }
    let open = handle.channel_open_direct_streamlocal(socket_path);
    tokio::pin!(open);
    tokio::select! {
        result = &mut open => (result.map_err(SessionError::Ssh), OpenDisposition::Reusable),
        () = cancellation.cancelled() => {
            match timeout(OPEN_CLEANUP_GRACE, async {
                let channel = open.await?;
                channel.close().await?;
                Ok::<(), russh::Error>(())
            }).await {
                Ok(_) => (Err(SessionError::Cancelled), OpenDisposition::Reusable),
                Err(_) => (Err(SessionError::Cancelled), OpenDisposition::Retire),
            }
        }
        () = shutdown.cancelled() => (Err(SessionError::Closed), OpenDisposition::Retire),
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OpenDisposition {
    Reusable,
    Retire,
}

fn reject_pending(requests: &mut mpsc::Receiver<OwnerRequest>, reason: SessionError) {
    while let Ok(request) = requests.try_recv() {
        match request {
            OwnerRequest::OpenSession { reply } | OwnerRequest::OpenStreamlocal { reply, .. } => {
                let mapped = match reason {
                    SessionError::Retired => SessionError::Retired,
                    _ => SessionError::Closed,
                };
                let _ = reply.send(Err(mapped));
            }
        }
    }
}

impl Error for SessionError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Ssh(source) => Some(source),
            _ => None,
        }
    }
}

impl fmt::Display for SessionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ssh(source) => source.fmt(formatter),
            Self::Closed => formatter.write_str("SSH connection closed"),
            Self::Cancelled => formatter.write_str("operation canceled"),
            Self::Retired => formatter.write_str("SSH connection retired"),
        }
    }
}

/// Failure to open a tunneled Unix stream.
#[derive(Debug)]
pub enum TunnelError {
    Cancelled(CancelledError),
    ConnectionRetired,
    ConnectionClosed,
    Open { source: russh::Error },
}

impl fmt::Display for TunnelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled(source) => write!(formatter, "canceled: {source}"),
            Self::ConnectionRetired => {
                formatter.write_str("SSH connection retired after canceled tunnel open")
            }
            Self::ConnectionClosed => formatter.write_str("SSH connection closed"),
            Self::Open { source } => {
                write!(formatter, "open remote Unix socket through SSH: {source}")
            }
        }
    }
}

impl Error for TunnelError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Open { source } => Some(source),
            Self::Cancelled(source) => Some(source),
            Self::ConnectionRetired | Self::ConnectionClosed => None,
        }
    }
}

/// Failure while joining the connection-owner task during close.
#[derive(Debug)]
pub enum CloseError {
    Ssh(russh::Error),
    OwnerTask(tokio::task::JoinError),
    Timeout,
}

impl fmt::Display for CloseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ssh(source) => write!(formatter, "close SSH connection: {source}"),
            Self::OwnerTask(source) => {
                write!(
                    formatter,
                    "join SSH connection owner during close: {source}"
                )
            }
            Self::Timeout => formatter.write_str("close SSH connection: cleanup timed out"),
        }
    }
}

impl Error for CloseError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Ssh(source) => Some(source),
            Self::OwnerTask(source) => Some(source),
            Self::Timeout => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_defaults_match_oracle() {
        assert_eq!(effective_port(0), 22);
        assert_eq!(effective_port(2222), 2222);
        assert_eq!(default_username("explicit"), "explicit");
    }
}
