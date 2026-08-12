//! Streaming access to systemd journal entries.

use std::ffi::OsString;
use std::fmt;
use std::io::{self, BufRead, BufReader};
use std::path::PathBuf;
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex, mpsc};
use std::thread::{self, JoinHandle};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use ployz_pkg_api::{LogEntry, LogStreamType, ServiceLogsOptions};

const JOURNALCTL: &str = "journalctl";
const INITIAL_SCAN_BUFFER_SIZE: usize = 4 * 1024;
const MAX_SCAN_TOKEN_SIZE: usize = 64 * 1024;

/// A configurable system journal reader.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Journal {
    executable: PathBuf,
}

impl Journal {
    /// Uses `executable` in place of the system `journalctl` command.
    ///
    /// This is useful when journalctl is installed outside `PATH` and for
    /// process-level tests of the journal protocol.
    pub fn new(executable: impl Into<PathBuf>) -> Self {
        Self {
            executable: executable.into(),
        }
    }

    /// Starts journalctl and returns its entries as a blocking iterator.
    ///
    /// Dropping the iterator cancels the child process. A non-zero journalctl
    /// exit status is intentionally ignored, matching the source service; only
    /// startup and stdout-read failures are observable.
    pub fn logs(&self, unit: &str, options: &ServiceLogsOptions) -> io::Result<LogStream> {
        let mut command = Command::new(&self.executable);
        command
            .args(journalctl_args(unit, options))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());

        let mut child = command.spawn()?;
        let stdout = child
            .stdout
            .take()
            .expect("stdout was configured as a pipe before spawning journalctl");

        let child = Arc::new(Mutex::new(Some(child)));
        let cancellation = LogCancellation {
            child: Arc::clone(&child),
        };
        let (sender, receiver) = mpsc::sync_channel(0);
        let worker_child = Arc::clone(&child);
        let worker = match thread::Builder::new()
            .name("journal-logs".into())
            .spawn(move || run_reader(stdout, sender, &worker_child))
        {
            Ok(worker) => worker,
            Err(error) => {
                stop_child(&child);
                return Err(error);
            }
        };

        Ok(LogStream {
            receiver: Some(receiver),
            cancellation,
            worker: Some(worker),
        })
    }
}

impl Default for Journal {
    fn default() -> Self {
        Self::new(JOURNALCTL)
    }
}

/// Starts the system journalctl and streams entries for `unit`.
pub fn logs(unit: &str, options: &ServiceLogsOptions) -> io::Result<LogStream> {
    Journal::default().logs(unit, options)
}

/// A blocking stream of journal entries.
///
/// The iterator owns journalctl. It waits for normal completion after stdout
/// reaches EOF and terminates the child when dropped early.
#[derive(Debug)]
pub struct LogStream {
    receiver: Option<mpsc::Receiver<LogEntry>>,
    cancellation: LogCancellation,
    worker: Option<JoinHandle<()>>,
}

impl LogStream {
    /// Returns a handle that can interrupt a blocked read from another thread.
    ///
    /// The machine service uses this handle to map request cancellation onto
    /// journalctl process cancellation while the blocking iterator is running.
    pub fn cancellation(&self) -> LogCancellation {
        self.cancellation.clone()
    }
}

impl Iterator for LogStream {
    type Item = LogEntry;

    fn next(&mut self) -> Option<Self::Item> {
        self.receiver.as_ref()?.recv().ok()
    }
}

