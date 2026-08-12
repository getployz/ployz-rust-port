use std::net::IpAddr;

use ployz_pkg_api::*;

fn ingress(container_port: u16, protocol: &str) -> PortSpec {
    PortSpec {
        container_port,
        protocol: protocol.into(),
        mode: PORT_MODE_INGRESS.into(),
        ..PortSpec::default()
    }
}

#[test]
fn parses_and_formats_every_supported_port_shape() {
    let cases = [
        ("8080", "8080/tcp"),
        ("8000:8080", "8000:8080/tcp"),
        ("0:8080", "8080/tcp"),
        ("8080/tcp", "8080/tcp"),
        ("8080/udp", "8080/udp"),
        ("8000:8080/udp", "8000:8080/udp"),
        ("app.example.com:8080", "app.example.com:8080/https"),
        ("8080/http", "8080/http"),
        ("app.example.com:8080/http", "app.example.com:8080/http"),
        (
            "app.example.com:6443:8080",
            "app.example.com:6443:8080/https",
        ),
        (
            "app.example.com:8000:8080/http",
            "app.example.com:8000:8080/http",
        ),
        ("80:8080/udp@host", "80:8080/udp@host"),
        ("127.0.0.1:80:8080@host", "127.0.0.1:80:8080/tcp@host"),
        (
            "[2001:db8::1234:5678]:80:8080@host",
            "[2001:db8::1234:5678]:80:8080/tcp@host",
        ),
        (
            "192.168.76.0/24:80:8080/udp@host",
            "192.168.76.0/24:80:8080/udp@host",
        ),
        (
            "[2001:db8::]/64:80:8080/udp@host",
            "[2001:db8::]/64:80:8080/udp@host",
        ),
    ];
    for (input, output) in cases {
        let port = parse_port_spec(input).unwrap_or_else(|error| panic!("{input}: {error}"));
        assert_eq!(port.format().unwrap(), output, "{input}");
    }
}

#[test]
fn rejects_the_oracle_error_corpus() {
    let cases = [
        ("", "invalid container port"),
        ("/", "unsupported protocol"),
        ("@", "invalid mode"),
        ("invalid", "invalid container port"),
        ("53/", "unsupported protocol"),
        ("53@", "invalid mode"),
        ("0", "container port must be non-zero"),
        ("100500", "invalid container port"),
        ("/tcp", "invalid container port"),
        ("@host", "invalid container port"),
        ("8080@host@host", "too many '@' symbols"),
        ("8080@invalid", "invalid mode: 'invalid'"),
        ("8080/tcp/udp", "unsupported protocol: 'tcp/udp'"),
        ("8080/invalid", "unsupported protocol: 'invalid'"),
        ("app.example.com:invalid:8080", "invalid published port"),
        ("app:8080/http", "invalid hostname 'app'"),
        (
            "app.example.com:8080/tcp",
            "hostname is only valid with 'http' or 'https' protocols",
        ),
        ("8080@host", "published port is required in host mode"),
        ("300.0.0.1:80:8080@host", "invalid host IP"),
        ("[:::1]:80:8080@host", "invalid host IP"),
        ("[::1:80:8080@host", "invalid host IP"),
        ("2001:db8::1234:5678:80:8080@host", "invalid host IP"),
        (
            "80:8080/http@host",
            "unsupported protocol 'http' in host mode",
        ),
        (
            "app.example.com:8080@host",
            "hostname cannot be specified in host mode",
        ),
        ("192.168.76.0/45:53:5353/udp@host", "invalid host prefix"),
        (
            "192.168.76.0/24:53:5353/@host",
            "unsupported protocol '' in host mode",
        ),
    ];
    for (input, expected) in cases {
        let error = parse_port_spec(input).expect_err(input);
        assert!(error.to_string().contains(expected), "{input}: {error}");
    }
}

#[test]
fn validates_mode_specific_constraints() {
    let mut port = ingress(8080, PROTOCOL_TCP);
    assert!(port.validate().is_ok());
    port.host_ip = Some("127.0.0.1".parse::<IpAddr>().unwrap().into());
    assert!(port.validate().unwrap_err().to_string().contains("host IP"));

    let port = PortSpec {
        published_port: 80,
        container_port: 8080,
        protocol: PROTOCOL_HTTP.into(),
        mode: PORT_MODE_HOST.into(),
        ..PortSpec::default()
    };
    assert!(
        port.validate()
            .unwrap_err()
            .to_string()
            .contains("unsupported protocol")
    );
}

#[test]
fn port_set_equality_ignores_order_but_not_invalid_values() {
    let tcp = ingress(80, PROTOCOL_TCP);
    let udp = ingress(53, PROTOCOL_UDP);
    assert!(ports_equal(&[tcp.clone(), udp.clone()], &[udp, tcp]));
    assert!(!ports_equal(&[PortSpec::default()], &[PortSpec::default()]));
}
