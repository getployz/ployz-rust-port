//! Caddy configuration generation and local admin control.

mod admin;
mod caddyfile;
mod controller;
mod json_config;
mod server;
mod service;
mod template;

pub use admin::{CaddyAdminClient, CaddyAdminClientError};
pub use caddyfile::{CaddyfileGenerator, CaddyfileValidator, GenerateError, ValidationError};
pub use controller::{CADDY_GROUP, CADDY_SERVICE_NAME, Controller, ControllerError, VERIFY_PATH};
pub use json_config::{CaddyConfig, generate_json_config};
pub use server::Server;
pub use service::{Caddyfile, Service, ServiceError};
