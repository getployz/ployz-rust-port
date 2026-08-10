//! Implementation of Config feature from the Compose spec

use std::collections::HashSet;
use std::path::Path;

use anyhow::Result;
use base64::Engine;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{anyhow_context, context};

/// ConfigSpec defines a configuration object that can be mounted into containers
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
#[serde(default)]
pub struct ConfigSpec {
    pub name: String,

    // Content of the config when specified inline
    #[serde(default, with = "base64_bytes", skip_serializing_if = "Vec::is_empty")]
    pub content: Vec<u8>,
    // Note: NOT IMPLEMENTED
    // External indicates this config already exists and should not be created
    // external: bool,

    // Note: NOT IMPLEMENTED
    // Labels for the config
    // labels: HashMap<String, String>,

    // TODO: add support for "environment"
}

impl ConfigSpec {
    pub fn validate(&self) -> Result<()> {
        if self.name.is_empty() {
            return Err(anyhow::anyhow!("config name is required"));
        }
        Ok(())
    }

    /// Equals compares two ConfigSpec instances
    pub fn equals(&self, other: &ConfigSpec) -> bool {
        self.name == other.name && self.content == other.content
    }
}

/// ConfigMount defines how a config is mounted into a container
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
#[serde(default)]
pub struct ConfigMount {
    // ConfigName references a config defined in ServiceSpec.Configs by its Name field
    pub config_name: String,
    // ContainerPath is the absolute path where the config is mounted in the container
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub container_path: String,
    // Uid for the mounted config file
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub uid: String,
    // Gid for the mounted config file
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub gid: String,
    // Mode (file permissions) for the mounted config file
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
}

impl ConfigMount {
    pub fn get_numeric_uid(&self) -> Result<Option<u64>> {
        if self.uid.is_empty() {
            return Ok(None);
        }
        let uid = parse_uint(&self.uid, "Uid")?;
        if isize::try_from(uid).is_err() {
            return Err(anyhow::anyhow!(
                "invalid Uid '{}': value too high",
                self.uid
            ));
        }
        Ok(Some(uid))
    }

    pub fn get_numeric_gid(&self) -> Result<Option<u64>> {
        if self.gid.is_empty() {
            return Ok(None);
        }
        let gid = parse_uint(&self.gid, "Gid")?;
        if isize::try_from(gid).is_err() {
            return Err(anyhow::anyhow!(
                "invalid Gid '{}': value too high",
                self.gid
            ));
        }
        Ok(Some(gid))
    }

    pub fn validate(&self) -> Result<()> {
        if self.config_name.is_empty() {
            return Err(anyhow::anyhow!("config mount source is required"));
        }
        self.get_numeric_uid()?;
        self.get_numeric_gid()?;
        if !self.container_path.is_empty() && !Path::new(&self.container_path).is_absolute() {
            return Err(anyhow::anyhow!("container path must be absolute"));
        }
        Ok(())
    }

    /// Compare compares this ConfigMount with another.
    /// Returns:
    ///
    /// - -1 if c < other
    /// -  0 if c == other
    /// - +1 if c > other
    pub fn compare(&self, other: &ConfigMount) -> i32 {
        if self.config_name != other.config_name {
            if self.config_name < other.config_name {
                return -1;
            }
            return 1;
        }
        if self.container_path != other.container_path {
            if self.container_path < other.container_path {
                return -1;
            }
            return 1;
        }
        if self.uid != other.uid {
            if self.uid < other.uid {
                return -1;
            }
            return 1;
        }
        if self.gid != other.gid {
            if self.gid < other.gid {
                return -1;
            }
            return 1;
        }
        // Compare Mode (handle nil cases)
        if self.mode.is_none() && other.mode.is_some() {
            return -1;
        }
        if self.mode.is_some() && other.mode.is_none() {
            return 1;
        }
        if let (Some(mode), Some(other_mode)) = (self.mode, other.mode) {
            if mode < other_mode {
                return -1;
            }
            if mode > other_mode {
                return 1;
            }
        }
        0
    }

    /// Equals compares two ConfigMount instances for equality.
    pub fn equals(&self, other: &ConfigMount) -> bool {
        self.compare(other) == 0
    }
}

impl Clone for ConfigMount {
    fn clone(&self) -> ConfigMount {
        ConfigMount {
            config_name: self.config_name.clone(),
            container_path: self.container_path.clone(),
            uid: self.uid.clone(),
            gid: self.gid.clone(),
            mode: self.mode,
        }
    }
}

