use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use ployz_pkg_api::{
    ApiError, SERVICE_MODE_GLOBAL, SERVICE_MODE_REPLICATED, ServiceSpec, VolumeSpec,
};

use crate::{ClusterState, ServiceScheduler};

#[derive(Clone, Debug)]
pub enum ScheduleError {
    InvalidServiceSpec(ApiError),
    DuplicateServiceName(String),
    ConflictingVolumeDefinitions(String),
    ExistingVolumeMismatch {
        volume: String,
        machine: String,
    },
    NoEligibleMachines,
    ServiceScheduling {
        service: String,
        source: Box<Self>,
    },
    ExistingVolumesConflict {
        service: String,
        volumes: Vec<String>,
    },
    SharedServicePlacement {
        services: Vec<String>,
        volume: String,
    },
    MixedGlobalAndReplicated(String),
    Internal(String),
    PropagationAfterScheduling {
        volume: String,
        machine: String,
        source: Box<Self>,
    },
    ContainerSchedulingUnsupported,
}

impl fmt::Display for ScheduleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidServiceSpec(error) => write!(formatter, "invalid service spec: {error}"),
            Self::DuplicateServiceName(name) => {
                write!(formatter, "duplicate service name: '{name}'")
            }
            Self::ConflictingVolumeDefinitions(name) => write!(
                formatter,
                "volume '{name}' is defined multiple times with different options"
            ),
            Self::ExistingVolumeMismatch { volume, machine } => write!(
                formatter,
                "volume '{volume}' specification does not match the existing volume on machine '{machine}'. Use a different volume name or adjust the volume options to match the existing volume. You can also remove the existing volume from the machine(s) with 'uc volume rm' (WARNING: the data will be lost) and run the deployment again to create a new volume with the correct specification"
            ),
            Self::NoEligibleMachines => {
                formatter.write_str("no machines available that satisfy all constraints")
            }
            Self::ServiceScheduling { service, source } => {
                write!(formatter, "schedule service '{service}': {source}")
            }
            Self::ExistingVolumesConflict { service, volumes } => write!(
                formatter,
                "unable to find a machine that satisfies service '{service}' placement constraints and has all required volumes: {}",
                volumes
                    .iter()
                    .map(|name| format!("'{name}'"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::SharedServicePlacement { services, volume } => write!(
                formatter,
                "unable to find a machine that satisfies placement constraints for services {} that must be placed together to share volume '{volume}'",
                services
                    .iter()
                    .map(|name| format!("'{name}'"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            Self::MixedGlobalAndReplicated(volume) => write!(
                formatter,
                "volume '{volume}' cannot be shared between global and replicated services: global services require the volume on all machines while replicated services require co-location with the volume"
            ),
            Self::Internal(message) => formatter.write_str(message),
            Self::PropagationAfterScheduling {
                volume,
                machine,
                source,
            } => write!(
                formatter,
                "unexpected error while propagating constraints after scheduling volume '{volume}' on machine '{machine}': {source}"
            ),
            Self::ContainerSchedulingUnsupported => formatter.write_str("not implemented"),
        }
    }
}

impl std::error::Error for ScheduleError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ServiceScheduling { source, .. }
            | Self::PropagationAfterScheduling { source, .. } => Some(source),
            Self::InvalidServiceSpec(source) => Some(source),
            _ => None,
        }
    }
}

/// Schedules missing named volumes while preserving service co-location.
pub struct VolumeScheduler<'a> {
    state: &'a mut ClusterState,
    service_specs: Vec<ServiceSpec>,
    volume_specs: BTreeMap<String, VolumeSpec>,
    volume_services: BTreeMap<String, Vec<String>>,
    existing_volume_machines: BTreeMap<String, BTreeSet<String>>,
}

