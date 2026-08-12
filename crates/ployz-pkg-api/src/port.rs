use std::fmt;
use std::net::IpAddr;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::{ApiError, Result, ScopedIpAddr};

pub const PORT_MODE_INGRESS: &str = "ingress";
pub const PORT_MODE_HOST: &str = "host";
pub const PROTOCOL_HTTP: &str = "http";
pub const PROTOCOL_HTTPS: &str = "https";
pub const PROTOCOL_TCP: &str = "tcp";
pub const PROTOCOL_UDP: &str = "udp";

#[derive(Clone, Debug, Default, Eq, Hash, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "PascalCase")]
pub struct PortSpec {
    pub hostname: String,
    #[serde(default, rename = "HostIP", with = "go_optional_ip")]
    pub host_ip: Option<ScopedIpAddr>,
    #[serde(default, with = "go_optional_prefix")]
    pub host_prefix: Option<IpPrefix>,
    pub published_port: u16,
    pub container_port: u16,
    pub protocol: String,
    pub mode: String,
}

mod go_optional_ip {
    use crate::ScopedIpAddr;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &Option<ScopedIpAddr>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.as_ref().map_or_else(String::new, ToString::to_string))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<ScopedIpAddr>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if value.is_empty() {
            Ok(None)
        } else {
            value.parse().map(Some).map_err(serde::de::Error::custom)
        }
    }
}

mod go_optional_prefix {
    use super::IpPrefix;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &Option<IpPrefix>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.map_or_else(String::new, |prefix| prefix.to_string()))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<IpPrefix>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if value.is_empty() {
            Ok(None)
        } else {
            value.parse().map(Some).map_err(serde::de::Error::custom)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct IpPrefix {
    address: IpAddr,
    bits: u8,
}

impl IpPrefix {
    pub fn new(address: IpAddr, bits: u8) -> Result<Self> {
        let maximum = if address.is_ipv4() { 32 } else { 128 };
        if bits > maximum {
            return Err(ApiError::invalid("invalid prefix"));
        }
        Ok(Self { address, bits })
    }

    #[must_use]
    pub fn address(self) -> IpAddr {
        self.address
    }

    #[must_use]
    pub fn bits(self) -> u8 {
        self.bits
    }
}

impl fmt::Display for IpPrefix {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}/{}", self.address, self.bits)
    }
}

impl FromStr for IpPrefix {
    type Err = ApiError;

    fn from_str(value: &str) -> Result<Self> {
        let (address, bits) = value
            .split_once('/')
            .ok_or_else(|| ApiError::invalid("invalid prefix"))?;
        let address = address
            .parse()
            .map_err(|error| ApiError::invalid(format!("invalid IP address: {error}")))?;
        let bits = bits
            .parse()
            .map_err(|error| ApiError::invalid(format!("invalid prefix length: {error}")))?;
        Self::new(address, bits)
    }
}

impl Serialize for IpPrefix {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for IpPrefix {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        String::deserialize(deserializer)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}

impl PortSpec {
    pub fn validate(&self) -> Result<()> {
        if self.container_port == 0 {
            return Err(ApiError::invalid("container port must be non-zero"));
        }
        match self.protocol.as_str() {
            "" => return Err(ApiError::invalid("protocol must be specified")),
            PROTOCOL_HTTP | PROTOCOL_HTTPS | PROTOCOL_TCP | PROTOCOL_UDP => {}
            protocol => {
                return Err(ApiError::invalid(format!(
                    "invalid protocol '{protocol}', supported protocols: 'http', 'https', 'tcp', 'udp'"
                )));
            }
        }
        match self.mode.as_str() {
            "" => return Err(ApiError::invalid("mode must be specified")),
            PORT_MODE_INGRESS => {
                if self.host_ip.is_some() {
                    return Err(ApiError::invalid(
                        "host IP cannot be specified in ingress mode",
                    ));
                }
                if self.host_prefix.is_some() {
                    return Err(ApiError::invalid(
                        "host prefix cannot be specified in ingress mode",
                    ));
                }
                if !self.hostname.is_empty() {
                    if !matches!(self.protocol.as_str(), PROTOCOL_HTTP | PROTOCOL_HTTPS) {
                        return Err(ApiError::invalid(
                            "hostname is only valid with 'http' or 'https' protocols",
                        ));
                    }
                    validate_hostname(&self.hostname)?;
                }
            }
            PORT_MODE_HOST => {
                if self.host_ip.is_some() && self.host_prefix.is_some() {
                    return Err(ApiError::invalid(
                        "host IP and prefix cannot both be specified in host mode",
                    ));
                }
                if self.published_port == 0 {
                    return Err(ApiError::invalid("published port is required in host mode"));
                }
                if !matches!(self.protocol.as_str(), PROTOCOL_TCP | PROTOCOL_UDP) {
                    return Err(ApiError::invalid(format!(
                        "unsupported protocol '{}' in host mode, only 'tcp' and 'udp' are supported",
                        self.protocol
                    )));
                }
                if !self.hostname.is_empty() {
                    return Err(ApiError::invalid(
                        "hostname cannot be specified in host mode",
                    ));
                }
            }
            mode => return Err(ApiError::invalid(format!("invalid mode: '{mode}'"))),
        }
        Ok(())
    }

