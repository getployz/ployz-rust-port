use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::io::{self, Read, Write};
use std::ops::{Deref, DerefMut};
#[cfg(unix)]
use std::os::fd::AsRawFd;
use std::path::PathBuf;
use std::process::{Child, ChildStderr, ChildStdout, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use crate::remote::trim_go_space;
use crate::{Cancellation, CancelledError};

const WAIT_POLL_INTERVAL: Duration = Duration::from_millis(10);
const INTERRUPT_GRACE: Duration = Duration::from_secs(5);
const FORCE_KILL_REAP_GRACE: Duration = Duration::from_secs(1);
const CHILD_REAP_TIMEOUT: Duration = Duration::from_secs(1);

/// Executes commands through the system `ssh` client.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SshCliRemote {
    user: String,
    host: String,
    port: u16,
    key_path: PathBuf,
}

impl SshCliRemote {
    #[must_use]
    pub fn new(
        user: impl Into<String>,
        host: impl Into<String>,
        port: u16,
        key_path: impl Into<PathBuf>,
    ) -> Self {
        Self {
            user: user.into(),
            host: host.into(),
            port,
            key_path: key_path.into(),
        }
    }

    /// Constructs the exact subprocess invocation used by [`run`](Self::run).
    #[must_use]
    pub fn command(&self, remote_command: &str) -> Command {
        let mut command = Command::new("ssh");
        command.args([
            OsString::from("-o"),
            OsString::from("ConnectTimeout=5"),
            OsString::from("-o"),
            OsString::from("StrictHostKeyChecking=accept-new"),
            OsString::from("-T"),
        ]);
        if self.port != 0 {
            command.arg("-p").arg(self.port.to_string());
        }
        if !self.key_path.as_os_str().is_empty() {
            command.arg("-i").arg(&self.key_path);
        }
        command.arg(self.destination()).arg(remote_command);
        command
    }

    #[must_use]
    pub fn destination(&self) -> String {
        if self.user.is_empty() {
            self.host.clone()
        } else {
            format!("{}@{}", self.user, self.host)
        }
    }

    /// Runs a command and returns trimmed stdout.
    ///
    /// This is intentionally a blocking API around `std::process`; async
    /// callers should invoke it on a blocking worker. Cancellation remains
    /// concurrent because [`Cancellation`] is thread-safe.
    pub fn run(
        &self,
        cancellation: &Cancellation,
        remote_command: &str,
    ) -> Result<String, CliCommandError> {
        let (child, child_stdout, child_stderr) = self.spawn_piped(remote_command)?;
        let capture = supervise_child(child, cancellation, child_stdout, child_stderr, None, None);
        let stdout = trim_go_space(capture.stdout);
        match capture.result {
            Ok(status) if status.success() => Ok(String::from_utf8_lossy(&stdout).into_owned()),
            Ok(status) => {
                let mut error = CliCommandError::exit(status);
                error.output = stdout;
                error.stderr = capture.stderr;
                error.include_stderr = true;
                Err(error)
            }
            Err(mut error) => {
                error.output = stdout;
                error.stderr = capture.stderr;
                error.include_stderr = true;
                Err(error)
            }
        }
    }

    /// Runs a command and copies stdout and stderr to independent writers.
    ///
    /// The writers are moved into supervised worker threads so a blocking
    /// writer cannot prevent process cancellation or pipe cleanup.
    pub fn stream<Stdout, Stderr>(
        &self,
        cancellation: &Cancellation,
        remote_command: &str,
        stdout: Stdout,
        stderr: Stderr,
    ) -> Result<(), CliCommandError>
    where
        Stdout: Write + Send + 'static,
        Stderr: Write + Send + 'static,
    {
        let (child, child_stdout, child_stderr) = self.spawn_piped(remote_command)?;
        let (forwarders, completions) = OutputForwarders::spawn(stdout, stderr);
        let capture = supervise_child(
            child,
            cancellation,
            child_stdout,
            child_stderr,
            Some(&forwarders),
            Some(&completions),
        );
        drop(forwarders);
        let status = match capture.result {
            Ok(status) => {
                finish_output_writers(&completions)?;
                status
            }
            Err(error) => return Err(error),
        };
        if status.success() {
            Ok(())
        } else {
            Err(CliCommandError::exit(status))
        }
    }

