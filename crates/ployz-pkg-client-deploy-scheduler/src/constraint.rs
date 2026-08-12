use ployz_pkg_api::{Driver, VOLUME_DRIVER_LOCAL, VOLUME_TYPE_VOLUME, VolumeSpec};

use crate::Machine;

/// A predicate used to determine whether a machine can host a service.
pub trait Constraint: std::fmt::Debug {
    fn evaluate(&self, machine: &Machine) -> bool;
    fn description(&mut self) -> String;
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PlacementConstraint {
    pub machines: Vec<String>,
}

impl Constraint for PlacementConstraint {
    fn evaluate(&self, machine: &Machine) -> bool {
        self.machines
            .iter()
            .any(|candidate| candidate == &machine.info.id || candidate == &machine.info.name)
    }

    fn description(&mut self) -> String {
        // slices.Sort mutates the exported Go field as a side effect.
        self.machines.sort_unstable();
        format!(
            "Placement constraint by machines: {}",
            self.machines.join(", ")
        )
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct VolumesConstraint {
    pub volumes: Vec<VolumeSpec>,
}

impl Constraint for VolumesConstraint {
    fn evaluate(&self, machine: &Machine) -> bool {
        self.volumes.iter().all(|required| {
            if required.kind != VOLUME_TYPE_VOLUME {
                return true;
            }

            if machine.volumes.iter().any(|existing| {
                required.docker_volume_name() == existing.name
                    && required.matches_docker_volume(existing)
            }) {
                return true;
            }

            machine.scheduled_volumes.iter().any(|scheduled| {
                if required.docker_volume_name() != scheduled.docker_volume_name() {
                    return false;
                }
                let Some(required_driver) = required
                    .volume_options
                    .as_ref()
                    .and_then(|options| options.driver.as_ref())
                else {
                    return true;
                };
                let scheduled = scheduled.with_defaults();
                let scheduled_driver = scheduled
                    .volume_options
                    .as_ref()
                    .and_then(|options| options.driver.as_ref());
                let default_driver = Driver {
                    name: VOLUME_DRIVER_LOCAL.into(),
                    options: Default::default(),
                };
                required_driver == scheduled_driver.unwrap_or(&default_driver)
            })
        })
    }

    fn description(&mut self) -> String {
        let mut names = self
            .volumes
            .iter()
            .filter(|volume| volume.kind == VOLUME_TYPE_VOLUME)
            .map(|volume| volume.docker_volume_name().to_owned())
            .collect::<Vec<_>>();
        names.sort_unstable();
        if names.is_empty() {
            "No volumes constraint".into()
        } else {
            format!("Volumes: {}", names.join(", "))
        }
    }
}

pub(crate) fn constraints_from_spec(spec: &ployz_pkg_api::ServiceSpec) -> Vec<Box<dyn Constraint>> {
    let mut constraints: Vec<Box<dyn Constraint>> = Vec::new();
    if !spec.placement.machines.is_empty() {
        constraints.push(Box::new(PlacementConstraint {
            machines: spec.placement.machines.iter().cloned().collect(),
        }));
    }

    let volumes = spec
        .container
        .volume_mounts
        .iter()
        .filter_map(|mount| spec.volume(&mount.volume_name))
        .filter(|volume| volume.kind == VOLUME_TYPE_VOLUME)
        .cloned()
        .collect::<Vec<_>>();
    if !volumes.is_empty() {
        constraints.push(Box::new(VolumesConstraint { volumes }));
    }
    constraints
}
