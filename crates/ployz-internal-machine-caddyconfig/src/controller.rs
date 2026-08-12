use std::error::Error as StdError;
use std::fmt;
use std::fs::OpenOptions;
use std::io::Write;
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use ployz_internal_machine_store::{ChangeNotifications, ContainerRecord, ListOptions, Store};
use ployz_pkg_api::{PortSpec, ports_equal};

use crate::admin::{CaddyAdminClient, CaddyAdminClientError};
use crate::caddyfile::CaddyfileGenerator;
use crate::json_config::generate_json_config;

pub const CADDY_SERVICE_NAME: &str = "caddy";
pub const CADDY_GROUP: &str = "uncloud";
pub const VERIFY_PATH: &str = "/.uncloud-verify";

type BoxError = Box<dyn StdError + Send + Sync>;

#[derive(Debug)]
pub struct ControllerError {
    context: String,
    source: Option<BoxError>,
    subscription: Option<ChangeNotifications>,
}

impl ControllerError {
    fn with_source(context: impl Into<String>, source: impl Into<BoxError>) -> Self {
        Self {
            context: context.into(),
            source: Some(source.into()),
            subscription: None,
        }
    }

    fn subscription(context: impl Into<String>, subscription: ChangeNotifications) -> Self {
        Self {
            context: context.into(),
            source: None,
            subscription: Some(subscription),
        }
    }
}

impl fmt::Display for ControllerError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.context)?;
        if let Some(source) = &self.source {
            write!(formatter, ": {source}")?;
        }
        Ok(())
    }
}

impl StdError for ControllerError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        if let Some(source) = self.source.as_deref() {
            return Some(source as &(dyn StdError + 'static));
        }
        self.subscription
            .as_ref()
            .and_then(ChangeNotifications::error)
            .map(|source| source as &(dyn StdError + 'static))
    }
}

#[derive(Clone, Debug)]
struct ContainerFingerprint {
    id: String,
    ip: Option<IpAddr>,
    ports: Vec<PortSpec>,
    caddy_config: String,
}

impl PartialEq for ContainerFingerprint {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
            && self.ip == other.ip
            && ports_equal(&self.ports, &other.ports)
            && self.caddy_config == other.caddy_config
    }
}

#[derive(Debug)]
pub struct Controller {
    machine_id: String,
    caddyfile_path: PathBuf,
    generator: Option<CaddyfileGenerator>,
    client: Arc<CaddyAdminClient>,
    store: Store,
    last_fingerprint: Vec<ContainerFingerprint>,
    last_caddyfile: String,
}

impl Controller {
    pub fn new(
        machine_id: impl Into<String>,
        config_dir: impl AsRef<Path>,
        admin_socket: impl Into<PathBuf>,
        store: Store,
    ) -> Result<Self, ControllerError> {
        let machine_id = machine_id.into();
        let config_dir = config_dir.as_ref();
        create_config_dir(config_dir).map_err(|source| {
            ControllerError::with_source(
                format!(
                    "create directory for Caddy configuration '{}'",
                    config_dir.display()
                ),
                Box::new(source) as BoxError,
            )
        })?;
        set_group(config_dir).map_err(|source| {
            ControllerError::with_source(
                format!(
                    "change owner of directory for Caddy configuration '{}'",
                    config_dir.display()
                ),
                source,
            )
        })?;
        let client = CaddyAdminClient::new(admin_socket.into()).map_err(|source| {
            ControllerError::with_source("create Caddy admin client", Box::new(source) as BoxError)
        })?;
        Ok(Self {
            machine_id,
            caddyfile_path: config_dir.join("Caddyfile"),
            generator: None,
            client: Arc::new(client),
            store,
            last_fingerprint: Vec::new(),
            last_caddyfile: String::new(),
        })
    }