impl<'a> VolumeScheduler<'a> {
    pub fn new(
        state: &'a mut ClusterState,
        specs: Vec<ServiceSpec>,
    ) -> Result<Self, ScheduleError> {
        let mut specs_with_volumes = Vec::new();
        let mut volume_specs: BTreeMap<String, VolumeSpec> = BTreeMap::new();
        let mut volume_services: BTreeMap<String, Vec<String>> = BTreeMap::new();
        let mut service_names = BTreeSet::new();

        for spec in specs {
            spec.validate().map_err(ScheduleError::InvalidServiceSpec)?;
            if !service_names.insert(spec.name.clone()) {
                return Err(ScheduleError::DuplicateServiceName(spec.name));
            }

            let mounted = spec.mounted_docker_volumes();
            if mounted.is_empty() {
                continue;
            }
            for mut volume in mounted {
                volume = volume.with_defaults();
                volume.name = volume.docker_volume_name().to_owned();
                if let Some(seen) = volume_specs.get(&volume.name) {
                    if !seen.equivalent(&volume) {
                        return Err(ScheduleError::ConflictingVolumeDefinitions(volume.name));
                    }
                } else {
                    volume_specs.insert(volume.name.clone(), volume.clone());
                }
                volume_services
                    .entry(volume.name.clone())
                    .or_default()
                    .push(spec.name.clone());
            }
            specs_with_volumes.push(spec);
        }

        let mut existing_volume_machines: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for machine in &state.machines {
            for existing in &machine.volumes {
                if let Some(spec) = volume_specs.get(&existing.name) {
                    if !spec.matches_docker_volume(existing) {
                        return Err(ScheduleError::ExistingVolumeMismatch {
                            volume: existing.name.clone(),
                            machine: machine.info.name.clone(),
                        });
                    }
                    existing_volume_machines
                        .entry(existing.name.clone())
                        .or_default()
                        .insert(machine.info.id.clone());
                }
            }
        }

        Ok(Self {
            state,
            service_specs: specs_with_volumes,
            volume_specs,
            volume_services,
            existing_volume_machines,
        })
    }

    pub fn schedule(&mut self) -> Result<BTreeMap<String, Vec<VolumeSpec>>, ScheduleError> {
        if self.service_specs.is_empty() {
            return Ok(BTreeMap::new());
        }

        let mut eligible: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for spec in &self.service_specs {
            let mut without_mounts = spec.clone();
            without_mounts.container.volume_mounts = Vec::new().into();
            let machines = ServiceScheduler::new(self.state, &without_mounts)
                .eligible_machines()
                .map_err(|source| ScheduleError::ServiceScheduling {
                    service: spec.name.clone(),
                    source: Box::new(source),
                })?;
            eligible.insert(
                spec.name.clone(),
                machines
                    .iter()
                    .map(|machine| machine.info.id.clone())
                    .collect(),
            );
        }

        let mut quoted_service_volumes: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (volume_name, volume_machines) in &self.existing_volume_machines {
            if self.is_volume_for_global_service(volume_name) {
                continue;
            }
            for service_name in &self.volume_services[volume_name] {
                quoted_service_volumes
                    .entry(service_name.clone())
                    .or_default()
                    .push(volume_name.clone());
                let narrowed = eligible[service_name]
                    .intersection(volume_machines)
                    .cloned()
                    .collect::<BTreeSet<_>>();
                if narrowed.is_empty() {
                    return Err(ScheduleError::ExistingVolumesConflict {
                        service: service_name.clone(),
                        volumes: quoted_service_volumes[service_name].clone(),
                    });
                }
                eligible.insert(service_name.clone(), narrowed);
            }
        }

        let mut placed = self
            .existing_volume_machines
            .keys()
            .filter(|volume| !self.is_volume_for_global_service(volume))
            .cloned()
            .collect::<BTreeSet<_>>();
        for volume in self.volume_specs.keys() {
            if self.is_volume_shared_between_global_and_replicated(volume) {
                return Err(ScheduleError::MixedGlobalAndReplicated(volume.clone()));
            }
            if self.is_volume_for_global_service(volume) {
                placed.insert(volume.clone());
            }
        }

        self.propagate_constraints(&mut eligible, &placed)?;

        let mut scheduled: BTreeMap<String, Vec<VolumeSpec>> = BTreeMap::new();
        for (volume_name, volume_spec) in &self.volume_specs {
            let service_names = &self.volume_services[volume_name];
            if service_names.is_empty() {
                return Err(ScheduleError::Internal(format!(
                    "bug detected: no services using volume '{volume_name}'"
                )));
            }
            let global = self.is_volume_for_global_service(volume_name);
            let eligible_machines = if global {
                service_names
                    .iter()
                    .flat_map(|service| eligible[service].iter().cloned())
                    .collect::<BTreeSet<_>>()
            } else {
                eligible[&service_names[0]].clone()
            };
            if eligible_machines.is_empty() {
                return Err(ScheduleError::Internal(format!(
                    "bug detected: no eligible machines for volume '{volume_name}'"
                )));
            }

            if global {
                let existing = self.existing_volume_machines.get(volume_name);
                for machine in eligible_machines {
                    if existing.is_some_and(|machines| machines.contains(&machine)) {
                        continue;
                    }
                    scheduled
                        .entry(machine)
                        .or_default()
                        .push(volume_spec.clone());
                }
                placed.insert(volume_name.clone());
            } else {
                if self
                    .existing_volume_machines
                    .get(volume_name)
                    .is_some_and(|machines| !machines.is_empty())
                {
                    continue;
                }
                let machine = eligible_machines
                    .first()
                    .expect("non-empty set was checked")
                    .clone();
                for service in service_names {
                    eligible.insert(service.clone(), BTreeSet::from([machine.clone()]));
                }
                placed.insert(volume_name.clone());
                scheduled
                    .entry(machine.clone())
                    .or_default()
                    .push(volume_spec.clone());
                self.propagate_constraints(&mut eligible, &placed)
                    .map_err(|source| ScheduleError::PropagationAfterScheduling {
                        volume: volume_name.clone(),
                        machine,
                        source: Box::new(source),
                    })?;
            }
        }

        for (machine_id, volumes) in &scheduled {
            if let Some(machine) = self
                .state
                .machines
                .iter_mut()
                .find(|machine| &machine.info.id == machine_id)
            {
                machine.scheduled_volumes.extend(volumes.iter().cloned());
            }
        }
        Ok(scheduled)
    }

