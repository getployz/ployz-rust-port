use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
};

use ipnet::IpNet;
use ployz_internal_secret::Secret;
use serde::{Deserialize, Serialize};

use crate::{DEFAULT_WIREGUARD_PORT, MAX_WIREGUARD_MTU, NetworkError, ip::single_ip_prefix};

/// Complete machine WireGuard configuration.
#[derive(Clone, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct Config {
    #[serde(with = "optional_prefix")]
    pub subnet: Option<IpNet>,
    #[serde(rename = "ManagementIP", with = "optional_addr")]
    pub management_ip: Option<IpAddr>,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub wire_guard_port: i32,
    #[serde(rename = "MTU", default, skip_serializing_if = "is_zero_i32")]
    pub mtu: i32,
    #[serde(with = "optional_secret")]
    pub private_key: Option<Secret>,
    #[serde(with = "optional_secret")]
    pub public_key: Option<Secret>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub endpoints: Vec<SocketAddr>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub peers: Vec<PeerConfig>,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Config")
            .field("subnet", &self.subnet)
            .field("management_ip", &self.management_ip)
            .field("wire_guard_port", &self.wire_guard_port)
            .field("mtu", &self.mtu)
            .field(
                "private_key",
                &self.private_key.as_ref().map(|_| "[REDACTED]"),
            )
            .field("public_key", &self.public_key)
            .field("endpoints", &self.endpoints)
            .field("peers", &self.peers)
            .finish()
    }
}

/// WireGuard and routed-address configuration for one remote machine.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct PeerConfig {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "optional_prefix"
    )]
    pub subnet: Option<IpNet>,
    #[serde(rename = "ManagementIP", with = "optional_addr")]
    pub management_ip: Option<IpAddr>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<Arc<SocketAddr>>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub all_endpoints: Vec<Arc<SocketAddr>>,
    #[serde(with = "optional_secret")]
    pub public_key: Option<Secret>,
}

mod optional_addr {
    use std::net::IpAddr;

    use serde::{Deserialize, Deserializer, Serializer, de::Error as _};

    pub fn serialize<S>(value: &Option<IpAddr>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.map_or_else(String::new, |value| value.to_string()))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<IpAddr>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if value.is_empty() {
            Ok(None)
        } else {
            value.parse().map(Some).map_err(D::Error::custom)
        }
    }
}

mod optional_prefix {
    use ipnet::IpNet;
    use serde::{Deserialize, Deserializer, Serializer, de::Error as _};

    pub fn serialize<S>(value: &Option<IpNet>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&value.map_or_else(String::new, |value| value.to_string()))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<IpNet>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if value.is_empty() {
            Ok(None)
        } else {
            value.parse().map(Some).map_err(D::Error::custom)
        }
    }
}

mod optional_secret {
    use ployz_internal_secret::Secret;
    use serde::{Deserialize, Deserializer, Serializer, de::Error as _};

    pub fn serialize<S>(value: &Option<Secret>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(
            &value
                .as_ref()
                .map_or_else(String::new, Secret::to_hex_string),
        )
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<Secret>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Secret::from_hex_string(&value)
            .map(Some)
            .map_err(D::Error::custom)
    }
}

const fn is_zero_i32(value: &i32) -> bool {
    *value == 0
}

impl Config {
    /// Returns whether all values needed to establish WireGuard are present.
    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.subnet.is_some()
            && self.management_ip.is_some()
            && self.private_key.is_some()
            && self.public_key.is_some()
    }

    /// Returns the configured listen port, or 51820 when it is zero.
    #[must_use]
    pub fn effective_wire_guard_port(&self) -> i32 {
        if self.wire_guard_port == 0 {
            i32::from(DEFAULT_WIREGUARD_PORT)
        } else {
            self.wire_guard_port
        }
    }

    /// Returns the configured interface MTU, or 1420 when it is zero.
    #[must_use]
    pub fn effective_mtu(&self) -> i32 {
        if self.mtu == 0 {
            MAX_WIREGUARD_MTU as i32
        } else {
            self.mtu
        }
    }
}

impl PeerConfig {
    pub(crate) fn prefixes(&self) -> Result<Vec<IpNet>, NetworkError> {
        let management_ip = self.management_ip.ok_or_else(|| {
            NetworkError::Invalid("parse management IP: invalid IP address".into())
        })?;
        let mut prefixes = vec![single_ip_prefix(management_ip)?];
        if let Some(subnet) = self.subnet {
            prefixes.push(subnet);
        }
        Ok(prefixes)
    }

    pub(crate) fn key_string(&self) -> Result<String, NetworkError> {
        self.public_key
            .as_ref()
            .map(Secret::to_hex_string)
            .ok_or_else(|| NetworkError::Invalid("peer public key is absent".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_match_go_zero_value_fallbacks() {
        let config = Config::default();
        assert_eq!(config.effective_wire_guard_port(), 51_820);
        assert_eq!(config.effective_mtu(), 1_420);
        assert!(!config.is_configured());
    }

    #[test]
    fn explicit_values_override_defaults() {
        let config = Config {
            wire_guard_port: 51_821,
            mtu: 1_300,
            ..Config::default()
        };
        assert_eq!(config.effective_wire_guard_port(), 51_821);
        assert_eq!(config.effective_mtu(), 1_300);
    }

    #[test]
    fn zero_value_json_matches_go_text_marshalling() {
        let value = serde_json::to_value(Config::default()).expect("serializable config");
        assert_eq!(value["Subnet"], "");
        assert_eq!(value["ManagementIP"], "");
        assert_eq!(value["PrivateKey"], "");
        assert_eq!(value["PublicKey"], "");
    }
}
