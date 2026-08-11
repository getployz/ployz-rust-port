//! Shell scripts shipped inside the Ployz CLI for execution on remote machines.

/// The Ployz machine installation script, embedded verbatim in the crate.
pub const INSTALL_SCRIPT: &str = include_str!("install.sh");
