use std::{error, fmt};

#[derive(Debug)]
pub enum Error {
    InvalidUrl(String),
    Transport(crate::TransportError),
    ReadResponse(std::io::Error),
    DecodeAuth(serde_json::Error),
    AuthNoDomain,
    AuthenticationFailed,
    UnexpectedStatus(u16),
    DecodeResponse {
        body: Vec<u8>,
        source: serde_json::Error,
    },
    TooManyRedirects,
    InvalidRedirect(String),
}

impl Error {
    pub fn is_auth_no_domain(&self) -> bool {
        matches!(self, Self::AuthNoDomain)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidUrl(message) => f.write_str(message),
            Self::Transport(error) => error.fmt(f),
            Self::ReadResponse(error) => write!(f, "read response body: {error}"),
            Self::DecodeAuth(error) => write!(f, "unmarshal auth error response: {error}"),
            Self::AuthNoDomain => f.write_str("the supplied domain failed authentication"),
            Self::AuthenticationFailed => f.write_str("authentication failed"),
            Self::UnexpectedStatus(code) => write!(f, "unexpected response status code: {code}"),
            Self::DecodeResponse { body, source } => write!(
                f,
                "unmarshal response body ({}): {source}",
                String::from_utf8_lossy(body)
            ),
            Self::TooManyRedirects => f.write_str("stopped after 10 redirects"),
            Self::InvalidRedirect(message) => {
                write!(f, "failed to parse Location header: {message}")
            }
        }
    }
}

impl error::Error for Error {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        match self {
            Self::Transport(error) => Some(error),
            Self::ReadResponse(error) => Some(error),
            Self::DecodeAuth(error) => Some(error),
            Self::DecodeResponse { source, .. } => Some(source),
            _ => None,
        }
    }
}

#[derive(Debug)]
pub struct CreateRecordsError {
    completed: Vec<crate::RecordResponse>,
    source: Error,
}

impl CreateRecordsError {
    pub fn completed(&self) -> &[crate::RecordResponse] {
        &self.completed
    }
    pub fn into_parts(self) -> (Vec<crate::RecordResponse>, Error) {
        (self.completed, self.source)
    }
    pub fn error(&self) -> &Error {
        &self.source
    }
}

impl fmt::Display for CreateRecordsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.source.fmt(f)
    }
}

impl error::Error for CreateRecordsError {
    fn source(&self) -> Option<&(dyn error::Error + 'static)> {
        Some(&self.source)
    }
}

impl CreateRecordsError {
    pub(crate) fn new(completed: Vec<crate::RecordResponse>, source: Error) -> Self {
        Self { completed, source }
    }
}
