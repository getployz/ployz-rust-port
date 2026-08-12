use std::error::Error;
use std::fmt;
use std::fmt::Write as _;
use std::io::{self, Write};
use std::time::SystemTime;

use jiff::{Timestamp, tz::TimeZone};
use ployz_internal_cli_tui::{BOLD, BOLD_RED, BOLD_YELLOW, Color, FAINT};
use ployz_pkg_api::{LogStreamStalled, LogStreamType, ServiceLogEntry};

const PALETTE: [Color; 10] = [
    Color::Green,
    Color::Yellow,
    Color::Blue,
    Color::Magenta,
    Color::Cyan,
    Color::DarkGreen,
    Color::DarkYellow,
    Color::DarkBlue,
    Color::DarkMagenta,
    Color::DarkCyan,
];

const GO_ZERO_TIME_UNIX_SECONDS: i64 = -62_135_596_800;

/// Destination selected by a log entry's stream type.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OutputStream {
    Stdout,
    Stderr,
}

/// A rendered entry and its required output destination.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FormattedEntry {
    pub stream: OutputStream,
    pub bytes: Vec<u8>,
}

/// Failure while rendering or writing an entry.
#[derive(Debug)]
pub enum FormatterError {
    Timestamp(jiff::Error),
    Io(io::Error),
}

impl fmt::Display for FormatterError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Timestamp(error) => write!(formatter, "format log timestamp: {error}"),
            Self::Io(error) => write!(formatter, "write formatted log entry: {error}"),
        }
    }
}

impl Error for FormatterError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Timestamp(error) => Some(error),
            Self::Io(error) => Some(error),
        }
    }
}

impl From<jiff::Error> for FormatterError {
    fn from(error: jiff::Error) -> Self {
        Self::Timestamp(error)
    }
}

impl From<io::Error> for FormatterError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// Formats log entries with stable source columns and palette assignment.
pub struct Formatter {
    machine_names: Vec<String>,
    service_names: Vec<String>,
    max_machine_width: usize,
    max_service_width: usize,
    time_zone: TimeZone,
}

impl Formatter {
    /// Builds a formatter and fixes its local time zone for the formatter's
    /// lifetime. Known names are sorted before colors are assigned.
    #[must_use]
    pub fn new(mut machine_names: Vec<String>, mut service_names: Vec<String>, utc: bool) -> Self {
        machine_names.sort_unstable();
        service_names.sort_unstable();

        let max_machine_width = machine_names.iter().map(String::len).max().unwrap_or(0);
        let max_service_width = service_names.iter().map(String::len).max().unwrap_or(0);
        let time_zone = if utc {
            TimeZone::UTC
        } else {
            TimeZone::system()
        };

        Self {
            machine_names,
            service_names,
            max_machine_width,
            max_service_width,
            time_zone,
        }
    }

    /// Renders one entry. Heartbeats and unknown stream types return `None`.
    pub fn format_entry(
        &mut self,
        entry: &ServiceLogEntry,
    ) -> Result<Option<FormattedEntry>, FormatterError> {
        if let Some(error) = entry.entry.error.as_deref() {
            return Ok(Some(self.format_error(entry, error)));
        }

        let stream = match entry.entry.stream {
            LogStreamType::Stdout => OutputStream::Stdout,
            LogStreamType::Stderr => OutputStream::Stderr,
            LogStreamType::Unknown | LogStreamType::Heartbeat => return Ok(None),
        };

        let mut bytes = Vec::with_capacity(entry.entry.message.len() + 80);
        let timestamp = self.format_timestamp(entry.entry.timestamp)?;
        write!(
            bytes,
            "{} {} {} ",
            timestamp,
            self.format_machine(&entry.metadata.machine_name),
            self.format_service(
                &entry.metadata.service_name,
                &entry.metadata.container_id,
                &entry.metadata.hook
            )
        )?;
        bytes.extend_from_slice(&entry.entry.message);

        Ok(Some(FormattedEntry { stream, bytes }))
    }

