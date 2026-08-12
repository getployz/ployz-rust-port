use std::error::Error;
use std::fmt;
use std::io;
use std::time::Duration;

use russh::{Channel, ChannelMsg, Sig, client};
use tokio::io::{AsyncWrite, AsyncWriteExt};
use tokio::time::timeout;

use crate::SessionError;
use crate::{Cancellation, CancelledError, Client};

const CHANNEL_CLEANUP_TIMEOUT: Duration = Duration::from_secs(2);
type BoxError = Box<dyn Error + Send + Sync>;

/// Executes commands over a reusable SSH connection.
#[derive(Clone, Debug)]
pub struct Remote {
    client: Client,
}

impl Remote {
    #[must_use]
    pub fn new(client: Client) -> Self {
        Self { client }
    }

    #[must_use]
    pub fn client(&self) -> &Client {
        &self.client
    }

    pub async fn run(
        &self,
        cancellation: &Cancellation,
        command: &str,
    ) -> Result<String, CommandError> {
        self.run_bytes(cancellation, command)
            .await
            .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
    }

    /// Runs a command while preserving arbitrary output bytes.
    pub async fn run_bytes(
        &self,
        cancellation: &Cancellation,
        command: &str,
    ) -> Result<Vec<u8>, CommandError> {
        let mut channel = self
            .client
            .open_session()
            .await
            .map_err(CommandError::create_session)?;
        let start = {
            let start = channel.exec(true, command);
            tokio::pin!(start);
            tokio::select! {
                result = &mut start => Some(result),
                () = cancellation.cancelled() => None,
            }
        };
        match start {
            Some(Ok(())) => {}
            Some(Err(source)) => {
                close_and_drain(&mut channel).await;
                return Err(CommandError::start_command(source));
            }
            None => {
                interrupt_and_cleanup(&mut channel)
                    .await
                    .map_err(CommandError::send_interrupt)?;
                return Err(CommandError::Cancelled(CancelledError));
            }
        }

        let mut output = Vec::new();
        let mut exit_status = None;
        let mut exit_signal = None;
        loop {
            tokio::select! {
                message = channel.wait() => match message {
                    Some(ChannelMsg::Data { data }) => output.extend_from_slice(&data),
                    Some(ChannelMsg::ExtendedData { data, ext: 1 }) => output.extend_from_slice(&data),
                    Some(ChannelMsg::ExitStatus { exit_status: status }) => exit_status = Some(status),
                    Some(ChannelMsg::ExitSignal { signal_name, .. }) => exit_signal = Some(signal_name),
                    Some(ChannelMsg::Failure) => {
                        close_and_drain(&mut channel).await;
                        return Err(CommandError::Command {
                            output: trim_go_space(output),
                            failure: CommandFailure {
                                exit_status,
                                exit_signal,
                                request_failed: true,
                            },
                        });
                    },
                    Some(ChannelMsg::Close) | None => break,
                    _ => {}
                },
                () = cancellation.cancelled() => {
                    interrupt_and_cleanup(&mut channel).await.map_err(CommandError::send_interrupt)?;
                    return Err(CommandError::Cancelled(CancelledError));
                }
            }
        }

        let output = trim_go_space(output);
        if exit_signal.is_some() || exit_status != Some(0) {
            return Err(CommandError::Command {
                output,
                failure: CommandFailure {
                    exit_status,
                    exit_signal,
                    request_failed: false,
                },
            });
        }
        Ok(output)
    }

