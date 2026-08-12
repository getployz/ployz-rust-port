use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::future::Future;
use std::net::Ipv6Addr;
use std::pin::Pin;

use ployz_internal_machine_api_pb::MachineInfo;
use tonic::Status;

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Boxed asynchronous store lookup returned by [`MachineStore`].
pub type StoreFuture<'a, T, E> = Pin<Box<dyn Future<Output = Result<Vec<T>, E>> + Send + 'a>>;

/// The store operation required to resolve machine names and IDs.
pub trait MachineStore: Send + Sync + 'static {
    type Error: fmt::Display + Send + Sync + 'static;

    fn list_machines(&self) -> StoreFuture<'_, MachineInfo, Self::Error>;
}

/// A resolved proxy target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MachineTarget {
    pub(crate) id: String,
    pub(crate) name: String,
    pub(crate) address: String,
    pub(crate) remote_address: Option<RemoteAddress>,
    pub(crate) address_is_utf8: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RemoteAddress {
    pub(crate) ip: Ipv6Addr,
    pub(crate) zone: Vec<u8>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(crate) enum AddressKey {
    Remote { ip: Ipv6Addr, zone: Vec<u8> },
    Text(String),
}

impl MachineTarget {
    #[must_use]
    pub fn new(id: impl Into<String>, name: impl Into<String>, address: impl Into<String>) -> Self {
        let address = address.into();
        let remote_address = parse_remote_address(address.as_bytes());
        Self {
            id: id.into(),
            name: name.into(),
            address,
            remote_address,
            address_is_utf8: true,
        }
    }

    pub(crate) fn from_management_address(
        id: impl Into<String>,
        name: impl Into<String>,
        ip: Ipv6Addr,
        zone: Vec<u8>,
    ) -> Self {
        let address_is_utf8 = std::str::from_utf8(&zone).is_ok();
        let address = if zone.is_empty() {
            ip.to_string()
        } else {
            format!("{ip}%{}", String::from_utf8_lossy(&zone))
        };
        Self {
            id: id.into(),
            name: name.into(),
            address,
            remote_address: Some(RemoteAddress { ip, zone }),
            address_is_utf8,
        }
    }

    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    #[must_use]
    pub fn address(&self) -> &str {
        &self.address
    }

    pub(crate) fn address_key(&self) -> AddressKey {
        self.remote_address.as_ref().map_or_else(
            || AddressKey::Text(self.address.clone()),
            |remote| AddressKey::Remote {
                ip: remote.ip,
                zone: remote.zone.clone(),
            },
        )
    }
}

/// One or more requested machines could not be resolved.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MachinesNotFoundError {
    not_found: Vec<String>,
}

impl MachinesNotFoundError {
    #[must_use]
    pub fn new(not_found: Vec<String>) -> Self {
        Self { not_found }
    }

    #[must_use]
    pub fn not_found(&self) -> &[String] {
        &self.not_found
    }
}

impl fmt::Display for MachinesNotFoundError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.not_found.len() == 1 {
            write!(formatter, "machine not found: {}", self.not_found[0])
        } else {
            write!(
                formatter,
                "machines not found: {}",
                self.not_found.join(", ")
            )
        }
    }
}

impl Error for MachinesNotFoundError {}

/// Resolution failures understood by the director.
#[derive(Debug)]
pub enum MapMachinesError {
    NotFound(MachinesNotFoundError),
    Status(Status),
    Other(String),
}

impl fmt::Display for MapMachinesError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(error) => error.fmt(formatter),
            Self::Status(status) => status.fmt(formatter),
            Self::Other(message) => formatter.write_str(message),
        }
    }
}

impl Error for MapMachinesError {}

impl From<MachinesNotFoundError> for MapMachinesError {
    fn from(error: MachinesNotFoundError) -> Self {
        Self::NotFound(error)
    }
}

impl From<Status> for MapMachinesError {
    fn from(status: Status) -> Self {
        Self::Status(status)
    }
}