impl Drop for LogStream {
    fn drop(&mut self) {
        // Disconnect a worker blocked on the unbuffered send before joining it.
        self.receiver.take();
        self.cancellation.cancel();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// A clonable handle for cancelling a journal log stream.
#[derive(Clone, Debug)]
pub struct LogCancellation {
    child: Arc<Mutex<Option<Child>>>,
}

impl LogCancellation {
    /// Terminates journalctl if it is still running.
    pub fn cancel(&self) {
        let mut child = self
            .child
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if let Some(child) = child.as_mut() {
            let _ = child.kill();
        }
    }
}

fn run_reader(
    stdout: ChildStdout,
    sender: mpsc::SyncSender<LogEntry>,
    child: &Arc<Mutex<Option<Child>>>,
) {
    let mut entries = EntryReader::new(BufReader::with_capacity(INITIAL_SCAN_BUFFER_SIZE, stdout));
    for entry in entries.by_ref() {
        if sender.send(entry).is_err() {
            stop_child(child);
            return;
        }
    }
    drop(entries);
    wait_for_child(child);
}

fn wait_for_child(child: &Arc<Mutex<Option<Child>>>) {
    loop {
        let finished = {
            let mut child = child.lock().unwrap_or_else(|poison| poison.into_inner());
            let Some(process) = child.as_mut() else {
                return;
            };
            match process.try_wait() {
                Ok(Some(_)) | Err(_) => {
                    child.take();
                    true
                }
                Ok(None) => false,
            }
        };
        if finished {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
}

fn stop_child(child: &Arc<Mutex<Option<Child>>>) {
    let child = child
        .lock()
        .unwrap_or_else(|poison| poison.into_inner())
        .take();
    if let Some(mut child) = child {
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn journalctl_args(unit: &str, options: &ServiceLogsOptions) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("-u"),
        OsString::from(unit),
        OsString::from("--no-hostname"),
        OsString::from("-n"),
        if options.tail > -1 {
            OsString::from(options.tail.to_string())
        } else {
            OsString::from("all")
        },
    ];

    if options.follow {
        args.push(OsString::from("-f"));
    }

    args.extend([OsString::from("-o"), OsString::from("short-unix")]);

    if !options.since.is_empty() {
        args.extend([OsString::from("-S"), OsString::from(&options.since)]);
    }
    if !options.until.is_empty() {
        args.extend([OsString::from("-U"), OsString::from(&options.until)]);
    }

    args
}

#[derive(Debug)]
struct EntryReader<R> {
    lines: ScanLines<R>,
    finished: bool,
}

impl<R: BufRead> EntryReader<R> {
    fn new(reader: R) -> Self {
        Self {
            lines: ScanLines::new(reader),
            finished: false,
        }
    }
}

impl<R: BufRead> Iterator for EntryReader<R> {
    type Item = LogEntry;

    fn next(&mut self) -> Option<Self::Item> {
        if self.finished {
            return None;
        }

        match self.lines.next_line() {
            Ok(Some(line)) => Some(entry(&line)),
            Ok(None) => {
                self.finished = true;
                None
            }
            Err(error) => {
                self.finished = true;
                Some(LogEntry {
                    error: Some(Box::new(JournalReadError(error))),
                    ..LogEntry::default()
                })
            }
        }
    }
}

#[derive(Debug)]
struct ScanLines<R> {
    reader: R,
    line: Vec<u8>,
    pending_error: Option<io::Error>,
}

impl<R: BufRead> ScanLines<R> {
    fn new(reader: R) -> Self {
        Self {
            reader,
            line: Vec::new(),
            pending_error: None,
        }
    }

    fn next_line(&mut self) -> io::Result<Option<Vec<u8>>> {
        self.line.clear();
        if let Some(error) = self.pending_error.take() {
            return Err(error);
        }

        loop {
            let available = match self.reader.fill_buf() {
                Ok(available) => available,
                Err(error) if !self.line.is_empty() => {
                    self.pending_error = Some(error);
                    if self.line.last() == Some(&b'\r') {
                        self.line.pop();
                    }
                    return Ok(Some(std::mem::take(&mut self.line)));
                }
                Err(error) => return Err(error),
            };
            if available.is_empty() {
                return if self.line.is_empty() {
                    Ok(None)
                } else {
                    if self.line.last() == Some(&b'\r') {
                        self.line.pop();
                    }
                    Ok(Some(std::mem::take(&mut self.line)))
                };
            }

            if let Some(newline) = available.iter().position(|byte| *byte == b'\n') {
                let consumed = newline + 1;
                if self.line.len() + consumed > MAX_SCAN_TOKEN_SIZE {
                    return Err(token_too_long());
                }
                self.line.extend_from_slice(&available[..newline]);
                self.reader.consume(consumed);
                if self.line.last() == Some(&b'\r') {
                    self.line.pop();
                }
                return Ok(Some(std::mem::take(&mut self.line)));
            }

            if self.line.len() + available.len() >= MAX_SCAN_TOKEN_SIZE {
                return Err(token_too_long());
            }
            let consumed = available.len();
            self.line.extend_from_slice(available);
            self.reader.consume(consumed);
        }
    }
}

fn token_too_long() -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, "bufio.Scanner: token too long")
}

#[derive(Debug)]
struct JournalReadError(io::Error);

impl fmt::Display for JournalReadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "journal logs: {}", self.0)
    }
}