    pub async fn run(&mut self) -> Result<(), ControllerError> {
        let machine_name = match self.store.get_machine(&self.machine_id).await {
            Ok(machine) => machine.name,
            Err(error) => {
                eprintln!(
                    "Failed to get machine from store, Caddy configuration will use machine ID as the name: machine_id={} error={error}",
                    self.machine_id
                );
                self.machine_id.clone()
            }
        };
        self.generator = Some(CaddyfileGenerator::new(
            self.machine_id.clone(),
            machine_name,
            Some(self.client.clone()),
        ));

        let (containers, mut changes) =
            self.store.subscribe_containers().await.map_err(|source| {
                ControllerError::with_source(
                    "subscribe to container changes",
                    Box::new(source) as BoxError,
                )
            })?;
        let mut containers = filter_healthy_containers(containers);
        self.generate_and_load_caddyfile(&mut containers).await;
        if let Err(error) = self.write_json_config(&containers) {
            eprintln!("Failed to generate Caddy JSON configuration to disk: {error}");
        }

        loop {
            tokio::select! {
                change = changes.recv() => {
                    if change.is_none() {
                        return Err(ControllerError::subscription(
                            "subscription to container changes in cluster store failed",
                            changes,
                        ));
                    }
                    let mut containers = match self.store.list_containers(&ListOptions::default()).await {
                        Ok(containers) => filter_healthy_containers(containers),
                        Err(error) => {
                            eprintln!("Failed to list containers: {error}");
                            continue;
                        }
                    };
                    self.generate_and_load_caddyfile(&mut containers).await;
                    if let Err(error) = self.write_json_config(&containers) {
                        eprintln!("Failed to generate Caddy JSON configuration to disk: {error}");
                    }
                }
            }
        }
    }

    async fn generate_and_load_caddyfile(&mut self, containers: &mut [ContainerRecord]) {
        let available = self.client.is_available().await;
        let fingerprint = fingerprint_containers(containers);
        if available && self.last_fingerprint == fingerprint {
            return;
        }
        let Some(generator) = &self.generator else {
            return;
        };
        let caddyfile = match generator.generate(containers, available).await {
            Ok(caddyfile) => caddyfile,
            Err(error) => {
                eprintln!("Failed to generate Caddyfile configuration: {error}");
                return;
            }
        };

        if !available {
            if let Err(error) = self.write_caddyfile_if_changed(&caddyfile) {
                eprintln!("Failed to write Caddyfile to disk: {error}");
            }
            return;
        }
        if let Err(error) = self.client.load(&caddyfile).await {
            eprintln!(
                "Failed to load new Caddy configuration into local Caddy instance: path={} error={error}",
                self.caddyfile_path.display()
            );
            self.last_fingerprint.clear();
            return;
        }
        self.last_fingerprint = fingerprint;
        if let Err(error) = self.write_caddyfile_if_changed(&caddyfile) {
            eprintln!("Failed to write Caddyfile to disk after successful load: {error}");
        }
    }

    fn write_caddyfile_if_changed(&mut self, caddyfile: &str) -> Result<(), ControllerError> {
        if caddyfile_body(caddyfile) == caddyfile_body(&self.last_caddyfile) {
            return Ok(());
        }
        write_mode(&self.caddyfile_path, caddyfile.as_bytes(), 0o640).map_err(|source| {
            ControllerError::with_source(
                format!(
                    "write Caddyfile to file '{}'",
                    self.caddyfile_path.display()
                ),
                Box::new(source) as BoxError,
            )
        })?;
        set_group(&self.caddyfile_path).map_err(|source| {
            ControllerError::with_source(
                format!(
                    "change owner of Caddyfile '{}'",
                    self.caddyfile_path.display()
                ),
                source,
            )
        })?;
        self.last_caddyfile = caddyfile.to_owned();
        Ok(())
    }

