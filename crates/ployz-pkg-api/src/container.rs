use std::collections::BTreeMap;
use std::fmt;
use std::io::{Read, Write};
use std::net::IpAddr;
use std::sync::{LazyLock, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    ApiError, GoMap, GoSlice, PORT_MODE_HOST, PortSpec, Result, ServiceSpec, parse_port_spec,
};

pub const DOCKER_NETWORK_NAME: &str = "uncloud";
pub const LABEL_DAEMON_MANAGED: &str = "uncloudd.managed";
pub const LABEL_MANAGED: &str = "uncloud.managed";
pub const LABEL_SERVICE_ID: &str = "uncloud.service.id";
pub const LABEL_SERVICE_NAME: &str = "uncloud.service.name";
pub const LABEL_SERVICE_MODE: &str = "uncloud.service.mode";
pub const LABEL_SERVICE_PORTS: &str = "uncloud.service.ports";
pub const LABEL_HOOK: &str = "uncloud.service.hook";
pub const LABEL_HOOK_PRE_DEPLOY: &str = "pre-deploy";

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct HealthConfig {
    pub test: GoSlice<String>,
    pub interval: i64,
    pub timeout: i64,
    pub start_period: i64,
    pub start_interval: i64,
    pub retries: i64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct ContainerConfig {
    pub hostname: String,
    pub domainname: String,
    pub user: String,
    pub attach_stdin: bool,
    pub attach_stdout: bool,
    pub attach_stderr: bool,
    pub tty: bool,
    pub open_stdin: bool,
    pub stdin_once: bool,
    pub env: GoSlice<String>,
    pub cmd: GoSlice<String>,
    pub image: String,
    pub working_dir: String,
    pub entrypoint: GoSlice<String>,
    #[serde(default)]
    pub labels: GoMap<String, String>,
    #[serde(default)]
    pub healthcheck: Option<HealthConfig>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct ContainerHealth {
    pub status: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct ContainerState {
    pub status: String,
    pub running: bool,
    pub paused: bool,
    pub restarting: bool,
    #[serde(rename = "OOMKilled")]
    pub oom_killed: bool,
    pub dead: bool,
    pub pid: i64,
    pub exit_code: i64,
    pub error: String,
    pub started_at: String,
    pub finished_at: String,
    #[serde(default)]
    pub health: Option<ContainerHealth>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct PortBinding {
    #[serde(rename = "HostIP")]
    pub host_ip: String,
    pub host_port: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct EndpointSettings {
    #[serde(rename = "IPAddress")]
    pub ip_address: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct NetworkSettings {
    pub ports: GoMap<String, GoSlice<PortBinding>>,
    pub networks: GoMap<String, EndpointSettings>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct DockerLogConfig {
    #[serde(rename = "Type")]
    pub kind: String,
    pub config: GoMap<String, String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct DockerRestartPolicy {
    pub name: String,
    pub maximum_retry_count: i64,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct DockerMount {
    #[serde(rename = "Type")]
    pub kind: String,
    pub source: String,
    pub target: String,
    pub read_only: bool,
    pub consistency: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct ContainerMountPoint {
    #[serde(rename = "Type")]
    pub kind: String,
    pub name: String,
    pub source: String,
    pub destination: String,
    pub driver: String,
    pub mode: String,
    #[serde(rename = "RW")]
    pub read_write: bool,
    pub propagation: String,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct ContainerHostConfig {
    pub binds: GoSlice<String>,
    pub init: Option<bool>,
    pub log_config: DockerLogConfig,
    pub mounts: GoSlice<DockerMount>,
    pub pid_mode: String,
    pub port_bindings: GoMap<String, GoSlice<PortBinding>>,
    pub privileged: bool,
    pub restart_policy: DockerRestartPolicy,
    #[serde(flatten)]
    pub resources: DockerHostResources,
    pub shm_size: i64,
    pub sysctls: GoMap<String, String>,
    pub cap_add: GoSlice<String>,
    pub cap_drop: GoSlice<String>,
    pub dns: GoSlice<String>,
    #[serde(rename = "DNSOptions")]
    pub dns_options: GoSlice<String>,
    #[serde(rename = "DNSSearch")]
    pub dns_search: GoSlice<String>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct DockerHostResources {
    #[serde(rename = "NanoCPUs")]
    pub nano_cpus: i64,
    pub memory: i64,
    pub memory_reservation: i64,
    pub devices: GoSlice<serde_json::Value>,
    pub device_requests: GoSlice<serde_json::Value>,
    pub ulimits: GoSlice<serde_json::Value>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct Container {
    #[serde(rename = "Id", alias = "ID")]
    pub id: String,
    pub created: String,
    pub path: String,
    pub args: GoSlice<String>,
    pub image: String,
    pub name: String,
    #[serde(default)]
    pub config: Option<ContainerConfig>,
    #[serde(default)]
    pub state: Option<ContainerState>,
    #[serde(default)]
    pub host_config: Option<ContainerHostConfig>,
    pub mounts: GoSlice<ContainerMountPoint>,
    pub network_settings: Option<NetworkSettings>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
    #[serde(skip)]
    created_cache: Option<Rfc3339Time>,
}

impl Container {
    pub fn from_json(data: &[u8]) -> Result<Self> {
        serde_json::from_slice(data).map_err(|error| ApiError::invalid(error.to_string()))
    }

    pub fn created_time(&mut self) -> Option<Rfc3339Time> {
        if self.created_cache.is_none() && !self.created.is_empty() {
            self.created_cache = parse_rfc3339(&self.created).ok();
        }
        self.created_cache
    }

    #[must_use]
    pub fn has_healthcheck(&self) -> bool {
        self.config
            .as_ref()
            .and_then(|config| config.healthcheck.as_ref())
            .as_ref()
            .is_some_and(|healthcheck| healthcheck.test.first().is_none_or(|value| value != "NONE"))
    }

    #[must_use]
    pub fn healthy(&self) -> bool {
        self.state.as_ref().is_some_and(|state| {
            state.running
                && !state.paused
                && !state.restarting
                && state
                    .health
                    .as_ref()
                    .is_none_or(|health| health.status == "healthy")
        })
    }

    pub fn human_state(&self) -> Result<String> {
        let state = self
            .state
            .as_ref()
            .ok_or_else(|| ApiError::invalid("container state is missing"))?;
        let started = parse_rfc3339(&state.started_at)
            .map_err(|error| ApiError::invalid(format!("parse started time: {error}")))?;
        let finished = parse_rfc3339(&state.finished_at)
            .map_err(|error| ApiError::invalid(format!("parse finished time: {error}")))?;
        let now = Rfc3339Time::now();
        if state.running {
            if state.paused {
                return Ok(format!(
                    "Up {} (Paused)",
                    human_duration(now.seconds_since(started))
                ));
            }
            if state.restarting {
                return Ok(format!(
                    "Restarting ({}) {} ago",
                    state.exit_code,
                    human_duration(now.seconds_since(finished))
                ));
            }
            if let Some(health) = &state.health {
                let status = if health.status == "starting" {
                    "health: starting".to_owned()
                } else {
                    health.status.clone()
                };
                return Ok(format!(
                    "Up {} ({status})",
                    human_duration(now.seconds_since(started))
                ));
            }
            return Ok(format!("Up {}", human_duration(now.seconds_since(started))));
        }
        if state.status == "removing" {
            return Ok("Removal In Progress".into());
        }
        if state.dead {
            return Ok("Dead".into());
        }
        if started.is_go_zero() {
            return Ok("Created".into());
        }
        if finished.is_go_zero() {
            return Ok(String::new());
        }
        Ok(format!(
            "Exited ({}) {} ago",
            state.exit_code,
            human_duration(now.seconds_since(finished))
        ))
    }

    #[must_use]
    pub fn uncloud_network_ip(&self) -> Option<IpAddr> {
        self.network_settings
            .as_ref()?
            .networks
            .get(DOCKER_NETWORK_NAME)?
            .ip_address
            .parse()
            .ok()
    }
}

impl<'de> Deserialize<'de> for Container {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(Default, Deserialize)]
        #[serde(default, rename_all = "PascalCase")]
        struct Wire {
            #[serde(rename = "Id", alias = "ID")]
            id: String,
            created: String,
            path: String,
            args: GoSlice<String>,
            image: String,
            name: String,
            config: Option<ContainerConfig>,
            state: Option<ContainerState>,
            host_config: Option<ContainerHostConfig>,
            mounts: GoSlice<ContainerMountPoint>,
            network_settings: Option<NetworkSettings>,
            #[serde(flatten)]
            extra: BTreeMap<String, serde_json::Value>,
        }
        let value = serde_json::Value::deserialize(deserializer)?;
        let object = value
            .as_object()
            .ok_or_else(|| serde::de::Error::custom("container must be a JSON object"))?;
        let has_base = [
            "Id",
            "ID",
            "Created",
            "Path",
            "Args",
            "State",
            "Image",
            "Name",
            "HostConfig",
        ]
        .iter()
        .any(|field| object.contains_key(*field));
        if !has_base {
            return Err(serde::de::Error::custom(format!(
                "container data is missing mandatory base fields: {value}"
            )));
        }
        let mut wire: Wire = serde_json::from_value(value).map_err(serde::de::Error::custom)?;
        wire.extra.remove("ServiceSpec");
        Ok(Self {
            id: wire.id,
            created: wire.created,
            path: wire.path,
            args: wire.args,
            image: wire.image,
            name: wire.name.trim_start_matches('/').to_owned(),
            config: wire.config,
            state: wire.state,
            host_config: wire.host_config,
            mounts: wire.mounts,
            network_settings: wire.network_settings,
            extra: wire.extra,
            created_cache: None,
        })
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct CreateContainerResponse {
    pub id: String,
    pub warnings: Vec<String>,
    pub name: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct ServiceContainer {
    #[serde(flatten)]
    pub container: Container,
    pub service_spec: ServiceSpec,
}

impl ServiceContainer {
    pub fn from_json(data: &[u8]) -> Result<Self> {
        let container = Container::from_json(data)?;
        let value: serde_json::Value =
            serde_json::from_slice(data).map_err(|error| ApiError::invalid(error.to_string()))?;
        let service_spec = value
            .get("ServiceSpec")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(|error| ApiError::invalid(error.to_string()))?
            .unwrap_or_default();
        Ok(Self {
            container,
            service_spec,
        })
    }

    #[must_use]
    pub fn short_id(&self) -> &str {
        let id = self
            .container
            .id
            .split_once(':')
            .map_or(self.container.id.as_str(), |(_, suffix)| suffix);
        id.get(..12).unwrap_or(id)
    }

    #[must_use]
    pub fn service_id(&self) -> &str {
        self.container
            .config
            .as_ref()
            .and_then(|config| config.labels.get(LABEL_SERVICE_ID))
            .map_or("", String::as_str)
    }

    #[must_use]
    pub fn service_name(&self) -> &str {
        self.container
            .config
            .as_ref()
            .and_then(|config| config.labels.get(LABEL_SERVICE_NAME))
            .map_or("", String::as_str)
    }

    #[must_use]
    pub fn service_mode(&self) -> &str {
        &self.service_spec.mode
    }

    #[must_use]
    pub fn is_hook(&self) -> bool {
        self.container
            .config
            .as_ref()
            .is_some_and(|config| config.labels.contains_key(LABEL_HOOK))
    }

    pub fn service_ports(&self) -> Result<Vec<PortSpec>> {
        let Some(encoded) = self
            .container
            .config
            .as_ref()
            .and_then(|config| config.labels.get(LABEL_SERVICE_PORTS))
        else {
            return Ok(Vec::new());
        };
        if encoded.trim().is_empty() {
            return Ok(Vec::new());
        }
        encoded
            .split(',')
            .map(|port| parse_port_spec(port.trim()))
            .collect()
    }

    pub fn conflicting_service_ports(&self, ports: &[PortSpec]) -> Result<Vec<PortSpec>> {
        let service_ports = self
            .service_ports()
            .map_err(|error| ApiError::invalid(format!("get service ports: {error}")))?;
        let mut conflicts = Vec::new();
        for port in ports.iter().filter(|port| port.mode == PORT_MODE_HOST) {
            for service_port in &service_ports {
                if service_port.mode == PORT_MODE_HOST
                    && service_port.published_port == port.published_port
                    && service_port.protocol == port.protocol
                    && (service_port.host_ip.is_none()
                        || port.host_ip.is_none()
                        || service_port.host_ip == port.host_ip)
                {
                    conflicts.push(port.clone());
                }
            }
        }
        Ok(conflicts)
    }
}

impl<'de> Deserialize<'de> for ServiceContainer {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = serde_json::Value::deserialize(deserializer)?;
        let container = serde_json::from_value(value.clone()).map_err(serde::de::Error::custom)?;
        let service_spec = value
            .get("ServiceSpec")
            .cloned()
            .map(serde_json::from_value)
            .transpose()
            .map_err(serde::de::Error::custom)?
            .unwrap_or_default();
        Ok(Self {
            container,
            service_spec,
        })
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MachineServiceContainer {
    pub machine_id: String,
    pub machine_name: String,
    pub container: ServiceContainer,
}

pub static DEFAULT_HEALTH_MONITOR_PERIOD: LazyLock<RwLock<i64>> = LazyLock::new(|| {
    RwLock::new(
        std::env::var("UNCLOUD_HEALTH_MONITOR_PERIOD")
            .ok()
            .and_then(|value| parse_go_duration(&value))
            .unwrap_or(5_000_000_000),
    )
});

#[must_use]
pub fn default_health_monitor_period() -> i64 {
    *DEFAULT_HEALTH_MONITOR_PERIOD
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

pub fn set_default_health_monitor_period(nanoseconds: i64) {
    *DEFAULT_HEALTH_MONITOR_PERIOD
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = nanoseconds;
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct WaitContainerHealthyOptions {
    pub monitor_period: Option<i64>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct ExecOptions {
    pub command: Vec<String>,
    pub attach_stdin: bool,
    pub attach_stdout: bool,
    pub attach_stderr: bool,
    pub tty: bool,
    pub detach: bool,
    pub user: String,
    pub privileged: bool,
    pub working_dir: String,
    pub env: Vec<String>,
    #[serde(skip)]
    pub stdin: Option<Box<dyn Read + Send>>,
    #[serde(skip)]
    pub stdout: Option<Box<dyn Write + Send>>,
    #[serde(skip)]
    pub stderr: Option<Box<dyn Write + Send>>,
}

impl fmt::Debug for ExecOptions {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ExecOptions")
            .field("command", &self.command)
            .field("attach_stdin", &self.attach_stdin)
            .field("attach_stdout", &self.attach_stdout)
            .field("attach_stderr", &self.attach_stderr)
            .field("tty", &self.tty)
            .field("detach", &self.detach)
            .field("user", &self.user)
            .field("privileged", &self.privileged)
            .field("working_dir", &self.working_dir)
            .field("env", &self.env)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Rfc3339Time {
    unix_seconds: i64,
    nanos: u32,
    year: i32,
}

impl Rfc3339Time {
    fn now() -> Self {
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        Self {
            unix_seconds: duration.as_secs() as i64,
            nanos: duration.subsec_nanos(),
            year: 1970,
        }
    }

    const fn is_go_zero(self) -> bool {
        self.year == 1 && self.unix_seconds == -62_135_596_800 && self.nanos == 0
    }

    const fn seconds_since(self, earlier: Self) -> i64 {
        self.unix_seconds - earlier.unix_seconds
    }
}

fn parse_rfc3339(value: &str) -> std::result::Result<Rfc3339Time, &'static str> {
    if value.len() < 20 {
        return Err("cannot parse as RFC3339Nano");
    }
    let year: i32 = value
        .get(0..4)
        .ok_or("invalid year")?
        .parse()
        .map_err(|_| "invalid year")?;
    let month: u32 = value
        .get(5..7)
        .ok_or("invalid month")?
        .parse()
        .map_err(|_| "invalid month")?;
    let day: u32 = value
        .get(8..10)
        .ok_or("invalid day")?
        .parse()
        .map_err(|_| "invalid day")?;
    let hour: i64 = value
        .get(11..13)
        .ok_or("invalid hour")?
        .parse()
        .map_err(|_| "invalid hour")?;
    let minute: i64 = value
        .get(14..16)
        .ok_or("invalid minute")?
        .parse()
        .map_err(|_| "invalid minute")?;
    let second: i64 = value
        .get(17..19)
        .ok_or("invalid second")?
        .parse()
        .map_err(|_| "invalid second")?;
    if value.as_bytes().get(4) != Some(&b'-')
        || value.as_bytes().get(7) != Some(&b'-')
        || value.as_bytes().get(10) != Some(&b'T')
        || value.as_bytes().get(13) != Some(&b':')
        || value.as_bytes().get(16) != Some(&b':')
    {
        return Err("cannot parse as RFC3339Nano");
    }
    if !(1..=12).contains(&month)
        || day == 0
        || day > days_in_month(year, month)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=59).contains(&second)
    {
        return Err("cannot parse as RFC3339Nano");
    }
    let tail = &value[19..];
    let (fraction, zone) = if let Some(rest) = tail.strip_prefix('.') {
        let end = rest.find(['Z', '+', '-']).ok_or("missing timezone")?;
        (&rest[..end], &rest[end..])
    } else {
        ("", tail)
    };
    if fraction.len() > 9
        || (tail.starts_with('.') && fraction.is_empty())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err("invalid fractional second");
    }
    let nanos = if fraction.is_empty() {
        0
    } else {
        fraction
            .parse::<u32>()
            .map_err(|_| "invalid fractional second")?
            * 10_u32.pow(9 - fraction.len() as u32)
    };
    let offset = if zone == "Z" {
        0
    } else {
        let sign = match zone.as_bytes().first() {
            Some(b'+') => 1,
            Some(b'-') => -1,
            _ => return Err("invalid timezone"),
        };
        if zone.len() != 6 || zone.as_bytes().get(3) != Some(&b':') {
            return Err("invalid timezone");
        }
        let hours: i64 = zone[1..3].parse().map_err(|_| "invalid timezone")?;
        let minutes: i64 = zone[4..6].parse().map_err(|_| "invalid timezone")?;
        if hours > 24 || minutes > 59 {
            return Err("invalid timezone");
        }
        sign * (hours * 3600 + minutes * 60)
    };
    let days = days_from_civil(year, month, day);
    Ok(Rfc3339Time {
        unix_seconds: days * 86_400 + hour * 3600 + minute * 60 + second - offset,
        nanos,
        year,
    })
}

const fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 0,
    }
}

const fn days_from_civil(year: i32, month: u32, day: u32) -> i64 {
    let adjusted_year = year - if month <= 2 { 1 } else { 0 };
    let era = if adjusted_year >= 0 {
        adjusted_year
    } else {
        adjusted_year - 399
    } / 400;
    let year_of_era = adjusted_year - era * 400;
    let shifted_month = month as i32 + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * shifted_month + 2) / 5 + day as i32 - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    (era * 146_097 + day_of_era - 719_468) as i64
}

fn human_duration(seconds: i64) -> String {
    if seconds < 1 {
        "Less than a second".into()
    } else if seconds == 1 {
        "1 second".into()
    } else if seconds < 60 {
        format!("{seconds} seconds")
    } else if seconds / 60 == 1 {
        "About a minute".into()
    } else if seconds / 60 < 60 {
        format!("{} minutes", seconds / 60)
    } else {
        let hours = (seconds as f64 / 3600.0 + 0.5) as i64;
        if hours == 1 {
            "About an hour".into()
        } else if hours < 48 {
            format!("{hours} hours")
        } else if hours < 24 * 7 * 2 {
            format!("{} days", hours / 24)
        } else if hours < 24 * 30 * 2 {
            format!("{} weeks", hours / 24 / 7)
        } else if hours < 24 * 365 * 2 {
            format!("{} months", hours / 24 / 30)
        } else {
            format!("{} years", seconds / 3600 / 24 / 365)
        }
    }
}

fn parse_go_duration(value: &str) -> Option<i64> {
    if value == "0" {
        return Some(0);
    }
    let (sign, mut rest) = if let Some(rest) = value.strip_prefix('-') {
        (-1_i128, rest)
    } else if let Some(rest) = value.strip_prefix('+') {
        (1, rest)
    } else {
        (1, value)
    };
    if rest.is_empty() {
        return None;
    }
    let mut total = 0_i128;
    while !rest.is_empty() {
        let number_end = rest
            .find(|c: char| !(c.is_ascii_digit() || c == '.'))
            .unwrap_or(rest.len());
        if number_end == 0 {
            return None;
        }
        let number = &rest[..number_end];
        let (whole, fraction) = number.split_once('.').map_or((number, ""), |parts| parts);
        if number.matches('.').count() > 1 || (whole.is_empty() && fraction.is_empty()) {
            return None;
        }
        let whole = if whole.is_empty() {
            0
        } else {
            whole.parse::<i128>().ok()?
        };
        if !fraction.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        rest = &rest[number_end..];
        let (unit, consumed) = if rest.starts_with("ns") {
            (1_i128, 2)
        } else if rest.starts_with("us") {
            (1_000_i128, 2)
        } else if rest.starts_with("µs") {
            (1_000_i128, "µs".len())
        } else if rest.starts_with("μs") {
            (1_000_i128, "μs".len())
        } else if rest.starts_with("ms") {
            (1_000_000_i128, 2)
        } else if rest.starts_with('s') {
            (1_000_000_000_i128, 1)
        } else if rest.starts_with('m') {
            (60_000_000_000_i128, 1)
        } else if rest.starts_with('h') {
            (3_600_000_000_000_i128, 1)
        } else {
            return None;
        };
        total = total.checked_add(whole.checked_mul(unit)?)?;
        let mut decimal_place = unit;
        for digit in fraction.bytes() {
            decimal_place /= 10;
            if decimal_place == 0 {
                break;
            }
            total = total.checked_add(i128::from(digit - b'0').checked_mul(decimal_place)?)?;
        }
        rest = &rest[consumed..];
    }
    i64::try_from(total.checked_mul(sign)?).ok()
}

#[cfg(test)]
mod tests {
    use super::parse_go_duration;

    #[test]
    fn go_duration_parser_handles_composites_signs_and_invalid_floats() {
        assert_eq!(parse_go_duration("1m30.5s"), Some(90_500_000_000));
        assert_eq!(parse_go_duration("-1.5s"), Some(-1_500_000_000));
        assert_eq!(parse_go_duration("1µs"), Some(1_000));
        assert_eq!(parse_go_duration("9223372036.854775807s"), Some(i64::MAX));
        assert_eq!(parse_go_duration("9223372036.854775808s"), None);
        for invalid in ["NaNs", "infs", "1e999s", "1", "", "1m-2s"] {
            assert_eq!(parse_go_duration(invalid), None, "{invalid}");
        }
    }
}