/// Asynchronous target resolution used by the director.
pub trait MachineMapper: Send + Sync + 'static {
    fn map_machines<'a>(
        &'a self,
        names_or_ids: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<MachineTarget>, MapMachinesError>>;
}

/// Resolves machines from the cluster store.
#[derive(Clone, Debug)]
pub struct CorrosionMapper<S> {
    store: S,
}

impl<S> CorrosionMapper<S> {
    #[must_use]
    pub fn new(store: S) -> Self {
        Self { store }
    }
}

impl<S> CorrosionMapper<S>
where
    S: MachineStore,
{
    pub async fn map_machines(
        &self,
        names_or_ids: &[String],
    ) -> Result<Vec<MachineTarget>, MapMachinesError> {
        if names_or_ids.is_empty() {
            return Err(MapMachinesError::Other("no machines specified".to_owned()));
        }

        let machines = self
            .store
            .list_machines()
            .await
            .map_err(|error| MapMachinesError::Other(format!("list machines: {error}")))?;
        let mut all_targets = Vec::with_capacity(machines.len());
        for machine in machines {
            let network = machine.network.as_ref().expect("machine network not set");
            let management_ip = network
                .management_ip
                .as_ref()
                .expect("machine management IP not set");
            let address = management_ip.to_addr().map_err(|error| {
                MapMachinesError::Other(format!(
                    "invalid management IP for machine '{}' in store: {error}",
                    machine.name
                ))
            })?;
            match address.ip() {
                std::net::IpAddr::V6(ip) => {
                    all_targets.push(MachineTarget::from_management_address(
                        machine.id,
                        machine.name,
                        ip,
                        address.zone().to_vec(),
                    ));
                }
                std::net::IpAddr::V4(ip) => {
                    all_targets.push(MachineTarget::new(machine.id, machine.name, ip.to_string()));
                }
            }
        }

        if names_or_ids.iter().any(|name| name == "*") {
            if all_targets.is_empty() {
                return Err(MapMachinesError::Other("no machines in cluster".to_owned()));
            }
            return Ok(all_targets);
        }

        let mut lookup = HashMap::with_capacity(all_targets.len() * 2);
        for target in &all_targets {
            lookup.insert(target.id.clone(), target.clone());
            lookup.insert(target.name.clone(), target.clone());
        }

        let mut targets = Vec::with_capacity(names_or_ids.len());
        let mut seen = HashSet::with_capacity(names_or_ids.len());
        let mut not_found = Vec::new();
        for name_or_id in names_or_ids {
            if let Some(target) = lookup.get(name_or_id) {
                if seen.insert(target.id.clone()) {
                    targets.push(target.clone());
                }
            } else {
                not_found.push(name_or_id.clone());
            }
        }

        if not_found.is_empty() {
            Ok(targets)
        } else {
            Err(MachinesNotFoundError::new(not_found).into())
        }
    }
}

fn parse_remote_address(address: &[u8]) -> Option<RemoteAddress> {
    let (ip, zone) = match address.iter().position(|byte| *byte == b'%') {
        Some(index) if index + 1 < address.len() => (&address[..index], &address[index + 1..]),
        Some(_) => return None,
        None => (address, &[][..]),
    };
    let ip = std::str::from_utf8(ip).ok()?.parse().ok()?;
    Some(RemoteAddress {
        ip,
        zone: zone.to_vec(),
    })
}

impl<S> MachineMapper for CorrosionMapper<S>
where
    S: MachineStore,
{
    fn map_machines<'a>(
        &'a self,
        names_or_ids: &'a [String],
    ) -> BoxFuture<'a, Result<Vec<MachineTarget>, MapMachinesError>> {
        Box::pin(self.map_machines(names_or_ids))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_retains_scoped_ipv6_zone_bytes() {
        let target = MachineTarget::from_management_address(
            "id",
            "name",
            "fe80::1".parse().unwrap(),
            vec![b'e', b'n', 0xff],
        );
        let remote = target.remote_address.unwrap();
        assert_eq!(remote.ip, "fe80::1".parse::<Ipv6Addr>().unwrap());
        assert_eq!(remote.zone, [b'e', b'n', 0xff]);
    }
}
