use std::cmp::Ordering;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::{ApiError, Result};

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct ConfigSpec {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty", with = "go_bytes")]
    pub content: Vec<u8>,
}

impl ConfigSpec {
    pub fn validate(&self) -> Result<()> {
        if self.name.is_empty() {
            return Err(ApiError::invalid("config name is required"));
        }
        Ok(())
    }
}

mod go_bytes {
    use base64::Engine;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(value))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let Some(encoded) = Option::<String>::deserialize(deserializer)? else {
            return Ok(Vec::new());
        };
        base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct ConfigMount {
    pub config_name: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub container_path: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub uid: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub gid: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
}

impl ConfigMount {
    pub fn numeric_uid(&self) -> Result<Option<u64>> {
        parse_numeric_id("Uid", &self.uid)
    }

    pub fn numeric_gid(&self) -> Result<Option<u64>> {
        parse_numeric_id("Gid", &self.gid)
    }

    pub fn validate(&self) -> Result<()> {
        if self.config_name.is_empty() {
            return Err(ApiError::invalid("config mount source is required"));
        }
        self.numeric_uid()?;
        self.numeric_gid()?;
        if !self.container_path.is_empty() && !Path::new(&self.container_path).is_absolute() {
            return Err(ApiError::invalid("container path must be absolute"));
        }
        Ok(())
    }
}

impl Ord for ConfigMount {
    fn cmp(&self, other: &Self) -> Ordering {
        (
            &self.config_name,
            &self.container_path,
            &self.uid,
            &self.gid,
            self.mode,
        )
            .cmp(&(
                &other.config_name,
                &other.container_path,
                &other.uid,
                &other.gid,
                other.mode,
            ))
    }
}

impl PartialOrd for ConfigMount {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

fn parse_numeric_id(field: &str, value: &str) -> Result<Option<u64>> {
    if value.is_empty() {
        return Ok(None);
    }
    let parsed = value
        .parse::<u64>()
        .map_err(|error| ApiError::invalid(format!("invalid {field} '{value}': {error}")))?;
    if parsed > isize::MAX as u64 {
        return Err(ApiError::invalid(format!(
            "invalid {field} '{value}': value too high"
        )));
    }
    Ok(Some(parsed))
}

pub fn validate_configs_and_mounts(configs: &[ConfigSpec], mounts: &[ConfigMount]) -> Result<()> {
    let mut names = std::collections::HashSet::with_capacity(configs.len());
    for config in configs {
        config
            .validate()
            .map_err(|error| ApiError::invalid(format!("invalid config: {error}")))?;
        if !names.insert(config.name.as_str()) {
            return Err(ApiError::invalid(format!(
                "duplicate config name: '{}'",
                config.name
            )));
        }
    }
    for mount in mounts {
        mount
            .validate()
            .map_err(|error| ApiError::invalid(format!("invalid config mount: {error}")))?;
        if !names.contains(mount.config_name.as_str()) {
            return Err(ApiError::invalid(format!(
                "config mount source '{}' does not refer to any defined config",
                mount.config_name
            )));
        }
    }
    Ok(())
}
