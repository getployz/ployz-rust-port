use std::collections::BTreeMap;
use std::future::Future;
use std::net::{IpAddr, Ipv6Addr};
use std::pin::Pin;

use ployz_internal_machine_api_pb::{Ip, MachineInfo, Metadata, NetworkConfig};
use ployz_internal_machine_api_proxy::{
    Backend, CorrosionMapper, Director, MachineMapper, MachineStore, MachineTarget,
    MachinesNotFoundError, MapMachinesError, Mode, Route,
};
use tonic::Code;
use tonic::metadata::MetadataMap;

#[derive(Clone)]
struct TestStore {
    result: Result<Vec<MachineInfo>, String>,
}

impl MachineStore for TestStore {
    type Error = String;

    fn list_machines(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<MachineInfo>, Self::Error>> + Send + '_>> {
        let result = self.result.clone();
        Box::pin(async move { result })
    }
}

fn machine(id: &str, name: &str, address: Ipv6Addr) -> MachineInfo {
    MachineInfo {
        id: id.to_owned(),
        name: name.to_owned(),
        network: Some(NetworkConfig {
            management_ip: Some(Ip::from(IpAddr::V6(address))),
            ..NetworkConfig::default()
        }),
        ..MachineInfo::default()
    }
}

fn metadata(entries: &[(&str, &[&str])]) -> MetadataMap {
    let mut metadata = MetadataMap::new();
    for (key, values) in entries {
        for value in *values {
            metadata.append(
                key.to_owned()
                    .parse::<tonic::metadata::MetadataKey<tonic::metadata::Ascii>>()
                    .unwrap(),
                value.parse().unwrap(),
            );
        }
    }
    metadata
}

#[tokio::test]
async fn mapper_preserves_store_order_wildcards_deduplication_and_errors() {
    let machines = vec![
        machine("id-1", "machine-a", "fd00::1".parse().unwrap()),
        machine("id-2", "machine-b", "fd00::2".parse().unwrap()),
    ];
    let mapper = CorrosionMapper::new(TestStore {
        result: Ok(machines),
    });

    assert_eq!(
        mapper.map_machines(&["*".to_owned()]).await.unwrap(),
        vec![
            MachineTarget::new("id-1", "machine-a", "fd00::1"),
            MachineTarget::new("id-2", "machine-b", "fd00::2"),
        ]
    );
    assert_eq!(
        mapper
            .map_machines(&[
                "machine-a".to_owned(),
                "id-1".to_owned(),
                "machine-b".to_owned(),
            ])
            .await
            .unwrap(),
        vec![
            MachineTarget::new("id-1", "machine-a", "fd00::1"),
            MachineTarget::new("id-2", "machine-b", "fd00::2"),
        ]
    );
    assert_eq!(
        mapper
            .map_machines(&["missing".to_owned(), "also-missing".to_owned()])
            .await
            .unwrap_err()
            .to_string(),
        "machines not found: missing, also-missing"
    );
    assert_eq!(
        mapper.map_machines(&[]).await.unwrap_err().to_string(),
        "no machines specified"
    );
    assert_eq!(
        mapper
            .map_machines(&["missing".to_owned(), "*".to_owned()])
            .await
            .unwrap(),
        vec![
            MachineTarget::new("id-1", "machine-a", "fd00::1"),
            MachineTarget::new("id-2", "machine-b", "fd00::2"),
        ]
    );

    let empty = CorrosionMapper::new(TestStore { result: Ok(vec![]) });
    assert_eq!(
        empty
            .map_machines(&["*".to_owned()])
            .await
            .unwrap_err()
            .to_string(),
        "no machines in cluster"
    );
    let failed = CorrosionMapper::new(TestStore {
        result: Err("store down".to_owned()),
    });
    assert_eq!(
        failed
            .map_machines(&["*".to_owned()])
            .await
            .unwrap_err()
            .to_string(),
        "list machines: store down"
    );
    let invalid = CorrosionMapper::new(TestStore {
        result: Ok(vec![MachineInfo {
            id: "bad".to_owned(),
            name: "bad-ip".to_owned(),
            network: Some(NetworkConfig {
                management_ip: Some(Ip::default()),
                ..NetworkConfig::default()
            }),
            ..MachineInfo::default()
        }]),
    });
    assert_eq!(
        invalid
            .map_machines(&["*".to_owned()])
            .await
            .unwrap_err()
            .to_string(),
        "invalid management IP for machine 'bad-ip' in store: invalid IP"
    );
}

#[tokio::test]
async fn director_routes_metadata_and_caches_remote_backends() {
    let mapper = CorrosionMapper::new(TestStore {
        result: Ok(vec![
            machine("id-1", "local", "fd00::1".parse().unwrap()),
            machine("id-2", "remote", "fd00::2".parse().unwrap()),
        ]),
    });
    let director = Director::new("/tmp/machine.sock", 8080, mapper);
    director.update_local_address("fd00::1");

    let route = director.route(&MetadataMap::new()).await.unwrap();
    assert!(matches!(
        &route,
        Route {
            mode: Mode::OneToOne,
            ..
        }
    ));
    assert!(route.backends()[0].is_local());

    let proxied = metadata(&[("proxy-authority", &["origin"]), ("machines", &["remote"])]);
    assert!(director.route(&proxied).await.unwrap().backends()[0].is_local());

    let singular = metadata(&[("machine", &["remote"])]);
    let first = director.route(&singular).await.unwrap();
    let second = director.route(&singular).await.unwrap();
    assert_eq!(first.mode(), Mode::OneToOne);
    assert_eq!(first.backends()[0].target(), "[fd00::2]:8080");
    assert_eq!(first.backends()[0].target(), second.backends()[0].target());

    let plural = metadata(&[("machines", &["local", "remote"])]);
    let route = director.route(&plural).await.unwrap();
    assert_eq!(route.mode(), Mode::OneToMany);
    assert_eq!(route.backends().len(), 2);
    assert_eq!(route.backends()[0].machine().unwrap().id(), "id-1");
    assert_eq!(route.backends()[1].machine().unwrap().id(), "id-2");

    director.flush_remote_backends();
    let third = director.route(&singular).await.unwrap();
    assert_eq!(first.backends()[0].target(), third.backends()[0].target());
}