    fn propagate_constraints(
        &self,
        eligible: &mut BTreeMap<String, BTreeSet<String>>,
        skipped_volumes: &BTreeSet<String>,
    ) -> Result<(), ScheduleError> {
        loop {
            let mut changed = false;
            for (volume, services) in &self.volume_services {
                if skipped_volumes.contains(volume) || services.is_empty() {
                    continue;
                }
                let intersection = services
                    .iter()
                    .map(|service| eligible[service].clone())
                    .reduce(|left, right| left.intersection(&right).cloned().collect())
                    .expect("non-empty services were checked");
                if intersection.is_empty() {
                    return Err(ScheduleError::SharedServicePlacement {
                        services: services.clone(),
                        volume: volume.clone(),
                    });
                }
                for service in services {
                    if eligible[service].len() != intersection.len() {
                        changed = true;
                    }
                    eligible.insert(service.clone(), intersection.clone());
                }
            }
            if !changed {
                return Ok(());
            }
        }
    }

    fn is_volume_for_global_service(&self, volume: &str) -> bool {
        self.volume_services[volume].iter().any(|service| {
            self.service_specs
                .iter()
                .any(|spec| spec.name == *service && spec.mode == SERVICE_MODE_GLOBAL)
        })
    }

    fn is_volume_shared_between_global_and_replicated(&self, volume: &str) -> bool {
        let mut global = false;
        let mut replicated = false;
        for service in &self.volume_services[volume] {
            for spec in &self.service_specs {
                if spec.name == *service {
                    if spec.mode == SERVICE_MODE_GLOBAL {
                        global = true;
                    } else if spec.mode.is_empty() || spec.mode == SERVICE_MODE_REPLICATED {
                        replicated = true;
                    } else {
                        // Validation rejects other modes before this point.
                        replicated = true;
                    }
                }
            }
        }
        global && replicated
    }
}
