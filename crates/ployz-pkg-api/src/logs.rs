use std::fmt;
use std::time::SystemTime;

use ployz_internal_machine_api_pb as pb;

pub const SYSTEM_SERVICE_CORROSION: &str = "corrosion";
pub const SYSTEM_SERVICE_DOCKER: &str = "docker";
pub const SYSTEM_SERVICE_UNCLOUD: &str = "uncloud";
pub const SYSTEM_SERVICES: [&str; 3] = [
    SYSTEM_SERVICE_CORROSION,
    SYSTEM_SERVICE_DOCKER,
    SYSTEM_SERVICE_UNCLOUD,
];

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
#[repr(i32)]
pub enum LogStreamType {
    #[default]
    Unknown = 0,
    Stdout = 1,
    Stderr = 2,
    Heartbeat = 3,
}

impl From<pb::log_entry::StreamType> for LogStreamType {
    fn from(stream: pb::log_entry::StreamType) -> Self {
        match stream {
            pb::log_entry::StreamType::Stdout => Self::Stdout,
            pb::log_entry::StreamType::Stderr => Self::Stderr,
            pb::log_entry::StreamType::Heartbeat => Self::Heartbeat,
            pb::log_entry::StreamType::Unknown => Self::Unknown,
        }
    }
}

impl From<LogStreamType> for pb::log_entry::StreamType {
    fn from(stream: LogStreamType) -> Self {
        match stream {
            LogStreamType::Stdout => Self::Stdout,
            LogStreamType::Stderr => Self::Stderr,
            LogStreamType::Heartbeat => Self::Heartbeat,
            LogStreamType::Unknown => Self::Unknown,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ServiceLogsOptions {
    pub follow: bool,
    pub tail: isize,
    pub since: String,
    pub until: String,
    pub containers: Vec<String>,
    pub machines: Vec<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ServiceLogEntryMetadata {
    pub service_id: String,
    pub service_name: String,
    pub container_id: String,
    pub machine_id: String,
    pub machine_name: String,
    pub hook: String,
}

#[derive(Debug)]
pub struct LogEntry {
    pub stream: LogStreamType,
    pub timestamp: Option<SystemTime>,
    pub message: Vec<u8>,
    pub error: Option<Box<dyn std::error::Error + Send + Sync>>,
}

impl Default for LogEntry {
    fn default() -> Self {
        Self {
            stream: LogStreamType::Unknown,
            timestamp: None,
            message: Vec::new(),
            error: None,
        }
    }
}

#[derive(Debug, Default)]
pub struct ServiceLogEntry {
    pub metadata: ServiceLogEntryMetadata,
    pub entry: LogEntry,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LogStreamStalled;

impl fmt::Display for LogStreamStalled {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("log stream stopped responding")
    }
}

impl std::error::Error for LogStreamStalled {}