#[derive(Clone, Copy)]
enum MapperFailure {
    NotFound,
    Status,
    Other,
}

struct FailingMapper(MapperFailure);

impl MachineMapper for FailingMapper {
    fn map_machines<'a>(
        &'a self,
        _names_or_ids: &'a [String],
    ) -> Pin<Box<dyn Future<Output = Result<Vec<MachineTarget>, MapMachinesError>> + Send + 'a>>
    {
        let failure = self.0;
        Box::pin(async move {
            Err(match failure {
                MapperFailure::NotFound => {
                    MachinesNotFoundError::new(vec!["a".to_owned(), "b".to_owned()]).into()
                }
                MapperFailure::Status => tonic::Status::deadline_exceeded("timeout").into(),
                MapperFailure::Other => MapMachinesError::Other("something broke".to_owned()),
            })
        })
    }
}

#[tokio::test]
async fn director_maps_resolution_failures_to_the_oracle_status_contract() {
    let metadata = metadata(&[("machine", &["target"])]);
    let cases = [
        (
            MapperFailure::NotFound,
            Code::InvalidArgument,
            "machines not found: a, b",
        ),
        (MapperFailure::Status, Code::DeadlineExceeded, "timeout"),
        (
            MapperFailure::Other,
            Code::Internal,
            "failed to resolve machines: something broke",
        ),
    ];
    for (failure, code, message) in cases {
        let director = Director::new("/tmp/machine.sock", 8080, FailingMapper(failure));
        let status = director.route(&metadata).await.unwrap_err();
        assert_eq!(status.code(), code);
        assert_eq!(status.message(), message);
    }

    let ipv4 = Backend::remote("127.0.0.1", 8080).unwrap_err();
    assert_eq!(ipv4.code(), Code::Internal);
    assert_eq!(
        ipv4.message(),
        "address must be a valid IPv6 address: 127.0.0.1"
    );
}

#[tokio::test]
async fn director_preserves_status_codes_and_exact_validation_messages() {
    let mapper = CorrosionMapper::new(TestStore { result: Ok(vec![]) });
    let director = Director::new("/tmp/machine.sock", 8080, mapper);

    let cases = [
        (
            metadata(&[("machine", &["one", "two"])]),
            "proxy metadata 'machine' must have exactly one value",
        ),
        (
            metadata(&[("machine", &["m1"]), ("machines", &["m1"])]),
            "both 'machine' and 'machines' proxy metadata are set",
        ),
    ];

    for (metadata, message) in cases {
        let status = director.route(&metadata).await.unwrap_err();
        assert_eq!(status.code(), Code::InvalidArgument);
        assert_eq!(status.message(), message);
    }
}

#[test]
fn remote_metadata_rewrite_preserves_repeated_and_binary_values() {
    let backend = Backend::remote("fd00::2", 8080).unwrap();
    let mut incoming = MetadataMap::new();
    incoming.append("x-repeat", "one".parse().unwrap());
    incoming.append("x-repeat", "two".parse().unwrap());
    incoming.insert_bin(
        "trace-bin",
        tonic::metadata::BinaryMetadataValue::from_bytes(&[0, 1, 2]),
    );
    incoming.insert("machine", "target".parse().unwrap());
    incoming.insert("machines", "other".parse().unwrap());

    let outgoing = backend.outgoing_metadata(&incoming, Some("client.example"));
    assert_eq!(
        outgoing
            .get_all("x-repeat")
            .iter()
            .map(|v| v.to_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["one", "two"]
    );
    assert_eq!(
        outgoing.get_bin("trace-bin").unwrap().to_bytes().unwrap(),
        &[0, 1, 2][..]
    );
    assert_eq!(outgoing.get("proxy-authority").unwrap(), "client.example");
    assert!(outgoing.get("machine").is_none());
    assert!(outgoing.get("machines").is_none());

    let unknown = backend.outgoing_metadata(&MetadataMap::new(), None);
    assert_eq!(unknown.get("proxy-authority").unwrap(), "unknown");

    let expected = BTreeMap::from([
        ("machine_id", "id-2"),
        ("machine_name", "machine-b"),
        ("machine_addr", "fd00::2"),
    ]);
    let metadata = Metadata {
        machine_id: "id-2".into(),
        machine_name: "machine-b".into(),
        machine_addr: "fd00::2".into(),
        ..Metadata::default()
    };
    assert_eq!(
        BTreeMap::from([
            ("machine_id", metadata.machine_id.as_str()),
            ("machine_name", metadata.machine_name.as_str()),
            ("machine_addr", metadata.machine_addr.as_str()),
        ]),
        expected
    );
}