    pub async fn stream<Stdout, Stderr>(
        &self,
        cancellation: &Cancellation,
        command: &str,
        stdout: &mut Stdout,
        stderr: &mut Stderr,
    ) -> Result<(), StreamError>
    where
        Stdout: AsyncWrite + Unpin,
        Stderr: AsyncWrite + Unpin,
    {
        let mut channel = self
            .client
            .open_session()
            .await
            .map_err(StreamError::create_session)?;
        let start = {
            let start = channel.exec(true, command);
            tokio::pin!(start);
            tokio::select! {
                result = &mut start => Some(result),
                () = cancellation.cancelled() => None,
            }
        };
        match start {
            Some(Ok(())) => {}
            Some(Err(source)) => {
                close_and_drain(&mut channel).await;
                return Err(StreamError::start_command(source));
            }
            None => {
                interrupt_and_cleanup(&mut channel)
                    .await
                    .map_err(StreamError::send_interrupt)?;
                return Err(StreamError::Cancelled(CancelledError));
            }
        }

        let mut exit_status = None;
        let mut exit_signal = None;
        loop {
            tokio::select! {
                message = channel.wait() => match message {
                    Some(ChannelMsg::Data { data }) => {
                        match write_or_cancel(stdout, &data, cancellation).await {
                            WriteOutcome::Complete => {}
                            WriteOutcome::Error(source) => {
                                close_and_drain(&mut channel).await;
                                return Err(StreamError::write_stdout(source));
                            }
                            WriteOutcome::Cancelled => {
                                interrupt_and_cleanup(&mut channel).await.map_err(StreamError::send_interrupt)?;
                                return Err(StreamError::Cancelled(CancelledError));
                            }
                        }
                    },
                    Some(ChannelMsg::ExtendedData { data, ext: 1 }) => {
                        match write_or_cancel(stderr, &data, cancellation).await {
                            WriteOutcome::Complete => {}
                            WriteOutcome::Error(source) => {
                                close_and_drain(&mut channel).await;
                                return Err(StreamError::write_stderr(source));
                            }
                            WriteOutcome::Cancelled => {
                                interrupt_and_cleanup(&mut channel).await.map_err(StreamError::send_interrupt)?;
                                return Err(StreamError::Cancelled(CancelledError));
                            }
                        }
                    },
                    Some(ChannelMsg::ExitStatus { exit_status: status }) => exit_status = Some(status),
                    Some(ChannelMsg::ExitSignal { signal_name, .. }) => exit_signal = Some(signal_name),
                    Some(ChannelMsg::Failure) => {
                        close_and_drain(&mut channel).await;
                        return Err(StreamError::Command {
                            failure: CommandFailure {
                                exit_status,
                                exit_signal,
                                request_failed: true,
                            },
                        });
                    },
                    Some(ChannelMsg::Close) | None => break,
                    _ => {}
                },
                () = cancellation.cancelled() => {
                    interrupt_and_cleanup(&mut channel).await.map_err(StreamError::send_interrupt)?;
                    return Err(StreamError::Cancelled(CancelledError));
                }
            }
        }
        if exit_signal.is_some() || exit_status != Some(0) {
            return Err(StreamError::Command {
                failure: CommandFailure {
                    exit_status,
                    exit_signal,
                    request_failed: false,
                },
            });
        }
        Ok(())
    }

    pub async fn close(&self) -> Result<(), crate::CloseError> {
        self.client.close().await
    }
}

async fn interrupt_and_cleanup(channel: &mut Channel<client::Msg>) -> Result<(), BoxError> {
    let signal_result = timeout(CHANNEL_CLEANUP_TIMEOUT, channel.signal(Sig::INT))
        .await
        .map_err(|_| {
            Box::new(io::Error::new(
                io::ErrorKind::TimedOut,
                "sending SSH interrupt timed out",
            )) as BoxError
        })?
        .map_err(|source| Box::new(source) as BoxError);
    close_and_drain(channel).await;
    signal_result
}

async fn close_and_drain(channel: &mut Channel<client::Msg>) {
    let _ = timeout(CHANNEL_CLEANUP_TIMEOUT, async {
        let _ = channel.close().await;
        while let Some(message) = channel.wait().await {
            if matches!(message, ChannelMsg::Close) {
                break;
            }
        }
    })
    .await;
}

enum WriteOutcome {
    Complete,
    Error(std::io::Error),
    Cancelled,
}

async fn write_or_cancel(
    writer: &mut (impl AsyncWrite + Unpin),
    data: &[u8],
    cancellation: &Cancellation,
) -> WriteOutcome {
    tokio::select! {
        result = writer.write_all(data) => match result {
            Ok(()) => WriteOutcome::Complete,
            Err(source) => WriteOutcome::Error(source),
        },
        () = cancellation.cancelled() => WriteOutcome::Cancelled,
    }
}

/// A remote command's non-success termination information.
#[derive(Clone, Debug)]
pub struct CommandFailure {
    pub exit_status: Option<u32>,
    pub exit_signal: Option<Sig>,
    pub request_failed: bool,
}

impl fmt::Display for CommandFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.request_failed {
            return formatter.write_str("remote command request rejected");
        }
        if let Some(signal) = &self.exit_signal {
            return write!(formatter, "remote command exited on signal {signal:?}");
        }
        match self.exit_status {
            Some(status) => write!(formatter, "remote command exited with status {status}"),
            None => formatter.write_str("remote command exited without status"),
        }
    }
}

impl Error for CommandFailure {}

/// Failure from [`Remote::run`] or [`Remote::run_bytes`].
#[derive(Debug)]
pub enum CommandError {
    CreateSession(SessionError),
    StartCommand(russh::Error),
    SendInterrupt(BoxError),
    Cancelled(CancelledError),
    Command {
        output: Vec<u8>,
        failure: CommandFailure,
    },
}

impl CommandError {
    fn create_session(error: SessionError) -> Self {
        Self::CreateSession(error)
    }

    fn start_command(error: russh::Error) -> Self {
        Self::StartCommand(error)
    }

    fn send_interrupt(error: BoxError) -> Self {
        Self::SendInterrupt(error)
    }

    #[must_use]
    pub fn output(&self) -> Option<&[u8]> {
        match self {
            Self::Command { output, .. } => Some(output),
            _ => None,
        }
    }
}

