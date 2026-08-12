//! Configuration and Docker lifecycle management for the Corrosion service.

mod backoff;
mod config;
mod docker;
mod error;
mod service;

pub use config::{
    AdminConfig, ApiAuthzConfig, ApiConfig, Config, DbConfig, GossipConfig, make_dir,
};
pub use docker::{CONTAINER_NAME, DockerService, IMAGE};
pub use error::{Error, Result};
pub use service::{Service, ServiceFuture, wait_ready};

pub const DEFAULT_USER: &str = "uncloud";
pub const DEFAULT_GOSSIP_PORT: u16 = 51_001;
pub const DEFAULT_API_PORT: u16 = 51_002;