impl std::error::Error for JournalReadError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&self.0)
    }
}

fn entry(data: &[u8]) -> LogEntry {
    let (timestamp, message) = data
        .iter()
        .position(|byte| *byte == b' ')
        .and_then(|separator| {
            parse_unix_timestamp(&data[..separator])
                .map(|timestamp| (Some(timestamp), &data[separator + 1..]))
        })
        .unwrap_or((None, data));

    let mut message = message.to_vec();
    message.push(b'\n');

    LogEntry {
        stream: LogStreamType::Stdout,
        timestamp,
        message,
        error: None,
    }
}

fn parse_unix_timestamp(data: &[u8]) -> Option<SystemTime> {
    let separator = data.iter().position(|byte| *byte == b'.')?;
    let seconds = parse_i64(&data[..separator])?;
    let microseconds = parse_i64(&data[separator + 1..])?;
    let nanoseconds = microseconds.wrapping_mul(1_000);
    let total_nanoseconds = i128::from(seconds) * 1_000_000_000 + i128::from(nanoseconds);

    if total_nanoseconds >= 0 {
        UNIX_EPOCH.checked_add(duration_from_nanoseconds(total_nanoseconds as u128))
    } else {
        UNIX_EPOCH.checked_sub(duration_from_nanoseconds(total_nanoseconds.unsigned_abs()))
    }
}

fn parse_i64(data: &[u8]) -> Option<i64> {
    std::str::from_utf8(data).ok()?.parse().ok()
}

fn duration_from_nanoseconds(nanoseconds: u128) -> Duration {
    Duration::new(
        (nanoseconds / 1_000_000_000) as u64,
        (nanoseconds % 1_000_000_000) as u32,
    )
}

#[cfg(test)]
mod tests {
    use std::io::{BufReader, Cursor};
    use std::sync::atomic::{AtomicU64, Ordering};

    #[cfg(unix)]
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    #[test]
    fn parses_oracle_unix_timestamps() {
        let cases = [
            ("1769188773.687500", Some((1_769_188_773, 687_500_000))),
            ("1769188773.000000", Some((1_769_188_773, 0))),
            ("0.000000", Some((0, 0))),
            ("1769188773", None),
            ("abc.000000", None),
            ("1769188773.abc", None),
            ("", None),
        ];

        for (input, expected) in cases {
            let actual = parse_unix_timestamp(input.as_bytes());
            let expected = expected
                .map(|(seconds, nanoseconds)| UNIX_EPOCH + Duration::new(seconds, nanoseconds));
            assert_eq!(actual, expected, "input {input:?}");
        }
    }

    #[test]
    fn normalizes_signed_and_out_of_range_microseconds_like_time_unix() {
        assert_eq!(
            parse_unix_timestamp(b"1.1500000"),
            Some(UNIX_EPOCH + Duration::from_millis(2_500))
        );
        assert_eq!(
            parse_unix_timestamp(b"1.-500000"),
            Some(UNIX_EPOCH + Duration::from_millis(500))
        );
        assert_eq!(
            parse_unix_timestamp(b"-1.500000"),
            Some(UNIX_EPOCH - Duration::from_millis(500))
        );
    }

    #[test]
    fn converts_oracle_entries_without_losing_bytes() {
        let cases: &[(&[u8], Option<Duration>, &[u8])] = &[
            (
                b"1769188773.687500 uncloudd[332455]: INFO  Starting daemon.",
                Some(Duration::new(1_769_188_773, 687_500_000)),
                b"uncloudd[332455]: INFO  Starting daemon.\n",
            ),
            (b"-- Boot 1234567890 --", None, b"-- Boot 1234567890 --\n"),
            (b"nospacehere", None, b"nospacehere\n"),
            (b"", None, b"\n"),
            (b"0.000000 \xff\xfe", Some(Duration::ZERO), b"\xff\xfe\n"),
            (b"bad  remains", None, b"bad  remains\n"),
        ];

        for (input, expected_since_epoch, expected_message) in cases {
            let actual = entry(input);
            assert_eq!(actual.stream, LogStreamType::Stdout);
            assert_eq!(
                actual.timestamp,
                expected_since_epoch.map(|duration| UNIX_EPOCH + duration)
            );
            assert_eq!(actual.message, *expected_message);
            assert!(actual.error.is_none());
        }
    }

