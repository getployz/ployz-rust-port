//! SSH command execution and Unix-socket tunneling for Ployz.

mod cancellation;
mod client;
mod remote;
mod shell;
mod ssh_cli;

pub use cancellation::{Cancellation, CancelledError};
pub use client::{
    Client, CloseError, ConnectError, SessionError, TunnelError, TunneledStream, connect,
};
pub use remote::{CommandError, CommandFailure, Remote, StreamError};
pub use shell::{quote, quote_command};
pub use ssh_cli::{CliCommandError, SshCliRemote};
