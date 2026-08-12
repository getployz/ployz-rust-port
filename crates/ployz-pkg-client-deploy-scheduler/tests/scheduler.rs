use std::collections::BTreeMap;
use std::error::Error as _;
use std::fmt;
use std::future::Future;
use std::task::{Context, Poll, Waker};

use ployz_internal_machine_api_pb as pb;
use ployz_pkg_api::{
    ApiError, ClientFuture, DockerVolume, Driver, GoMap, MachineClient, MachineFilter,
    MachineMembersList, MachineVolume, Placement, ServiceSpec, VOLUME_TYPE_VOLUME, VolumeClient,
    VolumeFilter, VolumeMount, VolumeOptions, VolumeSpec,
};
use ployz_pkg_client_deploy_scheduler::{
    ClusterState, Constraint, Machine, PlacementConstraint, ScheduleError, ServiceScheduler,
    VolumeScheduler, VolumesConstraint, inspect_cluster_state,
};

fn machine(id: &str) -> Machine {
    Machine {
        info: pb::MachineInfo {
            id: id.into(),
            name: format!("name-{id}"),
            ..pb::MachineInfo::default()
        },
        ..Machine::default()
    }
}

fn existing(name: &str) -> DockerVolume {
    DockerVolume {
        name: name.into(),
        ..DockerVolume::default()
    }
}

fn named_volume(name: &str) -> VolumeSpec {
    VolumeSpec {
        name: name.into(),
        kind: VOLUME_TYPE_VOLUME.into(),
        ..VolumeSpec::default()
    }
}

fn aliased_volume(alias: &str, actual: &str) -> VolumeSpec {
    VolumeSpec {
        name: alias.into(),
        kind: VOLUME_TYPE_VOLUME.into(),
        volume_options: Some(VolumeOptions {
            name: actual.into(),
            ..VolumeOptions::default()
        }),
        ..VolumeSpec::default()
    }
}

fn service(name: &str, mode: &str, placements: &[&str], volumes: Vec<VolumeSpec>) -> ServiceSpec {
    let mounts = volumes
        .iter()
        .map(|volume| VolumeMount {
            volume_name: volume.name.clone(),
            container_path: format!("/{}", volume.name),
            read_only: false,
        })
        .collect::<Vec<_>>();
    ServiceSpec {
        name: name.into(),
        mode: mode.into(),
        placement: Placement {
            machines: placements
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>()
                .into(),
        },
        container: ployz_pkg_api::ContainerSpec {
            image: "portainer/pause:latest".into(),
            volume_mounts: mounts.into(),
            ..ployz_pkg_api::ContainerSpec::default()
        },
        volumes: volumes.into(),
        ..ServiceSpec::default()
    }
}

fn scheduled_names(scheduled: &BTreeMap<String, Vec<VolumeSpec>>) -> BTreeMap<String, Vec<String>> {
    scheduled
        .iter()
        .map(|(machine, volumes)| {
            let mut names = volumes
                .iter()
                .map(|volume| volume.name.clone())
                .collect::<Vec<_>>();
            names.sort_unstable();
            (machine.clone(), names)
        })
        .collect()
}

#[test]
fn service_scheduler_applies_placement_and_volume_constraints_in_machine_order() {
    let mut first = machine("machine1");
    first.volumes.push(existing("data"));
    let mut second = machine("machine2");
    second.scheduled_volumes.push(named_volume("data"));
    let state = ClusterState {
        machines: vec![first, second, machine("machine3")],
    };
    let spec = service(
        "svc",
        "",
        &["name-machine1", "machine2"],
        vec![named_volume("data")],
    );

    let eligible = ServiceScheduler::new(&state, &spec)
        .eligible_machines()
        .expect("two machines satisfy the constraints");
    assert_eq!(
        eligible
            .iter()
            .map(|machine| machine.info.id.as_str())
            .collect::<Vec<_>>(),
        ["machine1", "machine2"]
    );
}

