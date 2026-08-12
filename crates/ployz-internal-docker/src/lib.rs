//! Docker Engine support for Ployz.

mod backoff;
mod cancel;
mod client;
mod config;
mod credentials;
mod error;
mod progress;
mod reference;

pub use cancel::Cancellation;
pub use client::{Client, ImagePlatform, PullOptions, PushOptions, ReadinessEvent};
pub use config::DaemonConfig;
pub use credentials::retrieve_local_registry_auth;
pub use error::{
    ApiError, ApiErrorKind, CancellationError, CryptoProviderConflict, DockerError, ProgressError,
};
pub use progress::{ErrorDetail, ProgressDetail, ProgressItem, ProgressMessage, ProgressStream};
