//! Machine WireGuard networking, ported from `internal/machine/network`.

mod address;
mod config;
mod ip;
mod mtu;
mod peer;
mod wireguard;

#[cfg(target_os = "macos")]
mod wireguard_darwin;
#[cfg(target_os = "linux")]
mod wireguard_linux;

pub use address::{get_public_ip, list_routable_ips};
pub use config::{Config, PeerConfig};
pub use ip::{machine_ip, management_ip};
pub use mtu::detect_mtu;
pub use wireguard::{
    DEFAULT_WIREGUARD_PORT, EndpointChangeEvent, EndpointWatcher, MAX_WIREGUARD_MTU,
    MIN_WIREGUARD_MTU, WIREGUARD_INTERFACE_NAME, WIREGUARD_KEEPALIVE_INTERVAL, new_machine_keys,
};

#[cfg(target_os = "macos")]
pub use wireguard_darwin::WireGuardNetwork;
#[cfg(target_os = "linux")]
pub use wireguard_linux::WireGuardNetwork;

use std::io;

use thiserror::Error;

/// An error produced while inspecting or configuring machine networking.
#[derive(Debug, Error)]
pub enum NetworkError {
    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: io::Error,
    },
    #[error("{context}: {source}")]
    Interface {
        context: &'static str,
        #[source]
        source: nix::errno::Errno,
    },
    #[error("{context}: {source}")]
    Http {
        context: &'static str,
        #[source]
        source: ureq::Error,
    },
    #[cfg(target_os = "linux")]
    #[error("{context}: {source}")]
    Netlink {
        context: String,
        #[source]
        source: rtnetlink::Error,
    },
    #[error("{0}")]
    Invalid(String),
}

pub(crate) fn io_error(context: impl Into<String>, source: io::Error) -> NetworkError {
    NetworkError::Io {
        context: context.into(),
        source,
    }
}