/// sort_config_mounts sorts a slice of ConfigMount instances.
#[allow(dead_code)]
pub(super) fn sort_config_mounts(mounts: &mut [ConfigMount]) {
    mounts.sort_by(|left, right| left.compare(right).cmp(&0));
}

/// validate_configs_and_mounts takes config specs and config mounts and validates that all mounts refer to existing specs
pub fn validate_configs_and_mounts(configs: &[ConfigSpec], mounts: &[ConfigMount]) -> Result<()> {
    let mut config_map = HashSet::new();
    for cfg in configs {
        cfg.validate()
            .map_err(|error| anyhow_context("invalid config", error))?;
        if config_map.contains(cfg.name.as_str()) {
            return Err(anyhow::anyhow!("duplicate config name: '{}'", cfg.name));
        }

        config_map.insert(cfg.name.as_str());
    }

    for mount in mounts {
        mount
            .validate()
            .map_err(|error| anyhow_context("invalid config mount", error))?;
        if !config_map.contains(mount.config_name.as_str()) {
            return Err(anyhow::anyhow!(
                "config mount source '{}' does not refer to any defined config",
                mount.config_name
            ));
        }
    }

    Ok(())
}

#[derive(Debug)]
struct ParseUintError {
    value: String,
    reason: ParseUintReason,
    source: Option<std::num::ParseIntError>,
}

impl std::fmt::Display for ParseUintError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            formatter,
            "strconv.ParseUint: parsing {:?}: {}",
            self.value, self.reason
        )
    }
}

impl std::error::Error for ParseUintError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.source
            .as_ref()
            .map(|source| source as &(dyn std::error::Error + 'static))
    }
}

#[derive(Debug)]
enum ParseUintReason {
    InvalidSyntax,
    ValueOutOfRange,
}

impl std::fmt::Display for ParseUintReason {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSyntax => formatter.write_str("invalid syntax"),
            Self::ValueOutOfRange => formatter.write_str("value out of range"),
        }
    }
}

fn parse_uint(value: &str, field: &str) -> Result<u64> {
    if value.starts_with('+') {
        return Err(context(
            format!("invalid {field} '{value}'"),
            ParseUintError {
                value: value.to_owned(),
                reason: ParseUintReason::InvalidSyntax,
                source: None,
            },
        ));
    }
    value.parse::<u64>().map_err(|source| {
        let reason = if matches!(source.kind(), std::num::IntErrorKind::PosOverflow) {
            ParseUintReason::ValueOutOfRange
        } else {
            ParseUintReason::InvalidSyntax
        };
        context(
            format!("invalid {field} '{value}'"),
            ParseUintError {
                value: value.to_owned(),
                reason,
                source: Some(source),
            },
        )
    })
}

mod base64_bytes {
    use super::*;

