use std::{
    io::Read,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    time::Duration,
};

use nix::{ifaddrs::getifaddrs, net::if_::InterfaceFlags};

use crate::{NetworkError, WIREGUARD_INTERFACE_NAME, io_error};

/// Lists routable unicast addresses, excluding Ployz, Docker, Tailscale, and unusable links.
pub fn list_routable_ips() -> Result<Vec<IpAddr>, NetworkError> {
    let interfaces = getifaddrs().map_err(|source| NetworkError::Interface {
        context: "list network interfaces",
        source,
    })?;
    let mut routable = Vec::new();

    for interface in interfaces {
        if should_skip_interface(&interface.interface_name)
            || !interface.flags.contains(InterfaceFlags::IFF_UP)
            || !interface.flags.contains(InterfaceFlags::IFF_RUNNING)
            || interface.flags.contains(InterfaceFlags::IFF_LOOPBACK)
        {
            continue;
        }
        let Some(address) = interface.address else {
            continue;
        };
        let ip = if let Some(address) = address.as_sockaddr_in() {
            IpAddr::V4(address.ip())
        } else if let Some(address) = address.as_sockaddr_in6() {
            IpAddr::V6(address.ip())
        } else {
            continue;
        };
        if is_go_global_unicast(ip) {
            routable.push(ip);
        }
    }
    Ok(routable)
}

fn should_skip_interface(name: &str) -> bool {
    name == WIREGUARD_INTERFACE_NAME
        || name.starts_with("docker")
        || name == "tailscale0"
        || contains_docker_bridge_name(name.as_bytes())
}

// The Go regexp is intentionally unanchored.
fn contains_docker_bridge_name(name: &[u8]) -> bool {
    name.windows(15).any(|window| {
        window.starts_with(b"br-")
            && window[3..]
                .iter()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    })
}

fn is_go_global_unicast(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => {
            !ip.is_unspecified()
                && !ip.is_loopback()
                && !ip.is_multicast()
                && ip != Ipv4Addr::BROADCAST
                && !ip.is_link_local()
        }
        IpAddr::V6(ip) => {
            !ip.is_unspecified()
                && !ip.is_loopback()
                && !ip.is_multicast()
                && !is_ipv6_link_local(ip)
        }
    }
}

fn is_ipv6_link_local(ip: Ipv6Addr) -> bool {
    (ip.segments()[0] & 0xffc0) == 0xfe80
}

/// Queries the same public-IP services, in the same order, as the Go package.
pub fn get_public_ip() -> Result<IpAddr, NetworkError> {
    let services = [
        "https://api.ipify.org",
        "https://ipinfo.io/ip",
        "http://ip-api.com/line/?fields=query",
    ];
    for service in services {
        if let Ok(ip) = query_ip(service) {
            return Ok(ip);
        }
    }
    Err(NetworkError::Invalid(
        "failed to get public IP from all services".into(),
    ))
}

fn query_ip(service: &str) -> Result<IpAddr, NetworkError> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(5)))
        .http_status_as_error(false)
        .build();
    let agent = ureq::Agent::new_with_config(config);
    let response = agent
        .get(service)
        .call()
        .map_err(|source| NetworkError::Http {
            context: "send request",
            source,
        })?;
    if response.status() != 200 {
        return Err(NetworkError::Invalid(format!(
            "unexpected status code: {}",
            response.status().as_u16()
        )));
    }
    let mut data = Vec::new();
    response
        .into_body()
        .into_reader()
        .read_to_end(&mut data)
        .map_err(|source| io_error("read response body", source))?;
    parse_plaintext_ip(&data)
}

fn parse_plaintext_ip(data: &[u8]) -> Result<IpAddr, NetworkError> {
    let text = std::str::from_utf8(data)
        .map_err(|error| NetworkError::Invalid(format!("invalid IP address: {error}")))?;
    text.parse()
        .map_err(|error| NetworkError::Invalid(format!("invalid IP address: {error}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_addresses_are_global_unicast_like_go() {
        assert!(is_go_global_unicast(IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3))));
    }

    #[test]
    fn docker_bridge_pattern_is_unanchored_like_go() {
        assert!(contains_docker_bridge_name(
            b"prefix-br-0123456789ab-suffix"
        ));
    }

    #[test]
    fn plaintext_parser_does_not_trim_service_output() {
        assert!(parse_plaintext_ip(b"203.0.113.10\n").is_err());
    }
}