    /// Routes one entry to supplied stdout and stderr writers.
    ///
    /// Returns `false` when the entry is a heartbeat or has an unknown stream.
    pub fn write_entry<WOut, WErr>(
        &mut self,
        entry: &ServiceLogEntry,
        stdout: &mut WOut,
        stderr: &mut WErr,
    ) -> Result<bool, FormatterError>
    where
        WOut: Write,
        WErr: Write,
    {
        let Some(formatted) = self.format_entry(entry)? else {
            return Ok(false);
        };

        match formatted.stream {
            OutputStream::Stdout => stdout.write_all(&formatted.bytes)?,
            OutputStream::Stderr => stderr.write_all(&formatted.bytes)?,
        }
        Ok(true)
    }

    /// Routes one entry to the process's stdout or stderr.
    pub fn print_entry(&mut self, entry: &ServiceLogEntry) -> Result<bool, FormatterError> {
        let mut stdout = io::stdout().lock();
        let mut stderr = io::stderr().lock();
        self.write_entry(entry, &mut stdout, &mut stderr)
    }

    fn format_timestamp(&self, system_time: Option<SystemTime>) -> Result<String, jiff::Error> {
        let timestamp = match system_time {
            Some(system_time) => Timestamp::try_from(system_time)?,
            None => Timestamp::new(GO_ZERO_TIME_UNIX_SECONDS, 0)?,
        };
        Ok(FAINT.render(
            timestamp
                .to_zoned(self.time_zone.clone())
                .strftime("%b %e %H:%M:%S.%3f")
                .to_string(),
        ))
    }

    fn format_machine(&mut self, name: &str) -> String {
        let mut style = BOLD.padding_right(padding(self.max_machine_width, name.len()));
        if self.service_names.len() == 1 {
            let index = find_or_append(&mut self.machine_names, name);
            style = style.foreground(PALETTE[index % PALETTE.len()]);
        }
        style.render(name)
    }

    fn format_service(&mut self, service_name: &str, container_id: &str, hook: &str) -> String {
        let mut service_style = BOLD;
        let right_padding = padding(self.max_service_width, service_name.len());
        if self.service_names.len() > 1 {
            let index = find_or_append(&mut self.service_names, service_name);
            service_style = service_style.foreground(PALETTE[index % PALETTE.len()]);
        }

        if container_id.is_empty() {
            return service_style
                .padding_right(right_padding)
                .render(service_name);
        }

        let mut output = service_style.render(service_name);
        output.push_str(
            &FAINT
                .padding_right(right_padding)
                .render(format!("/{}", byte_prefix(container_id, 5))),
        );
        if !hook.is_empty() {
            output.push_str(&FAINT.render(format!(" [{hook}]")));
        }
        output
    }

