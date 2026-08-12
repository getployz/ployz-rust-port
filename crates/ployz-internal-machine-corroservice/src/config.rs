use std::fs::{DirBuilder, OpenOptions};
use std::io::Write as _;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

#[cfg(unix)]
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::{Error, Result};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct Config {
    pub db: DbConfig,
    pub gossip: GossipConfig,
    pub api: ApiConfig,
    pub admin: AdminConfig,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DbConfig {
    pub path: PathBuf,
    pub schema_paths: Vec<PathBuf>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct GossipConfig {
    #[serde(
        serialize_with = "serialize_addr",
        deserialize_with = "deserialize_addr"
    )]
    pub addr: SocketAddr,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub max_mtu: u32,
    pub bootstrap: Vec<String>,
    pub plaintext: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ApiConfig {
    #[serde(
        serialize_with = "serialize_addr",
        deserialize_with = "deserialize_addr"
    )]
    pub addr: SocketAddr,
    pub authz: ApiAuthzConfig,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ApiAuthzConfig {
    #[serde(rename = "bearer-token")]
    pub bearer_token: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AdminConfig {
    pub path: PathBuf,
}

impl Config {
    pub fn write(&self, path: impl AsRef<Path>, owner: &str) -> Result<()> {
        let path = path.as_ref();
        let data = toml::to_string(self).map_err(|error| Error::wrap("encode config", error))?;
        let mut options = OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        options.mode(0o600);
        let mut file = options
            .open(path)
            .map_err(|error| Error::wrap(format!("write config {path:?}"), error))?;
        file.write_all(data.as_bytes())
            .map_err(|error| Error::wrap(format!("write config {path:?}"), error))?;

        #[cfg(target_os = "linux")]
        ployz_internal_fs::chown(path, owner, owner)
            .map_err(|error| Error::wrap(format!("change config ownership {path:?}"), error))?;

        #[cfg(not(target_os = "linux"))]
        let _ = owner;

        Ok(())
    }
}

/// Creates a Corrosion data or runtime directory with the oracle's permissions.
pub fn make_dir(dir: impl AsRef<Path>, owner: &str) -> Result<()> {
    let dir = dir.as_ref();
    let parent = dir.parent().unwrap_or_else(|| Path::new(""));
    let mut parents = DirBuilder::new();
    parents.recursive(true);
    #[cfg(unix)]
    parents.mode(0o711);
    parents
        .create(parent)
        .map_err(|error| Error::wrap(format!("create directory {parent:?}"), error))?;

    let mut leaf = DirBuilder::new();
    #[cfg(unix)]
    leaf.mode(0o700);
    match leaf.create(dir) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(Error::wrap(format!("create directory {dir:?}"), error)),
    }

    #[cfg(target_os = "linux")]
    if !owner.is_empty() {
        ployz_internal_fs::chown(dir, owner, owner)
            .map_err(|error| Error::wrap(format!("change directory ownership {dir:?}"), error))?;
    }

    #[cfg(not(target_os = "linux"))]
    let _ = owner;

    Ok(())
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

fn serialize_addr<S>(address: &SocketAddr, serializer: S) -> std::result::Result<S::Ok, S::Error>
where
    S: Serializer,
{
    serializer.serialize_str(&address.to_string())
}

fn deserialize_addr<'de, D>(deserializer: D) -> std::result::Result<SocketAddr, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    value.parse().map_err(serde::de::Error::custom)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt as _;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn fixture() -> Config {
        Config {
            db: DbConfig {
                path: "/var/lib/uncloud/corrosion/store.db".into(),
                schema_paths: vec!["/var/lib/uncloud/corrosion/schema.sql".into()],
            },
            gossip: GossipConfig {
                addr: "[fdcc::1]:51001".parse().unwrap(),
                max_mtu: 1232,
                bootstrap: vec!["[fdcc::2]:51001".into()],
                plaintext: true,
            },
            api: ApiConfig {
                addr: "127.0.0.1:51002".parse().unwrap(),
                authz: ApiAuthzConfig {
                    bearer_token: "quote'\"\\token".into(),
                },
            },
            admin: AdminConfig {
                path: "/run/uncloud/corrosion/admin.sock".into(),
            },
        }
    }

    fn temporary_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "ployz-corroservice-{label}-{}-{nonce}",
            std::process::id()
        ))
    }

    #[test]
    fn config_round_trips_semantically_and_omits_zero_max_mtu() {
        let config = fixture();
        let encoded = toml::to_string(&config).unwrap();
        assert_eq!(toml::from_str::<Config>(&encoded).unwrap(), config);
        assert!(encoded.contains("max_mtu = 1232"));
        assert!(encoded.contains("bearer-token"));

        let mut without_mtu = fixture();
        without_mtu.gossip.max_mtu = 0;
        let encoded = toml::to_string(&without_mtu).unwrap();
        assert!(!encoded.contains("max_mtu"));
        assert_eq!(toml::from_str::<Config>(&encoded).unwrap(), without_mtu);
    }

    #[test]
    fn write_creates_private_file_and_preserves_existing_mode() {
        let root = temporary_path("write");
        fs::create_dir(&root).unwrap();
        let path = root.join("config.toml");
        fixture().write(&path, "").unwrap();
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );

        #[cfg(unix)]
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        fixture().write(&path, "").unwrap();
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );
        assert_eq!(
            toml::from_str::<Config>(&fs::read_to_string(&path).unwrap()).unwrap(),
            fixture()
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn make_dir_sets_new_modes_and_accepts_an_existing_leaf() {
        let root = temporary_path("mkdir");
        let leaf = root.join("parent").join("corrosion");
        make_dir(&leaf, "").unwrap();
        #[cfg(unix)]
        {
            assert_eq!(
                fs::metadata(root.join("parent"))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o711
            );
            assert_eq!(
                fs::metadata(&leaf).unwrap().permissions().mode() & 0o777,
                0o700
            );
            fs::set_permissions(&leaf, fs::Permissions::from_mode(0o755)).unwrap();
        }
        make_dir(&leaf, "").unwrap();
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&leaf).unwrap().permissions().mode() & 0o777,
            0o755
        );
        fs::remove_dir_all(root).unwrap();
    }
}
