use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use ployz_internal_machine_constants::{EMBEDDED_REGISTRY_PORT, MACHINE_API_PORT};

#[test]
fn machine_api_port_matches_the_go_oracle() {
    let port: u16 = MACHINE_API_PORT;

    assert_eq!(port, 51_000);
    assert_eq!(port.to_string(), "51000");
}

#[test]
fn embedded_registry_port_matches_the_go_oracle() {
    let port: u16 = EMBEDDED_REGISTRY_PORT;

    assert_eq!(port, 51_500);
    assert_eq!(port.to_string(), "51500");
}

#[test]
fn ports_form_the_socket_addresses_used_by_direct_callers() {
    let management_address = SocketAddr::new(IpAddr::V6(Ipv6Addr::LOCALHOST), MACHINE_API_PORT);
    let registry_address = SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), EMBEDDED_REGISTRY_PORT);

    assert_eq!(management_address.to_string(), "[::1]:51000");
    assert_eq!(registry_address.to_string(), "127.0.0.1:51500");
}
