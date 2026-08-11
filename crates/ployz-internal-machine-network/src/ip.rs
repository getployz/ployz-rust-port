use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use ipnet::IpNet;
use ployz_internal_secret::Secret;

use crate::NetworkError;

/// Returns the machine address: the first usable address in its subnet.
pub fn machine_ip(subnet: IpNet) -> Option<IpAddr> {
    match subnet.trunc() {
        IpNet::V4(network) => u32::from(network.network())
            .checked_add(1)
            .map(Ipv4Addr::from)
            .map(IpAddr::V4),
        IpNet::V6(network) => u128::from(network.network())
            .checked_add(1)
            .map(Ipv6Addr::from)
            .map(IpAddr::V6),
    }
}

/// Derives the `fdcc::/16` management address from the first 14 public-key bytes.
pub fn management_ip(public_key: &Secret) -> Result<Ipv6Addr, NetworkError> {
    let key = public_key.as_bytes();
    let first_fourteen = key.get(..14).ok_or_else(|| {
        NetworkError::Invalid("derive management IP: public key is shorter than 14 bytes".into())
    })?;
    let mut bytes = [0_u8; 16];
    bytes[..2].copy_from_slice(&[0xfd, 0xcc]);
    bytes[2..].copy_from_slice(first_fourteen);
    Ok(Ipv6Addr::from(bytes))
}

pub(crate) fn single_ip_prefix(address: IpAddr) -> Result<IpNet, NetworkError> {
    IpNet::new(address, if address.is_ipv4() { 32 } else { 128 })
        .map_err(|error| NetworkError::Invalid(format!("invalid IP address: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_ip_is_first_address_after_masked_network() {
        let subnet: IpNet = "10.42.9.200/24".parse().expect("valid fixture");
        assert_eq!(
            machine_ip(subnet),
            Some("10.42.9.1".parse::<IpAddr>().expect("valid fixture"))
        );
    }

    #[test]
    fn machine_ip_maps_go_invalid_next_address_to_none() {
        let subnet: IpNet = "255.255.255.255/32".parse().expect("valid fixture");
        assert_eq!(machine_ip(subnet), None);
    }

    #[test]
    fn management_ip_uses_first_fourteen_key_bytes() {
        let key = Secret::from_hex_string(
            "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f",
        )
        .expect("valid fixture");
        assert_eq!(
            management_ip(&key).expect("valid key"),
            "fdcc:1:203:405:607:809:a0b:c0d"
                .parse::<Ipv6Addr>()
                .expect("valid fixture")
        );
    }
}
