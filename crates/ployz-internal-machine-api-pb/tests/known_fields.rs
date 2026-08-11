use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Debug;
use std::fs;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use ployz_internal_machine_api_pb::exec_container_request;
use ployz_internal_machine_api_pb::exec_container_response;
use ployz_internal_machine_api_pb::google::rpc::Status as RpcStatus;
use ployz_internal_machine_api_pb::init_cluster_request;
use ployz_internal_machine_api_pb::*;
use prost::Message;
use prost_types::{Any, Duration, Timestamp};
use tonic::Code;

#[test]
fn go_known_field_fixtures_decode_and_round_trip_semantically() {
    let mut exchange = FixtureExchange::from_environment();
    exchange.assert_fixture(
        "common/ip-v4",
        &Ip {
            ip: vec![192, 0, 2, 1],
        },
    );
    exchange.assert_fixture(
        "common/ip-port-v6",
        &IpPort {
            ip: Some(Ip {
                ip: vec![0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1],
            }),
            port: 51_820,
        },
    );
    exchange.assert_fixture(
        "common/status-any",
        &Metadata {
            machine_id: "machine-a".into(),
            machine_name: "alpha".into(),
            machine_addr: "10.0.0.2:51000".into(),
            error: "upstream failed".into(),
            status: Some(RpcStatus {
                code: 5,
                message: "missing".into(),
                details: vec![Any {
                    type_url: "type.googleapis.com/example.Detail".into(),
                    value: vec![0x08, 0x2a],
                }],
            }),
        },
    );
    exchange.assert_fixture(
        "common/log-entry-enum-timestamp",
        &LogEntry {
            stream: 777,
            timestamp: Some(Timestamp {
                seconds: 1_700_000_000,
                nanos: 123_456_789,
            }),
            message: b"line\n".to_vec(),
        },
    );
    exchange.assert_fixture(
        "machine/update-optional-presence",
        &UpdateMachineRequest {
            name: Some(String::new()),
            public_ip: Some(Ip::default()),
            endpoints: vec![IpPort {
                ip: Some(Ip {
                    ip: vec![10, 0, 0, 10],
                }),
                port: 65_537,
            }],
        },
    );
    exchange.assert_fixture(
        "machine/init-oneof-false",
        &InitClusterRequest {
            machine_name: "alpha".into(),
            network: Some(IpPrefix {
                ip: Some(Ip {
                    ip: vec![10, 210, 0, 0],
                }),
                bits: 24,
            }),
            wireguard_endpoints: Vec::new(),
            wireguard_port: 51_820,
            wireguard_mtu: 1_420,
            public_ip_config: Some(init_cluster_request::PublicIpConfig::PublicIpAuto(false)),
        },
    );
    exchange.assert_fixture(
        "machine/join-map-negative",
        &JoinClusterRequest {
            machine: Some(MachineInfo {
                id: "machine-a".into(),
                name: "alpha".into(),
                ..Default::default()
            }),
            min_store_version: BTreeMap::from([("actor-b".into(), 42), ("actor-a".into(), -1)]),
            ..Default::default()
        },
    );
    exchange.assert_fixture(
        "machine/details-maps-duration",
        &MachineDetails {
            metadata: Some(Metadata {
                machine_id: "machine-a".into(),
                ..Default::default()
            }),
            machine: Some(MachineInfo {
                id: "machine-a".into(),
                ..Default::default()
            }),
            rtts: BTreeMap::from([(
                "machine-b".into(),
                RttStats {
                    median: Some(Duration {
                        seconds: 0,
                        nanos: 12_345_000,
                    }),
                    std_dev: Some(Duration {
                        seconds: 0,
                        nanos: 2_000_000,
                    }),
                },
            )]),
            store_version: BTreeMap::from([("actor-a".into(), 9)]),
        },
    );
    exchange.assert_fixture(
        "cluster/dns-unknown-enum",
        &CreateDomainRecordsRequest {
            records: vec![DnsRecord {
                name: "app.example.test".into(),
                r#type: 777,
                values: vec!["192.0.2.1".into(), "2001:db8::1".into()],
            }],
        },
    );
    exchange.assert_fixture(
        "caddy/config-timestamp",
        &GetCaddyConfigResponse {
            caddyfile: "example.test { respond ok }".into(),
            modified_at: Some(Timestamp {
                seconds: 1_700_000_001,
                nanos: 0,
            }),
        },
    );
    exchange.assert_fixture(
        "docker/exec-config-oneof",
        &ExecContainerRequest {
            payload: Some(exec_container_request::Payload::Config(ExecConfig {
                container_id: "container-a".into(),
                options: b"{}".to_vec(),
            })),
        },
    );
    exchange.assert_fixture(
        "docker/exec-stdin-oneof",
        &ExecContainerRequest {
            payload: Some(exec_container_request::Payload::Stdin(vec![0, 1, 2])),
        },
    );
    exchange.assert_fixture(
        "docker/exec-resize-oneof",
        &ExecContainerRequest {
            payload: Some(exec_container_request::Payload::Resize(ResizeEvent {
                height: 24,
                width: 80,
            })),
        },
    );
    exchange.assert_fixture(
        "docker/exec-response-empty-exit-code",
        &ExecContainerResponse {
            payload: Some(exec_container_response::Payload::ExitCode(0)),
        },
    );
    exchange.assert_fixture(
        "docker/service-container-enum",
        &CreateServiceContainerRequest {
            service_id: "service-a".into(),
            service_spec: b"{}".to_vec(),
            container_name: "service-a-1".into(),
            container_type: create_service_container_request::ContainerType::PreDeploy.into(),
        },
    );
    exchange.finish();
}