    fn write_json_config(&self, records: &[ContainerRecord]) -> Result<(), ControllerError> {
        let containers = records
            .iter()
            .map(|record| record.container.clone())
            .collect::<Vec<_>>();
        let config = generate_json_config(&containers, &self.machine_id);
        let mut bytes = Vec::new();
        let formatter = serde_json::ser::PrettyFormatter::with_indent(b"    ");
        let mut serializer = serde_json::Serializer::with_formatter(&mut bytes, formatter);
        serde::Serialize::serialize(&config, &mut serializer).map_err(|source| {
            ControllerError::with_source(
                "marshal Caddy configuration",
                Box::new(source) as BoxError,
            )
        })?;
        let path = self
            .caddyfile_path
            .parent()
            .unwrap_or_else(|| Path::new(""))
            .join("caddy.json");
        write_mode(&path, &bytes, 0o640).map_err(|source| {
            ControllerError::with_source(
                format!("write Caddy configuration to file '{}'", path.display()),
                Box::new(source) as BoxError,
            )
        })?;
        set_group(&path).map_err(|source| {
            ControllerError::with_source(
                format!(
                    "change owner of Caddy configuration file '{}'",
                    path.display()
                ),
                source,
            )
        })
    }
}

fn filter_healthy_containers(containers: Vec<ContainerRecord>) -> Vec<ContainerRecord> {
    containers
        .into_iter()
        .filter(|record| !record.container.is_hook() && record.container.container.healthy())
        .collect()
}

fn fingerprint_containers(containers: &[ContainerRecord]) -> Vec<ContainerFingerprint> {
    let mut fingerprints = containers
        .iter()
        .map(|record| ContainerFingerprint {
            id: record.container.container.id.clone(),
            ip: record.container.container.uncloud_network_ip(),
            // The generator reports the parse failure; the fingerprint keeps
            // the oracle's intentional empty fallback without duplicating it.
            ports: record.container.service_ports().unwrap_or_default(),
            caddy_config: record.container.service_spec.caddy_config().to_owned(),
        })
        .collect::<Vec<_>>();
    fingerprints.sort_by(|left, right| left.id.cmp(&right.id));
    fingerprints
}

fn caddyfile_body(caddyfile: &str) -> &str {
    caddyfile
        .split_once('\n')
        .map_or(caddyfile, |(_, body)| body)
}

#[cfg(unix)]
fn create_config_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;

    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o750)
        .create(path)
}

#[cfg(not(unix))]
fn create_config_dir(path: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(path)
}

#[cfg(unix)]
fn write_mode(path: &Path, bytes: &[u8], mode: u32) -> std::io::Result<()> {
    use std::os::unix::fs::OpenOptionsExt;

    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(mode)
        .open(path)?;
    file.write_all(bytes)
}

#[cfg(not(unix))]
fn write_mode(path: &Path, bytes: &[u8], _mode: u32) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

#[cfg(target_os = "linux")]
fn set_group(path: &Path) -> Result<(), BoxError> {
    ployz_internal_fs::chown(path, "", CADDY_GROUP).map_err(|error| Box::new(error) as BoxError)
}

#[cfg(not(target_os = "linux"))]
fn set_group(_path: &Path) -> Result<(), BoxError> {
    Ok(())
}

impl From<CaddyAdminClientError> for ControllerError {
    fn from(source: CaddyAdminClientError) -> Self {
        Self::with_source("Caddy admin operation", Box::new(source) as BoxError)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_equality_covers_every_behavior_field() {
        let base = ContainerFingerprint {
            id: "container-1".into(),
            ip: Some("10.210.0.2".parse().unwrap()),
            ports: vec![PortSpec {
                hostname: "app.example.com".into(),
                container_port: 8080,
                protocol: "http".into(),
                mode: "ingress".into(),
                ..Default::default()
            }],
            caddy_config: "caddy-config".into(),
        };
        assert_eq!(base, base.clone());
        let mutations = [
            ContainerFingerprint {
                id: "different".into(),
                ..base.clone()
            },
            ContainerFingerprint {
                ip: Some("10.210.0.99".parse().unwrap()),
                ..base.clone()
            },
            ContainerFingerprint {
                ports: Vec::new(),
                ..base.clone()
            },
            ContainerFingerprint {
                caddy_config: "different".into(),
                ..base.clone()
            },
        ];
        for mutation in mutations {
            assert_ne!(base, mutation);
        }

        let mut reordered = base.clone();
        reordered.ports.push(PortSpec {
            hostname: "other.example.com".into(),
            container_port: 9090,
            protocol: "https".into(),
            mode: "ingress".into(),
            ..Default::default()
        });
        let mut same_ports_different_order = reordered.clone();
        same_ports_different_order.ports.reverse();
        assert_eq!(reordered, same_ports_different_order);
    }

    #[test]
    fn body_comparison_excludes_only_first_line() {
        assert_eq!(caddyfile_body("timestamp\nbody\n"), "body\n");
        assert_eq!(caddyfile_body("single"), "single");
    }

    #[test]
    fn initial_cache_matches_empty_fingerprint_like_go_nil_slice() {
        let initial_cache = Vec::<ContainerFingerprint>::new();
        assert_eq!(initial_cache, fingerprint_containers(&[]));
    }
}
