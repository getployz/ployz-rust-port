use std::collections::{BTreeMap, BTreeSet};

use ployz_internal_machine_api_pb::FILE_DESCRIPTOR_SET;
use prost::Message;
use prost_types::{DescriptorProto, FileDescriptorSet};

struct FileExpectation {
    name: &'static str,
    messages: usize,
    service: Option<(&'static str, usize)>,
}

const API_FILES: &[FileExpectation] = &[
    FileExpectation {
        name: "internal/machine/api/pb/caddy.proto",
        messages: 1,
        service: Some(("Caddy", 1)),
    },
    FileExpectation {
        name: "internal/machine/api/pb/cluster.proto",
        messages: 10,
        service: Some(("Cluster", 7)),
    },
    FileExpectation {
        name: "internal/machine/api/pb/common.proto",
        messages: 8,
        service: None,
    },
    FileExpectation {
        name: "internal/machine/api/pb/docker.proto",
        messages: 36,
        service: Some(("Docker", 19)),
    },
    FileExpectation {
        name: "internal/machine/api/pb/machine.proto",
        messages: 18,
        service: Some(("Machine", 11)),
    },
    FileExpectation {
        name: "google/rpc/status.proto",
        messages: 1,
        service: None,
    },
];

#[test]
fn checked_descriptor_contains_the_frozen_schema_and_rpc_surface() {
    let descriptor = descriptor();
    let files = descriptor
        .file
        .iter()
        .filter_map(|file| file.name.as_deref().map(|name| (name, file)))
        .collect::<BTreeMap<_, _>>();

    for expected in API_FILES {
        let file = files
            .get(expected.name)
            .unwrap_or_else(|| panic!("missing {}", expected.name));
        assert_eq!(
            file.message_type.len(),
            expected.messages,
            "{}",
            expected.name
        );
        match expected.service {
            Some((service_name, method_count)) => {
                assert_eq!(file.service.len(), 1, "{}", expected.name);
                assert_eq!(file.service[0].name.as_deref(), Some(service_name));
                assert_eq!(
                    file.service[0].method.len(),
                    method_count,
                    "{}",
                    expected.name
                );
            }
            None => assert!(file.service.is_empty(), "{}", expected.name),
        }
    }

    let rpc_paths = API_FILES
        .iter()
        .filter_map(|expected| files.get(expected.name))
        .flat_map(|file| {
            let package = file.package.as_deref().expect("package");
            file.service.iter().flat_map(move |service| {
                let service_name = service.name.as_deref().expect("service name");
                service.method.iter().map(move |method| {
                    format!(
                        "/{package}.{service_name}/{}",
                        method.name.as_deref().expect("method name")
                    )
                })
            })
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(rpc_paths.len(), 38);
    assert!(rpc_paths.contains("/api.Machine/CheckPrerequisites"));
    assert!(rpc_paths.contains("/api.Cluster/CreateDomainRecords"));
    assert!(rpc_paths.contains("/api.Caddy/GetConfig"));
    assert!(rpc_paths.contains("/api.Docker/ExecContainer"));

    let streaming = API_FILES
        .iter()
        .filter_map(|expected| files.get(expected.name))
        .flat_map(|file| file.service.iter())
        .flat_map(|service| service.method.iter())
        .filter(|method| method.client_streaming() || method.server_streaming())
        .map(|method| {
            (
                method.name.as_deref().expect("method name"),
                method.client_streaming(),
                method.server_streaming(),
            )
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(
        streaming,
        BTreeSet::from([
            ("ContainerLogs", false, true),
            ("ExecContainer", true, true),
            ("MachineLogs", false, true),
            ("PullImage", false, true),
        ])
    );
}

#[test]
fn descriptor_preserves_presence_oneofs_maps_enums_and_well_known_types() {
    let descriptor = descriptor();
    let api_messages = descriptor
        .file
        .iter()
        .filter(|file| file.package.as_deref() == Some("api"))
        .flat_map(|file| file.message_type.iter())
        .filter_map(|message| message.name.as_deref().map(|name| (name, message)))
        .collect::<BTreeMap<_, _>>();

    let update = api_messages["UpdateMachineRequest"];
    let name = field(update, "name");
    assert!(name.proto3_optional());
    assert!(name.oneof_index.is_some());
    assert_eq!(
        field(update, "public_ip").type_name.as_deref(),
        Some(".api.IP")
    );

    let init = api_messages["InitClusterRequest"];
    assert_eq!(init.oneof_decl.len(), 1);
    assert_eq!(init.oneof_decl[0].name.as_deref(), Some("public_ip_config"));
    assert_eq!(field(init, "public_ip").oneof_index, Some(0));
    assert_eq!(field(init, "public_ip_auto").oneof_index, Some(0));

    let exec_request = api_messages["ExecContainerRequest"];
    assert_eq!(exec_request.oneof_decl.len(), 1);
    assert_eq!(field(exec_request, "config").oneof_index, Some(0));
    assert_eq!(field(exec_request, "stdin").oneof_index, Some(0));
    assert_eq!(field(exec_request, "resize").oneof_index, Some(0));

    let join = api_messages["JoinClusterRequest"];
    let map_type = field(join, "min_store_version")
        .type_name
        .as_deref()
        .expect("map entry type");
    assert!(map_type.ends_with(".MinStoreVersionEntry"));
    assert!(join.nested_type.iter().any(|entry| {
        entry
            .options
            .as_ref()
            .is_some_and(|options| options.map_entry())
    }));

    assert_eq!(
        field(api_messages["LogEntry"], "timestamp")
            .type_name
            .as_deref(),
        Some(".google.protobuf.Timestamp")
    );
    assert_eq!(
        field(api_messages["RTTStats"], "median")
            .type_name
            .as_deref(),
        Some(".google.protobuf.Duration")
    );
    assert_eq!(
        field(api_messages["Metadata"], "status")
            .type_name
            .as_deref(),
        Some(".google.rpc.Status")
    );

    let status_file = descriptor
        .file
        .iter()
        .find(|file| file.name.as_deref() == Some("google/rpc/status.proto"))
        .expect("status file");
    assert_eq!(status_file.package.as_deref(), Some("google.rpc"));
    assert_eq!(
        field(&status_file.message_type[0], "details")
            .type_name
            .as_deref(),
        Some(".google.protobuf.Any")
    );

    let dns = api_messages["DNSRecord"];
    assert_eq!(dns.enum_type[0].name.as_deref(), Some("RecordType"));
    let values = dns.enum_type[0]
        .value
        .iter()
        .map(|value| (value.name.as_deref().expect("enum name"), value.number()))
        .collect::<Vec<_>>();
    assert_eq!(values, [("UNSPECIFIED", 0), ("A", 1), ("AAAA", 2)]);
}

#[test]
fn generated_service_modules_expose_all_four_canonical_names() {
    use ployz_internal_machine_api_pb::{
        caddy_server, cluster_server, docker_server, machine_server,
    };

    assert_eq!(caddy_server::SERVICE_NAME, "api.Caddy");
    assert_eq!(cluster_server::SERVICE_NAME, "api.Cluster");
    assert_eq!(docker_server::SERVICE_NAME, "api.Docker");
    assert_eq!(machine_server::SERVICE_NAME, "api.Machine");
}

fn descriptor() -> FileDescriptorSet {
    FileDescriptorSet::decode(FILE_DESCRIPTOR_SET).expect("checked descriptor decodes")
}

fn field<'a>(message: &'a DescriptorProto, name: &str) -> &'a prost_types::FieldDescriptorProto {
    message
        .field
        .iter()
        .find(|field| field.name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("missing field {name}"))
}