    fn format_error(
        &self,
        entry: &ServiceLogEntry,
        error: &(dyn Error + 'static),
    ) -> FormattedEntry {
        let message = if entry.metadata.service_name.is_empty() {
            BOLD_RED.render(format!("ERROR: {error}"))
        } else {
            let mut message = if entry.metadata.container_id.is_empty() {
                format!(
                    "WARNING: log stream from system service '{}' on machine '{}'",
                    entry.metadata.service_name, entry.metadata.machine_name
                )
            } else {
                format!(
                    "WARNING: log stream from container '{}/{}' on machine '{}'",
                    entry.metadata.service_name,
                    docker_short_id(&entry.metadata.container_id),
                    entry.metadata.machine_name
                )
            };

            if error_chain_contains_stalled(error) {
                message.push_str(" stopped responding");
            } else {
                write!(message, ": {error}").expect("writing to a String cannot fail");
            }
            BOLD_YELLOW.render(message)
        };

        FormattedEntry {
            stream: OutputStream::Stderr,
            bytes: format!("{message}\n").into_bytes(),
        }
    }
}

fn find_or_append(names: &mut Vec<String>, name: &str) -> usize {
    names
        .iter()
        .position(|known| known == name)
        .unwrap_or_else(|| {
            names.push(name.to_owned());
            names.len() - 1
        })
}

fn padding(max_width: usize, width: usize) -> u32 {
    u32::try_from(max_width.saturating_sub(width)).unwrap_or(u32::MAX)
}

fn docker_short_id(identifier: &str) -> &str {
    let identifier = identifier
        .split_once(':')
        .map_or(identifier, |(_, identifier)| identifier);
    byte_prefix(identifier, 12)
}

fn byte_prefix(value: &str, max_bytes: usize) -> &str {
    if value.len() <= max_bytes {
        return value;
    }

    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    &value[..end]
}

fn error_chain_contains_stalled(mut error: &(dyn Error + 'static)) -> bool {
    loop {
        if error.is::<LogStreamStalled>() {
            return true;
        }
        let Some(source) = error.source() else {
            return false;
        };
        error = source;
    }
}

#[cfg(test)]
mod tests {
    use std::fmt;
    use std::time::{Duration, UNIX_EPOCH};

    use ployz_pkg_api::{LogEntry, ServiceLogEntryMetadata};

    use super::*;

    fn entry(stream: LogStreamType, message: &[u8]) -> ServiceLogEntry {
        ServiceLogEntry {
            metadata: ServiceLogEntryMetadata {
                service_name: "web".to_owned(),
                container_id: "0123456789abcdef".to_owned(),
                machine_name: "worker-1".to_owned(),
                ..ServiceLogEntryMetadata::default()
            },
            entry: LogEntry {
                stream,
                timestamp: Some(UNIX_EPOCH + Duration::from_millis(1_735_734_645_987)),
                message: message.to_vec(),
                error: None,
            },
        }
    }

    #[test]
    fn utc_output_preserves_raw_message_bytes_and_routes_by_stream() {
        let mut formatter = Formatter::new(
            vec!["worker-long".to_owned(), "worker-1".to_owned()],
            vec!["web".to_owned()],
            true,
        );
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        assert!(
            formatter
                .write_entry(
                    &entry(LogStreamType::Stdout, b"hello\xff\n"),
                    &mut stdout,
                    &mut stderr,
                )
                .unwrap()
        );
        assert!(stderr.is_empty());
        assert!(stdout.ends_with(b"hello\xff\n"));
        let prefix = String::from_utf8(stdout[..stdout.len() - 7].to_vec()).unwrap();
        assert!(prefix.contains("Jan  1 12:30:45.987"));
        assert!(prefix.contains("worker-1"));
        assert!(prefix.contains("web"));
        assert!(prefix.contains("/01234"));

        stdout.clear();
        formatter
            .write_entry(
                &entry(LogStreamType::Stderr, b"bad\n"),
                &mut stdout,
                &mut stderr,
            )
            .unwrap();
        assert!(stdout.is_empty());
        assert!(stderr.ends_with(b"bad\n"));
    }

    #[test]
    fn ignores_unknown_and_heartbeat_entries() {
        let mut formatter = Formatter::new(vec![], vec![], true);
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();

        for stream in [LogStreamType::Unknown, LogStreamType::Heartbeat] {
            assert!(
                !formatter
                    .write_entry(&entry(stream, b"ignored"), &mut stdout, &mut stderr)
                    .unwrap()
            );
        }
        assert!(stdout.is_empty());
        assert!(stderr.is_empty());
    }

    #[test]
    fn multi_service_color_assignment_is_sorted_and_unknown_names_are_stable() {
        let mut formatter = Formatter::new(
            vec!["machine".to_owned()],
            vec!["web".to_owned(), "api".to_owned()],
            true,
        );

        let first = formatter
            .format_entry(&entry(LogStreamType::Stdout, b"one\n"))
            .unwrap()
            .unwrap();
        let second = formatter
            .format_entry(&ServiceLogEntry {
                metadata: ServiceLogEntryMetadata {
                    service_name: "new".to_owned(),
                    container_id: "abcde".to_owned(),
                    machine_name: "machine".to_owned(),
                    ..ServiceLogEntryMetadata::default()
                },
                entry: LogEntry {
                    stream: LogStreamType::Stdout,
                    timestamp: Some(UNIX_EPOCH),
                    message: b"two\n".to_vec(),
                    error: None,
                },
            })
            .unwrap()
            .unwrap();

        let first = String::from_utf8(first.bytes).unwrap();
        let second = String::from_utf8(second.bytes).unwrap();
        assert!(first.contains("\x1b[1;38;5;11mweb"));
        assert!(second.contains("\x1b[1;38;5;12mnew"));
    }

    #[test]
    fn system_service_and_hook_formats_match_the_oracle_columns() {
        let mut formatter = Formatter::new(
            vec!["a".to_owned(), "long".to_owned()],
            vec!["x".to_owned(), "service".to_owned()],
            true,
        );
        let system = ServiceLogEntry {
            metadata: ServiceLogEntryMetadata {
                service_name: "x".to_owned(),
                machine_name: "a".to_owned(),
                ..ServiceLogEntryMetadata::default()
            },
            entry: LogEntry {
                stream: LogStreamType::Stdout,
                timestamp: Some(UNIX_EPOCH),
                message: b"journal\n".to_vec(),
                error: None,
            },
        };
        let system = formatter.format_entry(&system).unwrap().unwrap();
        let plain = strip_sgr(&String::from_utf8(system.bytes).unwrap());
        assert_eq!(plain, "Jan  1 00:00:00.000 a    x       journal\n");

        let mut hooked = entry(LogStreamType::Stdout, b"hook\n");
        hooked.metadata.hook = "pre-deploy".to_owned();
        let hooked = formatter.format_entry(&hooked).unwrap().unwrap();
        let plain = strip_sgr(&String::from_utf8(hooked.bytes).unwrap());
        assert!(
            plain.contains("web/01234     [pre-deploy] hook\n"),
            "rendered entry: {plain:?}"
        );
    }

    #[test]
    fn global_stream_errors_are_red_and_source_errors_are_yellow() {
        let mut formatter = Formatter::new(vec![], vec![], true);
        let global = ServiceLogEntry {
            entry: LogEntry {
                error: Some(Box::new(io::Error::other("cluster disconnected"))),
                ..LogEntry::default()
            },
            ..ServiceLogEntry::default()
        };
        let formatted = formatter.format_entry(&global).unwrap().unwrap();
        assert_eq!(formatted.stream, OutputStream::Stderr);
        assert_eq!(
            strip_sgr(&String::from_utf8(formatted.bytes).unwrap()),
            "ERROR: cluster disconnected\n"
        );

        let mut container = entry(LogStreamType::Unknown, b"");
        container.entry.error = Some(Box::new(io::Error::other("socket closed")));
        let formatted = formatter.format_entry(&container).unwrap().unwrap();
        assert_eq!(
            strip_sgr(&String::from_utf8(formatted.bytes).unwrap()),
            "WARNING: log stream from container 'web/0123456789ab' on machine 'worker-1': socket closed\n"
        );
    }

    #[derive(Debug)]
    struct WrappedStall(LogStreamStalled);

    impl fmt::Display for WrappedStall {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("outer context")
        }
    }

    impl Error for WrappedStall {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            Some(&self.0)
        }
    }