    /// No-op: system SSH invocations do not share a persistent connection.
    pub fn close(&self) {}

    fn spawn_piped(
        &self,
        remote_command: &str,
    ) -> Result<(Child, ChildStdout, ChildStderr), CliCommandError> {
        let mut child = self.command(remote_command);
        child.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = child.spawn().map_err(CliCommandError::spawn)?;
        let stdout = child
            .stdout
            .take()
            .expect("stdout was configured as a pipe");
        let stderr = child
            .stderr
            .take()
            .expect("stderr was configured as a pipe");
        Ok((child, stdout, stderr))
    }
}

#[derive(Clone, Copy, Debug)]
enum OutputStream {
    Stdout,
    Stderr,
}

struct OutputForwarders {
    stdout: Sender<Vec<u8>>,
    stderr: Sender<Vec<u8>>,
}

type OutputCompletion = (OutputStream, io::Result<()>);

impl OutputForwarders {
    fn spawn(
        mut stdout: impl Write + Send + 'static,
        mut stderr: impl Write + Send + 'static,
    ) -> (Self, Receiver<OutputCompletion>) {
        let (stdout_tx, stdout_rx) = mpsc::channel::<Vec<u8>>();
        let (stderr_tx, stderr_rx) = mpsc::channel::<Vec<u8>>();
        let (completion_tx, completion_rx) = mpsc::channel();
        let stdout_completion = completion_tx.clone();
        thread::spawn(move || {
            let result = copy_chunks(&stdout_rx, &mut stdout);
            let _ = stdout_completion.send((OutputStream::Stdout, result));
        });
        thread::spawn(move || {
            let result = copy_chunks(&stderr_rx, &mut stderr);
            let _ = completion_tx.send((OutputStream::Stderr, result));
        });
        (
            Self {
                stdout: stdout_tx,
                stderr: stderr_tx,
            },
            completion_rx,
        )
    }
}

fn copy_chunks(chunks: &Receiver<Vec<u8>>, writer: &mut impl Write) -> io::Result<()> {
    for chunk in chunks {
        writer.write_all(&chunk)?;
    }
    Ok(())
}

fn finish_output_writers(completions: &Receiver<OutputCompletion>) -> Result<(), CliCommandError> {
    for _ in 0..2 {
        match completions.recv() {
            Ok((_stream, Ok(()))) => {}
            Ok((OutputStream::Stdout, Err(source))) => {
                return Err(CliCommandError::write_stdout(source));
            }
            Ok((OutputStream::Stderr, Err(source))) => {
                return Err(CliCommandError::write_stderr(source));
            }
            Err(_) => {
                return Err(CliCommandError::other("SSH output writer stopped"));
            }
        }
    }
    Ok(())
}

struct ChildCapture {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    result: Result<ExitStatus, CliCommandError>,
}

struct OwnedChild(Option<Child>);

impl OwnedChild {
    fn new(child: Child) -> Self {
        Self(Some(child))
    }
}

impl Deref for OwnedChild {
    type Target = Child;

    fn deref(&self) -> &Self::Target {
        self.0.as_ref().expect("owned child is present")
    }
}

impl DerefMut for OwnedChild {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.0.as_mut().expect("owned child is present")
    }
}

