use std::error::Error;
use std::fmt;
use std::io;
use std::time::{Duration, UNIX_EPOCH};

use ployz_internal_cli_logs::{Formatter, OutputStream};
use ployz_pkg_api::{
    LogEntry, LogStreamStalled, LogStreamType, ServiceLogEntry, ServiceLogEntryMetadata,
};

fn main() -> Result<(), Box<dyn Error>> {
    let timestamp = UNIX_EPOCH + Duration::from_nanos(1_735_734_645_987_654_321);
    let mut single = Formatter::new(
        vec!["worker-long".to_owned(), "worker-1".to_owned()],
        vec!["web".to_owned()],
        false,
    );
    emit(
        &mut single,
        &entry(LogStreamType::Stdout, Some(timestamp), b"hello\xff\n"),
    )?;
    emit(
        &mut single,
        &entry(LogStreamType::Stderr, Some(timestamp), b"bad\n"),
    )?;
    emit(
        &mut single,
        &entry(LogStreamType::Heartbeat, Some(timestamp), b"ignored"),
    )?;

    let mut multi = Formatter::new(
        vec!["long".to_owned(), "a".to_owned()],
        vec!["web".to_owned(), "api".to_owned()],
        false,
    );
    let mut multi_entry = entry(LogStreamType::Stdout, Some(timestamp), b"multi\n");
    multi_entry.metadata.machine_name = "a".to_owned();
    emit(&mut multi, &multi_entry)?;

    let mut system = entry(LogStreamType::Stdout, Some(timestamp), b"journal\n");
    system.metadata.machine_name = "a".to_owned();
    system.metadata.service_name = "api".to_owned();
    system.metadata.container_id.clear();
    emit(&mut multi, &system)?;

    let mut hooked = multi_entry;
    hooked.metadata.hook = "pre-deploy".to_owned();
    emit(&mut multi, &hooked)?;

    let mut global_error = ServiceLogEntry::default();
    global_error.entry.error = Some(Box::new(io::Error::other("cluster disconnected")));
    emit(&mut multi, &global_error)?;

    let mut stalled = entry(LogStreamType::Unknown, None, b"");
    stalled.entry.error = Some(Box::new(WrappedStall(LogStreamStalled)));
    emit(&mut multi, &stalled)?;

    let mut system_error = system;
    system_error.entry.error = Some(Box::new(io::Error::other("socket closed")));
    emit(&mut multi, &system_error)?;

    Ok(())
}

fn entry(
    stream: LogStreamType,
    timestamp: Option<std::time::SystemTime>,
    message: &[u8],
) -> ServiceLogEntry {
    ServiceLogEntry {
        metadata: ServiceLogEntryMetadata {
            service_name: "web".to_owned(),
            container_id: "0123456789abcdef".to_owned(),
            machine_name: "worker-1".to_owned(),
            ..ServiceLogEntryMetadata::default()
        },
        entry: LogEntry {
            stream,
            timestamp,
            message: message.to_vec(),
            error: None,
        },
    }
}

fn emit(formatter: &mut Formatter, entry: &ServiceLogEntry) -> Result<(), Box<dyn Error>> {
    let mut stdout = Vec::new();
    let mut stderr = Vec::new();
    if let Some(formatted) = formatter.format_entry(entry)? {
        match formatted.stream {
            OutputStream::Stdout => stdout = formatted.bytes,
            OutputStream::Stderr => stderr = formatted.bytes,
        }
    }
    // Exact Lip Gloss/iocraft SGR encoding is an approved terminal-stack
    // limitation. The differential fixture compares visible bytes and routing.
    println!("{}\t{}", hex(&strip_sgr(&stdout)), hex(&strip_sgr(&stderr)));
    Ok(())
}

fn strip_sgr(bytes: &[u8]) -> Vec<u8> {
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'\x1b' && bytes.get(index + 1) == Some(&b'[') {
            index += 2;
            while index < bytes.len() {
                let final_byte = bytes[index];
                index += 1;
                if (b'@'..=b'~').contains(&final_byte) {
                    break;
                }
            }
        } else {
            output.push(bytes[index]);
            index += 1;
        }
    }
    output
}

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;

    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
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
