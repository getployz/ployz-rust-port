use std::collections::BTreeMap;
use std::error::Error;
use std::net::{IpAddr, SocketAddr};

use ployz_internal_machine_api_pb as pb;
use ployz_pkg_api::*;

#[test]
fn validates_config_ids_paths_duplicates_and_references() {
    let config = ConfigSpec {
        name: "app".into(),
        content: b"debug=true".to_vec(),
    };
    let mount = ConfigMount {
        config_name: "app".into(),
        container_path: "/etc/app".into(),
        uid: "1000".into(),
        gid: "0".into(),
        mode: Some(0o644),
    };
    assert_eq!(mount.numeric_uid().unwrap(), Some(1000));
    assert_eq!(mount.numeric_gid().unwrap(), Some(0));
    assert!(validate_configs_and_mounts(std::slice::from_ref(&config), &[mount]).is_ok());

    let error = validate_configs_and_mounts(&[config.clone(), config], &[]).unwrap_err();
    assert!(error.to_string().contains("duplicate config name: 'app'"));
    let error = ConfigMount {
        config_name: "app".into(),
        container_path: "relative".into(),
        ..ConfigMount::default()
    }
    .validate()
    .unwrap_err();
    assert_eq!(error.to_string(), "container path must be absolute");
    assert!(
        ConfigMount {
            uid: u64::MAX.to_string(),
            ..ConfigMount::default()
        }
        .numeric_uid()
        .unwrap_err()
        .to_string()
        .contains("value too high")
    );
}

#[test]
fn volume_defaults_matching_and_filters_match_the_oracle() {
    let spec = VolumeSpec {
        name: "data".into(),
        kind: VOLUME_TYPE_VOLUME.into(),
        ..VolumeSpec::default()
    };
    let defaulted = spec.with_defaults();
    assert_eq!(defaulted.docker_volume_name(), "data");
    assert!(spec.equivalent(&defaulted));
    assert!(spec.matches_docker_volume(&DockerVolume {
        name: "data".into(),
        driver: "local".into(),
        options: BTreeMap::from([("foo".into(), "bar".into())]).into(),
        ..DockerVolume::default()
    }));

    let explicit = VolumeSpec {
        volume_options: Some(VolumeOptions {
            driver: Some(Driver {
                name: String::new(),
                options: BTreeMap::from([("foo".into(), "bar".into())]).into(),
            }),
            ..VolumeOptions::default()
        }),
        ..spec.clone()
    };
    assert!(explicit.matches_docker_volume(&DockerVolume {
        name: "data".into(),
        driver: "local".into(),
        options: BTreeMap::from([("foo".into(), "bar".into())]).into(),
        ..DockerVolume::default()
    }));

    let volume = MachineVolume {
        machine_id: "id-1".into(),
        machine_name: "node-1".into(),
        volume: DockerVolume {
            name: "data".into(),
            driver: "local".into(),
            labels: BTreeMap::from([("env".into(), "prod".into())]).into(),
            ..DockerVolume::default()
        },
    };
    assert!(volume.matches_filter(Some(&VolumeFilter {
        driver: "local".into(),
        labels: BTreeMap::from([("env".into(), "prod".into())]),
        machines: vec!["node-1".into()],
        names: vec!["data".into()],
    })));
}

#[test]
fn service_defaults_validation_lookup_and_endpoints() {
    let mut spec = ServiceSpec {
        name: "web".into(),
        container: ContainerSpec {
            image: "nginx:latest".into(),
            ..ContainerSpec::default()
        },
        ..ServiceSpec::default()
    };
    assert!(spec.validate().is_ok());
    let defaulted = spec.with_defaults();
    assert_eq!(defaulted.mode, SERVICE_MODE_REPLICATED);
    assert_eq!(defaulted.replicas, 1);
    assert_eq!(defaulted.container.pull_policy, PULL_POLICY_MISSING);
    assert_eq!(defaulted.container.log_driver.unwrap().name, "local");

    spec.caddy = Some(CaddySpec {
        config: " x ".into(),
    });
    spec.ports = vec![PortSpec {
        hostname: "web.example.com".into(),
        container_port: 80,
        protocol: PROTOCOL_HTTP.into(),
        mode: PORT_MODE_INGRESS.into(),
        ..PortSpec::default()
    }]
    .into();
    assert!(spec.validate().unwrap_err().to_string().contains("Caddy"));

    let container = service_container("abcdef0123456789", "web-1", "web.example.com:80:8080/http");
    let service = Service {
        containers: vec![MachineServiceContainer {
            machine_id: "machine-1".into(),
            machine_name: "node".into(),
            container,
        }],
        ..Service::default()
    };
    assert_eq!(
        service.find_container("abcdef").unwrap().machine_id,
        "machine-1"
    );
    assert_eq!(service.images(), vec!["nginx:latest"]);
    assert_eq!(service.endpoints(), vec!["http://web.example.com → :8080"]);
}