impl Drop for OwnedChild {
    fn drop(&mut self) {
        let Some(mut child) = self.0.take() else {
            return;
        };
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        let _ = child.kill();
        let deadline = Instant::now() + CHILD_REAP_TIMEOUT;
        while Instant::now() < deadline {
            if child.try_wait().ok().flatten().is_some() {
                return;
            }
            thread::sleep(WAIT_POLL_INTERVAL);
        }
        // The method remains bounded, but the child still has an explicit
        // owner responsible for the final wait rather than becoming a zombie.
        thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

#[cfg(unix)]
#[derive(Clone, Copy)]
struct SupervisorTimeouts {
    interrupt: Duration,
    force_kill_reap: Duration,
}

#[cfg(unix)]
fn supervise_child(
    child: Child,
    cancellation: &Cancellation,
    child_stdout: ChildStdout,
    child_stderr: ChildStderr,
    forwarders: Option<&OutputForwarders>,
    completions: Option<&Receiver<OutputCompletion>>,
) -> ChildCapture {
    supervise_child_with_timeouts(
        child,
        cancellation,
        child_stdout,
        child_stderr,
        forwarders,
        completions,
        SupervisorTimeouts {
            interrupt: INTERRUPT_GRACE,
            force_kill_reap: FORCE_KILL_REAP_GRACE,
        },
    )
}

#[cfg(unix)]
fn supervise_child_with_timeouts(
    child: Child,
    cancellation: &Cancellation,
    mut child_stdout: ChildStdout,
    mut child_stderr: ChildStderr,
    forwarders: Option<&OutputForwarders>,
    completions: Option<&Receiver<OutputCompletion>>,
    timeouts: SupervisorTimeouts,
) -> ChildCapture {
    let mut child = OwnedChild::new(child);
    if let Err(source) = set_nonblocking(&child_stdout) {
        let _ = child.kill();
        return ChildCapture {
            stdout: Vec::new(),
            stderr: Vec::new(),
            result: Err(CliCommandError::read_stdout(source)),
        };
    }
    if let Err(source) = set_nonblocking(&child_stderr) {
        let _ = child.kill();
        return ChildCapture {
            stdout: Vec::new(),
            stderr: Vec::new(),
            result: Err(CliCommandError::read_stderr(source)),
        };
    }

    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let mut stdout_open = true;
    let mut stderr_open = true;
    let mut interrupt_deadline = None;
    let mut force_kill_deadline = None;
    let mut pipe_deadline = None;
    let mut status = None;
    loop {
        if let Some(completions) = completions
            && let Ok((stream, result)) = completions.try_recv()
        {
            let source = match result {
                Ok(()) => io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "SSH output writer stopped before its input closed",
                ),
                Err(source) => source,
            };
            let _ = child.kill();
            return ChildCapture {
                stdout,
                stderr,
                result: Err(match stream {
                    OutputStream::Stdout => CliCommandError::write_stdout(source),
                    OutputStream::Stderr => CliCommandError::write_stderr(source),
                }),
            };
        }
        if stdout_open {
            match read_available(
                &mut child_stdout,
                &mut stdout,
                forwarders.map(|pipes| &pipes.stdout),
            ) {
                Ok(open) => stdout_open = open,
                Err(PumpError::Read(source)) => {
                    let _ = child.kill();
                    return ChildCapture {
                        stdout,
                        stderr,
                        result: Err(CliCommandError::read_stdout(source)),
                    };
                }
                Err(PumpError::Write(source)) => {
                    let _ = child.kill();
                    return ChildCapture {
                        stdout,
                        stderr,
                        result: Err(CliCommandError::write_stdout(source)),
                    };
                }
            }
        }
        if stderr_open {
            match read_available(
                &mut child_stderr,
                &mut stderr,
                forwarders.map(|pipes| &pipes.stderr),
            ) {
                Ok(open) => stderr_open = open,
                Err(PumpError::Read(source)) => {
                    let _ = child.kill();
                    return ChildCapture {
                        stdout,
                        stderr,
                        result: Err(CliCommandError::read_stderr(source)),
                    };
                }
                Err(PumpError::Write(source)) => {
                    let _ = child.kill();
                    return ChildCapture {
                        stdout,
                        stderr,
                        result: Err(CliCommandError::write_stderr(source)),
                    };
                }
            }
        }

        if cancellation.is_cancelled() && interrupt_deadline.is_none() {
            if let Err(source) = send_interrupt(&child) {
                let _ = child.kill();
                return ChildCapture {
                    stdout,
                    stderr,
                    result: Err(CliCommandError::interrupt(source)),
                };
            }
            interrupt_deadline = Some(Instant::now() + timeouts.interrupt);
        }
        status = match child.try_wait() {
            Ok(next) => next.or(status),
            Err(source) => {
                let _ = child.kill();
                return ChildCapture {
                    stdout,
                    stderr,
                    result: Err(CliCommandError::wait(source)),
                };
            }
        };
        if status.is_some() && pipe_deadline.is_none() {
            pipe_deadline = Some(Instant::now() + timeouts.interrupt);
        }
        if let Some(status) = status.filter(|_| !stdout_open && !stderr_open) {
            if cancellation.is_cancelled() {
                return ChildCapture {
                    stdout,
                    stderr,
                    result: Err(CliCommandError::cancelled()),
                };
            }
            return ChildCapture {
                stdout,
                stderr,
                result: Ok(status),
            };
        }
        if interrupt_deadline.is_some_and(|deadline| Instant::now() >= deadline)
            && force_kill_deadline.is_none()
        {
            if let Err(source) = child.kill() {
                return ChildCapture {
                    stdout,
                    stderr,
                    result: Err(CliCommandError::wait(source)),
                };
            }
            force_kill_deadline = Some(Instant::now() + timeouts.force_kill_reap);
        }
        if force_kill_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return ChildCapture {
                stdout,
                stderr,
                result: Err(CliCommandError::cancelled()),
            };
        }
        if pipe_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            return ChildCapture {
                stdout,
                stderr,
                result: Err(if cancellation.is_cancelled() {
                    CliCommandError::cancelled()
                } else {
                    CliCommandError::other("SSH output pipes did not close before timeout")
                }),
            };
        }
        thread::sleep(WAIT_POLL_INTERVAL);
    }
}

