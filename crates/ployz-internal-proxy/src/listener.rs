use std::error::Error;
use std::fmt;
use std::future::Future;
use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

#[cfg(unix)]
use std::path::PathBuf;

use tokio::net::{TcpListener, ToSocketAddrs};
use tokio_util::sync::CancellationToken;

use crate::BoxStream;

/// Future returned by a proxy listener.
pub type AcceptFuture<'a> = Pin<Box<dyn Future<Output = io::Result<BoxStream>> + Send + 'a>>;

/// The address on which a proxy accepts local connections.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ListenerAddress {
    /// An IPv4 or IPv6 TCP socket address.
    Tcp(SocketAddr),
    /// A filesystem Unix-domain socket address.
    #[cfg(unix)]
    Unix(PathBuf),
    /// A Linux abstract-namespace Unix-domain socket address (without its leading NUL).
    #[cfg(target_os = "linux")]
    AbstractUnix(Vec<u8>),
    /// An unnamed Unix-domain socket address.
    #[cfg(unix)]
    UnnamedUnix,
}

impl fmt::Display for ListenerAddress {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tcp(address) => address.fmt(formatter),
            #[cfg(unix)]
            Self::Unix(path) => path.display().fmt(formatter),
            #[cfg(target_os = "linux")]
            Self::AbstractUnix(name) => {
                write!(formatter, "@{}", String::from_utf8_lossy(name))
            }
            #[cfg(unix)]
            Self::UnnamedUnix => Ok(()),
        }
    }
}

/// An asynchronously accepted stream source that can be closed concurrently.
pub trait ProxyListener: Send + Sync + 'static {
    /// Accepts one local connection.
    fn accept(&self) -> AcceptFuture<'_>;

    /// Closes the listener and wakes a pending [`accept`](Self::accept).
    fn close(&self) -> io::Result<()>;

    /// Returns the bound local address, including after close.
    fn local_addr(&self) -> ListenerAddress;
}

/// Marker used for the Rust equivalent of Go's `net.ErrClosed`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ConnectionClosed;

impl fmt::Display for ConnectionClosed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("use of closed network connection")
    }
}

impl Error for ConnectionClosed {}

fn closed_error() -> io::Error {
    io::Error::other(ConnectionClosed)
}

/// A closeable adapter around [`TcpListener`].
pub struct TcpProxyListener {
    listener: Mutex<Option<Arc<TcpListener>>>,
    closed: CancellationToken,
    address: SocketAddr,
}

impl TcpProxyListener {
    /// Binds a TCP proxy listener.
    pub async fn bind(address: impl ToSocketAddrs) -> io::Result<Self> {
        Self::new(TcpListener::bind(address).await?)
    }

    /// Wraps an existing Tokio TCP listener.
    pub fn new(listener: TcpListener) -> io::Result<Self> {
        let address = listener.local_addr()?;
        Ok(Self {
            listener: Mutex::new(Some(Arc::new(listener))),
            closed: CancellationToken::new(),
            address,
        })
    }

    /// Returns the bound TCP address.
    #[must_use]
    pub fn socket_addr(&self) -> SocketAddr {
        self.address
    }
}

impl ProxyListener for TcpProxyListener {
    fn accept(&self) -> AcceptFuture<'_> {
        Box::pin(async move {
            let listener = self
                .listener
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .clone()
                .ok_or_else(closed_error)?;
            tokio::select! {
                result = listener.accept() => {
                    result.map(|(stream, _)| Box::new(stream) as BoxStream)
                }
                () = self.closed.cancelled() => Err(closed_error()),
            }
        })
    }

    fn close(&self) -> io::Result<()> {
        self.closed.cancel();
        self.listener
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        Ok(())
    }

    fn local_addr(&self) -> ListenerAddress {
        ListenerAddress::Tcp(self.address)
    }
}

impl Drop for TcpProxyListener {
    fn drop(&mut self) {
        let _ = self.close();
    }
}

#[cfg(unix)]
mod unix {
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::{Arc, Mutex};

    use tokio::net::UnixListener;
    use tokio_util::sync::CancellationToken;

    use crate::BoxStream;

    use super::{AcceptFuture, ListenerAddress, ProxyListener, closed_error};

    /// A closeable Unix-domain listener that unlinks its filesystem socket on close.
    pub struct UnixProxyListener {
        state: Mutex<UnixListenerState>,
        closed: CancellationToken,
        path: Option<PathBuf>,
        address: ListenerAddress,
    }

    struct UnixListenerState {
        listener: Option<Arc<UnixListener>>,
        cleanup_path: Option<PathBuf>,
    }

    impl UnixProxyListener {
        /// Binds a filesystem Unix-domain proxy listener.
        pub fn bind(path: impl AsRef<Path>) -> io::Result<Self> {
            Self::new(UnixListener::bind(path)?)
        }

        /// Wraps an existing Tokio Unix-domain listener.
        pub fn new(listener: UnixListener) -> io::Result<Self> {
            let socket_address = listener.local_addr()?;
            let path = socket_address.as_pathname().map(Path::to_owned);
            let address = listener_address(&socket_address, path.as_ref());
            Ok(Self {
                state: Mutex::new(UnixListenerState {
                    listener: Some(Arc::new(listener)),
                    cleanup_path: path.clone(),
                }),
                closed: CancellationToken::new(),
                path,
                address,
            })
        }

        /// Returns the filesystem path, if this is a pathname socket.
        #[must_use]
        pub fn path(&self) -> Option<&Path> {
            self.path.as_deref()
        }
    }

    impl ProxyListener for UnixProxyListener {
        fn accept(&self) -> AcceptFuture<'_> {
            Box::pin(async move {
                let listener = self
                    .state
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .listener
                    .clone()
                    .ok_or_else(closed_error)?;
                tokio::select! {
                    result = listener.accept() => {
                        result.map(|(stream, _)| Box::new(stream) as BoxStream)
                    }
                    () = self.closed.cancelled() => Err(closed_error()),
                }
            })
        }

        fn close(&self) -> io::Result<()> {
            self.closed.cancel();
            let mut state = self
                .state
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.listener.take();
            if let Some(path) = state.cleanup_path.take() {
                match fs::remove_file(&path) {
                    Ok(()) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(error),
                }
            }
            Ok(())
        }

        fn local_addr(&self) -> ListenerAddress {
            self.address.clone()
        }
    }

    impl Drop for UnixProxyListener {
        fn drop(&mut self) {
            let _ = self.close();
        }
    }

    fn listener_address(
        _socket_address: &tokio::net::unix::SocketAddr,
        path: Option<&PathBuf>,
    ) -> ListenerAddress {
        if let Some(path) = path {
            return ListenerAddress::Unix(path.clone());
        }
        #[cfg(target_os = "linux")]
        {
            if let Some(name) = _socket_address.as_abstract_name() {
                return ListenerAddress::AbstractUnix(name.to_vec());
            }
        }
        ListenerAddress::UnnamedUnix
    }
}

#[cfg(unix)]
pub use unix::UnixProxyListener;