#[test]
fn service_scheduler_reports_no_match_and_preserves_unsupported_operation() {
    let state = ClusterState {
        machines: vec![machine("machine1")],
    };
    let spec = service("svc", "", &["missing"], Vec::new());
    let scheduler = ServiceScheduler::new(&state, &spec);
    assert!(matches!(
        scheduler.eligible_machines(),
        Err(ScheduleError::NoEligibleMachines)
    ));
    assert!(matches!(
        scheduler.schedule_container(),
        Err(ScheduleError::ContainerSchedulingUnsupported)
    ));
}

#[test]
fn constraint_descriptions_and_driver_matching_follow_the_oracle() {
    let mut placement = PlacementConstraint {
        machines: vec!["z".into(), "a".into()],
    };
    assert_eq!(
        placement.description(),
        "Placement constraint by machines: a, z"
    );
    assert_eq!(placement.machines, ["a", "z"]);

    let mut none = VolumesConstraint {
        volumes: vec![VolumeSpec::default()],
    };
    assert_eq!(none.description(), "No volumes constraint");

    let required = VolumeSpec {
        volume_options: Some(VolumeOptions {
            driver: Some(Driver {
                name: "local".into(),
                options: GoMap::default(),
            }),
            ..VolumeOptions::default()
        }),
        ..named_volume("data")
    };
    let constraint = VolumesConstraint {
        volumes: vec![required],
    };
    let mut target = machine("machine1");
    target.scheduled_volumes.push(named_volume("data"));
    assert!(constraint.evaluate(&target));
}

#[test]
fn empty_deployment_schedules_nothing() {
    let mut state = ClusterState::default();
    let mut scheduler = VolumeScheduler::new(&mut state, Vec::new()).expect("valid scheduler");
    assert!(scheduler.schedule().expect("empty schedule").is_empty());
}

#[test]
fn missing_shared_volumes_are_created_once_and_update_state() {
    let mut state = ClusterState {
        machines: vec![machine("machine2"), machine("machine1")],
    };
    let specs = vec![
        service(
            "service1",
            "",
            &[],
            vec![named_volume("vol1"), named_volume("vol2")],
        ),
        service(
            "service2",
            "",
            &[],
            vec![named_volume("vol2"), named_volume("vol3")],
        ),
        service(
            "service3",
            "",
            &[],
            vec![named_volume("vol3"), named_volume("vol4")],
        ),
    ];
    let scheduled = VolumeScheduler::new(&mut state, specs)
        .expect("valid scheduler")
        .schedule()
        .expect("volumes schedule");
    assert_eq!(
        scheduled_names(&scheduled),
        BTreeMap::from([(
            "machine1".into(),
            vec!["vol1".into(), "vol2".into(), "vol3".into(), "vol4".into()]
        )])
    );
    assert_eq!(
        state.machine("machine1").unwrap().scheduled_volumes.len(),
        4
    );
    assert!(
        state
            .machine("machine2")
            .unwrap()
            .scheduled_volumes
            .is_empty()
    );
}

#[test]
fn shared_volume_propagates_conflicting_placement_error() {
    let mut state = ClusterState {
        machines: vec![machine("machine1"), machine("machine2")],
    };
    let specs = vec![
        service("service1", "", &["machine1"], vec![named_volume("shared")]),
        service("service2", "", &["machine2"], vec![named_volume("shared")]),
    ];
    let error = VolumeScheduler::new(&mut state, specs)
        .expect("valid scheduler")
        .schedule()
        .expect_err("placements conflict");
    assert_eq!(
        error.to_string(),
        "unable to find a machine that satisfies placement constraints for services 'service1', 'service2' that must be placed together to share volume 'shared'"
    );
}

#[test]
fn existing_volume_constrains_service_and_prevents_duplicate_creation() {
    let mut first = machine("machine1");
    first.volumes.push(existing("data"));
    let mut state = ClusterState {
        machines: vec![first, machine("machine2")],
    };
    let scheduled = VolumeScheduler::new(
        &mut state,
        vec![service("service1", "", &[], vec![named_volume("data")])],
    )
    .expect("existing volume matches")
    .schedule()
    .expect("service uses existing volume");
    assert!(scheduled.is_empty());
}