#[cfg(unix)]
enum PumpError {
    Read(io::Error),
    Write(io::Error),
}

#[cfg(unix)]
fn read_available(
    reader: &mut impl Read,
    output: &mut Vec<u8>,
    forward: Option<&Sender<Vec<u8>>>,
) -> Result<bool, PumpError> {
    let mut buffer = [0_u8; 16 * 1024];
    for _ in 0..16 {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(false),
            Ok(read) => {
                if let Some(forward) = forward {
                    forward.send(buffer[..read].to_vec()).map_err(|_| {
                        PumpError::Write(io::Error::new(
                            io::ErrorKind::BrokenPipe,
                            "SSH output writer stopped",
                        ))
                    })?;
                } else {
                    output.extend_from_slice(&buffer[..read]);
                }
            }
            Err(source) if source.kind() == io::ErrorKind::WouldBlock => return Ok(true),
            Err(source) if source.kind() == io::ErrorKind::Interrupted => {}
            Err(source) => return Err(PumpError::Read(source)),
        }
    }
    Ok(true)
}

#[cfg(unix)]
fn set_nonblocking(descriptor: &impl AsRawFd) -> io::Result<()> {
    unsafe extern "C" {
        fn fcntl(fd: i32, command: i32, ...) -> i32;
    }
    const F_GETFL: i32 = 3;
    const F_SETFL: i32 = 4;
    #[cfg(any(target_os = "linux", target_os = "android"))]
    const O_NONBLOCK: i32 = 0x800;
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    const O_NONBLOCK: i32 = 0x4;

    // SAFETY: `fcntl` receives a live borrowed descriptor and commands which
    // only inspect or update that descriptor's status flags.
    let flags = unsafe { fcntl(descriptor.as_raw_fd(), F_GETFL) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: as above; preserving the existing flags avoids changing any
    // status other than nonblocking reads.
    if unsafe { fcntl(descriptor.as_raw_fd(), F_SETFL, flags | O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

#[cfg(not(unix))]
fn supervise_child(
    child: Child,
    cancellation: &Cancellation,
    mut child_stdout: ChildStdout,
    mut child_stderr: ChildStderr,
    forwarders: Option<&OutputForwarders>,
    _completions: Option<&Receiver<OutputCompletion>>,
) -> ChildCapture {
    let mut child = OwnedChild::new(child);
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    let result = child
        .wait()
        .map_err(CliCommandError::wait)
        .and_then(|status| {
            child_stdout
                .read_to_end(&mut stdout)
                .map_err(CliCommandError::read_stdout)?;
            child_stderr
                .read_to_end(&mut stderr)
                .map_err(CliCommandError::read_stderr)?;
            if let Some(forwarders) = forwarders {
                forwarders
                    .stdout
                    .send(stdout.clone())
                    .map_err(|_| CliCommandError::other("SSH stdout writer stopped"))?;
                forwarders
                    .stderr
                    .send(stderr.clone())
                    .map_err(|_| CliCommandError::other("SSH stderr writer stopped"))?;
            }
            if cancellation.is_cancelled() {
                Err(CliCommandError::cancelled())
            } else {
                Ok(status)
            }
        });
    ChildCapture {
        stdout,
        stderr,
        result,
    }
}

#[cfg(unix)]
fn send_interrupt(child: &Child) -> io::Result<()> {
    unsafe extern "C" {
        fn kill(pid: i32, signal: i32) -> i32;
    }
    const SIGINT: i32 = 2;
    // SAFETY: `kill` accepts any process ID and signal integer. The child ID is
    // live at this point, and errors are recovered from `errno`.
    if unsafe { kill(child.id() as i32, SIGINT) } == -1 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(unix))]
fn send_interrupt(_child: &Child) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "SSH graceful interruption requires Unix",
    ))
}