    pub fn format(&self) -> Result<String> {
        self.validate()?;
        let mut parts = Vec::new();
        match self.mode.as_str() {
            PORT_MODE_INGRESS => {
                if !self.hostname.is_empty() {
                    parts.push(self.hostname.clone());
                }
                if self.published_port != 0 {
                    parts.push(self.published_port.to_string());
                }
                parts.push(self.container_port.to_string());
                Ok(format!("{}/{}", parts.join(":"), self.protocol))
            }
            PORT_MODE_HOST => {
                if let Some(address) = &self.host_ip {
                    parts.push(if address.is_ipv6() {
                        format!("[{address}]")
                    } else {
                        address.to_string()
                    });
                }
                if let Some(prefix) = self.host_prefix {
                    parts.push(match prefix.address() {
                        IpAddr::V6(address) => format!("[{address}]/{}", prefix.bits()),
                        IpAddr::V4(_) => prefix.to_string(),
                    });
                }
                parts.push(self.published_port.to_string());
                parts.push(self.container_port.to_string());
                Ok(format!("{}/{}@host", parts.join(":"), self.protocol))
            }
            mode => Err(ApiError::invalid(format!(
                "not implemented for mode: '{mode}'"
            ))),
        }
    }
}

impl fmt::Display for PortSpec {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.format() {
            Ok(value) => formatter.write_str(&value),
            Err(error) => write!(formatter, "<invalid port: {error}>"),
        }
    }
}