#[test]
fn container_health_labels_ports_and_json_are_preserved() {
    let json = br#"{
        "Id":"sha256:abcdef0123456789","Created":"2026-08-12T12:00:00.123456789Z","Name":"/web-1",
        "Config":{"Image":"nginx:latest","Labels":{"uncloud.service.id":"svc","uncloud.service.name":"web","uncloud.service.ports":"8080:80/tcp@host"}},
        "State":{"Running":true,"Paused":false,"Restarting":false,"StartedAt":"2026-08-12T12:00:00Z","FinishedAt":"0001-01-01T00:00:00Z"},
        "NetworkSettings":{"Networks":{"uncloud":{"IPAddress":"10.0.0.2"}}}
    }"#;
    let container = Container::from_json(json).unwrap();
    assert_eq!(container.name, "web-1");
    assert!(container.healthy());
    assert_eq!(
        container.uncloud_network_ip(),
        Some("10.0.0.2".parse().unwrap())
    );
    let service_container = ServiceContainer {
        container,
        service_spec: ServiceSpec {
            mode: SERVICE_MODE_GLOBAL.into(),
            ..ServiceSpec::default()
        },
    };
    assert_eq!(service_container.short_id(), "abcdef012345");
    assert_eq!(service_container.service_id(), "svc");
    assert_eq!(service_container.service_name(), "web");
    assert_eq!(service_container.service_mode(), SERVICE_MODE_GLOBAL);
    assert_eq!(service_container.service_ports().unwrap().len(), 1);
    assert_eq!(
        service_container
            .conflicting_service_ports(&[PortSpec {
                published_port: 8080,
                container_port: 81,
                protocol: PROTOCOL_TCP.into(),
                mode: PORT_MODE_HOST.into(),
                ..PortSpec::default()
            }])
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn machine_projection_and_pascal_case_json_match_go() {
    let member = pb::MachineMember {
        machine: Some(pb::MachineInfo {
            id: "abc123".into(),
            name: "vm-1".into(),
            hostname: "vm-1.example.com".into(),
            arch: "amd64".into(),
            os_pretty_name: "Ubuntu 24.04.4 LTS".into(),
            kernel_version: "6.8.0".into(),
            docker_version: "27.1.1".into(),
            daemon_version: "0.9.0".into(),
            public_ip: Some(pb::Ip::from("203.0.113.5".parse::<IpAddr>().unwrap())),
            network: Some(pb::NetworkConfig {
                subnet: Some(pb::IpPrefix::new("10.210.0.0".parse().unwrap(), 24)),
                management_ip: Some(pb::Ip::from("10.210.0.1".parse::<IpAddr>().unwrap())),
                endpoints: vec![pb::IpPort::from(
                    "203.0.113.5:51820".parse::<SocketAddr>().unwrap(),
                )],
                public_key: vec![1, 2, 3, 4],
            }),
        }),
        state: pb::machine_member::MembershipState::Up as i32,
    };
    let list = MachineMembersList::from(vec![member]);
    assert!(list.find_by_name_or_id("vm-1").is_some());
    let json = serde_json::to_string(&list.to_native()).unwrap();
    for expected in [
        "\"ID\"",
        "\"OSPrettyName\"",
        "\"PublicIP\":\"203.0.113.5\"",
        "\"Subnet\":\"10.210.0.0/24\"",
        "\"Endpoints\":[\"203.0.113.5:51820\"]",
        "\"PublicKey\":\"AQIDBA==\"",
    ] {
        assert!(json.contains(expected), "{expected}: {json}");
    }
}

#[test]
fn log_stream_proto_conversion_is_total() {
    for stream in [
        LogStreamType::Unknown,
        LogStreamType::Stdout,
        LogStreamType::Stderr,
        LogStreamType::Heartbeat,
    ] {
        let proto = pb::log_entry::StreamType::from(stream);
        assert_eq!(LogStreamType::from(proto), stream);
    }
    assert_eq!(
        LogStreamStalled.to_string(),
        "log stream stopped responding"
    );
}

#[test]
fn wire_json_preserves_go_bytes_nil_collections_and_signed_durations() {
    let config = ConfigSpec {
        name: "cfg".into(),
        content: b"hello".to_vec(),
    };
    let json = serde_json::to_value(&config).unwrap();
    assert_eq!(json["Content"], "aGVsbG8=");
    assert_eq!(
        serde_json::from_value::<ConfigSpec>(serde_json::json!({"Name":"cfg","Content":null}))
            .unwrap()
            .content,
        Vec::<u8>::new()
    );

    let nil = ServiceSpec::default();
    let nil_json = serde_json::to_value(&nil).unwrap();
    assert!(nil_json["Configs"].is_null());
    assert!(nil_json["Ports"].is_null());
    assert!(serde_json::to_value(nil.with_defaults()).unwrap()["Volumes"].is_null());
    let empty = ServiceSpec {
        configs: Vec::new().into(),
        ports: Vec::new().into(),
        ..ServiceSpec::default()
    };
    let empty_json = serde_json::to_value(&empty).unwrap();
    assert_eq!(empty_json["Configs"], serde_json::json!([]));
    assert_eq!(empty_json["Ports"], serde_json::json!([]));
    let decoded: ServiceSpec = serde_json::from_value(serde_json::json!({
        "Configs": null, "Ports": null, "Volumes": null,
        "Container": {"Image":"busybox", "StopGracePeriod":-1,
            "CapAdd":null,"Env":null,"Sysctls":null}
    }))
    .unwrap();
    assert_eq!(decoded.container.stop_grace_period, Some(-1));
    assert!(serde_json::to_value(decoded).unwrap()["Container"]["CapAdd"].is_null());
}

#[test]
fn docker_wire_models_preserve_caller_observable_data() {
    let raw = serde_json::json!({
        "Id":"abcdef012345", "Name":"/web", "Path":"/bin/sh", "Args":null,
        "State":{"Running":false,"OOMKilled":true,"ExitCode":137,
            "StartedAt":"2026-13-40T99:99:99Z","FinishedAt":"0001-01-01T00:00:00Z"},
        "Config":{"Image":"busybox","Cmd":["echo","ok"],"Env":["A=B"],"User":"1000","Labels":null},
        "HostConfig":{"Init":true,"Privileged":true,"NanoCPUs":1000000000,"Memory":1024,
            "MemoryReservation":512,"Binds":null,"Mounts":[],"PortBindings":null,
            "LogConfig":{"Type":"local","Config":null},"FutureHostField":{"x":1}},
        "Mounts":[{"Type":"volume","Name":"data","Source":"/var/lib/data","Destination":"/data","RW":true}],
        "NetworkSettings":{"Ports":null,"Networks":{"uncloud":{"IPAddress":"10.0.0.2","FutureEndpoint":7}}},
        "FutureInspectField":{"preserved":true}
    });
    let container: Container = serde_json::from_value(raw).unwrap();
    assert_eq!(container.name, "web");
    assert!(container.state.as_ref().unwrap().oom_killed);
    assert_eq!(container.config.as_ref().unwrap().cmd[0], "echo");
    assert_eq!(
        container.host_config.as_ref().unwrap().resources.nano_cpus,
        1_000_000_000
    );
    assert_eq!(container.mounts[0].destination, "/data");
    let encoded = serde_json::to_value(&container).unwrap();
    assert_eq!(encoded["FutureInspectField"]["preserved"], true);
    assert_eq!(encoded["HostConfig"]["FutureHostField"]["x"], 1);
    assert!(encoded["HostConfig"]["Binds"].is_null());
    assert!(
        container
            .human_state()
            .unwrap_err()
            .to_string()
            .contains("parse started time")
    );
    assert!(serde_json::from_str::<Container>("{}").is_err());

    let service: ServiceContainer = serde_json::from_value(serde_json::json!({
        "Id":"abc", "Name":"/svc", "ServiceSpec":{"Mode":"global"}
    }))
    .unwrap();
    assert_eq!(service.container.name, "svc");
    assert_eq!(service.service_spec.mode, "global");
    let service_json = serde_json::to_value(service).unwrap();
    assert_eq!(service_json["Name"], "svc");
    assert_eq!(service_json["ServiceSpec"]["Mode"], "global");
}

#[test]
fn exec_image_and_volume_wire_contracts_are_complete() {
    let exec = ExecOptions {
        command: vec!["echo".into()],
        attach_stdout: true,
        ..ExecOptions::default()
    };
    let exec_json = serde_json::to_value(&exec).unwrap();
    assert_eq!(exec_json["Command"], serde_json::json!(["echo"]));
    assert_eq!(exec_json["AttachStdout"], true);
    assert!(exec_json.get("Stdin").is_none());
    let _: ExecOptions = serde_json::from_value(exec_json).unwrap();

    let summary: DockerImageSummary = serde_json::from_value(serde_json::json!({
        "Id":"sha256:main","Manifests":[{"ID":"sha256:other","Kind":"image","Available":true,
            "Descriptor":{"mediaType":"application/vnd.oci.image.manifest.v1+json","digest":"sha256:other","size":12},
            "Size":{"Content":12,"Total":20},"ImageData":{"Platform":{"architecture":"arm64","os":"linux","variant":"v8"}}}]
    })).unwrap();
    assert_eq!(
        summary.manifests[0]
            .image_data
            .as_ref()
            .unwrap()
            .platform
            .architecture,
        "arm64"
    );

    let volume: DockerVolume = serde_json::from_value(serde_json::json!({
        "Name":"data","Driver":"local","CreatedAt":"2026-01-01T00:00:00Z","Mountpoint":"/var/lib/data",
        "Scope":"local","Options":null,"Labels":null,"Status":{"healthy":true},
        "UsageData":{"RefCount":2,"Size":4096},"ClusterVolume":{"ID":"cluster-id"}
    })).unwrap();
    let volume_json = serde_json::to_value(volume).unwrap();
    assert_eq!(volume_json["UsageData"]["Size"], 4096);
    assert_eq!(volume_json["ClusterVolume"]["ID"], "cluster-id");
}

#[test]
fn docker_reference_policy_and_operational_errors_are_preserved() {
    for image in ["busybox", "Foo/bar:Tag", "[fc00::1]:5000/repo:tag"] {
        assert!(
            ContainerSpec {
                image: image.into(),
                ..ContainerSpec::default()
            }
            .validate()
            .is_ok(),
            "{image}"
        );
    }
    let upper_digest = format!("busybox@sha256:{}", "A".repeat(64));
    assert!(
        ContainerSpec {
            image: upper_digest,
            ..ContainerSpec::default()
        }
        .validate()
        .is_err()
    );
    assert!(
        ContainerSpec {
            image: "busybox:täg".into(),
            ..ContainerSpec::default()
        }
        .validate()
        .is_err()
    );

    let error = ApiError::operational(std::io::Error::new(
        std::io::ErrorKind::ConnectionReset,
        "transport reset",
    ));
    assert_eq!(error.to_string(), "transport reset");
    assert_eq!(error.source().unwrap().to_string(), "transport reset");
}

#[test]
fn semantic_equality_equates_nil_and_empty_collections() {
    let nil_container = ContainerSpec {
        image: "busybox".into(),
        ..ContainerSpec::default()
    };
    let empty_container = ContainerSpec {
        image: "busybox".into(),
        cap_add: Vec::new().into(),
        cap_drop: Vec::new().into(),
        command: Vec::new().into(),
        entrypoint: Vec::new().into(),
        env: BTreeMap::new().into(),
        sysctls: BTreeMap::new().into(),
        volume_mounts: Vec::new().into(),
        config_mounts: Vec::new().into(),
        volumes: Vec::new().into(),
        ..ContainerSpec::default()
    };
    assert!(nil_container.equivalent(&empty_container));

    let nil_volume = VolumeSpec {
        name: "data".into(),
        kind: VOLUME_TYPE_VOLUME.into(),
        volume_options: Some(VolumeOptions {
            driver: Some(Driver {
                name: "local".into(),
                ..Driver::default()
            }),
            ..VolumeOptions::default()
        }),
        ..VolumeSpec::default()
    };
    let empty_volume = VolumeSpec {
        volume_options: Some(VolumeOptions {
            driver: Some(Driver {
                name: "local".into(),
                options: BTreeMap::new().into(),
            }),
            labels: BTreeMap::new().into(),
            ..VolumeOptions::default()
        }),
        ..nil_volume.clone()
    };
    assert!(nil_volume.equivalent(&empty_volume));
    let docker_nil = DockerVolume {
        name: "data".into(),
        driver: "local".into(),
        ..DockerVolume::default()
    };
    let docker_empty = DockerVolume {
        options: BTreeMap::new().into(),
        ..docker_nil.clone()
    };
    assert!(nil_volume.matches_docker_volume(&docker_nil));
    assert!(!nil_volume.matches_docker_volume(&docker_empty));
    assert!(!empty_volume.matches_docker_volume(&docker_nil));
    assert!(empty_volume.matches_docker_volume(&docker_empty));

    let nil_hook = PreDeployHook::default();
    let empty_hook = PreDeployHook {
        command: Vec::new().into(),
        env: BTreeMap::new().into(),
        ..PreDeployHook::default()
    };
    assert!(PreDeployHook::equivalent(
        Some(&nil_hook),
        Some(&empty_hook)
    ));
}

#[test]
fn docker_optional_json_fields_match_omitempty_contracts() {
    let zero_port = serde_json::to_value(PortSpec::default()).unwrap();
    assert_eq!(zero_port["HostIP"], "");
    assert_eq!(zero_port["HostPrefix"], "");
    let decoded: PortSpec = serde_json::from_value(zero_port).unwrap();
    assert!(decoded.host_ip.is_none() && decoded.host_prefix.is_none());
    let populated = PortSpec {
        host_ip: Some("127.0.0.1".parse().unwrap()),
        host_prefix: Some("192.0.2.0/24".parse().unwrap()),
        ..PortSpec::default()
    };
    let populated_json = serde_json::to_value(populated).unwrap();
    assert_eq!(populated_json["HostIP"], "127.0.0.1");
    assert_eq!(populated_json["HostPrefix"], "192.0.2.0/24");

    let image_json = serde_json::to_value(DockerImageSummary::default()).unwrap();
    for omitted in ["VirtualSize", "Descriptor", "Manifests"] {
        assert!(image_json.get(omitted).is_none(), "{omitted}: {image_json}");
    }
    let volume_json = serde_json::to_value(DockerVolume::default()).unwrap();
    for omitted in ["CreatedAt", "Status", "UsageData", "ClusterVolume"] {
        assert!(
            volume_json.get(omitted).is_none(),
            "{omitted}: {volume_json}"
        );
    }
    let volume_with_empty_status = DockerVolume {
        status: BTreeMap::new().into(),
        ..DockerVolume::default()
    };
    assert!(
        serde_json::to_value(volume_with_empty_status)
            .unwrap()
            .get("Status")
            .is_none()
    );
}

#[test]
fn zero_remote_image_and_log_timestamp_are_distinct_states() {
    let remote = MachineRemoteImage {
        metadata: Some(pb::Metadata {
            error: "registry unavailable".into(),
            ..pb::Metadata::default()
        }),
        ..MachineRemoteImage::default()
    };
    assert!(remote.image.reference.is_none());
    assert!(LogEntry::default().timestamp.is_none());
    assert_eq!(
        ServiceLogsOptions {
            tail: isize::MAX,
            ..ServiceLogsOptions::default()
        }
        .tail,
        isize::MAX
    );
}

#[test]
fn scoped_ipv6_host_port_round_trips() {
    for (zone, input) in [
        ("eth0", "[fe80::1%eth0]:8080:80/tcp@host"),
        ("zone%part", "[fe80::1%zone%part]:8080:80/tcp@host"),
    ] {
        let port = parse_port_spec(input).unwrap();
        assert_eq!(port.host_ip.as_ref().unwrap().zone, zone.as_bytes());
        assert_eq!(port.format().unwrap(), input);
        let json = serde_json::to_value(&port).unwrap();
        assert_eq!(json["HostIP"], format!("fe80::1%{zone}"));
        assert_eq!(serde_json::from_value::<PortSpec>(json).unwrap(), port);
    }
}

fn service_container(id: &str, name: &str, ports: &str) -> ServiceContainer {
    let mut container = Container::default();
    container.id = id.into();
    container.name = name.into();
    container.config = Some(ContainerConfig {
        image: "nginx:latest".into(),
        labels: BTreeMap::from([
            (LABEL_SERVICE_ID.into(), "svc".into()),
            (LABEL_SERVICE_NAME.into(), "web".into()),
            (LABEL_SERVICE_PORTS.into(), ports.into()),
        ])
        .into(),
        ..ContainerConfig::default()
    });
    ServiceContainer {
        container,
        service_spec: ServiceSpec::default(),
    }
}
