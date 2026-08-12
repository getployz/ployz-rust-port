use std::fmt;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum ApiError {
    NotFound,
    Invalid(String),
    Cancelled(Arc<dyn std::error::Error + Send + Sync>),
    Operational(Arc<dyn std::error::Error + Send + Sync>),
}

impl ApiError {
    pub(crate) fn invalid(message: impl Into<String>) -> Self {
        Self::Invalid(message.into())
    }

    pub fn cancelled(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Cancelled(Arc::new(error))
    }

    pub fn operational(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::Operational(Arc::new(error))
    }
}

impl fmt::Display for ApiError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => formatter.write_str("not found"),
            Self::Invalid(message) => formatter.write_str(message),
            Self::Cancelled(error) | Self::Operational(error) => error.fmt(formatter),
        }
    }
}

impl std::error::Error for ApiError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Cancelled(error) | Self::Operational(error) => Some(error.as_ref()),
            Self::NotFound | Self::Invalid(_) => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, ApiError>;
