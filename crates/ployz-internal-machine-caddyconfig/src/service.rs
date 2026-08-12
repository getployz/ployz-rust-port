use std::error::Error as StdError;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Caddyfile {
    pub content: String,
    pub modified_at: SystemTime,
}

#[derive(Debug)]
pub enum ServiceError {
    Read { path: PathBuf, source: io::Error },
    Metadata { path: PathBuf, source: io::Error },
}

impl ServiceError {
    #[must_use]
    pub fn is_not_found(&self) -> bool {
        match self {
            Self::Read { source, .. } | Self::Metadata { source, .. } => {
                source.kind() == io::ErrorKind::NotFound
            }
        }
    }
}

impl fmt::Display for ServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Read { path, source } => {
                write!(
                    formatter,
                    "read Caddyfile from file '{}': {source}",
                    path.display()
                )
            }
            Self::Metadata { path, source } => write!(
                formatter,
                "get Caddyfile file info '{}': {source}",
                path.display()
            ),
        }
    }
}

impl StdError for ServiceError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        match self {
            Self::Read { source, .. } | Self::Metadata { source, .. } => Some(source),
        }
    }
}

#[derive(Clone, Debug)]
pub struct Service {
    config_dir: PathBuf,
}

impl Service {
    #[must_use]
    pub fn new(config_dir: impl Into<PathBuf>) -> Self {
        Self {
            config_dir: config_dir.into(),
        }
    }

    #[must_use]
    pub fn config_dir(&self) -> &Path {
        &self.config_dir
    }

    pub fn caddyfile(&self) -> Result<Caddyfile, ServiceError> {
        let path = self.config_dir.join("Caddyfile");
        let content = std::fs::read_to_string(&path).map_err(|source| ServiceError::Read {
            path: path.clone(),
            source,
        })?;
        let modified_at = std::fs::metadata(&path)
            .and_then(|metadata| metadata.modified())
            .map_err(|source| ServiceError::Metadata { path, source })?;
        Ok(Caddyfile {
            content,
            modified_at,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;

    static NEXT_DIR: AtomicU64 = AtomicU64::new(0);

    #[test]
    fn reads_content_and_metadata_and_retains_not_found() {
        let dir = std::env::temp_dir().join(format!(
            "ployz-caddy-service-{}-{}",
            std::process::id(),
            NEXT_DIR.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).unwrap();
        let service = Service::new(&dir);
        let error = service.caddyfile().unwrap_err();
        assert!(error.is_not_found());

        std::fs::write(dir.join("Caddyfile"), "config").unwrap();
        let file = service.caddyfile().unwrap();
        assert_eq!(file.content, "config");
        assert!(file.modified_at <= SystemTime::now());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
