use std::fmt;
use std::net::{IpAddr, SocketAddr};
use std::str::FromStr;

use base64::Engine;
use ployz_internal_machine_api_pb as pb;
use serde::ser::{Serialize, SerializeStruct, Serializer};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MachineFilter {
    pub available: bool,
    pub names_or_ids: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MachineMembersList(pub Vec<pb::MachineMember>);

impl MachineMembersList {
    #[must_use]
    pub fn find_by_name_or_id(&self, name_or_id: &str) -> Option<&pb::MachineMember> {
        self.0.iter().find(|member| {
            member
                .machine
                .as_ref()
                .is_some_and(|machine| machine.id == name_or_id || machine.name == name_or_id)
        })
    }

    #[must_use]
    pub fn to_native(&self) -> Vec<MachineMember> {
        self.0.iter().map(machine_member_from_proto).collect()
    }
}

impl From<Vec<pb::MachineMember>> for MachineMembersList {
    fn from(members: Vec<pb::MachineMember>) -> Self {
        Self(members)
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MachineMember {
    pub id: String,
    pub name: String,
    pub state: String,
    pub network: MachineNetwork,
    pub public_ip: Option<ScopedIpAddr>,
    pub daemon_version: String,
    pub docker_version: String,
    pub hostname: String,
    pub arch: String,
    pub os_pretty_name: String,
    pub kernel_version: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MachineNetwork {
    pub subnet: Option<(IpAddr, u8)>,
    pub management_ip: Option<ScopedIpAddr>,
    pub endpoints: Vec<ScopedAddrPort>,
    pub public_key: Vec<u8>,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ScopedIpAddr {
    pub address: IpAddr,
    pub zone: Vec<u8>,
}

impl ScopedIpAddr {
    #[must_use]
    pub const fn is_ipv6(&self) -> bool {
        self.address.is_ipv6()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScopedAddrPort {
    pub address: ScopedIpAddr,
    pub port: u16,
}

impl fmt::Display for ScopedIpAddr {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.address)?;
        if !self.zone.is_empty() {
            write!(formatter, "%{}", String::from_utf8_lossy(&self.zone))?;
        }
        Ok(())
    }
}

impl fmt::Display for ScopedAddrPort {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.address.address.is_ipv6() {
            write!(formatter, "[{}]:{}", self.address, self.port)
        } else {
            write!(formatter, "{}:{}", self.address, self.port)
        }
    }
}

impl From<IpAddr> for ScopedIpAddr {
    fn from(address: IpAddr) -> Self {
        Self {
            address,
            zone: Vec::new(),
        }
    }
}

impl FromStr for ScopedIpAddr {
    type Err = std::net::AddrParseError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        if let Some((address, zone)) = value.split_once('%') {
            let address = address.parse::<IpAddr>()?;
            if address.is_ipv6() && !zone.is_empty() {
                return Ok(Self {
                    address,
                    zone: zone.as_bytes().to_vec(),
                });
            }
        }
        Ok(Self::from(value.parse::<IpAddr>()?))
    }
}

impl serde::Serialize for ScopedIpAddr {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> serde::Deserialize<'de> for ScopedIpAddr {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        <String as serde::Deserialize>::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

impl From<SocketAddr> for ScopedAddrPort {
    fn from(address: SocketAddr) -> Self {
        Self {
            address: address.ip().into(),
            port: address.port(),
        }
    }
}

fn machine_member_from_proto(member: &pb::MachineMember) -> MachineMember {
    let machine = member.machine.as_ref().expect("Machine not set");
    let mut native = MachineMember {
        id: machine.id.clone(),
        name: machine.name.clone(),
        state: capitalise(
            pb::machine_member::MembershipState::try_from(member.state)
                .unwrap_or(pb::machine_member::MembershipState::Unknown)
                .as_str_name(),
        ),
        daemon_version: machine.daemon_version.clone(),
        docker_version: machine.docker_version.clone(),
        hostname: machine.hostname.clone(),
        arch: machine.arch.clone(),
        os_pretty_name: machine.os_pretty_name.clone(),
        kernel_version: machine.kernel_version.clone(),
        ..MachineMember::default()
    };
    if let Some(public_ip) = &machine.public_ip
        && let Ok(address) = public_ip.to_addr()
    {
        native.public_ip = Some(ScopedIpAddr {
            address: address.ip(),
            zone: address.zone().to_vec(),
        });
    }
    if let Some(network) = &machine.network {
        native.network.public_key.clone_from(&network.public_key);
        if let Some(subnet) = &network.subnet
            && let Ok(prefix) = subnet.to_prefix()
        {
            native.network.subnet = Some(prefix);
        }
        if let Some(management_ip) = &network.management_ip
            && let Ok(address) = management_ip.to_addr()
        {
            native.network.management_ip = Some(ScopedIpAddr {
                address: address.ip(),
                zone: address.zone().to_vec(),
            });
        }
        native.network.endpoints = network
            .endpoints
            .iter()
            .filter_map(|endpoint| endpoint.to_addr_port().ok())
            .map(|endpoint| ScopedAddrPort {
                address: ScopedIpAddr {
                    address: endpoint.address().ip(),
                    zone: endpoint.address().zone().to_vec(),
                },
                port: endpoint.port(),
            })
            .collect();
    }
    native
}

fn capitalise(value: &str) -> String {
    let mut bytes = value.to_ascii_lowercase().into_bytes();
    if let Some(first) = bytes.first_mut() {
        first.make_ascii_uppercase();
    }
    String::from_utf8(bytes).expect("ASCII transformation remains UTF-8")
}

impl Serialize for MachineMember {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("MachineMember", 11)?;
        state.serialize_field("ID", &self.id)?;
        state.serialize_field("Name", &self.name)?;
        state.serialize_field("State", &self.state)?;
        state.serialize_field("Network", &self.network)?;
        state.serialize_field(
            "PublicIP",
            &self
                .public_ip
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
        )?;
        state.serialize_field("DaemonVersion", &self.daemon_version)?;
        state.serialize_field("DockerVersion", &self.docker_version)?;
        state.serialize_field("Hostname", &self.hostname)?;
        state.serialize_field("Arch", &self.arch)?;
        state.serialize_field("OSPrettyName", &self.os_pretty_name)?;
        state.serialize_field("KernelVersion", &self.kernel_version)?;
        state.end()
    }
}

impl Serialize for MachineNetwork {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut state = serializer.serialize_struct("MachineNetwork", 4)?;
        state.serialize_field(
            "Subnet",
            &self
                .subnet
                .map(|(address, bits)| format!("{address}/{bits}"))
                .unwrap_or_default(),
        )?;
        state.serialize_field(
            "ManagementIP",
            &self
                .management_ip
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default(),
        )?;
        state.serialize_field(
            "Endpoints",
            &self
                .endpoints
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
        )?;
        state.serialize_field(
            "PublicKey",
            &base64::engine::general_purpose::STANDARD.encode(&self.public_key),
        )?;
        state.end()
    }
}
