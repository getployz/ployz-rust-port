use std::future::Future;
use std::path::Path;
use std::pin::Pin;

use backon::Retryable as _;

use crate::backoff::ReadinessBackoff;
use crate::{Config, Error, Result};

pub type ServiceFuture<'a, T> = Pin<Box<dyn Future<Output = Result<T>> + Send + 'a>>;

pub trait Service: Send + Sync {
    fn start(&self) -> ServiceFuture<'_, ()>;
    fn stop(&self) -> ServiceFuture<'_, ()>;
    fn restart(&self) -> ServiceFuture<'_, ()>;
    fn cleanup(&self) -> ServiceFuture<'_, ()>;
    fn running(&self) -> ServiceFuture<'_, bool>;
}

/// Waits until Corrosion serves a query against the applied Uncloud schema.
pub async fn wait_ready(data_dir: impl AsRef<Path>) -> Result<()> {
    let config_path = data_dir.as_ref().join("config.toml");
    let config_data = std::fs::read_to_string(&config_path)
        .map_err(|error| Error::wrap("read config file", error))?;
    let config: Config =
        toml::from_str(&config_data).map_err(|error| Error::wrap("unmarshal config", error))?;
    let client =
        ployz_internal_corrosion::ApiClient::new(config.api.addr, config.api.authz.bearer_token)
            .map_err(|error| Error::wrap("create corrosion API client", error))?;

    (|| async {
        client
            .query("SELECT 1 FROM cluster LIMIT 1", None)
            .await
            .map(|mut rows| rows.close())
            .map_err(|error| Error::wrap("query cluster table", error))
    })
    .retry(ReadinessBackoff::default())
    .await
    .map_err(|error| Error::wrap("corrosion service did not become ready", error))
}
