use std::collections::{BTreeSet, HashMap, HashSet};

use oci_spec::distribution::Reference;
use ployz_internal_machine_api_pb as pb;
use serde::{Deserialize, Serialize};

use crate::{
    ApiError, ConfigMount, ConfigSpec, GoMap, GoSlice, MachineServiceContainer, PORT_MODE_INGRESS,
    PROTOCOL_HTTP, PROTOCOL_HTTPS, PortSpec, Result, VOLUME_TYPE_VOLUME, VolumeMount, VolumeSpec,
    validate_configs_and_mounts,
};

pub const SERVICE_MODE_REPLICATED: &str = "replicated";
pub const SERVICE_MODE_GLOBAL: &str = "global";
pub const UPDATE_ORDER_START_FIRST: &str = "start-first";
pub const UPDATE_ORDER_STOP_FIRST: &str = "stop-first";
pub const PULL_POLICY_ALWAYS: &str = "always";
pub const PULL_POLICY_MISSING: &str = "missing";
pub const PULL_POLICY_NEVER: &str = "never";

#[must_use]
pub fn validate_service_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct CaddySpec {
    pub config: String,
}

impl CaddySpec {
    #[must_use]
    pub fn equivalent(left: Option<&Self>, right: Option<&Self>) -> bool {
        left.map_or("", |spec| spec.config.trim()) == right.map_or("", |spec| spec.config.trim())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct Placement {
    pub machines: GoSlice<String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct ServiceSpec {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub caddy: Option<CaddySpec>,
    pub configs: GoSlice<ConfigSpec>,
    pub container: ContainerSpec,
    pub mode: String,
    pub name: String,
    pub placement: Placement,
    pub ports: GoSlice<PortSpec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_deploy: Option<PreDeployHook>,
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub replicas: u64,
    pub update_config: UpdateConfig,
    pub volumes: GoSlice<VolumeSpec>,
}

impl ServiceSpec {
    #[must_use]
    pub fn caddy_config(&self) -> &str {
        self.caddy.as_ref().map_or("", |caddy| caddy.config.trim())
    }

    #[must_use]
    pub fn volume(&self, name: &str) -> Option<&VolumeSpec> {
        self.volumes.iter().find(|volume| volume.name == name)
    }

    #[must_use]
    pub fn config(&self, name: &str) -> Option<&ConfigSpec> {
        self.configs.iter().find(|config| config.name == name)
    }

    #[must_use]
    pub fn mounted_docker_volumes(&self) -> Vec<VolumeSpec> {
        self.container
            .volume_mounts
            .iter()
            .filter_map(|mount| self.volume(&mount.volume_name))
            .filter(|volume| volume.kind == VOLUME_TYPE_VOLUME)
            .map(|volume| (volume.name.clone(), volume.clone()))
            .collect::<HashMap<_, _>>()
            .into_values()
            .collect()
    }

    #[must_use]
    pub fn with_defaults(&self) -> Self {
        let mut spec = self.clone();
        if spec.mode.is_empty() {
            spec.mode = SERVICE_MODE_REPLICATED.into();
        }
        if spec.mode == SERVICE_MODE_REPLICATED && spec.replicas == 0 {
            spec.replicas = 1;
        }
        spec.container = spec.container.with_defaults();
        if !spec.volumes.is_nil() {
            spec.volumes = spec.volumes.iter().map(VolumeSpec::with_defaults).collect();
        }
        spec
    }

    pub fn validate(&self) -> Result<()> {
        self.container.validate()?;
        if !matches!(
            self.mode.as_str(),
            "" | SERVICE_MODE_GLOBAL | SERVICE_MODE_REPLICATED
        ) {
            return Err(ApiError::invalid(format!("invalid mode: {:?}", self.mode)));
        }
        if !self.name.is_empty() {
            if self.name.len() > 63 {
                return Err(ApiError::invalid(format!(
                    "service name too long (max 63 characters): {:?}",
                    self.name
                )));
            }
            let bytes = self.name.as_bytes();
            let valid = bytes.first().is_some_and(u8::is_ascii_alphanumeric)
                && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
                && bytes.iter().all(|byte| {
                    byte.is_ascii_lowercase() || byte.is_ascii_digit() || *byte == b'-'
                });
            if !valid {
                return Err(ApiError::invalid(format!(
                    "invalid service name: {:?}. must be 1-63 characters, lowercase letters, numbers, and dashes only; must start and end with a letter or number",
                    self.name
                )));
            }
        }
        for port in &self.ports {
            if (port.mode.is_empty() || port.mode == PORT_MODE_INGRESS)
                && !matches!(port.protocol.as_str(), PROTOCOL_HTTP | PROTOCOL_HTTPS)
            {
                return Err(ApiError::invalid(format!(
                    "unsupported protocol for ingress port {}: {}",
                    port.container_port, port.protocol
                )));
            }
        }
        if !self.caddy_config().is_empty()
            && self
                .ports
                .iter()
                .any(|port| port.mode.is_empty() || port.mode == PORT_MODE_INGRESS)
        {
            return Err(ApiError::invalid(
                "ingress ports and Caddy configuration cannot be specified simultaneously: Caddy config is auto-generated from ingress ports, use only one of them. Host mode ports can be used with Caddy config",
            ));
        }
        let mut volume_names = HashSet::with_capacity(self.volumes.len());
        for volume in &self.volumes {
            volume
                .validate()
                .map_err(|error| ApiError::invalid(format!("invalid volume: {error}")))?;
            if !volume_names.insert(volume.name.as_str()) {
                return Err(ApiError::invalid(format!(
                    "duplicate volume name: '{}'",
                    volume.name
                )));
            }
        }
        for mount in &self.container.volume_mounts {
            if !volume_names.contains(mount.volume_name.as_str()) {
                return Err(ApiError::invalid(format!(
                    "volume mount references a volume that doesn't exist in the service spec: '{}'",
                    mount.volume_name
                )));
            }
        }
        validate_configs_and_mounts(&self.configs, &self.container.config_mounts).map_err(
            |error| ApiError::invalid(format!("validate service configs and mounts: {error}")),
        )?;
        if let Some(hook) = &self.pre_deploy {
            hook.validate()?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct ContainerSpec {
    pub cap_add: GoSlice<String>,
    pub cap_drop: GoSlice<String>,
    pub command: GoSlice<String>,
    pub entrypoint: GoSlice<String>,
    pub env: EnvVars,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub healthcheck: Option<HealthcheckSpec>,
    pub image: String,
    pub init: Option<bool>,
    pub log_driver: Option<LogDriver>,
    pub pid_mode: String,
    pub tty: bool,
    pub open_stdin: bool,
    pub privileged: bool,
    pub pull_policy: String,
    pub resources: ContainerResources,
    pub stop_grace_period: Option<i64>,
    pub sysctls: GoMap<String, String>,
    pub user: String,
    pub volume_mounts: GoSlice<VolumeMount>,
    pub config_mounts: GoSlice<ConfigMount>,
    pub volumes: GoSlice<String>,
}

impl ContainerSpec {
    #[must_use]
    pub fn with_defaults(&self) -> Self {
        let mut spec = self.clone();
        spec.log_driver.get_or_insert_with(|| LogDriver {
            name: "local".into(),
            options: std::collections::BTreeMap::new().into(),
        });
        if spec.pull_policy.is_empty() {
            spec.pull_policy = PULL_POLICY_MISSING.into();
        }
        spec
    }

    pub fn validate(&self) -> Result<()> {
        validate_docker_reference(&self.image)?;
        for mount in &self.volume_mounts {
            mount
                .validate()
                .map_err(|error| ApiError::invalid(format!("invalid volume mount: {error}")))?;
        }
        Ok(())
    }

    #[must_use]
    pub fn equivalent(&self, other: &Self) -> bool {
        let mut left = self.with_defaults();
        let mut right = other.with_defaults();
        left.volumes.sort_unstable();
        right.volumes.sort_unstable();
        left.volume_mounts.sort_unstable();
        right.volume_mounts.sort_unstable();
        left.config_mounts.sort_unstable();
        right.config_mounts.sort_unstable();
        left.equate_empty_collections();
        right.equate_empty_collections();
        left == right
    }

    fn equate_empty_collections(&mut self) {
        self.cap_add.make_non_nil();
        self.cap_drop.make_non_nil();
        self.command.make_non_nil();
        self.entrypoint.make_non_nil();
        self.env.make_non_nil();
        self.sysctls.make_non_nil();
        self.volume_mounts.make_non_nil();
        self.config_mounts.make_non_nil();
        self.volumes.make_non_nil();
        if let Some(healthcheck) = &mut self.healthcheck {
            healthcheck.test.make_non_nil();
        }
        if let Some(log_driver) = &mut self.log_driver {
            log_driver.options.make_non_nil();
        }
        self.resources.devices.make_non_nil();
        self.resources.device_reservations.make_non_nil();
        self.resources.ulimits.make_non_nil();
        for request in &mut self.resources.device_reservations {
            request.device_ids.make_non_nil();
            request.capabilities.make_non_nil();
            request.options.make_non_nil();
            for capabilities in &mut request.capabilities {
                capabilities.make_non_nil();
            }
        }
    }
}

fn validate_docker_reference(input: &str) -> Result<()> {
    let mut parse_input = input.to_owned();
    if input.starts_with('[') {
        let slash = input.find('/').ok_or_else(|| {
            ApiError::invalid(format!(
                "invalid image '{input}': invalid bracketed registry"
            ))
        })?;
        let authority = &input[..slash];
        let close = authority.find(']').ok_or_else(|| {
            ApiError::invalid(format!(
                "invalid image '{input}': invalid bracketed registry"
            ))
        })?;
        authority[1..close]
            .parse::<std::net::Ipv6Addr>()
            .map_err(|error| ApiError::invalid(format!("invalid image '{input}': {error}")))?;
        let port = &authority[close + 1..];
        if !port.is_empty()
            && (port.strip_prefix(':').is_none_or(|value| {
                value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit())
            }))
        {
            return Err(ApiError::invalid(format!(
                "invalid image '{input}': invalid registry port"
            )));
        }
        parse_input = format!("ipv6.invalid{port}{}", &input[slash..]);
    }

    let reference = Reference::try_from(parse_input.as_str())
        .map_err(|error| ApiError::invalid(format!("invalid image '{input}': {error}")))?;
    if let Some(tag) = reference.tag()
        && (!tag.is_ascii()
            || tag.is_empty()
            || tag.len() > 128
            || !tag.bytes().enumerate().all(|(index, byte)| {
                byte.is_ascii_alphanumeric()
                    || byte == b'_'
                    || (index > 0 && matches!(byte, b'.' | b'-'))
            }))
    {
        return Err(ApiError::invalid(format!(
            "invalid image '{input}': invalid tag"
        )));
    }
    if let Some(digest) = reference.digest() {
        let (algorithm, encoded) = digest
            .split_once(':')
            .ok_or_else(|| ApiError::invalid(format!("invalid image '{input}': invalid digest")))?;
        let expected = match algorithm {
            "sha256" => 64,
            "sha384" => 96,
            "sha512" => 128,
            _ => {
                return Err(ApiError::invalid(format!(
                    "invalid image '{input}': unsupported digest algorithm"
                )));
            }
        };
        if encoded.len() != expected
            || !encoded
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err(ApiError::invalid(format!(
                "invalid image '{input}': invalid digest"
            )));
        }
    }
    Ok(())
}

pub type EnvVars = GoMap<String, String>;

pub trait EnvVarsExt {
    fn to_env(&self) -> Vec<String>;
}

impl EnvVarsExt for EnvVars {
    fn to_env(&self) -> Vec<String> {
        self.iter()
            .filter(|(key, _)| !key.is_empty())
            .map(|(key, value)| format!("{key}={value}"))
            .collect()
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct HealthcheckSpec {
    pub test: GoSlice<String>,
    pub interval: i64,
    pub timeout: i64,
    pub start_period: i64,
    pub start_interval: i64,
    pub retries: u64,
    pub disable: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct LogDriver {
    pub name: String,
    pub options: GoMap<String, String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct DeviceMapping {
    pub host_path: String,
    pub container_path: String,
    pub cgroup_permissions: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct DeviceRequest {
    pub driver: String,
    pub count: i64,
    pub device_ids: GoSlice<String>,
    pub capabilities: GoSlice<GoSlice<String>>,
    pub options: GoMap<String, String>,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct Ulimit {
    pub soft: i64,
    pub hard: i64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct ContainerResources {
    pub cpu: i64,
    pub memory: i64,
    pub memory_reservation: i64,
    pub devices: GoSlice<DeviceMapping>,
    pub device_reservations: GoSlice<DeviceRequest>,
    pub shared_memory: i64,
    pub ulimits: GoMap<String, Ulimit>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct PreDeployHook {
    pub command: GoSlice<String>,
    pub env: EnvVars,
    pub privileged: Option<bool>,
    pub timeout: Option<i64>,
    pub user: String,
}

impl PreDeployHook {
    pub fn validate(&self) -> Result<()> {
        if self.command.is_empty() {
            return Err(ApiError::invalid("pre-deploy hook command is required"));
        }
        Ok(())
    }

    #[must_use]
    pub fn equivalent(left: Option<&Self>, right: Option<&Self>) -> bool {
        match (left, right) {
            (Some(left), Some(right)) => {
                let mut left = left.clone();
                let mut right = right.clone();
                left.command.make_non_nil();
                left.env.make_non_nil();
                right.command.make_non_nil();
                right.env.make_non_nil();
                left == right
            }
            (None, None) => true,
            (Some(_), None) | (None, Some(_)) => false,
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct UpdateConfig {
    pub order: String,
    pub monitor_period: Option<i64>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct RunServiceResponse {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct Service {
    pub id: String,
    pub name: String,
    pub mode: String,
    pub containers: Vec<MachineServiceContainer>,
    pub hook_containers: Vec<MachineServiceContainer>,
}

impl Service {
    pub fn find_container(&self, name_or_id: &str) -> Result<&MachineServiceContainer> {
        let mut prefixes = Vec::new();
        for container in self.containers.iter().chain(&self.hook_containers) {
            if container.container.container.id == name_or_id
                || container.container.container.name == name_or_id
            {
                return Ok(container);
            }
            if container.container.container.id.starts_with(name_or_id) {
                prefixes.push(container);
            }
        }
        match prefixes.as_slice() {
            [container] => Ok(container),
            [] => Err(ApiError::NotFound),
            _ => Err(ApiError::invalid(format!(
                "multiple containers found with ID prefix '{name_or_id}'"
            ))),
        }
    }

    #[must_use]
    pub fn machine_ids(&self) -> Vec<String> {
        self.containers
            .iter()
            .map(|container| container.machine_id.clone())
            .collect::<HashSet<_>>()
            .into_iter()
            .collect()
    }

    #[must_use]
    pub fn images(&self) -> Vec<String> {
        self.containers
            .iter()
            .filter_map(|container| container.container.container.config.as_ref())
            .map(|config| config.image.clone())
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    #[must_use]
    pub fn endpoints(&self) -> Vec<String> {
        let mut endpoints = BTreeSet::new();
        for machine_container in &self.containers {
            let Ok(ports) = machine_container.container.service_ports() else {
                continue;
            };
            for port in ports {
                if port.hostname.is_empty()
                    || !matches!(port.protocol.as_str(), PROTOCOL_HTTP | PROTOCOL_HTTPS)
                {
                    continue;
                }
                let mut endpoint = format!("{}://{}", port.protocol, port.hostname);
                if port.published_port != 0
                    && !((port.protocol == PROTOCOL_HTTP && port.published_port == 80)
                        || (port.protocol == PROTOCOL_HTTPS && port.published_port == 443))
                {
                    endpoint.push_str(&format!(":{}", port.published_port));
                }
                endpoint.push_str(&format!(" → :{}", port.container_port));
                endpoints.insert(endpoint);
            }
        }
        endpoints.into_iter().collect()
    }
}

pub fn service_from_proto(service: &pb::Service) -> Result<Service> {
    let containers = service
        .containers
        .iter()
        .map(|source| {
            let container = crate::Container::from_json(&source.container)
                .map_err(|error| ApiError::invalid(format!("unmarshal container: {error}")))?;
            Ok(MachineServiceContainer {
                machine_id: source.machine_id.clone(),
                machine_name: String::new(),
                container: crate::ServiceContainer {
                    container,
                    service_spec: ServiceSpec::default(),
                },
            })
        })
        .collect::<Result<Vec<_>>>()?;
    Ok(Service {
        id: service.id.clone(),
        name: service.name.clone(),
        mode: service.mode.clone(),
        containers,
        hook_containers: Vec::new(),
    })
}

const fn is_zero_u64(value: &u64) -> bool {
    *value == 0
}