    #[test]
    fn builds_journalctl_arguments_in_oracle_order() {
        let options = ServiceLogsOptions {
            follow: true,
            tail: -1,
            since: "yesterday".into(),
            until: "now".into(),
            ..ServiceLogsOptions::default()
        };
        assert_eq!(
            journalctl_args("uncloud.service", &options),
            os_strings(&[
                "-u",
                "uncloud.service",
                "--no-hostname",
                "-n",
                "all",
                "-f",
                "-o",
                "short-unix",
                "-S",
                "yesterday",
                "-U",
                "now",
            ])
        );

        let options = ServiceLogsOptions {
            tail: 0,
            ..ServiceLogsOptions::default()
        };
        assert_eq!(
            journalctl_args("docker", &options),
            os_strings(&[
                "-u",
                "docker",
                "--no-hostname",
                "-n",
                "0",
                "-o",
                "short-unix"
            ])
        );
    }

    #[test]
    fn scan_lines_matches_scanner_crlf_eof_and_empty_line_behavior() {
        let input = b"first\r\n\nlast\r";
        let mut lines = ScanLines::new(BufReader::with_capacity(2, Cursor::new(input)));
        assert_eq!(lines.next_line().unwrap(), Some(b"first".to_vec()));
        assert_eq!(lines.next_line().unwrap(), Some(Vec::new()));
        assert_eq!(lines.next_line().unwrap(), Some(b"last".to_vec()));
        assert_eq!(lines.next_line().unwrap(), None);

        let mut trailing = ScanLines::new(Cursor::new(b"one\n"));
        assert_eq!(trailing.next_line().unwrap(), Some(b"one".to_vec()));
        assert_eq!(trailing.next_line().unwrap(), None);
    }

    #[test]
    fn scan_lines_enforces_the_oracle_token_limit() {
        let mut exact = ScanLines::new(Cursor::new(vec![b'x'; MAX_SCAN_TOKEN_SIZE - 1]));
        assert_eq!(
            exact.next_line().unwrap().unwrap().len(),
            MAX_SCAN_TOKEN_SIZE - 1
        );

        let mut exact_with_newline = vec![b'x'; MAX_SCAN_TOKEN_SIZE - 1];
        exact_with_newline.push(b'\n');
        let mut exact_with_newline = ScanLines::new(Cursor::new(exact_with_newline));
        assert_eq!(
            exact_with_newline.next_line().unwrap().unwrap().len(),
            MAX_SCAN_TOKEN_SIZE - 1
        );

        let mut over = ScanLines::new(Cursor::new(vec![b'x'; MAX_SCAN_TOKEN_SIZE]));
        assert_eq!(
            over.next_line().unwrap_err().to_string(),
            "bufio.Scanner: token too long"
        );

        let mut newline_over = vec![b'x'; MAX_SCAN_TOKEN_SIZE];
        newline_over.push(b'\n');
        let mut newline_over = ScanLines::new(Cursor::new(newline_over));
        assert_eq!(
            newline_over.next_line().unwrap_err().to_string(),
            "bufio.Scanner: token too long"
        );
    }

    #[test]
    fn read_error_is_emitted_once_as_a_log_entry() {
        let mut entries = EntryReader::new(FailingReader);
        let failure = entries.next().unwrap();
        assert_eq!(failure.stream, LogStreamType::Unknown);
        assert_eq!(
            failure.error.unwrap().to_string(),
            "journal logs: broken reader"
        );
        assert!(entries.next().is_none());
    }

    #[test]
    fn buffered_fragment_is_emitted_before_a_read_error() {
        let mut entries = EntryReader::new(FragmentThenError::new(b"partial\r"));

        let fragment = entries.next().unwrap();
        assert_eq!(fragment.stream, LogStreamType::Stdout);
        assert_eq!(fragment.message, b"partial\n");
        assert!(fragment.error.is_none());

        let failure = entries.next().unwrap();
        assert_eq!(failure.stream, LogStreamType::Unknown);
        assert_eq!(
            failure.error.unwrap().to_string(),
            "journal logs: broken reader"
        );
        assert!(entries.next().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn streams_process_output_and_ignores_exit_status() {
        let fixture = Fixture::new(
            "printf '%s\\n' \"$@\" > \"$(dirname \"$0\")/args\"\n\
             printf '1769188773.687500 first\\n-- Boot marker --'\n\
             exit 19\n",
        );
        let options = ServiceLogsOptions {
            follow: true,
            tail: 3,
            since: "10 minutes ago".into(),
            until: "now".into(),
            ..ServiceLogsOptions::default()
        };

        let entries: Vec<_> = Journal::new(&fixture.executable)
            .logs("uncloud", &options)
            .unwrap()
            .collect();

        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].message, b"first\n");
        assert_eq!(entries[1].message, b"-- Boot marker --\n");
        assert!(entries.iter().all(|entry| entry.error.is_none()));
        assert_eq!(
            fs::read(fixture.directory.join("args")).unwrap(),
            b"-u\nuncloud\n--no-hostname\n-n\n3\n-f\n-o\nshort-unix\n-S\n10 minutes ago\n-U\nnow\n"
        );
    }