#[test]
fn existing_volume_on_ineligible_machine_reports_exact_constraint_error() {
    let mut first = machine("machine1");
    first.volumes.push(existing("data"));
    let mut state = ClusterState {
        machines: vec![first, machine("machine2")],
    };
    let error = VolumeScheduler::new(
        &mut state,
        vec![service(
            "service1",
            "",
            &["machine2"],
            vec![named_volume("data")],
        )],
    )
    .expect("valid scheduler")
    .schedule()
    .expect_err("existing location conflicts");
    assert_eq!(
        error.to_string(),
        "unable to find a machine that satisfies service 'service1' placement constraints and has all required volumes: 'data'"
    );
}

#[test]
fn aliases_and_existing_volumes_drive_transitive_placement() {
    let mut second = machine("machine2");
    second.volumes.push(existing("vol2"));
    let mut third = machine("machine3");
    third.volumes.push(existing("vol1"));
    third.volumes.push(existing("vol2"));
    let mut state = ClusterState {
        machines: vec![machine("machine1"), second, third],
    };
    let specs = vec![
        service(
            "service1",
            "",
            &[],
            vec![named_volume("vol3"), named_volume("vol4")],
        ),
        service(
            "service2",
            "",
            &[],
            vec![
                named_volume("vol1"),
                named_volume("vol2"),
                named_volume("vol3"),
            ],
        ),
        service(
            "service3",
            "",
            &[],
            vec![named_volume("vol2"), aliased_volume("vol4-alias", "vol4")],
        ),
        service(
            "service4",
            "",
            &[],
            vec![aliased_volume("vol2-alias", "vol2"), named_volume("vol5")],
        ),
    ];
    let scheduled = VolumeScheduler::new(&mut state, specs)
        .expect("equivalent aliases canonicalise")
        .schedule()
        .expect("constraints converge");
    assert_eq!(
        scheduled_names(&scheduled),
        BTreeMap::from([
            ("machine2".into(), vec!["vol5".into()]),
            ("machine3".into(), vec!["vol3".into(), "vol4".into()]),
        ])
    );
}

#[test]
fn global_volume_schedules_all_eligible_missing_locations() {
    let mut first = machine("machine1");
    first.volumes.push(existing("data"));
    let mut state = ClusterState {
        machines: vec![first, machine("machine2"), machine("machine3")],
    };
    let scheduled = VolumeScheduler::new(
        &mut state,
        vec![service(
            "global-service",
            "global",
            &["machine1", "machine3"],
            vec![named_volume("data")],
        )],
    )
    .expect("valid global scheduler")
    .schedule()
    .expect("global volume schedules");
    assert_eq!(
        scheduled_names(&scheduled),
        BTreeMap::from([("machine3".into(), vec!["data".into()])])
    );
}

#[test]
fn global_services_union_their_placement_sets() {
    let mut state = ClusterState {
        machines: vec![
            machine("machine1"),
            machine("machine2"),
            machine("machine3"),
        ],
    };
    let specs = vec![
        service(
            "global1",
            "global",
            &["machine1", "machine2"],
            vec![named_volume("shared")],
        ),
        service(
            "global2",
            "global",
            &["machine2", "machine3"],
            vec![named_volume("shared")],
        ),
    ];
    let scheduled = VolumeScheduler::new(&mut state, specs)
        .expect("valid global schedulers")
        .schedule()
        .expect("union schedules");
    assert_eq!(scheduled.len(), 3);
    assert!(
        scheduled
            .values()
            .all(|volumes| volumes[0].name == "shared")
    );
}