#[test]
fn accepted_unknown_field_limitation_is_executable() {
    let known = decode_hex("0a04c0000201");
    let mut with_unknowns = known.clone();
    // Unknown field 99, varint 42; then unknown group 100 containing field 1.
    with_unknowns.extend_from_slice(&[0x98, 0x06, 0x2a, 0xa3, 0x06, 0x08, 0x01, 0xa4, 0x06]);

    let decoded = Ip::decode(with_unknowns.as_slice()).expect("valid unknown fields decode");
    assert_eq!(decoded.encode_to_vec(), known);
}

#[test]
fn address_helpers_and_network_validation_match_the_oracle_contract() {
    let v4: IpAddr = Ipv4Addr::new(192, 0, 2, 7).into();
    let v6: IpAddr = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 7).into();
    assert_eq!(Ip::new(v4).to_addr(), Ok(v4.into()));
    assert_eq!(Ip::new(v6).to_addr(), Ok(v6.into()));
    assert_eq!(Ip::default().to_addr(), Err(AddressError::InvalidIp));
    assert_eq!(
        Ip { ip: vec![1, 2, 3] }.to_addr(),
        Err(AddressError::UnexpectedSliceSize)
    );

    let mut scoped_wire = match v6 {
        IpAddr::V6(address) => address.octets().to_vec(),
        IpAddr::V4(_) => unreachable!(),
    };
    scoped_wire.extend_from_slice(b"en0");
    let scoped = Ip {
        ip: scoped_wire.clone(),
    }
    .to_addr()
    .expect("scoped IPv6");
    assert_eq!(scoped.ip(), v6);
    assert_eq!(scoped.zone(), b"en0");
    assert_eq!(Ip::new(scoped).ip, scoped_wire);

    let address = SocketAddr::new(v4, 51_820);
    assert_eq!(IpPort::new(address).to_socket_addr(), Ok(address));
    assert_eq!(
        IpPort {
            ip: Some(Ip::new(v4)),
            port: 65_537,
        }
        .to_socket_addr(),
        Ok(SocketAddr::new(v4, 1))
    );

    assert_eq!(IpPrefix::new(v4, 24).to_prefix(), Ok((v4, 24)));
    assert_eq!(
        IpPrefix {
            ip: Some(Ip::new(v4)),
            bits: 33,
        }
        .to_prefix(),
        Err(AddressError::InvalidPrefix)
    );

    let valid = NetworkConfig {
        subnet: Some(IpPrefix::new(v4, 24)),
        management_ip: Some(Ip::new(v4)),
        endpoints: vec![IpPort::new(address)],
        public_key: vec![7; KEY_LEN],
    };
    assert!(valid.validate().is_ok());

    let missing_key = NetworkConfig::default()
        .validate()
        .expect_err("missing key");
    assert_eq!(missing_key.code(), Code::InvalidArgument);
    assert_eq!(missing_key.message(), "public key not set");

    let invalid_management_ip = NetworkConfig {
        management_ip: Some(Ip { ip: vec![1, 2, 3] }),
        public_key: vec![7; KEY_LEN],
        ..Default::default()
    }
    .validate()
    .expect_err("invalid management IP");
    assert_eq!(invalid_management_ip.code(), Code::InvalidArgument);
    assert_eq!(
        invalid_management_ip.message(),
        "invalid management IP: unmarshal IP: unexpected slice size"
    );

    let malformed_endpoint = NetworkConfig {
        endpoints: vec![IpPort { ip: None, port: 1 }],
        public_key: vec![7; KEY_LEN],
        ..Default::default()
    };
    assert!(std::panic::catch_unwind(|| malformed_endpoint.validate()).is_err());
}