/// Failure from a system-SSH command.
#[derive(Debug)]
pub struct CliCommandError {
    kind: CliErrorKind,
    output: Vec<u8>,
    stderr: Vec<u8>,
    include_stderr: bool,
}

#[derive(Debug)]
enum CliErrorKind {
    Spawn(io::Error),
    ReadStdout(io::Error),
    ReadStderr(io::Error),
    WriteStdout(io::Error),
    WriteStderr(io::Error),
    Wait(io::Error),
    Interrupt(io::Error),
    Cancelled(CancelledError),
    Exit(ExitStatus),
    Other(&'static str),
}

impl CliCommandError {
    fn new(kind: CliErrorKind) -> Self {
        Self {
            kind,
            output: Vec::new(),
            stderr: Vec::new(),
            include_stderr: false,
        }
    }
    fn spawn(source: io::Error) -> Self {
        Self::new(CliErrorKind::Spawn(source))
    }
    fn read_stdout(source: io::Error) -> Self {
        Self::new(CliErrorKind::ReadStdout(source))
    }
    fn read_stderr(source: io::Error) -> Self {
        Self::new(CliErrorKind::ReadStderr(source))
    }
    fn write_stdout(source: io::Error) -> Self {
        Self::new(CliErrorKind::WriteStdout(source))
    }
    fn write_stderr(source: io::Error) -> Self {
        Self::new(CliErrorKind::WriteStderr(source))
    }
    fn wait(source: io::Error) -> Self {
        Self::new(CliErrorKind::Wait(source))
    }
    fn interrupt(source: io::Error) -> Self {
        Self::new(CliErrorKind::Interrupt(source))
    }
    fn cancelled() -> Self {
        Self::new(CliErrorKind::Cancelled(CancelledError))
    }
    fn exit(status: ExitStatus) -> Self {
        Self::new(CliErrorKind::Exit(status))
    }
    fn other(message: &'static str) -> Self {
        Self::new(CliErrorKind::Other(message))
    }
    #[must_use]
    pub fn output(&self) -> &[u8] {
        &self.output
    }

    #[must_use]
    pub fn stderr(&self) -> &[u8] {
        &self.stderr
    }
}

impl fmt::Display for CliCommandError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            CliErrorKind::Spawn(source) => {
                write!(formatter, "run command on remote host: start ssh: {source}")
            }
            CliErrorKind::ReadStdout(source) => write!(
                formatter,
                "run command on remote host: read stdout: {source}"
            ),
            CliErrorKind::ReadStderr(source) => write!(
                formatter,
                "run command on remote host: read stderr: {source}"
            ),
            CliErrorKind::WriteStdout(source) => write!(
                formatter,
                "run command on remote host: write stdout: {source}"
            ),
            CliErrorKind::WriteStderr(source) => write!(
                formatter,
                "run command on remote host: write stderr: {source}"
            ),
            CliErrorKind::Wait(source) => write!(
                formatter,
                "run command on remote host: wait for ssh: {source}"
            ),
            CliErrorKind::Interrupt(source) => {
                write!(formatter, "send interrupt signal to SSH process: {source}")
            }
            CliErrorKind::Cancelled(source) => write!(formatter, "canceled: {source}"),
            CliErrorKind::Exit(status) => {
                write!(formatter, "run command on remote host: {status}")
            }
            CliErrorKind::Other(message) => formatter.write_str(message),
        }?;
        if self.include_stderr {
            write!(formatter, ": {}", String::from_utf8_lossy(&self.stderr))?;
        }
        Ok(())
    }
}