#[test]
fn volume_cannot_be_shared_between_global_and_replicated_services() {
    let mut state = ClusterState {
        machines: vec![machine("machine1")],
    };
    let specs = vec![
        service("global", "global", &[], vec![named_volume("shared")]),
        service(
            "replicated",
            "replicated",
            &[],
            vec![named_volume("shared")],
        ),
    ];
    let error = VolumeScheduler::new(&mut state, specs)
        .expect("individual specs are valid")
        .schedule()
        .expect_err("mixed modes are invalid");
    assert!(matches!(
        error,
        ScheduleError::MixedGlobalAndReplicated(volume) if volume == "shared"
    ));
}

#[test]
fn constructor_rejects_invalid_duplicate_and_conflicting_inputs() {
    let mut state = ClusterState {
        machines: vec![machine("machine1")],
    };
    let duplicate = vec![
        service("same", "", &[], vec![named_volume("one")]),
        service("same", "", &[], vec![named_volume("two")]),
    ];
    assert!(matches!(
        VolumeScheduler::new(&mut state, duplicate),
        Err(ScheduleError::DuplicateServiceName(name)) if name == "same"
    ));

    let mut conflicting = named_volume("shared");
    conflicting.volume_options = Some(VolumeOptions {
        driver: Some(Driver {
            name: "other".into(),
            options: GoMap::default(),
        }),
        ..VolumeOptions::default()
    });
    let specs = vec![
        service("one", "", &[], vec![named_volume("shared")]),
        service("two", "", &[], vec![conflicting]),
    ];
    assert!(matches!(
        VolumeScheduler::new(&mut state, specs),
        Err(ScheduleError::ConflictingVolumeDefinitions(name)) if name == "shared"
    ));
}

#[test]
fn constructor_rejects_mismatched_existing_driver() {
    let mut target = machine("machine1");
    target.volumes.push(DockerVolume {
        name: "data".into(),
        driver: "other".into(),
        ..DockerVolume::default()
    });
    let mut state = ClusterState {
        machines: vec![target],
    };
    let spec = VolumeSpec {
        volume_options: Some(VolumeOptions {
            driver: Some(Driver {
                name: "local".into(),
                options: GoMap::default(),
            }),
            ..VolumeOptions::default()
        }),
        ..named_volume("data")
    };
    let error =
        match VolumeScheduler::new(&mut state, vec![service("service", "", &[], vec![spec])]) {
            Ok(_) => panic!("driver mismatch must fail"),
            Err(error) => error,
        };
    assert!(error.to_string().contains(
        "volume 'data' specification does not match the existing volume on machine 'name-machine1'"
    ));
}

#[derive(Clone)]
struct SnapshotClient {
    members: MachineMembersList,
    volumes: Vec<MachineVolume>,
    fail_machines: bool,
    fail_volumes: bool,
}

#[derive(Debug)]
struct SnapshotFailure(&'static str);

impl fmt::Display for SnapshotFailure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl std::error::Error for SnapshotFailure {}

impl MachineClient for SnapshotClient {
    fn inspect_machine<'a>(&'a self, _id: &'a str) -> ClientFuture<'a, pb::MachineMember> {
        Box::pin(async { unreachable!("not used by scheduler inspection") })
    }

    fn list_machines<'a>(
        &'a self,
        filter: Option<MachineFilter>,
    ) -> ClientFuture<'a, MachineMembersList> {
        assert!(filter.expect("available filter").available);
        if self.fail_machines {
            return Box::pin(async {
                Err(ApiError::operational(SnapshotFailure(
                    "machine query failed",
                )))
            });
        }
        let members = self.members.clone();
        Box::pin(async move { Ok(members) })
    }

    fn update_machine<'a>(
        &'a self,
        _name_or_id: &'a str,
        _request: pb::UpdateMachineRequest,
    ) -> ClientFuture<'a, pb::MachineInfo> {
        Box::pin(async { unreachable!("not used by scheduler inspection") })
    }

    fn rename_machine<'a>(
        &'a self,
        _name_or_id: &'a str,
        _new_name: &'a str,
    ) -> ClientFuture<'a, pb::MachineInfo> {
        Box::pin(async { unreachable!("not used by scheduler inspection") })
    }
}