struct FixtureExchange {
    inbound: BTreeMap<String, Vec<u8>>,
    seen: BTreeSet<String>,
    outbound: Vec<String>,
    output_path: Option<String>,
}

impl FixtureExchange {
    fn from_environment() -> Self {
        let inbound = std::env::var("PLOYZ_GO_FIXTURES_IN")
            .ok()
            .map(|path| parse_fixture_file(&path))
            .unwrap_or_default();
        Self {
            inbound,
            seen: BTreeSet::new(),
            outbound: Vec::new(),
            output_path: std::env::var("PLOYZ_RUST_FIXTURES_OUT").ok(),
        }
    }

    fn assert_fixture<M>(&mut self, name: &str, message: &M)
    where
        M: Message + Default + PartialEq + Debug,
    {
        assert!(
            self.seen.insert(name.to_owned()),
            "duplicate fixture {name}"
        );
        if !self.inbound.is_empty() {
            let encoded = self
                .inbound
                .get(name)
                .unwrap_or_else(|| panic!("Go did not emit fixture {name}"));
            let decoded = M::decode(encoded.as_slice()).expect("Go fixture must decode in Rust");
            assert_eq!(&decoded, message, "Go fixture {name}");
        }

        let encoded = message.encode_to_vec();
        let round_trip = M::decode(encoded.as_slice()).expect("Rust encoding must decode");
        assert_eq!(&round_trip, message, "Rust fixture {name}");
        self.outbound
            .push(format!("{name}\t{}", encode_hex(&encoded)));
    }

    fn finish(self) {
        if !self.inbound.is_empty() {
            let inbound_names = self.inbound.keys().cloned().collect::<BTreeSet<_>>();
            assert_eq!(inbound_names, self.seen, "Go/Rust fixture names differ");
        }
        if let Some(path) = self.output_path {
            let mut output = self.outbound.join("\n");
            output.push('\n');
            fs::write(path, output).expect("write Rust fixture exchange");
        }
    }
}

fn parse_fixture_file(path: &str) -> BTreeMap<String, Vec<u8>> {
    fs::read_to_string(path)
        .expect("read fixture exchange")
        .lines()
        .map(|line| {
            let (name, hex) = line.split_once('\t').expect("name and hex columns");
            (name.to_owned(), decode_hex(hex))
        })
        .collect()
}

fn encode_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn decode_hex(input: &str) -> Vec<u8> {
    assert_eq!(input.len() % 2, 0);
    input
        .as_bytes()
        .chunks_exact(2)
        .map(|pair| {
            let text = std::str::from_utf8(pair).expect("hex is ASCII");
            u8::from_str_radix(text, 16).expect("valid hex")
        })
        .collect()
}