    pub fn serialize<S>(bytes: &[u8], serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> std::result::Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = Option::<String>::deserialize(deserializer)?;
        match encoded {
            Some(encoded) => base64::engine::general_purpose::STANDARD
                .decode(encoded)
                .map_err(serde::de::Error::custom),
            None => Ok(Vec::new()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_mount_get_numeric_uid() {
        let tests = [
            ("empty uid returns nil", "", None, ""),
            ("valid numeric uid", "1000", Some(1000), ""),
            ("zero uid", "0", Some(0), ""),
            (
                "invalid non-numeric uid",
                "root",
                None,
                "invalid Uid 'root'",
            ),
            ("negative uid", "-1", None, "invalid Uid"),
            (
                "very large uid",
                "18446744073709551615", // max uint64
                None,
                "invalid Uid '18446744073709551615': value too high",
            ),
        ];

        for (name, uid, expected, want_err) in tests {
            let mount = ConfigMount {
                uid: uid.to_owned(),
                ..Default::default()
            };
            let result = mount.get_numeric_uid();
            if !want_err.is_empty() {
                let err = result.expect_err(name);
                assert!(err.to_string().contains(want_err), "{name}: {err}");
                continue;
            }
            assert_eq!(result.expect(name), expected, "{name}");
        }
    }

    #[test]
    fn config_mount_get_numeric_gid() {
        let tests = [
            ("empty gid returns nil", "", None, ""),
            ("valid numeric gid", "1000", Some(1000), ""),
            ("zero gid", "0", Some(0), ""),
            (
                "invalid non-numeric gid",
                "wheel",
                None,
                "invalid Gid 'wheel'",
            ),
            ("negative gid", "-1", None, "invalid Gid"),
        ];

        for (name, gid, expected, want_err) in tests {
            let mount = ConfigMount {
                gid: gid.to_owned(),
                ..Default::default()
            };
            let result = mount.get_numeric_gid();
            if !want_err.is_empty() {
                let err = result.expect_err(name);
                assert!(err.to_string().contains(want_err), "{name}: {err}");
                continue;
            }
            assert_eq!(result.expect(name), expected, "{name}");
        }
    }

    #[test]
    fn validate_configs_and_mounts_cases() {
        struct Test {
            name: &'static str,
            configs: Vec<ConfigSpec>,
            mounts: Vec<ConfigMount>,
            want_err: &'static str,
        }

        fn config(name: &str, content: &str) -> ConfigSpec {
            ConfigSpec {
                name: name.to_owned(),
                content: content.as_bytes().to_vec(),
            }
        }

        fn mount(name: &str, path: &str) -> ConfigMount {
            ConfigMount {
                config_name: name.to_owned(),
                container_path: path.to_owned(),
                ..Default::default()
            }
        }

        let tests = vec![
            Test {
                name: "empty configs and mounts",
                configs: vec![],
                mounts: vec![],
                want_err: "",
            },
            Test {
                name: "valid configs without mounts",
                configs: vec![config("config1", "content1"), config("config2", "content2")],
                mounts: vec![],
                want_err: "",
            },
            Test {
                name: "valid configs with valid mounts",
                configs: vec![config("config1", "content1"), config("config2", "content2")],
                mounts: vec![
                    mount("config1", "/etc/config1"),
                    ConfigMount {
                        config_name: "config2".into(),
                        container_path: "/etc/config2".into(),
                        uid: "1000".into(),
                        gid: "1000".into(),
                        mode: None,
                    },
                ],
                want_err: "",
            },
            Test {
                name: "config with empty name",
                configs: vec![config("", "content")],
                mounts: vec![],
                want_err: "config name is required",
            },
            Test {
                name: "duplicate config names",
                configs: vec![config("config1", "content1"), config("config1", "content2")],
                mounts: vec![],
                want_err: "duplicate config name: 'config1'",
            },
            Test {
                name: "mount with empty config name",
                configs: vec![config("config1", "content1")],
                mounts: vec![mount("", "/etc/config")],
                want_err: "config mount source is required",
            },
            Test {
                name: "mount referencing non-existent config",
                configs: vec![config("config1", "content1")],
                mounts: vec![mount("nonexistent", "/etc/config")],
                want_err: "config mount source 'nonexistent' does not refer to any defined config",
            },
            Test {
                name: "mount with invalid uid",
                configs: vec![config("config1", "content1")],
                mounts: vec![ConfigMount {
                    uid: "invalid".into(),
                    ..mount("config1", "/etc/config")
                }],
                want_err: "invalid Uid 'invalid'",
            },
            Test {
                name: "mount with invalid gid",
                configs: vec![config("config1", "content1")],
                mounts: vec![ConfigMount {
                    gid: "invalid".into(),
                    ..mount("config1", "/etc/config")
                }],
                want_err: "invalid Gid 'invalid'",
            },
            Test {
                name: "mount with relative container path",
                configs: vec![config("config1", "content1")],
                mounts: vec![mount("config1", "relative/path")],
                want_err: "container path must be absolute",
            },
            // Empty path is allowed
            Test {
                name: "mount with empty container path",
                configs: vec![config("config1", "content1")],
                mounts: vec![mount("config1", "")],
                want_err: "",
            },
            Test {
                name: "mount with absolute container path",
                configs: vec![config("config1", "content1")],
                mounts: vec![mount("config1", "/absolute/path")],
                want_err: "",
            },
            Test {
                name: "complex valid scenario",
                configs: vec![
                    config("nginx-conf", "server { listen 80; }"),
                    config("app-config", "debug=true"),
                    config("cert", "-----BEGIN CERTIFICATE-----"),
                ],
                mounts: vec![
                    ConfigMount {
                        uid: "0".into(),
                        gid: "0".into(),
                        mode: Some(0o644),
                        ..mount("nginx-conf", "/etc/nginx/nginx.conf")
                    },
                    mount("app-config", "/app/config.env"),
                    ConfigMount {
                        uid: "1000".into(),
                        gid: "1000".into(),
                        ..mount("cert", "/etc/ssl/cert.pem")
                    },
                ],
                want_err: "",
            },
        ];

        for test in tests {
            let result = validate_configs_and_mounts(&test.configs, &test.mounts);
            if !test.want_err.is_empty() {
                let err = result.expect_err(test.name);
                assert!(
                    err.to_string().contains(test.want_err),
                    "{}: {err}",
                    test.name
                );
                continue;
            }
            result.expect(test.name);
        }
    }

    #[test]
    fn numeric_ids_reject_leading_plus_and_preserve_parse_source() {
        let signed = ConfigMount {
            uid: "+1".to_owned(),
            ..Default::default()
        }
        .get_numeric_uid()
        .unwrap_err();
        assert_eq!(
            signed.to_string(),
            "invalid Uid '+1': strconv.ParseUint: parsing \"+1\": invalid syntax"
        );
        assert!(signed.chain().any(|source| source.is::<ParseUintError>()));

        let invalid_uid = ConfigMount {
            uid: "root".to_owned(),
            ..Default::default()
        }
        .get_numeric_uid()
        .unwrap_err();
        assert_eq!(
            invalid_uid.to_string(),
            "invalid Uid 'root': strconv.ParseUint: parsing \"root\": invalid syntax"
        );
        assert!(
            invalid_uid
                .chain()
                .any(|source| source.is::<ParseUintError>())
        );

        let invalid = ConfigMount {
            gid: "wheel".to_owned(),
            ..Default::default()
        }
        .get_numeric_gid()
        .unwrap_err();
        assert_eq!(
            invalid.to_string(),
            "invalid Gid 'wheel': strconv.ParseUint: parsing \"wheel\": invalid syntax"
        );
        assert!(invalid.chain().any(|source| source.is::<ParseUintError>()));
        assert!(
            invalid
                .chain()
                .any(|source| source.is::<std::num::ParseIntError>())
        );

        let above_platform_max = (isize::MAX as u64) + 1;
        let too_large = ConfigMount {
            uid: above_platform_max.to_string(),
            ..Default::default()
        }
        .get_numeric_uid()
        .unwrap_err();
        assert!(too_large.to_string().ends_with(": value too high"));

        let overflow = ConfigMount {
            gid: "18446744073709551616".to_owned(),
            ..Default::default()
        }
        .get_numeric_gid()
        .unwrap_err();
        assert_eq!(
            overflow.to_string(),
            "invalid Gid '18446744073709551616': strconv.ParseUint: parsing \"18446744073709551616\": value out of range"
        );
        assert!(overflow.chain().any(|source| source.is::<ParseUintError>()));

        let wrapped = validate_configs_and_mounts(
            &[ConfigSpec {
                name: "config".to_owned(),
                content: Vec::new(),
            }],
            &[ConfigMount {
                config_name: "config".to_owned(),
                uid: "root".to_owned(),
                ..Default::default()
            }],
        )
        .unwrap_err();
        assert!(
            wrapped
                .to_string()
                .starts_with("invalid config mount: invalid Uid 'root': strconv.ParseUint: ")
        );
        assert!(
            wrapped
                .chain()
                .any(|source| source.is::<std::num::ParseIntError>())
        );
    }

    #[test]
    fn json_matches_go_omitempty_and_byte_encoding() {
        let config = ConfigSpec {
            name: "config".to_owned(),
            content: vec![0xff, 0x00],
        };
        assert_eq!(
            serde_json::to_string(&config).unwrap(),
            r#"{"Name":"config","Content":"/wA="}"#
        );
        assert_eq!(
            serde_json::to_string(&ConfigSpec {
                name: "config".to_owned(),
                content: Vec::new(),
            })
            .unwrap(),
            r#"{"Name":"config"}"#
        );

        let decoded: ConfigSpec =
            serde_json::from_str(r#"{"Name":"config","Content":"/wA="}"#).unwrap();
        assert_eq!(decoded.content, vec![0xff, 0x00]);
        let zero_value: ConfigSpec = serde_json::from_str("{}").unwrap();
        assert!(zero_value.name.is_empty());
        assert!(zero_value.content.is_empty());

        let mount = ConfigMount {
            config_name: "config".to_owned(),
            ..Default::default()
        };
        assert_eq!(
            serde_json::to_string(&mount).unwrap(),
            r#"{"ConfigName":"config"}"#
        );
    }
}