    #[test]
    fn returns_process_start_errors_immediately() {
        let missing = std::env::temp_dir().join(format!(
            "ployz-journal-missing-{}-{}",
            std::process::id(),
            NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
        ));
        let error = Journal::new(missing)
            .logs("uncloud", &ServiceLogsOptions::default())
            .unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_interrupts_a_blocked_follow_read() {
        let fixture = Fixture::new("while :; do :; done\n");
        let mut stream = Journal::new(&fixture.executable)
            .logs("uncloud", &ServiceLogsOptions::default())
            .unwrap();
        stream.cancellation().cancel();
        assert!(stream.next().is_none());
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_remains_available_after_stdout_closes() {
        let fixture = Fixture::new(
            "exec 1>&-\n\
             printf '%s' \"$$\" > \"$(dirname \"$0\")/pid\"\n\
             printf ready > \"$(dirname \"$0\")/ready\"\n\
             while :; do :; done\n",
        );
        let mut stream = Journal::new(&fixture.executable)
            .logs("uncloud", &ServiceLogsOptions::default())
            .unwrap();
        wait_for_file(&fixture.directory.join("ready"));
        // Give the reader time to observe EOF and enter child waiting.
        thread::sleep(Duration::from_millis(50));
        stream.cancellation().cancel();
        assert!(stream.next().is_none());
    }

    fn os_strings(values: &[&str]) -> Vec<OsString> {
        values.iter().map(OsString::from).collect()
    }

    #[cfg(unix)]
    fn wait_for_file(path: &std::path::Path) {
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while !path.exists() {
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {path:?}"
            );
            thread::yield_now();
        }
    }

    #[derive(Debug)]
    struct FailingReader;

    impl io::Read for FailingReader {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("broken reader"))
        }
    }

    impl BufRead for FailingReader {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            Err(io::Error::other("broken reader"))
        }

        fn consume(&mut self, _amount: usize) {}
    }

    #[derive(Debug)]
    struct FragmentThenError {
        fragment: Vec<u8>,
        consumed: bool,
    }

    impl FragmentThenError {
        fn new(fragment: &[u8]) -> Self {
            Self {
                fragment: fragment.to_vec(),
                consumed: false,
            }
        }
    }

    impl io::Read for FragmentThenError {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let available = self.fill_buf()?;
            let length = available.len().min(buffer.len());
            buffer[..length].copy_from_slice(&available[..length]);
            self.consume(length);
            Ok(length)
        }
    }

    impl BufRead for FragmentThenError {
        fn fill_buf(&mut self) -> io::Result<&[u8]> {
            if self.consumed {
                Err(io::Error::other("broken reader"))
            } else {
                Ok(&self.fragment)
            }
        }

        fn consume(&mut self, amount: usize) {
            assert_eq!(amount, self.fragment.len());
            self.consumed = true;
        }
    }

    static NEXT_FIXTURE: AtomicU64 = AtomicU64::new(0);

    #[cfg(unix)]
    struct Fixture {
        directory: PathBuf,
        executable: PathBuf,
    }

    #[cfg(unix)]
    impl Fixture {
        fn new(body: &str) -> Self {
            let directory = std::env::temp_dir().join(format!(
                "ployz-journal-fixture-{}-{}",
                std::process::id(),
                NEXT_FIXTURE.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&directory).unwrap();
            let executable = directory.join("journalctl");
            fs::write(&executable, format!("#!/bin/sh\n{body}")).unwrap();
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
            Self {
                directory,
                executable,
            }
        }
    }

    #[cfg(unix)]
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.directory);
        }
    }
}
