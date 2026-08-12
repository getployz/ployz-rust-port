use std::sync::Arc;

use ployz_internal_machine_api_pb::{GetCaddyConfigResponse, caddy_server};
use tonic::{Code, Request, Response, Status};

use crate::service::Service;

#[derive(Clone, Debug)]
pub struct Server {
    service: Arc<Service>,
}

impl Server {
    #[must_use]
    pub fn new(service: impl Into<Arc<Service>>) -> Self {
        Self {
            service: service.into(),
        }
    }
}

#[tonic::async_trait]
impl caddy_server::Caddy for Server {
    async fn get_config(
        &self,
        _request: Request<()>,
    ) -> Result<Response<GetCaddyConfigResponse>, Status> {
        let service = self.service.clone();
        let file = tokio::task::spawn_blocking(move || service.caddyfile())
            .await
            .map_err(|error| Status::internal(format!("read Caddyfile worker failed: {error}")))?
            .map_err(|error| {
                Status::new(
                    if error.is_not_found() {
                        Code::NotFound
                    } else {
                        Code::Internal
                    },
                    error.to_string(),
                )
            })?;
        Ok(Response::new(GetCaddyConfigResponse {
            caddyfile: file.content,
            modified_at: Some(file.modified_at.into()),
        }))
    }
}