impl Error for CliCommandError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match &self.kind {
            CliErrorKind::Spawn(source)
            | CliErrorKind::ReadStdout(source)
            | CliErrorKind::ReadStderr(source)
            | CliErrorKind::WriteStdout(source)
            | CliErrorKind::WriteStderr(source)
            | CliErrorKind::Wait(source)
            | CliErrorKind::Interrupt(source) => Some(source),
            CliErrorKind::Cancelled(source) => Some(source),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    struct BlockingWriter;

    impl Write for BlockingWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            loop {
                thread::park();
            }
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    struct FailingWriter;

    impl Write for FailingWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::other("writer failed"))
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::other("flush must not be called"))
        }
    }

    struct DelayedWriter {
        output: Arc<Mutex<Vec<u8>>>,
    }

    #[derive(Default)]
    struct FlushFailWriter(Vec<u8>);

    impl Write for FlushFailWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.0.extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::other("flush must not be called"))
        }
    }

    impl Write for DelayedWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            thread::sleep(Duration::from_millis(1));
            self.output.lock().unwrap().extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::other("flush must not be called"))
        }
    }

    #[test]
    fn command_arguments_match_oracle() {
        let cases = [
            (
                SshCliRemote::new("", "example.com", 0, PathBuf::new()),
                "whoami",
                vec![
                    "-o",
                    "ConnectTimeout=5",
                    "-o",
                    "StrictHostKeyChecking=accept-new",
                    "-T",
                    "example.com",
                    "whoami",
                ],
            ),
            (
                SshCliRemote::new("admin", "server.local", 2222, "~/.ssh/id_rsa"),
                "sudo bash -c 'echo hello'",
                vec![
                    "-o",
                    "ConnectTimeout=5",
                    "-o",
                    "StrictHostKeyChecking=accept-new",
                    "-T",
                    "-p",
                    "2222",
                    "-i",
                    "~/.ssh/id_rsa",
                    "admin@server.local",
                    "sudo bash -c 'echo hello'",
                ],
            ),
        ];

        for (remote, remote_command, expected) in cases {
            let command = remote.command(remote_command);
            assert_eq!(
                command
                    .get_args()
                    .map(|argument| argument.to_string_lossy().into_owned())
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }

    #[test]
    fn cancellation_sends_interrupt_before_force_kill_deadline() {
        let cancellation = Cancellation::new();
        let cancel_from_thread = cancellation.clone();
        let cancel_task = thread::spawn(move || {
            thread::sleep(Duration::from_millis(25));
            cancel_from_thread.cancel();
        });
        let mut child = Command::new("sh")
            .args(["-c", "trap 'exit 0' INT; while :; do :; done"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let child_stdout = child.stdout.take().unwrap();
        let child_stderr = child.stderr.take().unwrap();

        let capture = supervise_child(child, &cancellation, child_stdout, child_stderr, None, None);
        let error = capture.result.unwrap_err();
        cancel_task.join().unwrap();
        assert!(matches!(error.kind, CliErrorKind::Cancelled(_)));
    }

    #[test]
    fn blocked_output_writer_cannot_mask_cancellation() {
        let mut child = Command::new("sh")
            .args([
                "-c",
                "printf blocked; trap 'exit 0' INT; while :; do :; done",
            ])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let pid = child.id();
        let child_stdout = child.stdout.take().unwrap();
        let child_stderr = child.stderr.take().unwrap();
        let (forwarders, completions) = OutputForwarders::spawn(BlockingWriter, io::sink());
        let cancellation = Cancellation::new();
        let cancel_from_thread = cancellation.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(25));
            cancel_from_thread.cancel();
        });

        let capture = supervise_child(
            child,
            &cancellation,
            child_stdout,
            child_stderr,
            Some(&forwarders),
            Some(&completions),
        );
        drop(forwarders);
        let error = capture.result.unwrap_err();
        assert!(matches!(error.kind, CliErrorKind::Cancelled(_)));
        assert!(error.source().is_some());
        assert!(!process_exists(pid), "cancelled SSH child was not reaped");
    }

    #[cfg(unix)]
    #[test]
    fn inherited_output_descriptors_are_abandoned_at_cleanup_deadline() {
        let mut child = Command::new("sh")
            .args(["-c", "sleep 30 & echo $!; exit 0"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let child_stdout = child.stdout.take().unwrap();
        let child_stderr = child.stderr.take().unwrap();
        let capture = supervise_child_with_timeouts(
            child,
            &Cancellation::new(),
            child_stdout,
            child_stderr,
            None,
            None,
            SupervisorTimeouts {
                interrupt: Duration::from_millis(100),
                force_kill_reap: Duration::from_millis(50),
            },
        );

        assert!(matches!(
            capture.result,
            Err(CliCommandError {
                kind: CliErrorKind::Other(_),
                ..
            })
        ));
        let descendant = String::from_utf8(capture.stdout).unwrap();
        let _ = Command::new("kill").arg(descendant.trim()).status();
    }

    #[cfg(unix)]
    #[test]
    fn writer_failure_kills_and_reaps_child() {
        let mut child = Command::new("sh")
            .args(["-c", "while :; do printf '0123456789abcdef'; done"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let pid = child.id();
        let child_stdout = child.stdout.take().unwrap();
        let child_stderr = child.stderr.take().unwrap();
        let (forwarders, completions) = OutputForwarders::spawn(FailingWriter, io::sink());
        let capture = supervise_child(
            child,
            &Cancellation::new(),
            child_stdout,
            child_stderr,
            Some(&forwarders),
            Some(&completions),
        );
        drop(forwarders);

        assert!(matches!(
            capture.result,
            Err(CliCommandError {
                kind: CliErrorKind::WriteStdout(_),
                ..
            })
        ));
        assert!(!process_exists(pid), "killed SSH child was not reaped");
    }

    #[cfg(unix)]
    #[test]
    fn slow_writer_receives_large_output_losslessly() {
        const OUTPUT_SIZE: usize = 512 * 1024;
        let mut child = Command::new("sh")
            .args(["-c", "head -c 524288 /dev/zero"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let child_stdout = child.stdout.take().unwrap();
        let child_stderr = child.stderr.take().unwrap();
        let output = Arc::new(Mutex::new(Vec::new()));
        let (forwarders, completions) = OutputForwarders::spawn(
            DelayedWriter {
                output: Arc::clone(&output),
            },
            io::sink(),
        );
        let capture = supervise_child(
            child,
            &Cancellation::new(),
            child_stdout,
            child_stderr,
            Some(&forwarders),
            Some(&completions),
        );
        drop(forwarders);

        assert!(capture.result.unwrap().success());
        finish_output_writers(&completions).unwrap();
        assert_eq!(output.lock().unwrap().len(), OUTPUT_SIZE);
    }

    #[test]
    fn writer_is_not_implicitly_flushed() {
        let (sender, receiver) = mpsc::channel();
        sender.send(b"output".to_vec()).unwrap();
        drop(sender);
        let mut writer = FlushFailWriter::default();

        copy_chunks(&receiver, &mut writer).unwrap();
        assert_eq!(writer.0, b"output");
    }

    #[cfg(unix)]
    fn process_exists(pid: u32) -> bool {
        unsafe extern "C" {
            fn kill(pid: i32, signal: i32) -> i32;
        }
        // SAFETY: signal zero performs permission/existence checking without
        // delivering a signal; `pid` came from a child created by this test.
        unsafe { kill(pid as i32, 0) == 0 }
    }
}
