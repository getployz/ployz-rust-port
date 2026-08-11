use tokio_util::sync::CancellationToken;

use crate::{Config, EndpointWatcher, NetworkError};

/// Darwin stub retained because the machine daemon only runs on Linux.
pub struct WireGuardNetwork;

impl WireGuardNetwork {
    pub async fn new() -> Result<Self, NetworkError> {
        Ok(Self)
    }

    pub async fn configure(&self, _config: Config) -> Result<(), NetworkError> {
        Err(not_implemented())
    }

    pub async fn run(&self, _cancellation: CancellationToken) -> Result<(), NetworkError> {
        Err(not_implemented())
    }

    pub async fn watch_endpoints(&self) -> EndpointWatcher {
        EndpointWatcher::never()
    }

    pub async fn cleanup(&self) -> Result<(), NetworkError> {
        Err(not_implemented())
    }
}

pub(crate) async fn detect_egress_mtu() -> Result<u32, NetworkError> {
    Err(not_implemented())
}

fn not_implemented() -> NetworkError {
    NetworkError::Invalid("not implemented on darwin".into())
}