pub fn parse_port_spec(port: &str) -> Result<PortSpec> {
    let mut spec = PortSpec {
        protocol: PROTOCOL_TCP.into(),
        ..PortSpec::default()
    };
    if port.matches('@').count() > 1 {
        return Err(ApiError::invalid("too many '@' symbols"));
    }
    let parts = split_port_parts(port);
    let mut terminal = parts.last().map_or("", String::as_str);
    let mut specified_protocol = "";
    if let Some((before, mode)) = terminal.split_once('@') {
        spec.mode = PORT_MODE_HOST.into();
        if mode != PORT_MODE_HOST {
            return Err(ApiError::invalid(format!("invalid mode: '{mode}'")));
        }
        terminal = before;
        if let Some((port_value, protocol)) = terminal.split_once('/') {
            if !matches!(protocol, PROTOCOL_TCP | PROTOCOL_UDP) {
                return Err(ApiError::invalid(format!(
                    "unsupported protocol '{protocol}' in host mode, only 'tcp' and 'udp' are supported"
                )));
            }
            spec.protocol = protocol.into();
            terminal = port_value;
        }
    } else {
        spec.mode = PORT_MODE_INGRESS.into();
        if let Some((port_value, protocol)) = terminal.split_once('/') {
            if !matches!(
                protocol,
                PROTOCOL_TCP | PROTOCOL_UDP | PROTOCOL_HTTP | PROTOCOL_HTTPS
            ) {
                return Err(ApiError::invalid(format!(
                    "unsupported protocol: '{protocol}'"
                )));
            }
            spec.protocol = protocol.into();
            specified_protocol = protocol;
            terminal = port_value;
        }
    }
    spec.container_port = parse_port(terminal).map_err(|error| {
        ApiError::invalid(format!("invalid container port '{terminal}': {error}"))
    })?;
    match parts.len() {
        1 => {}
        2 => {
            if parts[0].is_empty() {
                return Err(ApiError::invalid(
                    "hostname or published port must be specified, format: hostname:container_port or published_port:container_port",
                ));
            }
            match parse_port(&parts[0]) {
                Ok(value) => spec.published_port = value,
                Err(_) if spec.mode == PORT_MODE_HOST => {
                    return Err(ApiError::invalid(
                        "hostname cannot be specified in host mode",
                    ));
                }
                Err(_) => spec.hostname.clone_from(&parts[0]),
            }
        }
        3 => {
            spec.published_port = parse_port(&parts[1]).map_err(|error| {
                ApiError::invalid(format!("invalid published port '{}': {error}", parts[1]))
            })?;
            if spec.mode == PORT_MODE_HOST {
                let mut host = parts[0].clone();
                if host.contains(':') {
                    let end = host.find(']').ok_or_else(|| {
                        ApiError::invalid(format!(
                            "invalid host IP '{}': IPv6 address must be enclosed in square brackets",
                            parts[0]
                        ))
                    })?;
                    if !host.starts_with('[') {
                        return Err(ApiError::invalid(format!(
                            "invalid host IP '{}': IPv6 address must be enclosed in square brackets",
                            parts[0]
                        )));
                    }
                    host = format!("{}{}", &host[1..end], &host[end + 1..]);
                }
                if host.contains('/') {
                    spec.host_prefix = Some(host.parse().map_err(|error| {
                        ApiError::invalid(format!("invalid host prefix '{}': {error}", parts[0]))
                    })?);
                } else {
                    spec.host_ip = Some(host.parse().map_err(|error| {
                        ApiError::invalid(format!("invalid host IP '{}': {error}", parts[0]))
                    })?);
                }
            } else {
                spec.hostname.clone_from(&parts[0]);
            }
        }
        count => {
            return Err(ApiError::invalid(format!(
                "unexpected number of parts in port spec: {count}"
            )));
        }
    }
    if !spec.hostname.is_empty() {
        if specified_protocol.is_empty() {
            spec.protocol = PROTOCOL_HTTPS.into();
        } else if !matches!(specified_protocol, PROTOCOL_HTTP | PROTOCOL_HTTPS) {
            return Err(ApiError::invalid(format!(
                "hostname is only valid with 'http' or 'https' protocols, specified: '{specified_protocol}'"
            )));
        }
    }
    spec.validate()?;
    Ok(spec)
}

fn split_port_parts(port: &str) -> Vec<String> {
    let parts: Vec<_> = port.split(':').map(str::to_owned).collect();
    if parts.len() > 3 {
        let split = parts.len() - 2;
        return vec![
            parts[..split].join(":"),
            parts[split].clone(),
            parts[split + 1].clone(),
        ];
    }
    parts
}

fn parse_port(value: &str) -> std::result::Result<u16, std::num::ParseIntError> {
    value.parse()
}

fn validate_hostname(hostname: &str) -> Result<()> {
    if !hostname.contains('.') {
        return Err(ApiError::invalid(format!(
            "invalid hostname '{hostname}': must be a valid domain name containing at least one dot"
        )));
    }
    Ok(())
}

#[must_use]
pub fn ports_equal(left: &[PortSpec], right: &[PortSpec]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let serialise = |ports: &[PortSpec]| -> Option<Vec<String>> {
        let mut values: Vec<_> = ports
            .iter()
            .map(PortSpec::format)
            .collect::<Result<_>>()
            .ok()?;
        values.sort_unstable();
        Some(values)
    };
    match (serialise(left), serialise(right)) {
        (Some(left), Some(right)) => left == right,
        _ => false,
    }
}