impl fmt::Display for CommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CreateSession(source) => write!(formatter, "create session: {source}"),
            Self::StartCommand(source) => write!(formatter, "run command on remote host: {source}"),
            Self::SendInterrupt(source) => write!(
                formatter,
                "send interrupt signal to remote process: {source}"
            ),
            Self::Cancelled(source) => write!(formatter, "canceled: {source}"),
            Self::Command { output, failure } => write!(
                formatter,
                "run command on remote host: {failure}: {}",
                String::from_utf8_lossy(output)
            ),
        }
    }
}

impl Error for CommandError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CreateSession(source) => Some(source),
            Self::StartCommand(source) => Some(source),
            Self::SendInterrupt(source) => Some(source.as_ref()),
            Self::Command { failure, .. } => Some(failure),
            Self::Cancelled(source) => Some(source),
        }
    }
}

/// Failure from [`Remote::stream`].
#[derive(Debug)]
pub enum StreamError {
    CreateSession(SessionError),
    StartCommand(russh::Error),
    WriteStdout(std::io::Error),
    WriteStderr(std::io::Error),
    SendInterrupt(BoxError),
    Cancelled(CancelledError),
    Command { failure: CommandFailure },
}

impl StreamError {
    fn create_session(error: SessionError) -> Self {
        Self::CreateSession(error)
    }
    fn start_command(error: russh::Error) -> Self {
        Self::StartCommand(error)
    }
    fn write_stdout(error: std::io::Error) -> Self {
        Self::WriteStdout(error)
    }
    fn write_stderr(error: std::io::Error) -> Self {
        Self::WriteStderr(error)
    }
    fn send_interrupt(error: BoxError) -> Self {
        Self::SendInterrupt(error)
    }
}

impl fmt::Display for StreamError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CreateSession(source) => write!(formatter, "create session: {source}"),
            Self::StartCommand(source) => write!(formatter, "run command on remote host: {source}"),
            Self::WriteStdout(source) => write!(formatter, "write remote stdout: {source}"),
            Self::WriteStderr(source) => write!(formatter, "write remote stderr: {source}"),
            Self::SendInterrupt(source) => write!(
                formatter,
                "send interrupt signal to remote process: {source}"
            ),
            Self::Cancelled(source) => write!(formatter, "canceled: {source}"),
            Self::Command { failure } => write!(formatter, "run command on remote host: {failure}"),
        }
    }
}

impl Error for StreamError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CreateSession(source) => Some(source),
            Self::StartCommand(source) => Some(source),
            Self::SendInterrupt(source) => Some(source.as_ref()),
            Self::WriteStdout(source) | Self::WriteStderr(source) => Some(source),
            Self::Command { failure } => Some(failure),
            Self::Cancelled(source) => Some(source),
        }
    }
}

pub(crate) fn trim_go_space(mut bytes: Vec<u8>) -> Vec<u8> {
    let mut start = 0;
    while start < bytes.len() {
        let Some(width) = leading_whitespace_width(&bytes[start..]) else {
            break;
        };
        start += width;
    }
    let mut end = bytes.len();
    while end > start {
        let Some(width) = trailing_whitespace_width(&bytes[start..end]) else {
            break;
        };
        end -= width;
    }
    if end < bytes.len() {
        bytes.truncate(end);
    }
    if start > 0 {
        bytes.drain(..start);
    }
    bytes
}

fn leading_whitespace_width(bytes: &[u8]) -> Option<usize> {
    let first = *bytes.first()?;
    if first.is_ascii() {
        return matches!(first, b'\t' | b'\n' | 0x0b | 0x0c | b'\r' | b' ').then_some(1);
    }
    let width = utf8_width(first)?;
    let character = std::str::from_utf8(bytes.get(..width)?)
        .ok()?
        .chars()
        .next()?;
    character.is_whitespace().then_some(width)
}

fn trailing_whitespace_width(bytes: &[u8]) -> Option<usize> {
    let last = *bytes.last()?;
    if last.is_ascii() {
        return matches!(last, b'\t' | b'\n' | 0x0b | 0x0c | b'\r' | b' ').then_some(1);
    }
    let lower = bytes.len().saturating_sub(4);
    let start = (lower..bytes.len())
        .rev()
        .find(|index| std::str::from_utf8(&bytes[*index..]).is_ok())?;
    let character = std::str::from_utf8(&bytes[start..]).ok()?.chars().next()?;
    character.is_whitespace().then_some(bytes.len() - start)
}

fn utf8_width(first: u8) -> Option<usize> {
    match first {
        0xc2..=0xdf => Some(2),
        0xe0..=0xef => Some(3),
        0xf0..=0xf4 => Some(4),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trim_matches_go_unicode_and_invalid_utf8_edges() {
        assert_eq!(
            trim_go_space(" \t\u{2003}hello\u{00a0}\n".as_bytes().to_vec()),
            b"hello"
        );
        assert_eq!(trim_go_space(b" \xffvalue\xff ".to_vec()), b"\xffvalue\xff");
        assert_eq!(trim_go_space(b" \xc2broken ".to_vec()), b"\xc2broken");
    }
}
