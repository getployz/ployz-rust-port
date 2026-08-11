use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::Context;

/// CLI configuration stored in a YAML file.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct Config {
    /// Name of the default context.
    pub current_context: String,
    /// Contexts keyed by their persisted names.
    pub contexts: BTreeMap<String, Context>,
    #[serde(skip)]
    path: PathBuf,
}

#[derive(Deserialize)]
struct StoredConfig {
    #[serde(default)]
    current_context: Option<String>,
    #[serde(default)]
    contexts: Option<BTreeMap<String, Context>>,
}

impl Config {
    /// Opens a configuration file, or returns an empty configuration when it
    /// does not exist.
    ///
    /// # Errors
    ///
    /// Returns [`ConfigError::CheckPermissions`] when metadata lookup fails for
    /// a reason other than absence, or a read/parse error for an existing file.
    pub fn new_from_file(path: impl Into<PathBuf>) -> Result<Self, ConfigError> {
        let path = path.into();
        match fs::metadata(&path) {
            Ok(_) => {}
            Err(source) if source.kind() == io::ErrorKind::NotFound => {
                return Ok(Self::empty(path));
            }
            Err(source) => return Err(ConfigError::CheckPermissions { path, source }),
        }

        let mut config = Self::empty(path);
        config.read()?;
        Ok(config)
    }

    /// Returns the file from which this configuration is read and saved.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Replaces the persisted fields with values read from the config file.
    ///
    /// # Errors
    ///
    /// Returns a contextual read or YAML parse error.
    pub fn read(&mut self) -> Result<(), ConfigError> {
        let data = fs::read(&self.path).map_err(|source| ConfigError::Read {
            path: self.path.clone(),
            source,
        })?;
        let stored: StoredConfig =
            serde_yaml::from_slice(&data).map_err(|source| ConfigError::Parse {
                path: self.path.clone(),
                source,
            })?;
        if let Some(current_context) = stored.current_context {
            self.current_context = current_context;
        }
        if let Some(contexts) = stored.contexts {
            self.contexts = contexts;
        }
        Ok(())
    }

    /// Creates parent directories and writes this configuration as YAML.
    ///
    /// New directories use mode `0700` and a newly created file uses mode
    /// `0600` on Unix. Existing modes are not changed.
    ///
    /// # Errors
    ///
    /// Returns a contextual directory creation, file creation, YAML encoding,
    /// flush, or close-equivalent synchronization error.
    pub fn save(&self) -> Result<(), ConfigError> {
        let directory = self
            .path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        create_config_directory(directory).map_err(|source| ConfigError::CreateDirectory {
            path: directory.to_owned(),
            source,
        })?;

        let mut file = open_config_file(&self.path).map_err(|source| ConfigError::Write {
            path: self.path.clone(),
            source,
        })?;
        serde_yaml::to_writer(&mut file, self).map_err(|source| ConfigError::Encode {
            path: self.path.clone(),
            source,
        })?;
        use std::io::Write as _;
        file.flush().map_err(|source| ConfigError::Close {
            path: self.path.clone(),
            source,
        })
    }

    fn empty(path: PathBuf) -> Self {
        Self {
            current_context: String::new(),
            contexts: BTreeMap::new(),
            path,
        }
    }
}

#[cfg(unix)]
fn create_config_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::DirBuilderExt as _;
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true).mode(0o700).create(path)
}

#[cfg(not(unix))]
fn create_config_directory(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)
}

#[cfg(unix)]
fn open_config_file(path: &Path) -> io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt as _;
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn open_config_file(path: &Path) -> io::Result<fs::File> {
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
}