    #[test]
    fn stalled_errors_use_the_special_warning_for_containers_and_system_services() {
        let mut formatter = Formatter::new(vec![], vec![], true);
        let mut container = entry(LogStreamType::Unknown, b"");
        container.entry.error = Some(Box::new(WrappedStall(LogStreamStalled)));
        let formatted = formatter.format_entry(&container).unwrap().unwrap();
        assert_eq!(
            strip_sgr(&String::from_utf8(formatted.bytes).unwrap()),
            "WARNING: log stream from container 'web/0123456789ab' on machine 'worker-1' stopped responding\n"
        );

        container.metadata.container_id.clear();
        let formatted = formatter.format_entry(&container).unwrap().unwrap();
        assert_eq!(
            strip_sgr(&String::from_utf8(formatted.bytes).unwrap()),
            "WARNING: log stream from system service 'web' on machine 'worker-1' stopped responding\n"
        );
    }

    #[test]
    fn absent_timestamp_preserves_go_zero_time_in_utc() {
        let mut formatter = Formatter::new(vec![], vec![], true);
        let mut entry = entry(LogStreamType::Stdout, b"zero\n");
        entry.entry.timestamp = None;

        let formatted = formatter.format_entry(&entry).unwrap().unwrap();
        assert!(
            strip_sgr(&String::from_utf8(formatted.bytes).unwrap())
                .starts_with("Jan  1 00:00:00.000 ")
        );
    }

    #[test]
    fn byte_prefixes_do_not_break_utf8_for_defensive_non_docker_input() {
        assert_eq!(byte_prefix("1234界rest", 5), "1234");
        assert_eq!(docker_short_id("kind:12345678901界rest"), "12345678901");
    }

    fn strip_sgr(value: &str) -> String {
        let mut output = String::new();
        let mut chars = value.chars().peekable();
        while let Some(character) = chars.next() {
            if character == '\x1b' && chars.peek() == Some(&'[') {
                chars.next();
                for next in chars.by_ref() {
                    if ('@'..='~').contains(&next) {
                        break;
                    }
                }
            } else {
                output.push(character);
            }
        }
        output
    }
}
