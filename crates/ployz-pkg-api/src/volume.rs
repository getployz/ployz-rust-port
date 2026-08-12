use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{ApiError, GoMap, GoSlice, Result};

pub const VOLUME_TYPE_BIND: &str = "bind";
pub const VOLUME_TYPE_VOLUME: &str = "volume";
pub const VOLUME_TYPE_TMPFS: &str = "tmpfs";
pub const VOLUME_DRIVER_LOCAL: &str = "local";

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct Driver {
    pub name: String,
    pub options: GoMap<String, String>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct BindOptions {
    pub host_path: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub create_host_path: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub propagation: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub recursive: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct TmpfsOptions {
    pub size_bytes: i64,
    pub mode: u32,
    pub options: GoSlice<GoSlice<String>>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct VolumeOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub driver: Option<Driver>,
    pub labels: GoMap<String, String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub no_copy: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub sub_path: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct VolumeSpec {
    pub name: String,
    #[serde(rename = "Type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bind_options: Option<BindOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tmpfs_options: Option<TmpfsOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume_options: Option<VolumeOptions>,
}

impl VolumeSpec {
    #[must_use]
    pub fn docker_volume_name(&self) -> &str {
        if self.kind != VOLUME_TYPE_VOLUME {
            return "";
        }
        self.volume_options
            .as_ref()
            .map(|options| options.name.as_str())
            .filter(|name| !name.is_empty())
            .unwrap_or(&self.name)
    }

    #[must_use]
    pub fn with_defaults(&self) -> Self {
        let mut spec = self.clone();
        if spec.kind == VOLUME_TYPE_VOLUME {
            let options = spec
                .volume_options
                .get_or_insert_with(VolumeOptions::default);
            if options.name.is_empty() {
                options.name.clone_from(&spec.name);
            }
            if let Some(driver) = &mut options.driver
                && driver.name.is_empty()
            {
                driver.name = VOLUME_DRIVER_LOCAL.into();
            }
        }
        spec
    }

    pub fn validate(&self) -> Result<()> {
        if self.name.is_empty() {
            return Err(ApiError::invalid("volume name must not be empty"));
        }
        match self.kind.as_str() {
            VOLUME_TYPE_BIND if self.bind_options.is_none() => {
                Err(ApiError::invalid("bind volume must have bind options"))
            }
            VOLUME_TYPE_BIND | VOLUME_TYPE_VOLUME | VOLUME_TYPE_TMPFS => Ok(()),
            kind => Err(ApiError::invalid(format!(
                "invalid volume type: '{kind}', must be one of 'bind', 'volume', 'tmpfs')"
            ))),
        }
    }

    #[must_use]
    pub fn equivalent(&self, other: &Self) -> bool {
        let mut left = self.with_defaults();
        let mut right = other.with_defaults();
        left.equate_empty_collections();
        right.equate_empty_collections();
        left == right
    }

    #[must_use]
    pub fn matches_docker_volume(&self, volume: &DockerVolume) -> bool {
        if self.kind != VOLUME_TYPE_VOLUME {
            return false;
        }
        let spec = self.with_defaults();
        if spec.docker_volume_name() != volume.name {
            return false;
        }
        let Some(driver) = spec.volume_options.and_then(|options| options.driver) else {
            return true;
        };
        let volume_driver = if volume.driver.is_empty() {
            VOLUME_DRIVER_LOCAL
        } else {
            &volume.driver
        };
        driver.name == volume_driver && driver.options == volume.options
    }

    fn equate_empty_collections(&mut self) {
        if let Some(options) = &mut self.tmpfs_options {
            options.options.make_non_nil();
            for option in &mut options.options {
                option.make_non_nil();
            }
        }
        if let Some(options) = &mut self.volume_options {
            options.labels.make_non_nil();
            if let Some(driver) = &mut options.driver {
                driver.options.make_non_nil();
            }
        }
    }
}

#[derive(Clone, Debug, Default, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct VolumeMount {
    pub volume_name: String,
    pub container_path: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub read_only: bool,
}

impl VolumeMount {
    pub fn validate(&self) -> Result<()> {
        if self.volume_name.is_empty() {
            return Err(ApiError::invalid("volume name must not be empty"));
        }
        if !Path::new(&self.container_path).is_absolute() {
            return Err(ApiError::invalid(format!(
                "invalid container path: '{}', must be an absolute path in the container",
                self.container_path
            )));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct DockerVolume {
    pub name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub created_at: String,
    pub driver: String,
    pub mountpoint: String,
    pub options: GoMap<String, String>,
    pub labels: GoMap<String, String>,
    pub scope: String,
    #[serde(default, skip_serializing_if = "GoMap::is_empty")]
    pub status: GoMap<String, serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage_data: Option<DockerVolumeUsageData>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cluster_volume: Option<serde_json::Value>,
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct DockerVolumeUsageData {
    pub ref_count: i64,
    pub size: i64,
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct MachineVolume {
    pub machine_id: String,
    pub machine_name: String,
    pub volume: DockerVolume,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct VolumeFilter {
    pub driver: String,
    pub labels: BTreeMap<String, String>,
    pub machines: Vec<String>,
    pub names: Vec<String>,
}

impl MachineVolume {
    #[must_use]
    pub fn matches_filter(&self, filter: Option<&VolumeFilter>) -> bool {
        let Some(filter) = filter else {
            return true;
        };
        (filter.names.is_empty() || filter.names.contains(&self.volume.name))
            && (filter.driver.is_empty() || filter.driver == self.volume.driver)
            && filter
                .labels
                .iter()
                .all(|(key, value)| self.volume.labels.get(key) == Some(value))
            && (filter.machines.is_empty()
                || filter
                    .machines
                    .iter()
                    .any(|value| value == &self.machine_id || value == &self.machine_name))
    }
}