/// A filesystem or YAML failure while loading or saving CLI configuration.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// Metadata lookup failed for a reason other than absence.
    #[error("check file permissions '{}': {source}", path.display())]
    CheckPermissions {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// Existing config contents could not be read.
    #[error("read config file '{}': {source}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// Existing config contents were not valid YAML for this schema.
    #[error("parse config file '{}': {source}", path.display())]
    Parse {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
    /// The config directory could not be created.
    #[error("create config directory '{}': {source}", path.display())]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// The config file could not be created or truncated.
    #[error("write config file '{}': {source}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    /// The configuration could not be encoded.
    #[error("encode config file '{}': {source}", path.display())]
    Encode {
        path: PathBuf,
        #[source]
        source: serde_yaml::Error,
    },
    /// Buffered output could not be flushed.
    #[error("close config file '{}': {source}", path.display())]
    Close {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_yaml::Value;
    use tempfile::tempdir;

    use super::*;
    use crate::{MachineConnection, SshDestination};

    #[test]
    fn new_from_file_returns_empty_config_for_missing_path() {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("missing/config.yaml");

        let config = Config::new_from_file(&path).expect("missing config is valid");

        assert_eq!(config.path(), path);
        assert!(config.current_context.is_empty());
        assert!(config.contexts.is_empty());
    }

    #[test]
    fn save_supports_bare_relative_prefixed_relative_and_absolute_paths() {
        let directory = tempdir().expect("temp directory");
        let original = std::env::current_dir().expect("current directory");
        std::env::set_current_dir(directory.path()).expect("change current directory");

        for (path, context_name) in [
            (PathBuf::from("test-config.yaml"), "test"),
            (PathBuf::from("./test-config-2.yaml"), "test2"),
            (directory.path().join("absolute-config.yaml"), "test3"),
        ] {
            let mut config = Config::new_from_file(&path).expect("new config");
            config.current_context = context_name.into();
            config.contexts.insert(
                context_name.into(),
                Context {
                    name: context_name.into(),
                    ..Default::default()
                },
            );
            config.save().expect("save config");
            assert!(path.exists());
        }

        std::env::set_current_dir(original).expect("restore current directory");
    }

    #[test]
    fn save_and_read_preserve_yaml_shape_and_omit_empty_connection_fields() {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("nested/config.yaml");
        let mut config = Config::new_from_file(&path).expect("new config");
        config.current_context = "prod".into();
        config.contexts.insert(
            "prod".into(),
            Context {
                name: "runtime-only".into(),
                connections: vec![MachineConnection {
                    ssh: SshDestination::from("root@example.com:22"),
                    ..Default::default()
                }],
            },
        );

        config.save().expect("save config");

        let yaml: Value =
            serde_yaml::from_slice(&fs::read(&path).expect("read yaml")).expect("valid yaml");
        let root = yaml.as_mapping().expect("root mapping");
        assert_eq!(root.len(), 2);
        assert_eq!(root["current_context"], Value::String("prod".into()));
        let prod = &root["contexts"]["prod"];
        assert!(prod.get("name").is_none());
        assert_eq!(
            prod["connections"][0]["ssh"],
            Value::String("root@example.com:22".into())
        );
        assert_eq!(
            prod["connections"][0]
                .as_mapping()
                .expect("connection mapping")
                .len(),
            1
        );

        let loaded = Config::new_from_file(&path).expect("reload config");
        assert_eq!(loaded.current_context, "prod");
        assert!(loaded.contexts["prod"].name.is_empty());
        assert_eq!(
            loaded.contexts["prod"].connections[0].ssh.as_ref(),
            "root@example.com:22"
        );
    }

    #[cfg(unix)]
    #[test]
    fn save_uses_private_modes_for_new_directories_and_files() {
        use std::os::unix::fs::PermissionsExt as _;

        let directory = tempdir().expect("temp directory");
        let nested = directory.path().join("private");
        let path = nested.join("config.yaml");
        Config::new_from_file(&path)
            .expect("new config")
            .save()
            .expect("save config");

        assert_eq!(
            fs::metadata(nested)
                .expect("directory metadata")
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&path)
                .expect("file metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );

        fs::set_permissions(&path, fs::Permissions::from_mode(0o640))
            .expect("change existing mode");
        Config::new_from_file(&path)
            .expect("read existing config")
            .save()
            .expect("save existing config");
        assert_eq!(
            fs::metadata(path)
                .expect("existing file metadata")
                .permissions()
                .mode()
                & 0o777,
            0o640
        );
    }

    #[test]
    fn read_preserves_fields_omitted_by_a_later_document() {
        let directory = tempdir().expect("temp directory");
        let path = directory.path().join("config.yaml");
        fs::write(&path, "current_context: prod\ncontexts: {}\n").expect("write first fixture");
        let mut config = Config::new_from_file(&path).expect("read first fixture");

        fs::write(&path, "contexts: {}\n").expect("write second fixture");
        config.read().expect("read second fixture");

        assert_eq!(config.current_context, "prod");
    }

    #[test]
    fn errors_keep_the_go_operation_and_path_context() {
        let directory = tempdir().expect("temp directory");
        let invalid = directory.path().join("invalid.yaml");
        fs::write(&invalid, "contexts: [not-a-map]").expect("write fixture");
        let parse_error = Config::new_from_file(&invalid)
            .expect_err("invalid yaml")
            .to_string();
        assert!(parse_error.starts_with(&format!("parse config file '{}':", invalid.display())));

        let parent_file = directory.path().join("parent-file");
        fs::write(&parent_file, "file").expect("write parent fixture");
        let path = parent_file.join("config.yaml");
        let error = Config::new_from_file(&path)
            .expect_err("metadata should fail")
            .to_string();
        assert!(error.starts_with(&format!("check file permissions '{}':", path.display())));
    }
}
