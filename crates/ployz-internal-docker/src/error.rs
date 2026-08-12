use std::{error::Error, fmt};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApiErrorKind {
    InvalidParameter,
    NotFound,
    Conflict,
    System,
}

#[derive(Debug)]
pub struct ApiError {
    pub status: u16,
    pub kind: ApiErrorKind,
    pub message: String,
}

impl fmt::Display for ApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl Error for ApiError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CryptoProviderConflict;

impl fmt::Display for CryptoProviderConflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a non-Ring Rustls crypto provider is already installed")
    }
}

impl Error for CryptoProviderConflict {}

#[derive(Debug)]
pub enum DockerError {
    Configuration(String),
    Engine(bollard::errors::Error),
    Connection(reqwest::Error),
    Api(ApiError),
    Credential(docker_credential::CredentialRetrievalError),
    Task(tokio::task::JoinError),
    CryptoProviderConflict(CryptoProviderConflict),
    Cancelled,
    DeadlineExceeded,
    Timeout,
    Progress(ProgressError),
    Operation {
        context: &'static str,
        source: Box<DockerError>,
    },
}

impl DockerError {
    pub fn is_not_found(&self) -> bool {
        matches!(
            self,
            Self::Api(ApiError {
                kind: ApiErrorKind::NotFound,
                ..
            }) | Self::Engine(bollard::errors::Error::DockerResponseServerError {
                status_code: 404,
                ..
            })
        )
    }

    pub fn is_connection_failed(&self) -> bool {
        matches!(
            self,
            Self::Connection(_)
                | Self::Engine(bollard::errors::Error::SocketNotFoundError(_))
                | Self::Engine(bollard::errors::Error::HyperLegacyError { .. })
                | Self::Engine(bollard::errors::Error::HyperResponseError { .. })
                | Self::Engine(bollard::errors::Error::RequestTimeoutError)
                | Self::Engine(bollard::errors::Error::IOError { .. })
        )
    }
}

impl fmt::Display for DockerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration(message) => write!(f, "Docker configuration: {message}"),
            Self::Engine(error) => write!(f, "Docker Engine: {error}"),
            Self::Connection(error) => write!(f, "connect to Docker daemon: {error}"),
            Self::Api(error) => error.fmt(f),
            Self::Credential(error) => write!(f, "Docker registry credentials: {error}"),
            Self::Task(error) => write!(f, "Docker blocking task: {error}"),
            Self::CryptoProviderConflict(error) => {
                write!(f, "Docker TLS configuration: {error}")
            }
            Self::Cancelled => f.write_str("context canceled"),
            Self::DeadlineExceeded => f.write_str("context deadline exceeded"),
            Self::Timeout => f.write_str("timeout"),
            Self::Progress(error) => error.fmt(f),
            Self::Operation { context, source } => write!(f, "{context}: {source}"),
        }
    }
}

impl Error for DockerError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Engine(error) => Some(error),
            Self::Connection(error) => Some(error),
            Self::Api(error) => Some(error),
            Self::Credential(error) => Some(error),
            Self::Task(error) => Some(error),
            Self::CryptoProviderConflict(error) => Some(error),
            Self::Progress(error) => Some(error),
            Self::Operation { source, .. } => Some(source),
            Self::Configuration(_) | Self::Cancelled | Self::DeadlineExceeded | Self::Timeout => {
                None
            }
        }
    }
}

impl DockerError {
    pub(crate) fn context(self, context: &'static str) -> Self {
        Self::Operation {
            context,
            source: Box::new(self),
        }
    }
}

impl From<bollard::errors::Error> for DockerError {
    fn from(value: bollard::errors::Error) -> Self {
        Self::Engine(value)
    }
}

impl From<docker_credential::CredentialRetrievalError> for DockerError {
    fn from(value: docker_credential::CredentialRetrievalError) -> Self {
        Self::Credential(value)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CancellationError;

impl fmt::Display for CancellationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("context canceled")
    }
}

impl Error for CancellationError {}

#[derive(Debug)]
pub enum ProgressError {
    Embedded(String),
    DecodeJson(serde_json::Error),
    DecodeTransport(reqwest::Error),
    DecodeIo(std::io::Error),
    DecodeCancelled(CancellationError),
    Cancelled(CancellationError),
}

impl fmt::Display for ProgressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Embedded(message) => f.write_str(message),
            Self::DecodeJson(error) => write!(f, "decode image pull/push message: {error}"),
            Self::DecodeTransport(error) => write!(f, "decode image pull/push message: {error}"),
            Self::DecodeIo(error) => write!(f, "decode image pull/push message: {error}"),
            Self::DecodeCancelled(error) => write!(f, "decode image pull/push message: {error}"),
            Self::Cancelled(error) => error.fmt(f),
        }
    }
}

impl Error for ProgressError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::DecodeJson(error) => Some(error),
            Self::DecodeTransport(error) => Some(error),
            Self::DecodeIo(error) => Some(error),
            Self::DecodeCancelled(error) | Self::Cancelled(error) => Some(error),
            Self::Embedded(_) => None,
        }
    }
}
