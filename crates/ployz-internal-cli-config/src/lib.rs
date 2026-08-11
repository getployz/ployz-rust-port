//! Ployz CLI context and machine-connection configuration.

mod config;
mod connection;
mod context;

pub use config::{Config, ConfigError};
pub use connection::{
    ConnectionValidationError, MachineConnection, ParseSshDestinationError, SshDestination,
    new_ssh_destination,
};
pub use context::Context;