impl VolumeClient for SnapshotClient {
    fn create_volume<'a>(
        &'a self,
        _machine_name_or_id: &'a str,
        _options: ployz_pkg_api::CreateVolumeOptions,
    ) -> ClientFuture<'a, MachineVolume> {
        Box::pin(async { unreachable!("not used by scheduler inspection") })
    }

    fn list_volumes<'a>(
        &'a self,
        filter: Option<VolumeFilter>,
    ) -> ClientFuture<'a, Vec<MachineVolume>> {
        assert!(filter.is_none());
        if self.fail_volumes {
            return Box::pin(async {
                Err(ApiError::cancelled(SnapshotFailure(
                    "volume query cancelled",
                )))
            });
        }
        let volumes = self.volumes.clone();
        Box::pin(async move { Ok(volumes) })
    }

    fn remove_volume<'a>(
        &'a self,
        _machine_name_or_id: &'a str,
        _volume_name: &'a str,
        _force: bool,
    ) -> ClientFuture<'a, ()> {
        Box::pin(async { unreachable!("not used by scheduler inspection") })
    }
}

fn block_on<F: Future>(future: F) -> F::Output {
    let mut context = Context::from_waker(Waker::noop());
    let mut future = Box::pin(future);
    loop {
        match future.as_mut().poll(&mut context) {
            Poll::Ready(output) => return output,
            Poll::Pending => std::thread::yield_now(),
        }
    }
}

#[test]
fn cluster_inspection_filters_available_machines_and_attaches_volumes_by_id() {
    let client = SnapshotClient {
        members: vec![
            pb::MachineMember {
                machine: Some(machine("machine1").info),
                ..pb::MachineMember::default()
            },
            pb::MachineMember {
                machine: Some(machine("machine2").info),
                ..pb::MachineMember::default()
            },
        ]
        .into(),
        volumes: vec![MachineVolume {
            machine_id: "machine2".into(),
            volume: existing("data"),
            ..MachineVolume::default()
        }],
        fail_machines: false,
        fail_volumes: false,
    };
    let state = block_on(inspect_cluster_state(&client)).expect("snapshot succeeds");
    assert!(state.machine("name-machine1").is_some());
    assert_eq!(state.machine_name("machine2"), Some("name-machine2"));
    assert!(state.machine("machine1").unwrap().volumes.is_empty());
    assert_eq!(state.machine("machine2").unwrap().volumes[0].name, "data");
}

#[test]
fn contextual_errors_retain_typed_client_and_validation_sources() {
    let client = SnapshotClient {
        members: MachineMembersList::default(),
        volumes: Vec::new(),
        fail_machines: true,
        fail_volumes: false,
    };
    let error = block_on(inspect_cluster_state(&client)).expect_err("machine query fails");
    assert_eq!(error.to_string(), "list machines: machine query failed");
    assert!(matches!(
        error
            .source()
            .and_then(|source| source.downcast_ref::<ApiError>()),
        Some(ApiError::Operational(_))
    ));

    let client = SnapshotClient {
        fail_machines: false,
        fail_volumes: true,
        ..client
    };
    let error = block_on(inspect_cluster_state(&client)).expect_err("volume query fails");
    assert_eq!(error.to_string(), "list volumes: volume query cancelled");
    assert!(matches!(
        error
            .source()
            .and_then(|source| source.downcast_ref::<ApiError>()),
        Some(ApiError::Cancelled(_))
    ));

    let mut state = ClusterState::default();
    let error = match VolumeScheduler::new(&mut state, vec![ServiceSpec::default()]) {
        Ok(_) => panic!("invalid spec must fail"),
        Err(error) => error,
    };
    assert!(error.to_string().starts_with("invalid service spec: "));
    assert!(matches!(
        error
            .source()
            .and_then(|source| source.downcast_ref::<ApiError>()),
        Some(ApiError::Invalid(_))
    ));
}
